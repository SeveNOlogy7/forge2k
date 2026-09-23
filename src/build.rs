use anyhow::{anyhow, Context, Result};
use chrono::Local;
use colored::Colorize;
use std::io::{BufRead, BufReader};

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| anyhow!("Cannot chmod +x {}: {}", path.display(), e))
}
#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

    /// Find a bundled Dockerfile matching this configuration, if one exists.
    fn find_bundled_dockerfile(&self) -> Option<PathBuf> {
        let filename = self.dockerfile_name();
        for base in dockerfile_search_paths() {
            let path = base.join(&filename);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    fn dockerfile_name(&self) -> String {
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

    /// Generate a Dockerfile from embedded templates
    pub fn generate_dockerfile(&self) -> Result<PathBuf> {
        let content = generate_dockerfile_content(self)?;
        let tmp_dir = std::env::temp_dir().join("forge2k");
        std::fs::create_dir_all(&tmp_dir).context("Failed to create temp dir for Dockerfile")?;

        let filename = format!("{}.Dockerfile", self.default_tag().replace(['/', ':'], "_"));
        let path = tmp_dir.join(&filename);
        std::fs::write(&path, &content).context("Failed to write generated Dockerfile")?;
        Ok(path)
    }
}

// ============================================================
// Docker Engine Detection
// ============================================================

#[derive(Debug)]
pub enum DockerStatus {
    Installed { version: String, running: bool },
    NotInstalled,
    NotRunning,
}

pub fn check_docker() -> DockerStatus {
    // Check if docker binary exists
    let output = Command::new("docker").arg("version").output();
    match output {
        Ok(out) => {
            let version = String::from_utf8_lossy(&out.stdout).to_string();
            let running = version.contains("Server:");
            if running {
                // Extract version string
                let ver = version
                    .lines()
                    .find(|l| l.contains("Version"))
                    .unwrap_or("unknown")
                    .to_string();
                DockerStatus::Installed {
                    version: ver,
                    running: true,
                }
            } else {
                DockerStatus::NotRunning
            }
        }
        Err(_) => DockerStatus::NotInstalled,
    }
}

pub fn print_docker_install_guide() {
    println!(
        "{}",
        "╔══════════════════════════════════════════════════════════╗".yellow()
    );
    println!(
        "{}",
        "║       Docker Engine Not Found - Installation Guide      ║".yellow()
    );
    println!(
        "{}",
        "╚══════════════════════════════════════════════════════════╝".yellow()
    );
    println!();

    #[cfg(target_os = "windows")]
    {
        println!("{}", "Windows Installation:".cyan().bold());
        println!("  1. Download Docker Desktop from: https://docs.docker.com/desktop/setup/install/windows-install/");
        println!("  2. Run the installer (Docker Desktop Installer.exe)");
        println!("  3. Make sure 'Use WSL 2 instead of Hyper-V' is selected");
        println!("  4. Restart your computer after installation");
        println!("  5. Launch Docker Desktop from Start Menu");
        println!();
        println!(
            "{}",
            "WSL 2 Ubuntu Installation (alternative):".cyan().bold()
        );
        println!("  curl -fsSL https://get.docker.com -o get-docker.sh");
        println!("  sudo sh get-docker.sh");
        println!("  sudo usermod -aG docker $USER");
        println!("  newgrp docker");
    }

    #[cfg(target_os = "linux")]
    {
        if Path::new("/etc/wsl.conf").exists() || std::env::var("WSL_DISTRO_NAME").is_ok() {
            println!("{}", "WSL Ubuntu/Debian Installation:".cyan().bold());
            println!("  curl -fsSL https://get.docker.com -o get-docker.sh");
            println!("  sudo sh get-docker.sh");
            println!("  sudo usermod -aG docker $USER");
            println!("  newgrp docker");
            println!();
            println!(
                "{}",
                "Or with Docker Desktop for Windows (WSL 2 backend):"
                    .cyan()
                    .bold()
            );
            println!("  Install Docker Desktop on Windows, then enable WSL 2 integration");
            println!("  Settings → Resources → WSL Integration → Enable your distro");
        } else {
            println!("{}", "Linux Installation (Ubuntu/Debian):".cyan().bold());
            println!("  # Add Docker's official GPG key:");
            println!("  sudo apt-get update");
            println!("  sudo apt-get install ca-certificates curl");
            println!("  sudo install -m 0755 -d /etc/apt/keyrings");
            println!("  curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo tee /etc/apt/keyrings/docker.asc");
            println!();
            println!("  # Add the repository:");
            println!("  echo \"deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo \"$VERSION_CODENAME\") stable\" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null");
            println!();
            println!("  # Install Docker:");
            println!("  sudo apt-get update");
            println!("  sudo apt-get install docker-ce docker-ce-cli containerd.io");
            println!("  sudo usermod -aG docker $USER");
            println!("  newgrp docker");
        }
    }

    #[cfg(target_os = "macos")]
    {
        println!("{}", "macOS Installation:".cyan().bold());
        println!("  1. Download Docker Desktop from: https://docs.docker.com/desktop/setup/install/mac-install/");
        println!("  2. Drag Docker.app to Applications folder");
        println!("  3. Launch Docker Desktop from Applications");
    }
}

// ============================================================
// Docker Build Execution
// ============================================================

#[derive(Debug, Clone)]
pub struct LogLine {
    pub timestamp: String,
    pub text: String,
    pub is_error: bool,
}

/// Execute a docker build and send log lines through the channel
pub fn execute_build(
    config: &BuildConfig,
    log_tx: mpsc::Sender<LogLine>,
    cancel_flag: Arc<Mutex<bool>>,
) -> Result<BuildOutcome> {
    let dockerfile = config.resolve_dockerfile()?;

    let log = |text: &str, is_err: bool| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            is_error: is_err,
        });
    };

    log(
        &format!("🔨 Forge2K Build Engine v{}", env!("CARGO_PKG_VERSION")),
        false,
    );
    log(&format!("   Method:    {}", config.method), false);
    if config.dockerfile.is_none() && config.find_bundled_dockerfile().is_none() {
        log("ℹ️ No bundled Dockerfile matches this configuration; using a SYNTHESIZED Dockerfile generated at runtime.", false);
    }
    log(&format!("   Version:   {}", config.version), false);
    log(&format!("   MPI:       {}", config.mpi), false);
    log(&format!("   CPU:       {}", config.cpu), false);
    log(&format!("   CUDA:      {}", config.cuda), false);
    log(&format!("   Variant:   {}", config.variant), false);
    log(&format!("   Jobs:      {}", config.jobs), false);
    log(&format!("   Tag:       {}", config.default_tag()), false);
    log(&format!("   Dockerfile: {}", dockerfile.display()), false);
    log("", false);
    log("🚀 Starting build (this may take 1-3 hours)...", false);
    log("", false);

    let args = config.build_docker_args(&dockerfile);
    log(&format!("$ docker {}", args.join(" ")), false);
    log("", false);

    let mut child = Command::new("docker")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to launch docker build: {}", e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Read stdout in a thread
    let tx_stdout = log_tx.clone();
    let cancel_stdout = cancel_flag.clone();
    let stdout_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if *cancel_stdout.lock().unwrap() {
                break;
            }
            if let Ok(line) = line {
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx_stdout.send(LogLine {
                    timestamp: ts,
                    text: line,
                    is_error: false,
                });
            }
        }
    });

    // Read stderr in a thread (most Docker output goes to stderr)
    let tx_stderr = log_tx.clone();
    let cancel_stderr = cancel_flag.clone();
    let stderr_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if *cancel_stderr.lock().unwrap() {
                break;
            }
            if let Ok(line) = line {
                let ts = Local::now().format("%H:%M:%S").to_string();
                // Docker build outputs progress to stderr, which is not an error
                let is_err = line.to_lowercase().contains("error")
                    || line.to_lowercase().contains("failed")
                    || line.to_lowercase().contains("fatal");
                let _ = tx_stderr.send(LogLine {
                    timestamp: ts,
                    text: line,
                    is_error: is_err,
                });
            }
        }
    });

    // Wait for completion or cancellation
    loop {
        if *cancel_flag.lock().unwrap() {
            let _ = child.kill();
            log("⛔ Build cancelled by user.", true);
            return Ok(BuildOutcome::Cancelled);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                drop(stdout_thread);
                drop(stderr_thread);
                log("", false);
                if status.success() {
                    log("✅ Build completed successfully!", false);
                    log(&format!("   Image: {}", config.default_tag()), false);
                    log("", false);
                    log("💡 Run with:", false);
                    log(
                        &format!(
                            "   docker run --rm -v $(pwd):/work {} cp2k --help",
                            config.default_tag()
                        ),
                        false,
                    );
                    return Ok(BuildOutcome::Success);
                } else {
                    log(
                        &format!("❌ Build failed with exit code: {:?}", status.code()),
                        true,
                    );
                    return Ok(BuildOutcome::Failed);
                }
            }
            Ok(None) => {
                // Still running, check cancel flag periodically
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                log(&format!("⚠️ Error waiting for build: {}", e), true);
                return Err(anyhow!("Build process error: {}", e));
            }
        }
    }
}

