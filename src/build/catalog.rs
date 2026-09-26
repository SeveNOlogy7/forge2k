use super::{BuildConfig, ConfigInfo};
use std::path::PathBuf;

// ============================================================
// Configuration Listing
// ============================================================

impl BuildConfig {
    /// Find a bundled Dockerfile matching this configuration, if one exists.
    pub(crate) fn find_bundled_dockerfile(&self) -> Option<PathBuf> {
        let filename = self.dockerfile_name();
        for base in dockerfile_search_paths() {
            let path = base.join(&filename);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    pub(crate) fn dockerfile_name(&self) -> String {
        match self.method.as_str() {
            "toolchain" => {
                let cuda_part = if self.cuda != "none" {
                    format!("_cuda_{}", self.cuda)
                } else {
                    String::new()
                };
                format!(
                    "toolchain/{}_{}_{}{}_{}.Dockerfile",
                    self.version, self.mpi, self.cpu, cuda_part, self.variant
                )
            }
            _ => {
                // spack
                format!(
                    "spack/{}_{}_{}_{}.Dockerfile",
                    self.version, self.mpi, self.cpu, self.variant
                )
            }
        }
    }
}

/// Where a configuration's Dockerfile comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerfileSource {
    /// A Dockerfile bundled in the dockerfiles/ directory.
    Bundled,
    /// No bundled Dockerfile matches; one is synthesized at runtime.
    Synthesized,
}

/// Directories searched for bundled Dockerfiles (relative to the
/// executable first, then the working directory).
pub(crate) fn dockerfile_search_paths() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    vec![
        exe_dir.join("dockerfiles"),
        PathBuf::from("dockerfiles"),
        PathBuf::from("."),
    ]
}

/// Derive the base image from a config tuple (kept in sync with the
/// synthesized Dockerfile templates).
fn base_image_for(method: &str, cuda: &str) -> &'static str {
    match method {
        "toolchain" => {
            if cuda != "none" {
                "nvidia/cuda:12.2.0-devel-ubuntu22.04"
            } else {
                "ubuntu:22.04"
            }
        }
        _ => "ubuntu:24.04",
    }
}

/// Human-readable description for a scanned configuration.
fn describe(method: &str, version: &str, mpi: &str, cpu: &str, cuda: &str) -> String {
    let mut parts = vec![format!("CP2K v{} with {}", version, mpi)];
    if method == "toolchain" {
        parts.push("toolchain build".to_string());
    } else {
        parts.push("Spack build".to_string());
    }
    if cuda != "none" {
        parts.push(format!("CUDA {} GPU acceleration", cuda));
    } else if cpu != "generic" {
        parts.push(format!("optimized for {} CPUs", cpu));
    }
    parts.join(", ")
}

/// Parse a bundled Dockerfile filename into a ConfigInfo.
/// Accepted shapes: <version>_<mpi>_<cpu>_<variant>.Dockerfile and
/// <version>_<mpi>_<cpu>_cuda_<GPU>_<variant>.Dockerfile.
fn parse_dockerfile_name(method_dir: &str, name: &str) -> Option<ConfigInfo> {
    let stem = name.strip_suffix(".Dockerfile")?;
    let f: Vec<&str> = stem.split('_').collect();
    if f.len() < 4 {
        return None;
    }
    let variant = f[f.len() - 1].to_string();
    // Shapes: v_mpi_cpu_variant | v_mpi_cpu_cuda_GPU_variant | v_mpi_x86_64_variant
    let (version, mpi, cuda, cpu) = if f.len() == 6 && f[3] == "cuda" {
        (
            f[0].to_string(),
            f[1].to_string(),
            f[4].to_string(),
            f[2].to_string(),
        )
    } else if f.len() == 5 {
        // cpu targets like x86_64 contribute an extra underscore-separated field
        (
            f[0].to_string(),
            f[1].to_string(),
            "none".to_string(),
            format!("{}_{}", f[2], f[3]),
        )
    } else if f.len() == 4 {
        (
            f[0].to_string(),
            f[1].to_string(),
            "none".to_string(),
            f[2].to_string(),
        )
    } else {
        return None;
    };
    Some(ConfigInfo {
        method: method_dir.to_string(),
        description: describe(method_dir, &version, &mpi, &cpu, &cuda),
        base_image: base_image_for(method_dir, &cuda).to_string(),
        version,
        mpi,
        cpu,
        cuda,
        variant,
        source: DockerfileSource::Bundled,
    })
}

/// List all available build configurations by scanning the bundled
/// dockerfiles/ directory. Every bundled entry corresponds to an actual
/// Dockerfile on disk; well-known configurations without a bundled file
/// (2026.1, master) are included and marked Synthesized so they stay
/// selectable/buildable (a Dockerfile is generated at runtime and the
/// build log says so).
pub fn list_available_configs() -> Vec<ConfigInfo> {
    let mut configs: Vec<ConfigInfo> = Vec::new();

    for base in dockerfile_search_paths() {
        for method_dir in ["spack", "toolchain"] {
            let entries = match std::fs::read_dir(base.join(method_dir)) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let name = match entry.path().file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if let Some(cfg) = parse_dockerfile_name(method_dir, &name) {
                    configs.push(cfg);
                }
            }
        }
        if !configs.is_empty() {
            break;
        }
    }

    // Well-known advertised configurations with no bundled Dockerfile.
    let synthesized: &[(&str, &str, &str, &str, &str, &str)] = &[
        ("spack", "2026.1", "mpich", "x86_64", "none", "psmp"),
        ("spack", "2026.1", "mpich", "cascadelake", "none", "psmp"),
        ("spack", "2026.1", "openmpi", "cascadelake", "none", "psmp"),
        ("toolchain", "master", "mpich", "generic", "none", "psmp"),
    ];
    for (method, version, mpi, cpu, cuda, variant) in synthesized {
        let bundled = configs.iter().any(|c| {
            c.method == *method
                && c.version == *version
                && c.mpi == *mpi
                && c.cpu == *cpu
                && c.cuda == *cuda
                && c.variant == *variant
        });
        if !bundled {
            configs.push(ConfigInfo {
                method: method.to_string(),
                version: version.to_string(),
                mpi: mpi.to_string(),
                cpu: cpu.to_string(),
                cuda: cuda.to_string(),
                variant: variant.to_string(),
                base_image: base_image_for(method, cuda).to_string(),
                description: describe(method, version, mpi, cpu, cuda),
                source: DockerfileSource::Synthesized,
            });
        }
    }

    configs.sort_by(|a, b| {
        a.method
            .cmp(&b.method)
            .then(b.version.cmp(&a.version))
            .then(a.mpi.cmp(&b.mpi))
            .then(a.cpu.cmp(&b.cpu))
            .then(a.cuda.cmp(&b.cuda))
    });
    configs
}
