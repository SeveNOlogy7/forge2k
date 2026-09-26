mod catalog;
mod docker;
mod execute;
mod generate;
mod registry;

pub use catalog::{list_available_configs, DockerfileSource};
pub use docker::{check_docker, print_docker_install_guide, restart_docker, DockerStatus};
pub use execute::{cuda_options_for_method, execute_build, execute_native_build};
pub use registry::{
    check_registry, daemon_config_path, detect_best_mirror, get_registry_mirror,
    remove_registry_mirror, set_registry_mirror, NetworkStatus,
};

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

// Test-only imports kept here so that `use super::*;` inside `mod tests`
// resolves them through the parent scope, as in the pre-split build.rs.
#[cfg(test)]
use chrono::Local;
#[cfg(test)]
use std::process::{Command, Stdio};
#[cfg(test)]
use std::sync::mpsc;
#[cfg(test)]
use std::sync::{Arc, Mutex, PoisonError};
#[cfg(test)]
use std::time::Duration;

// Test-only internals of the submodules, re-exported so that
// `use super::*;` in `mod tests` resolves them unchanged.
#[cfg(test)]
pub(crate) use catalog::dockerfile_search_paths;
#[cfg(test)]
pub(crate) use execute::{run_cmd_logged, CmdStatus};
#[cfg(test)]
pub(crate) use generate::generate_dockerfile_content;

// ============================================================
// Build Outcome
// ============================================================

/// Terminal result of a build. Cancellation is an explicit outcome,
/// not an error; `Err` is reserved for infrastructure failures
/// (spawn errors, wait errors) that abort the build abnormally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    Success,
    Failed,
    Cancelled,
}

// ============================================================
// Build Configuration
// ============================================================

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub method: String,  // "spack" or "toolchain"
    pub version: String, // "2025.2", "2023.2"
    pub mpi: String,     // "mpich", "openmpi"
    pub cpu: String,     // "x86_64", "generic", "cascadelake"
    pub cuda: String,    // "none", "P100", "V100"
    pub variant: String, // "psmp", "ssmp", "pdbg", "sdbg"
    pub jobs: u32,
    pub tag: String,
    pub no_cache: bool,
    pub shm_size: String,
    pub dockerfile: Option<PathBuf>,
}

impl BuildConfig {
    /// Generate the default image tag based on config
    pub fn default_tag(&self) -> String {
        if !self.tag.is_empty() {
            return self.tag.clone();
        }
        let cuda_suffix = if self.cuda != "none" {
            format!("_cuda_{}", self.cuda)
        } else {
            String::new()
        };
        let ver = if self.version == "master" {
            format!("master_{}", chrono::Local::now().format("%Y%m%d"))
        } else {
            self.version.clone()
        };
        format!(
            "cp2k/cp2k:{}_{}_{}{}_{}",
            ver, self.mpi, self.cpu, cuda_suffix, self.variant
        )
    }

    /// Resolve the Dockerfile path: use custom if provided, else find bundled
    pub fn resolve_dockerfile(&self) -> Result<PathBuf> {
        if let Some(ref df) = self.dockerfile {
            if df.exists() {
                return Ok(df.clone());
            }
            return Err(anyhow!("Custom Dockerfile not found: {}", df.display()));
        }

        if let Some(path) = self.find_bundled_dockerfile() {
            return Ok(path);
        }

        // If no bundled file found, we'll generate one at runtime
        let generated = self.generate_dockerfile()?;
        Ok(generated)
    }

    /// Build the docker build command arguments
    pub fn build_docker_args(&self, dockerfile: &Path) -> Vec<String> {
        let mut args = vec!["build".to_string()];

        // Shared memory size (needed for OpenMPI with many ranks)
        args.push("--shm-size".to_string());
        args.push(self.shm_size.clone());

        // No cache
        if self.no_cache {
            args.push("--no-cache".to_string());
        }

        // Dockerfile
        args.push("-f".to_string());
        args.push(dockerfile.to_string_lossy().to_string());

        // Tag
        args.push("-t".to_string());
        args.push(self.default_tag());

        // Build args
        args.push("--build-arg".to_string());
        args.push(format!("NUM_PROCS={}", self.jobs));

        // Context (current directory where Dockerfile lives)
        args.push(
            dockerfile
                .parent()
                .unwrap_or(Path::new("."))
                .to_string_lossy()
                .to_string(),
        );

        args
    }
}

/// Severity of a log line. Replaces the former `is_error: bool`
/// (old `true` → [`LogLevel::Error`], old `false` → [`LogLevel::Info`]);
/// [`LogLevel::Warn`] is reserved for future producers — no existing
/// call site emits it, so the rendered severity split is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub timestamp: String,
    pub text: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone)]
pub struct ConfigInfo {
    pub method: String,
    pub version: String,
    pub mpi: String,
    pub cpu: String,
    pub cuda: String,
    pub variant: String,
    pub base_image: String,
    pub description: String,
    /// Whether the Dockerfile is bundled or synthesized at runtime.
    pub source: DockerfileSource,
}

// Tests moved to tests.rs (Wave 5 O-R1: keeps mod.rs under the 800-line budget).
#[cfg(test)]
mod tests;