// ============================================================
// Native Build (direct on host, no Docker)
// ============================================================

/// Run a command with real-time output streaming
/// Result of a single external command step. `Cancelled` means the
/// cancel flag was observed while the child was running and the child
/// was killed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CmdStatus {
    Completed,
    Cancelled,
}

fn run_cmd_logged<S: AsRef<str> + std::fmt::Display>(
    cmd: &str,
    args: &[S],
    workdir: Option<&Path>,
    log_tx: &mpsc::Sender<LogLine>,
    cancel_flag: &Arc<Mutex<bool>>,
) -> Result<CmdStatus> {
    let log = |text: &str, is_err: bool| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            is_error: is_err,
        });
    };

    let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
    log(&format!("$ {} {}", cmd, args_str.join(" ")), false);

    let mut child = {
        let mut c = Command::new(cmd);
        c.args(&args_str);
        if let Some(wd) = workdir {
            c.current_dir(wd);
        }
        c.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to run '{}': {}", cmd, e))?
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let tx1 = log_tx.clone();
    let cancel1 = cancel_flag.clone();
    let stdout_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if *cancel1.lock().unwrap() {
                break;
            }
            if let Ok(l) = line {
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx1.send(LogLine {
                    timestamp: ts,
                    text: l,
                    is_error: false,
                });
            }
        }
    });

    let tx2 = log_tx.clone();
    let cancel2 = cancel_flag.clone();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if *cancel2.lock().unwrap() {
                break;
            }
            if let Ok(l) = line {
                let is_err =
                    l.to_lowercase().contains("error") || l.to_lowercase().contains("failed");
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx2.send(LogLine {
                    timestamp: ts,
                    text: l,
                    is_error: is_err,
                });
            }
        }
    });

    // Poll with try_wait so cancellation can kill the child promptly;
    // log-reader threads keep draining pipes until EOF or cancel.
    loop {
        if *cancel_flag.lock().unwrap() {
            let _ = child.kill();
            // Detach readers: pipes EOF when the child dies, but a
            // grandchild could hold them open, so never join here.
            drop(stdout_thread);
            drop(stderr_thread);
            log(&format!("⛔ '{}' cancelled by user.", cmd), true);
            return Ok(CmdStatus::Cancelled);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                drop(stdout_thread);
                drop(stderr_thread);
                if !status.success() {
                    return Err(anyhow!(
                        "Command '{}' failed with exit code {:?}",
                        cmd,
                        status.code()
                    ));
                }
                return Ok(CmdStatus::Completed);
            }
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                return Err(anyhow!("Command '{}' wait error: {}", cmd, e));
            }
        }
    }
}

/// Check if a command/tool is available on the host
fn check_prereq(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

/// Execute a native build directly on the host (no Docker)
pub fn execute_native_build(
    config: &BuildConfig,
    log_tx: mpsc::Sender<LogLine>,
    cancel_flag: Arc<Mutex<bool>>,
) -> Result<BuildOutcome> {
    // Run one external step; propagate cancellation as an explicit outcome.
    macro_rules! run_step {
        ($($arg:tt)*) => {
            match run_cmd_logged($($arg)*) {
                Ok(CmdStatus::Completed) => {}
                Ok(CmdStatus::Cancelled) => return Ok(BuildOutcome::Cancelled),
                Err(e) => return Err(e),
            }
        };
    }
    let log = |text: &str, is_err: bool| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            is_error: is_err,
        });
    };

    log(
        &format!("🔨 Forge2K Native Build v{}", env!("CARGO_PKG_VERSION")),
        false,
    );
    log("   Method:    native", false);

    // ── Step 0: Validate the configuration combination ──
    // The native path is hardcoded for exactly one supported combination
    // (x86_64 CPU, system MPICH, psmp variant, no CUDA). Anything else
    // must fail explicitly instead of silently ignoring the settings.
    const NATIVE_CPU: &str = "x86_64";
    const NATIVE_MPI: &str = "mpich";
    const NATIVE_VARIANT: &str = "psmp";
    if config.cuda != "none"
        || config.cpu != NATIVE_CPU
        || config.mpi != NATIVE_MPI
        || config.variant != NATIVE_VARIANT
    {
        let msg = format!(
            "Unsupported native build configuration: cpu='{}', mpi='{}', variant='{}', cuda='{}'. \
             The native method currently only supports: cpu='{}', mpi='{}', variant='{}', cuda='none'. \
             Use the Spack (Docker) method for other combinations.",
            config.cpu, config.mpi, config.variant, config.cuda,
            NATIVE_CPU, NATIVE_MPI, NATIVE_VARIANT
        );
        log(&format!("✗ {}", msg), true);
        return Err(anyhow::anyhow!(msg));
    }
    log("✓ Configuration combination supported by native build (x86_64 / system-mpich / psmp / no CUDA)", false);
    log(&format!("   Version:   {}", config.version), false);
    log(&format!("   MPI:       {}", config.mpi), false);
    log(&format!("   CPU:       {}", config.cpu), false);
    log(&format!("   CUDA:      {}", config.cuda), false);
    log(&format!("   Variant:   {}", config.variant), false);
    log(&format!("   Jobs:      {}", config.jobs), false);
    log("", false);

    // ── Step 1: Check prerequisites ──
    log("📋 Step 1/6: Checking system prerequisites...", false);
    let required = [
        "gcc", "g++", "gfortran", "git", "make", "cmake", "wget", "bunzip2",
    ];
    let mut missing: Vec<&str> = Vec::new();
    for tool in &required {
        if !check_prereq(tool) {
            missing.push(*tool);
        }
    }
    if !missing.is_empty() {
        log(&format!("   Missing: {}", missing.join(", ")), true);
        log("   Attempting to install missing packages...", false);
        run_step!("apt-get", &["update", "-qq"], None, &log_tx, &cancel_flag);
        let mut pkgs: Vec<String> = missing.iter().map(|s| s.to_string()).collect();
        for extra in &[
            "autoconf",
            "autogen",
            "automake",
            "libtool",
            "libtool-bin",
            "ninja-build",
            "pkg-config",
            "python3-dev",
            "python3-pip",
            "xxd",
            "xz-utils",
            "zlib1g-dev",
        ] {
            pkgs.push(extra.to_string());
        }
        let mut args: Vec<String> = vec![
            "install".into(),
            "-qq".into(),
            "--no-install-recommends".into(),
            "-y".into(),
        ];
        args.extend(pkgs.iter().cloned());
        let result = run_cmd_logged("apt-get", &args, None, &log_tx, &cancel_flag);
        match result {
            Ok(CmdStatus::Completed) => {}
            Ok(CmdStatus::Cancelled) => return Ok(BuildOutcome::Cancelled),
            Err(_) => {
                log(
                    "   ⚠️  Some packages failed to install. Trying with sudo...",
                    true,
                );
                let missing_str = missing.join(" ");
                let sudo_args: Vec<String> = vec![
                    "apt-get".into(),
                    "install".into(),
                    "-qq".into(),
                    "-y".into(),
                    missing_str,
                ];
                if let Ok(CmdStatus::Cancelled) =
                    run_cmd_logged("sudo", &sudo_args, None, &log_tx, &cancel_flag)
                {
                    return Ok(BuildOutcome::Cancelled);
                }
            }
        }
    } else {
        log("   ✅ All required tools found", false);
    }
    log("", false);

    // ── Step 2: Create working directory ──
    log("📋 Step 2/6: Setting up working directory...", false);
    let work_dir = PathBuf::from("/opt/cp2k_build");
    std::fs::create_dir_all(&work_dir).context("Failed to create /opt/cp2k_build")?;
    log(&format!("   Work dir: {}", work_dir.display()), false);
    log("", false);

    // ── Step 3: Clone CP2K ──
    log("📋 Step 3/6: Cloning CP2K source...", false);
    let cp2k_dir = work_dir.join("cp2k");
    if cp2k_dir.exists() {
        log("   CP2K directory already exists, pulling latest...", false);
        run_step!(
            "git",
            &["-C", cp2k_dir.to_str().unwrap(), "pull"],
            None,
            &log_tx,
            &cancel_flag
        );
    } else {
        let clone_url = "https://github.com/cp2k/cp2k.git";
        let mut git_args: Vec<String> = vec!["clone".into(), "--recursive".into()];
        if config.version != "master" {
            let branch = format!("support/v{}", config.version);
            git_args.push("-b".into());
            git_args.push(branch);
        }
        git_args.push(clone_url.to_string());
        git_args.push(cp2k_dir.to_str().unwrap().to_string());
        run_step!("git", &git_args, None, &log_tx, &cancel_flag);
    }
    log("", false);

    // ── Step 4: Install toolchain dependencies ──
    log(
        "📋 Step 4/6: Installing CP2K toolchain dependencies...",
        false,
    );
    log(
        "   This will download and compile many libraries (30-60 min)...",
        false,
    );
    log("", false);

    let toolchain_dir = cp2k_dir.join("tools").join("toolchain");
    let toolchain_script = toolchain_dir.join("install_cp2k_toolchain.sh");
    if !toolchain_script.exists() {
        return Err(anyhow!(
            "Toolchain script not found at {}",
            toolchain_script.display()
        ));
    }

    let tc_script = toolchain_dir.join("install_cp2k_toolchain.sh");
    let tc_args: Vec<String> = vec![
        tc_script.to_string_lossy().into_owned(),
        "-j".into(),
        config.jobs.to_string(),
        "--install-all".into(),
        "--enable-cuda=no".into(),
        "--with-deepmd=no".into(),
        "--target-cpu=x86_64".into(),
        "--with-cusolvermp=no".into(),
        "--with-gcc=system".into(),
        "--with-mpich=system".into(),
    ];
    match run_cmd_logged(
        "bash",
        &tc_args,
        Some(toolchain_dir.as_path()),
        &log_tx,
        &cancel_flag,
    ) {
        Ok(CmdStatus::Completed) => {}
        Ok(CmdStatus::Cancelled) => return Ok(BuildOutcome::Cancelled),
        Err(e) => return Err(anyhow!("Toolchain installation failed: {}", e)),
    }
    log("", false);

    // ── Step 5: Build CP2K ──
    log("📋 Step 5/6: Building CP2K...", false);
    log("", false);

    let use_cmake = config.version == "master";
    if use_cmake {
        // CMake + Ninja (master branch)
        let setup_script = toolchain_dir.join("install").join("setup");
        // Create a build script that sources setup then runs cmake+ninja
        let build_sh = cp2k_dir.join("build_native.sh");
        let script = format!(
            r#"#!/bin/bash
set -e
source {}
cmake -GNinja \
    -DCMAKE_INSTALL_PREFIX=/opt/cp2k/install \
    -DCP2K_USE_EVERYTHING=ON \
    -DCP2K_USE_DLAF=OFF \
    -DCP2K_USE_PEXSI=OFF \
    -DCP2K_USE_DEEPMD=OFF \
    -DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF \
    -DCMAKE_C_FLAGS="-fno-lto" \
    -DCMAKE_CXX_FLAGS="-fno-lto" \
    -DCMAKE_Fortran_FLAGS="-fno-lto" \
    -DCMAKE_EXE_LINKER_FLAGS="-fno-lto" \
    -Werror=dev \
    -B build -S .
ninja -C build -j {}
cmake --install build --prefix /opt/cp2k/install
echo "BUILD_COMPLETE"
"#,
            setup_script.display(),
            config.jobs
        );
        std::fs::write(&build_sh, &script)?;

        set_executable(&build_sh)?;
        run_step!(
            "bash",
            &[build_sh.to_str().unwrap()],
            Some(cp2k_dir.as_path()),
            &log_tx,
            &cancel_flag
        );
    } else {
        // Legacy make approach
        let arch_dir = "local";
        // Find arch file
        let arch_file = toolchain_dir
            .join("install")
            .join("arch")
            .join(format!("{}.psmp", arch_dir));
        let arch_dest = cp2k_dir.join("arch").join(format!("{}.psmp", arch_dir));

        if arch_file.exists() {
            std::fs::copy(&arch_file, &arch_dest)?;
        }

        let setup_script = toolchain_dir.join("install").join("setup");
        let build_sh = cp2k_dir.join("build_native.sh");
        let script = format!(
            r#"#!/bin/bash
set -e
source {}
make -j {} ARCH={} VERSION=psmp
echo "BUILD_COMPLETE"
"#,
            setup_script.display(),
            config.jobs,
            arch_dir
        );
        std::fs::write(&build_sh, &script)?;

        set_executable(&build_sh)?;

        run_step!(
            "bash",
            &[build_sh.to_str().unwrap()],
            Some(cp2k_dir.as_path()),
            &log_tx,
            &cancel_flag
        );
    }
    log("", false);

    // ── Step 6: Verify installation ──
    log("📋 Step 6/6: Verifying installation...", false);
    let cp2k_binary = if use_cmake {
        PathBuf::from("/opt/cp2k/install/bin/cp2k.psmp")
    } else {
        cp2k_dir.join("exe").join("local").join("cp2k.psmp")
    };

    if cp2k_binary.exists() {
        let size = std::fs::metadata(&cp2k_binary)
            .map(|m| m.len())
            .unwrap_or(0);
        log(
            &format!("   ✅ CP2K built successfully: {}", cp2k_binary.display()),
            false,
        );
        log(&format!("   Binary size: {} MB", size / 1_048_576), false);
        log("", false);
        log("   🎉 To use CP2K, add to your PATH:", false);
        log(
            &format!(
                "      export PATH={}:$PATH",
                cp2k_binary.parent().unwrap().display()
            ),
            false,
        );
    } else {
        log(
            "   ⚠️  CP2K binary not found at expected location. Check build output above.",
            true,
        );
    }

    log("", false);
    log("✅ Native build completed!", false);
    Ok(BuildOutcome::Success)
}

// ============================================================
// Network Diagnostics & Registry Mirror
// ============================================================

#[derive(Debug)]
pub enum NetworkStatus {
    Good,
    Slow(String),
    Blocked(String),
    Unknown(String),
}

/// Test Docker registry connectivity
pub fn check_registry() -> NetworkStatus {
    let start = std::time::Instant::now();

    let output = Command::new("docker")
        .args(["pull", "alpine:latest"])
        .args(["--quiet"])
        .output();

    match output {
        Ok(out) => {
            let elapsed = start.elapsed();
            if out.status.success() {
                // Clean up - remove the pulled image
                let _ = Command::new("docker")
                    .args(["rmi", "alpine:latest"])
                    .output();

                if elapsed < Duration::from_secs(10) {
                    NetworkStatus::Good
                } else {
                    NetworkStatus::Slow(format!("{:.1}s", elapsed.as_secs_f64()))
                }
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if stderr.contains("timeout")
                    || stderr.contains("refused")
                    || stderr.contains("no route")
                {
                    NetworkStatus::Blocked(stderr)
                } else {
                    NetworkStatus::Unknown(stderr)
                }
            }
        }
        Err(e) => NetworkStatus::Unknown(e.to_string()),
    }
}

/// Try to find a working registry mirror
pub fn detect_best_mirror() -> Option<String> {
    for mirror in crate::mirrors::registry_mirror_urls() {
        if test_mirror(mirror) {
            return Some(mirror.to_string());
        }
    }
    None
}

fn test_mirror(url: &str) -> bool {
    // Quick HTTP check using curl or PowerShell
    #[cfg(target_os = "windows")]
    {
        let test_url = format!("{}/v2/_catalog", url.trim_end_matches('/'));
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "try {{ (Invoke-WebRequest -Uri '{}' -TimeoutSec 5 -UseBasicParsing).StatusCode -eq 200 }} catch {{ $false }}",
                    test_url
                ),
            ])
            .output();
        match output {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                s == "True"
            }
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let test_url = format!("{}/v2/_catalog", url.trim_end_matches('/'));
        let output = Command::new("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--connect-timeout",
                "5",
                &test_url,
            ])
            .output();
        match output {
            Ok(out) => {
                let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
                return code == "200" || code == "401"; // 401 means registry exists but auth required
            }
            Err(_) => return false,
        }
    }
}

/// Get Docker daemon config path
pub fn daemon_config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\Default".to_string());
        PathBuf::from(home).join(".docker").join("daemon.json")
    }

    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from("/etc/docker/daemon.json")
    }
}

/// Configure a registry mirror
pub fn set_registry_mirror(url: &str) -> Result<()> {
    let config_path = daemon_config_path();

    let mut config: serde_json::Value = if config_path.exists() {
        let content =
            std::fs::read_to_string(&config_path).context("Failed to read Docker daemon config")?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let mirrors = vec![url.to_string()];
    config["registry-mirrors"] = serde_json::json!(mirrors);

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create .docker directory")?;
    }

    let content = serde_json::to_string_pretty(&config).context("Failed to serialize config")?;
    std::fs::write(&config_path, &content).context("Failed to write Docker daemon config")?;

    Ok(())
}

/// Remove registry mirror configuration
pub fn remove_registry_mirror() -> Result<()> {
    let config_path = daemon_config_path();

    if !config_path.exists() {
        return Ok(());
    }

    let content =
        std::fs::read_to_string(&config_path).context("Failed to read Docker daemon config")?;
    let mut config: serde_json::Value = serde_json::from_str(&content)?;

    if let Some(obj) = config.as_object_mut() {
        obj.remove("registry-mirrors");
    }

    let content = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, &content)?;

    Ok(())
}

/// Get current registry mirror config
pub fn get_registry_mirror() -> Option<String> {
    let config_path = daemon_config_path();
    if !config_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&content).ok()?;

    config
        .get("registry-mirrors")?
        .as_array()?
        .first()?
        .as_str()
        .map(|s| s.to_string())
}

// ============================================================
// Configuration Listing
// ============================================================

/// Where a configuration's Dockerfile comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerfileSource {
    /// A Dockerfile bundled in the dockerfiles/ directory.
    Bundled,
    /// No bundled Dockerfile matches; one is synthesized at runtime.
    Synthesized,
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

/// Directories searched for bundled Dockerfiles (relative to the
/// executable first, then the working directory).
fn dockerfile_search_paths() -> Vec<PathBuf> {
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

/// Substitute `{{TOKEN}}` placeholders in an include_str! Dockerfile
/// template. Byte-exact replacement; every token must be provided.
fn render_template(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in params {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    out
}

/// Generate Dockerfile content from build config
fn generate_dockerfile_content(config: &BuildConfig) -> Result<String> {
    match config.method.as_str() {
        "toolchain" => generate_toolchain_dockerfile(config),
        _ => generate_spack_dockerfile(config),
    }
}

fn generate_spack_dockerfile(config: &BuildConfig) -> Result<String> {
    let cpu = &config.cpu;
    let mpi = &config.mpi;
    let version = &config.version;
    let git_clone = if version == "master" {
        "RUN git clone --recursive https://github.com/cp2k/cp2k.git /opt/cp2k".to_string()
    } else {
        format!("RUN git clone --recursive -b support/v{version} https://github.com/cp2k/cp2k.git /opt/cp2k")
    };

    // v2026.1+ renamed cp2k_deps_all_psmp.yaml -> cp2k_deps_psmp.yaml
    // and added require: target="" inside packages:all section
    let use_new_deps = version == "master"
        || version.starts_with("2026")
        || version.starts_with("2027")
        || version.starts_with("2028");
    let (deps_yaml, spack_ver, spack_pkgs_ver) = if use_new_deps {
        ("cp2k_deps_${CP2K_VERSION}.yaml", "1.1.1", "2026.03.0")
    } else {
        ("cp2k_deps_all_${CP2K_VERSION}.yaml", "1.0.0", "2025.07.0")
    };

    let mpi_setup = match mpi.as_str() {
        "openmpi" => {
            if use_new_deps {
                format!("RUN sed -i 's/target=\"\"/target=\"{cpu}\"/' /opt/cp2k/tools/spack/{deps} && \
                         sed -e 's/- mpich/- openmpi/' \
                         -e '/^\\s*xpmem:/i\\    openmpi:\\n      require:\\n        - +internal-hwloc' \
                         -e '/^\\s*- \"mpich@/ s/^ /#/' \
                         -e '/^#\\s*- \"openmpi@/ s/^#/ /' \
                         -i /opt/cp2k/tools/spack/{deps}",
                        cpu = cpu, deps = deps_yaml)
            } else {
                format!("RUN sed -e '/^\\s*mpi:/i\\      require: target=\"{cpu}\"' \
                         -e 's/- mpich/- openmpi/' \
                         -e '/^\\s*xpmem:/i\\    openmpi:\\n      require:\\n        - +internal-hwloc' \
                         -e '/^\\s*- \"mpich@/ s/^ /#/' \
                         -e '/^#\\s*- \"openmpi@/ s/^#/ /' \
                         -i /opt/cp2k/tools/spack/{deps}",
                        cpu = cpu, deps = deps_yaml)
            }
        }
        _ => {
            if use_new_deps {
                format!(
                    "RUN sed -i 's/target=\"\"/target=\"{cpu}\"/' /opt/cp2k/tools/spack/{deps}",
                    cpu = cpu,
                    deps = deps_yaml
                )
            } else {
                format!(
                    "RUN sed -e '/^\\s*mpi:/i\\      require: target=\"{cpu}\"' \
                         -i /opt/cp2k/tools/spack/{deps}",
                    cpu = cpu,
                    deps = deps_yaml
                )
            }
        }
    };

    Ok(render_template(
        include_str!("templates/spack.Dockerfile.tmpl"),
        &[
            ("GIT_CLONE", &git_clone),
            ("SPACK_VER", spack_ver),
            ("SPACK_PKGS_VER", spack_pkgs_ver),
            ("MPI_SETUP", &mpi_setup),
            ("DEPS_YAML", deps_yaml),
        ],
    ))
}

fn generate_toolchain_dockerfile(config: &BuildConfig) -> Result<String> {
    let cuda_enabled = config.cuda != "none";
    let gpu_ver = if config.cuda == "none" {
        "no"
    } else {
        &config.cuda
    };
    let cuda_flag = if cuda_enabled { "yes" } else { "no" };
    let base_image = if cuda_enabled {
        "nvidia/cuda:12.2.0-devel-ubuntu22.04"
    } else {
        "ubuntu:22.04"
    };
    let arch_dir = if cuda_enabled { "local_cuda" } else { "local" };
    let cuda_extra = if cuda_enabled {
        format!("--gpu-ver={} --with-libtorch=no", gpu_ver)
    } else {
        String::new()
    };
    let use_cmake = config.version == "master";
    let git_clone = if config.version == "master" {
        "RUN git clone --recursive https://github.com/cp2k/cp2k.git /opt/cp2k".to_string()
    } else {
        format!(
            "RUN git clone --recursive -b support/v{} https://github.com/cp2k/cp2k.git /opt/cp2k",
            config.version
        )
    };

    let build_step = if use_cmake {
        // Master branch uses CMake + Ninja (arch file no longer exists)
        r#"SHELL ["/bin/bash", "-c"]
WORKDIR /opt/cp2k
ENV TOOLCHAIN_DIR=/opt/cp2k/tools/toolchain
RUN source ${TOOLCHAIN_DIR}/install/setup && \
    cmake -GNinja \
    -DCMAKE_INSTALL_PREFIX=/opt/cp2k/install \
    -DCP2K_USE_EVERYTHING=ON \
    -DCP2K_USE_DLAF=OFF \
    -DCP2K_USE_PEXSI=OFF \
    -DCP2K_USE_DEEPMD=OFF \
    -DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF \
    -DCMAKE_C_FLAGS="-fno-lto" \
    -DCMAKE_CXX_FLAGS="-fno-lto" \
    -DCMAKE_Fortran_FLAGS="-fno-lto" \
    -DCMAKE_EXE_LINKER_FLAGS="-fno-lto" \
    -Werror=dev \
    -B build -S . && \
    ninja -C build -j ${NUM_PROCS:-8} && \
    cmake --install build --prefix /opt/cp2k/install

RUN mkdir -p /toolchain/install /toolchain/scripts && \
    for d in /opt/cp2k/tools/toolchain/install/*/; do \
        libdir=$(basename "$d"); \
        cp -a "$d" /toolchain/install/; \
    done && \
    cp /opt/cp2k/tools/toolchain/install/setup /toolchain/install/ && \
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#
            .to_string()
    } else {
        // Tagged releases use old make approach with arch files
        format!(
            r#"WORKDIR /opt/cp2k
RUN cp ./tools/toolchain/install/arch/{arch}.psmp ./arch/ && \
    source ./tools/toolchain/install/setup && \
    make -j ${{NUM_PROCS:-8}} ARCH={arch} VERSION=psmp

RUN mkdir -p /toolchain/install /toolchain/scripts && \
    for libdir in $(ldd ./exe/{arch}/cp2k.psmp | \
                     grep /opt/cp2k/tools/toolchain/install | \
                     awk '{{print $3}}' | cut -d/ -f7 | \
                     sort | uniq) setup; do \
       cp -ar /opt/cp2k/tools/toolchain/install/${{libdir}} /toolchain/install; \
    done && \
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#,
            arch = arch_dir
        )
    };

    let copy_step: String = if use_cmake {
        "COPY --from=build /opt/cp2k/install/ /opt/cp2k/install/\n\
         COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/\n\
         COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/"
            .into()
    } else {
        format!(
            "COPY --from=build /opt/cp2k/exe/{arch}/ /opt/cp2k/exe/{arch}/\n\
                 COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/\n\
                 COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/",
            arch = arch_dir
        )
    };

    let link_step: String = if use_cmake {
        r#"RUN ln -sf /opt/cp2k/install/bin/cp2k.psmp /usr/local/bin/cp2k && \
    ln -sf /opt/cp2k/install/bin/cp2k_shell.psmp /usr/local/bin/cp2k_shell && \
    ln -sf /opt/cp2k/install/bin/cp2k.popt /usr/local/bin/cp2k.popt
ENV PATH="/opt/cp2k/install/bin:${PATH}"
RUN printf '#!/bin/bash\n\
ulimit -c 0 -s unlimited\n\
export OMP_STACKSIZE=16M\n\
source /opt/cp2k/tools/toolchain/install/setup\n\
export LD_LIBRARY_PATH="/opt/cp2k/install/lib:${LD_LIBRARY_PATH}"\n\
exec "$@"' > /usr/local/bin/entrypoint.sh && chmod 755 /usr/local/bin/entrypoint.sh"#
            .to_string()
    } else {
        format!(
            r#"RUN for binary in cp2k dumpdcd graph xyz2dcd; do \
        ln -sf /opt/cp2k/exe/{arch}/${{binary}}.psmp /usr/local/bin/${{binary}}; \
    done && \
    ln -sf /opt/cp2k/exe/{arch}/cp2k.psmp /usr/local/bin/cp2k_shell
ENV PATH="/opt/cp2k/exe/{arch}:${{PATH}}"
RUN printf '#!/bin/bash\n\
ulimit -c 0 -s unlimited\n\
export OMP_STACKSIZE=16M\n\
source /opt/cp2k/tools/toolchain/install/setup\n\
export LD_LIBRARY_PATH="/opt/cp2k/install/lib:${{LD_LIBRARY_PATH}}"\n\
exec "$@"' > /usr/local/bin/entrypoint.sh && chmod 755 /usr/local/bin/entrypoint.sh"#,
            arch = arch_dir
        )
    };

    Ok(render_template(
        include_str!("templates/toolchain.Dockerfile.tmpl"),
        &[
            ("BASE_IMAGE", base_image),
            (
                "CUDA_ENV",
                if cuda_enabled {
                    "ENV CUDA_PATH /usr/local/cuda\nENV LD_LIBRARY_PATH /usr/local/cuda/lib64\nENV CUDA_CACHE_DISABLE 1"
                } else {
                    ""
                },
            ),
            ("CUDA_FLAG", cuda_flag),
            ("CUDA_EXTRA", &cuda_extra),
            ("CPU", &config.cpu),
            ("GIT_CLONE", &git_clone),
            ("BUILD_STEP", &build_step),
            ("COPY_STEP", &copy_step),
            ("LINK_STEP", &link_step),
        ],
    ))
}

// ============================================================
// Tests (Wave 4: T-013 pure-function unit tests,
// T-014 failure-propagation & cancellation behavior tests)
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --------------------------------------------------------
    // Shared helpers
    // --------------------------------------------------------

    fn base_config(
        method: &str,
        version: &str,
        mpi: &str,
        cpu: &str,
        cuda: &str,
        variant: &str,
    ) -> BuildConfig {
        BuildConfig {
            method: method.to_string(),
            version: version.to_string(),
            mpi: mpi.to_string(),
            cpu: cpu.to_string(),
            cuda: cuda.to_string(),
            variant: variant.to_string(),
            jobs: 8,
            tag: String::new(),
            no_cache: false,
            shm_size: "1g".to_string(),
            dockerfile: None,
        }
    }

    /// Serializes every test that spawns child processes or mutates PATH.
    /// std::env mutations are process-global; without this lock a
    /// PATH-clearing test would break a concurrently-spawning sibling.
    static PROCESS_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Restores PATH on drop, even if the assertion panics.
    struct PathGuard {
        old: String,
    }

    impl PathGuard {
        fn set(new_path: &str) -> PathGuard {
            let old = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", new_path);
            PathGuard { old }
        }
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            std::env::set_var("PATH", &self.old);
        }
    }

    /// A unique temp directory removed on drop (best effort).
    struct TempGuard(PathBuf);

    impl TempGuard {
        fn new(tag: &str) -> TempGuard {
            let dir =
                std::env::temp_dir().join(format!("forge2k_w4_{}_{}", tag, std::process::id()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempGuard(dir)
        }
    }

    impl Drop for TempGuard {
        fn drop(&mut self) {
            // On Windows, handles from a just-killed child are released
            // asynchronously; retry briefly so no temp dir is left behind.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                if std::fs::remove_dir_all(&self.0).is_ok() || std::time::Instant::now() >= deadline
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// Install a fake `docker` executable (a copy of the shell) in `dir`
    /// so `Command::new("docker")` resolves without a real daemon.
    fn install_fake_docker(dir: &Path) {
        #[cfg(target_os = "windows")]
        {
            let src =
                std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
            std::fs::copy(&src, dir.join("docker.exe")).expect("copy shell as fake docker.exe");
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::fs::copy("/bin/sh", dir.join("docker")).expect("copy shell as fake docker");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("docker"), std::fs::Permissions::from_mode(0o755))
                .expect("chmod fake docker");
        }
    }

    /// A short-lived child process used to drive the cancel flag from a
    /// background thread (event-driven, no wall-clock assertion).
    fn spawn_short_sleep_subprocess() -> std::process::Child {
        #[cfg(target_os = "windows")]
        {
            Command::new("cmd")
                .args(["/D", "/C", "ping -n 1 127.0.0.1 > nul"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn short sleep subprocess (cmd/ping)")
        }
        #[cfg(not(target_os = "windows"))]
        {
            Command::new("sleep")
                .arg("1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn short sleep subprocess (sleep)")
        }
    }

    /// Long-running scripted command: sleeps ~30s, then writes a sentinel
    /// file. If the process is killed before finishing, the sentinel is
    /// never written — an event-based (not timing-based) kill proof.
    fn long_sleep_with_sentinel(sentinel: &Path) -> (&'static str, Vec<String>) {
        #[cfg(target_os = "windows")]
        {
            (
                "cmd",
                vec![
                    "/D".to_string(),
                    "/C".to_string(),
                    format!(
                        "ping -n 30 127.0.0.1 > nul & echo done > \"{}\"",
                        sentinel.display()
                    ),
                ],
            )
        }
        #[cfg(not(target_os = "windows"))]
        {
            (
                "sh",
                vec![
                    "-c".to_string(),
                    format!("sleep 30; echo done > '{}'", sentinel.display()),
                ],
            )
        }
    }

    fn drain_rx(rx: mpsc::Receiver<LogLine>) {
        for _ in rx.try_iter() {}
    }

    // --------------------------------------------------------
    // T-013: pure-function unit tests
    // --------------------------------------------------------

    #[test]
    fn t13_dockerfile_name_spack_combinations() {
        let c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
        assert_eq!(
            c.dockerfile_name(),
            "spack/2025.2_mpich_x86_64_psmp.Dockerfile",
            "T-013: spack name must be version_mpi_cpu_variant.Dockerfile (no cuda segment)"
        );
        let c = base_config("spack", "2026.1", "openmpi", "generic", "none", "ssmp");
        assert_eq!(
            c.dockerfile_name(),
            "spack/2026.1_openmpi_generic_ssmp.Dockerfile",
            "T-013: spack openmpi/generic/ssmp combination must assemble in fixed order"
        );
        // cuda is not part of the spack filename scheme
        let c = base_config("spack", "2025.2", "mpich", "cascadelake", "P100", "psmp");
        assert_eq!(
            c.dockerfile_name(),
            "spack/2025.2_mpich_cascadelake_psmp.Dockerfile",
            "T-013: spack filenames must not embed a cuda segment"
        );
    }

    #[test]
    fn t13_dockerfile_name_toolchain_combinations() {
        let c = base_config("toolchain", "2023.2", "mpich", "generic", "P100", "psmp");
        assert_eq!(
            c.dockerfile_name(),
            "toolchain/2023.2_mpich_generic_cuda_P100_psmp.Dockerfile",
            "T-013: toolchain + cuda must insert '_cuda_<GPU>' before the variant"
        );
        let c = base_config("toolchain", "2025.2", "mpich", "x86_64", "none", "sdbg");
        assert_eq!(
            c.dockerfile_name(),
            "toolchain/2025.2_mpich_x86_64_sdbg.Dockerfile",
            "T-013: toolchain without cuda must omit the cuda segment entirely"
        );
    }

    #[test]
    fn t13_dockerfile_search_paths_fallback_order() {
        let paths = dockerfile_search_paths();
        assert_eq!(
            paths.len(),
            3,
            "T-013: exactly three search locations expected"
        );
        let exe_dir = std::env::current_exe()
            .expect("current_exe")
            .parent()
            .expect("exe parent")
            .to_path_buf();
        assert_eq!(
            paths[0],
            exe_dir.join("dockerfiles"),
            "T-013: first fallback must be dockerfiles/ next to the executable"
        );
        assert_eq!(
            paths[1],
            PathBuf::from("dockerfiles"),
            "T-013: second fallback must be ./dockerfiles"
        );
        assert_eq!(
            paths[2],
            PathBuf::from("."),
            "T-013: last fallback must be the working directory itself"
        );
    }

    #[test]
    fn t13_default_tag_versioned_release() {
        let c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
        assert_eq!(
            c.default_tag(),
            "cp2k/cp2k:2025.2_mpich_x86_64_psmp",
            "T-013: tag must be cp2k/cp2k:<ver>_<mpi>_<cpu>_<variant>"
        );
        let c = base_config(
            "toolchain",
            "2026.1",
            "openmpi",
            "cascadelake",
            "V100",
            "ssmp",
        );
        assert_eq!(
            c.default_tag(),
            "cp2k/cp2k:2026.1_openmpi_cascadelake_cuda_V100_ssmp",
            "T-013: cuda suffix '_cuda_<GPU>' must sit between cpu and variant"
        );
    }

    #[test]
    fn t13_default_tag_master_gets_yyyymmdd_datestamp() {
        let before = Local::now().format("%Y%m%d").to_string();
        let c = base_config("toolchain", "master", "mpich", "generic", "none", "psmp");
        let tag = c.default_tag();
        let after = Local::now().format("%Y%m%d").to_string();
        assert!(
            tag.starts_with("cp2k/cp2k:master_"),
            "T-013: master builds must be tagged 'master_<date>', got: {}",
            tag
        );
        let date_part = tag
            .trim_start_matches("cp2k/cp2k:master_")
            .split('_')
            .next()
            .expect("date segment")
            .to_string();
        assert_eq!(
            date_part.len(),
            8,
            "T-013: datestamp must be YYYYMMDD (8 digits), got: {}",
            date_part
        );
        assert!(
            date_part.chars().all(|ch| ch.is_ascii_digit()),
            "T-013: datestamp must be all digits, got: {}",
            date_part
        );
        assert!(
            date_part == before || date_part == after,
            "T-013: datestamp {} must match the call-time date ({}..{})",
            date_part,
            before,
            after
        );
    }

    #[test]
    fn t13_default_tag_explicit_tag_overrides_default() {
        let mut c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
        c.tag = "myrepo/cp2k-custom:v9".to_string();
        assert_eq!(
            c.default_tag(),
            "myrepo/cp2k-custom:v9",
            "T-013: an explicit tag must be returned verbatim"
        );
    }

    #[test]
    fn t13_mirrors_table_complete() {
        let mirrors = crate::mirrors::known_mirrors();
        assert!(
            mirrors.len() >= 5,
            "T-013: mirrors table must keep at least 5 entries, got {}",
            mirrors.len()
        );
        let mut urls: Vec<&str> = Vec::new();
        for m in mirrors {
            assert!(
                !m.name.trim().is_empty(),
                "T-013: mirror name must be non-empty"
            );
            assert!(
                !m.desc.trim().is_empty(),
                "T-013: mirror desc must be non-empty"
            );
            assert!(
                m.url.starts_with("https://"),
                "T-013: mirror url must be https, got: {}",
                m.url
            );
            assert!(
                !m.url.ends_with('/'),
                "T-013: mirror url must not end with '/'"
            );
            urls.push(m.url);
        }
        let n = urls.len();
        urls.sort();
        urls.dedup();
        assert_eq!(urls.len(), n, "T-013: mirror urls must be unique");
        // single-source contract: registry_mirror_urls() must yield exactly
        // the table's urls in order
        let iter_urls: Vec<&str> = crate::mirrors::registry_mirror_urls().collect();
        let table_urls: Vec<&str> = mirrors.iter().map(|m| m.url).collect();
        assert_eq!(
            iter_urls, table_urls,
            "T-013: registry_mirror_urls must be derived from known_mirrors in order"
        );
    }

    #[test]
    fn t13_spack_dockerfile_synthesized_key_fields() {
        // pre-2026 release: old deps yaml + spack 1.0.0
        let c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
        let content = generate_dockerfile_content(&c).expect("render spack template");
        assert!(
            content.contains("FROM ubuntu:24.04"),
            "T-013: spack image must start FROM ubuntu:24.04"
        );
        assert!(
            content.contains("git clone --recursive -b support/v2025.2"),
            "T-013: release build must clone the support/v<version> branch"
        );
        assert!(
            content.contains("ARG NUM_PROCS"),
            "T-013: NUM_PROCS build-arg must be present"
        );
        assert!(
            content.contains("cp2k_deps_all_${CP2K_VERSION}.yaml"),
            "T-013: 2025.x must use the legacy cp2k_deps_all deps yaml"
        );
        assert!(
            content.contains("SPACK_VERSION=1.0.0"),
            "T-013: 2025.x must pin spack 1.0.0"
        );
        assert!(
            !content.contains("{{"),
            "T-013: no template placeholder may survive rendering, found '{{' in:\n{}",
            content
        );
        // 2026.1: new deps yaml + spack 1.1.1
        let c = base_config("spack", "2026.1", "openmpi", "cascadelake", "none", "psmp");
        let content = generate_dockerfile_content(&c).expect("render spack template");
        assert!(
            content.contains("cp2k_deps_${CP2K_VERSION}.yaml"),
            "T-013: 2026.1 must use the renamed cp2k_deps deps yaml"
        );
        assert!(
            content.contains("SPACK_VERSION=1.1.1"),
            "T-013: 2026.1 must pin spack 1.1.1"
        );
        assert!(
            content.contains("openmpi"),
            "T-013: openmpi config must contain an openmpi sed/replace step"
        );
        assert!(
            !content.contains("{{"),
            "T-013: no template placeholder may survive rendering (2026.1 openmpi)"
        );
    }

    #[test]
    fn t13_toolchain_dockerfile_synthesized_key_fields() {
        let c = base_config("toolchain", "2023.2", "mpich", "generic", "none", "psmp");
        let content = generate_dockerfile_content(&c).expect("render toolchain template");
        assert!(
            content.contains("FROM ubuntu:22.04"),
            "T-013: non-cuda toolchain must be FROM ubuntu:22.04"
        );
        assert!(
            content.contains("--enable-cuda=no"),
            "T-013: cuda disabled must render --enable-cuda=no"
        );
        assert!(
            content.contains("--target-cpu=generic"),
            "T-013: CPU placeholder must be substituted with the configured target"
        );
        assert!(
            content.contains("ENTRYPOINT [\"/usr/local/bin/entrypoint.sh\"]"),
            "T-013: toolchain image must keep the entrypoint"
        );
        assert!(
            !content.contains("{{"),
            "T-013: no placeholder may survive (toolchain no-cuda)"
        );

        let c = base_config("toolchain", "2023.2", "mpich", "generic", "P100", "psmp");
        let content = generate_dockerfile_content(&c).expect("render toolchain template");
        assert!(
            content.contains("FROM nvidia/cuda:12.2.0-devel-ubuntu22.04"),
            "T-013: cuda toolchain must be FROM the nvidia/cuda devel image"
        );
        assert!(
            content.contains("--enable-cuda=yes"),
            "T-013: cuda enabled must render --enable-cuda=yes"
        );
        assert!(
            content.contains("--gpu-ver=P100"),
            "T-013: cuda build must pass --gpu-ver=<GPU>"
        );
        assert!(
            !content.contains("{{"),
            "T-013: no placeholder may survive (toolchain cuda)"
        );

        // master: cmake/ninja build path
        let c = base_config("toolchain", "master", "mpich", "generic", "none", "psmp");
        let content = generate_dockerfile_content(&c).expect("render toolchain template");
        assert!(
            content.contains("git clone --recursive https://github.com/cp2k/cp2k.git"),
            "T-013: master must clone the default branch (no -b support/...)"
        );
        assert!(
            content.contains("cmake -GNinja"),
            "T-013: master must use the cmake+ninja build step"
        );
        assert!(
            !content.contains("{{"),
            "T-013: no placeholder may survive (toolchain master)"
        );
    }

    #[test]
    fn t13_resolve_dockerfile_custom_paths() {
        let tmp = TempGuard::new("t13_resolve");
        let df = tmp.0.join("custom.Dockerfile");
        std::fs::write(&df, "FROM scratch\n").expect("write custom dockerfile");

        let mut c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
        c.dockerfile = Some(df.clone());
        assert_eq!(
            c.resolve_dockerfile().expect("existing custom dockerfile"),
            df,
            "T-013: an existing custom dockerfile must be used as-is"
        );

        let missing = tmp.0.join("does-not-exist.Dockerfile");
        c.dockerfile = Some(missing.clone());
        let err = c
            .resolve_dockerfile()
            .expect_err("missing custom dockerfile must error");
        assert!(
            err.to_string().contains("Custom Dockerfile not found"),
            "T-013: missing custom dockerfile error must say so, got: {}",
            err
        );
    }

    #[test]
    fn t13_resolve_dockerfile_falls_back_to_synthesized_when_no_bundled_match() {
        // 2099.9 matches nothing bundled anywhere: must synthesize at runtime
        let c = base_config("spack", "2099.9", "mpich", "x86_64", "none", "psmp");
        let path = c.resolve_dockerfile().expect("synthesized fallback");
        assert!(
            path.exists(),
            "T-013: synthesized fallback must write a real file, got: {}",
            path.display()
        );
        let content = std::fs::read_to_string(&path).expect("read synthesized file");
        assert!(
            content.contains("FROM ubuntu:24.04") && !content.contains("{{"),
            "T-013: synthesized file must be a fully rendered spack Dockerfile"
        );
        // cleanup receipt: remove the generated file
        let _ = std::fs::remove_file(&path);
        assert!(
            !path.exists(),
            "T-13: generated dockerfile must be cleaned up"
        );
    }

    // --------------------------------------------------------
    // T-014: failure propagation & cancellation behavior
    // --------------------------------------------------------

    #[test]
    fn t14_execute_build_docker_unreachable_returns_err_not_success() {
        let _serial = PROCESS_TESTS.lock().unwrap();
        let tmp = TempGuard::new("t14_nodocker");
        let df = tmp.0.join("df.Dockerfile");
        std::fs::write(&df, "FROM scratch\n").expect("write dockerfile");

        let mut c = base_config("spack", "2099.9", "mpich", "x86_64", "none", "psmp");
        c.dockerfile = Some(df);

        let (tx, rx) = mpsc::channel::<LogLine>();
        let flag = Arc::new(Mutex::new(false));

        // Inject "docker does not exist" by emptying PATH (restored on drop).
        let _guard = PathGuard::set("");
        let result = execute_build(&c, tx, flag);
        drop(_guard);
        drain_rx(rx);

        match result {
            Ok(outcome) => panic!(
                "T-014: execute_build with docker unreachable must not succeed, got {:?}",
                outcome
            ),
            Err(e) => assert!(
                e.to_string().contains("docker"),
                "T-014: spawn failure must mention docker, got: {}",
                e
            ),
        }
    }

    #[test]
    fn t14_execute_build_cancel_flag_pre_set_returns_cancelled_and_kills_child() {
        let _serial = PROCESS_TESTS.lock().unwrap();
        let tmp = TempGuard::new("t14_cancel_build");
        let df = tmp.0.join("df.Dockerfile");
        std::fs::write(&df, "FROM scratch\n").expect("write dockerfile");
        install_fake_docker(&tmp.0);

        let mut c = base_config("spack", "2099.9", "mpich", "x86_64", "none", "psmp");
        c.dockerfile = Some(df);

        let (tx, rx) = mpsc::channel::<LogLine>();
        let flag = Arc::new(Mutex::new(true)); // cancel flag already set

        // A short sleep subprocess drives the flag from a background thread
        // as well (idempotent: the flag is what matters, not who set it).
        let flag2 = flag.clone();
        std::thread::spawn(move || {
            let mut short = spawn_short_sleep_subprocess();
            let _ = short.wait();
            *flag2.lock().unwrap() = true;
        });

        // The fake `docker` (a shell copy) is spawned, then the poll loop
        // observes the cancel flag before try_wait and kills the child.
        let _guard = PathGuard::set(&tmp.0.to_string_lossy());
        let result = execute_build(&c, tx, flag);
        drop(_guard);
        drain_rx(rx);

        assert_eq!(
            result.expect("execute_build must not error"),
            BuildOutcome::Cancelled,
            "T-014: execute_build with a pre-set cancel flag must return Cancelled (child killed, not joined)"
        );
    }

    #[test]
    fn t14_run_cmd_logged_cancel_kills_child_before_sentinel_is_written() {
        let _serial = PROCESS_TESTS.lock().unwrap();
        let tmp = TempGuard::new("t14_cancel_cmd");
        let sentinel = tmp.0.join("sentinel.txt");
        let _ = std::fs::remove_file(&sentinel);

        let (cmd, args) = long_sleep_with_sentinel(&sentinel);
        let (tx, rx) = mpsc::channel::<LogLine>();
        let flag = Arc::new(Mutex::new(false));

        // Background thread: run a short sleep subprocess, then set the
        // cancel flag (event-driven; no precise timing assertion).
        let flag2 = flag.clone();
        std::thread::spawn(move || {
            let mut short = spawn_short_sleep_subprocess();
            let _ = short.wait();
            *flag2.lock().unwrap() = true;
        });

        let status = run_cmd_logged(&cmd, &args, None, &tx, &flag)
            .expect("run_cmd_logged must not error on cancel");
        drain_rx(rx);

        assert_eq!(
            status,
            CmdStatus::Cancelled,
            "T-014: run_cmd_logged must return Cancelled when the flag is set mid-run"
        );

        // Kill proof (event-based, generous slack): the sentinel is written
        // by the child *after* its 30s sleep; it must never appear because
        // the child was killed. Poll ~3s to catch a premature write.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            assert!(
                !sentinel.exists(),
                "T-014: sentinel file appeared -> the child was NOT killed and ran to completion"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn t14_run_cmd_logged_completion_returns_completed() {
        let _serial = PROCESS_TESTS.lock().unwrap();
        let (tx, rx) = mpsc::channel::<LogLine>();
        let flag = Arc::new(Mutex::new(false));
        #[cfg(target_os = "windows")]
        let (cmd, args): (&str, Vec<String>) =
            ("cmd", vec!["/D".into(), "/C".into(), "echo hi".into()]);
        #[cfg(not(target_os = "windows"))]
        let (cmd, args): (&str, Vec<String>) = ("echo", vec!["hi".to_string()]);
        let status =
            run_cmd_logged(&cmd, &args, None, &tx, &flag).expect("trivial command must not error");
        drain_rx(rx);
        assert_eq!(
            status,
            CmdStatus::Completed,
            "T-014: a successful command must map to CmdStatus::Completed"
        );
    }

    #[test]
    fn t14_execute_native_build_rejects_unsupported_combination() {
        // Native path is hardcoded for x86_64/mpich/psmp/cuda=none; anything
        // else must fail explicitly before any step runs.
        let c = base_config("native", "master", "openmpi", "generic", "none", "psmp");
        let (tx, rx) = mpsc::channel::<LogLine>();
        let flag = Arc::new(Mutex::new(false));
        let result = execute_native_build(&c, tx, flag);
        drain_rx(rx);
        let err = result.expect_err("T-014: unsupported native combination must return Err");
        assert!(
            err.to_string()
                .contains("Unsupported native build configuration"),
            "T-014: error must name the unsupported combination, got: {}",
            err
        );
    }

    // Note on execute_native_build cancellation coverage: every cancellable
    // step in execute_native_build funnels through the same `run_step!` ->
    // run_cmd_logged -> CmdStatus::Cancelled -> `return
    // Ok(BuildOutcome::Cancelled)` mapping exercised by
    // t14_run_cmd_logged_cancel_kills_child_before_sentinel_is_written.
    // The surrounding steps (git clone of the full cp2k tree, apt-get, the
    // toolchain script) are hardcoded and cannot be injected without a real
    // Linux build host, so the mapping is covered at the run_cmd_logged
    // level (documented in the results JSON).
}
