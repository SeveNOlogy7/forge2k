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
fn set_executable(_path: &Path) -> Result<()> { Ok(()) }
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ============================================================
// Build Configuration
// ============================================================

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub method: String,       // "spack" or "toolchain"
    pub version: String,      // "2025.2", "2023.2"
    pub mpi: String,          // "mpich", "openmpi"
    pub cpu: String,          // "x86_64", "generic", "cascadelake"
    pub cuda: String,         // "none", "P100", "V100"
    pub variant: String,      // "psmp", "ssmp", "pdbg", "sdbg"
    pub jobs: u32,
    pub tag: String,
    pub no_cache: bool,
    pub shm_size: String,
    pub dockerfile: Option<PathBuf>,
    pub _output: String,       // "docker" or "image"
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

        // Look for bundled Dockerfiles relative to the executable
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();

        // Search paths for Dockerfiles
        let search_paths = vec![
            exe_dir.join("dockerfiles"),
            PathBuf::from("dockerfiles"),
            PathBuf::from("."),
        ];

        let filename = self.dockerfile_name();
        for base in &search_paths {
            let path = base.join(&filename);
            if path.exists() {
                return Ok(path);
            }
        }

        // If no bundled file found, we'll generate one at runtime
        let generated = self.generate_dockerfile()?;
        Ok(generated)
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
        args.push(dockerfile.parent().unwrap_or(Path::new(".")).to_string_lossy().to_string());

        args
    }

    /// Generate a Dockerfile from embedded templates
    pub fn generate_dockerfile(&self) -> Result<PathBuf> {
        let content = generate_dockerfile_content(self)?;
        let tmp_dir = std::env::temp_dir().join("forge2k");
        std::fs::create_dir_all(&tmp_dir).context("Failed to create temp dir for Dockerfile")?;

        let filename = format!("{}.Dockerfile", self.default_tag().replace('/', "_").replace(':', "_"));
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
    println!("{}", "╔══════════════════════════════════════════════════════════╗".yellow());
    println!("{}", "║       Docker Engine Not Found - Installation Guide      ║".yellow());
    println!("{}", "╚══════════════════════════════════════════════════════════╝".yellow());
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
        println!("{}", "WSL 2 Ubuntu Installation (alternative):".cyan().bold());
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
            println!("{}", "Or with Docker Desktop for Windows (WSL 2 backend):".cyan().bold());
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
) -> Result<()> {
    let dockerfile = config.resolve_dockerfile()?;

    let log = |text: &str, is_err: bool| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            is_error: is_err,
        });
    };

    log(&format!("🔨 Forge2K Build Engine v{}", env!("CARGO_PKG_VERSION")), false);
    log(&format!("   Method:    {}", config.method), false);
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
            return Ok(());
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
                    log(&format!("   docker run --rm -v $(pwd):/work {} cp2k --help", config.default_tag()), false);
                } else {
                    log(&format!("❌ Build failed with exit code: {:?}", status.code()), true);
                }
                return Ok(());
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
fn run_cmd_logged<S: AsRef<str> + std::fmt::Display>(
    cmd: &str,
    args: &[S],
    workdir: Option<&Path>,
    log_tx: &mpsc::Sender<LogLine>,
    cancel_flag: &Arc<Mutex<bool>>,
) -> Result<()> {
    let log = |text: &str, is_err: bool| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine { timestamp: ts, text: text.to_string(), is_error: is_err });
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
            if *cancel1.lock().unwrap() { break; }
            if let Ok(l) = line {
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx1.send(LogLine { timestamp: ts, text: l, is_error: false });
            }
        }
    });

    let tx2 = log_tx.clone();
    let cancel2 = cancel_flag.clone();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if *cancel2.lock().unwrap() { break; }
            if let Ok(l) = line {
                let is_err = l.to_lowercase().contains("error") || l.to_lowercase().contains("failed");
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx2.send(LogLine { timestamp: ts, text: l, is_error: is_err });
            }
        }
    });

    let status = child.wait().map_err(|e| anyhow!("Command '{}' wait error: {}", cmd, e))?;
    drop(stdout_thread);
    drop(stderr_thread);

    if !status.success() {
        return Err(anyhow!("Command '{}' failed with exit code {:?}", cmd, status.code()));
    }
    Ok(())
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
) -> Result<()> {
    let log = |text: &str, is_err: bool| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine { timestamp: ts, text: text.to_string(), is_error: is_err });
    };

    log(&format!("🔨 Forge2K Native Build v{}", env!("CARGO_PKG_VERSION")), false);
    log(&format!("   Method:    native"), false);
    log(&format!("   Version:   {}", config.version), false);
    log(&format!("   MPI:       {}", config.mpi), false);
    log(&format!("   CPU:       {}", config.cpu), false);
    log(&format!("   CUDA:      {}", config.cuda), false);
    log(&format!("   Variant:   {}", config.variant), false);
    log(&format!("   Jobs:      {}", config.jobs), false);
    log("", false);

    // ── Step 1: Check prerequisites ──
    log("📋 Step 1/6: Checking system prerequisites...", false);
    let required = ["gcc", "g++", "gfortran", "git", "make", "cmake", "wget", "bunzip2"];
    let mut missing: Vec<&str> = Vec::new();
    for tool in &required {
        if !check_prereq(tool) { missing.push(*tool); }
    }
    if !missing.is_empty() {
        log(&format!("   Missing: {}", missing.join(", ")), true);
        log("   Attempting to install missing packages...", false);
        run_cmd_logged("apt-get", &["update", "-qq"], None, &log_tx, &cancel_flag)?;
        let mut pkgs: Vec<String> = missing.iter().map(|s| s.to_string()).collect();
        for extra in &["autoconf", "autogen", "automake", "libtool", "libtool-bin", "ninja-build", "pkg-config", "python3-dev", "python3-pip", "xxd", "xz-utils", "zlib1g-dev"] {
            pkgs.push(extra.to_string());
        }
        let mut args: Vec<String> = vec!["install".into(), "-qq".into(), "--no-install-recommends".into(), "-y".into()];
        args.extend(pkgs.iter().cloned());
        let result = run_cmd_logged("apt-get", &args, None, &log_tx, &cancel_flag);
        if result.is_err() {
            log("   ⚠️  Some packages failed to install. Trying with sudo...", true);
            let missing_str = missing.join(" ");
            let sudo_args: Vec<String> = vec!["apt-get".into(), "install".into(), "-qq".into(), "-y".into(), missing_str];
            let _ = run_cmd_logged("sudo", &sudo_args, None, &log_tx, &cancel_flag);
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
        run_cmd_logged("git", &["-C", cp2k_dir.to_str().unwrap(), "pull"], None, &log_tx, &cancel_flag)?;
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
        run_cmd_logged("git", &git_args, None, &log_tx, &cancel_flag)?;
    }
    log("", false);

    // ── Step 4: Install toolchain dependencies ──
    log("📋 Step 4/6: Installing CP2K toolchain dependencies...", false);
    log("   This will download and compile many libraries (30-60 min)...", false);
    log("", false);

    let toolchain_dir = cp2k_dir.join("tools").join("toolchain");
    let toolchain_script = toolchain_dir.join("install_cp2k_toolchain.sh");
    if !toolchain_script.exists() {
        return Err(anyhow!("Toolchain script not found at {}", toolchain_script.display()));
    }

    let tc_script = toolchain_dir.join("install_cp2k_toolchain.sh");
    let tc_args: Vec<String> = vec![
        tc_script.to_string_lossy().into_owned(),
        "-j".into(), config.jobs.to_string(),
        "--install-all".into(),
        "--enable-cuda=no".into(), "--with-deepmd=no".into(),
        "--target-cpu=x86_64".into(),
        "--with-cusolvermp=no".into(),
        "--with-gcc=system".into(),
        "--with-mpich=system".into(),
    ];
    run_cmd_logged(
        "bash", &tc_args,
        Some(toolchain_dir.as_path()),
        &log_tx, &cancel_flag,
    ).map_err(|e| anyhow!("Toolchain installation failed: {}", e))?;
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
        run_cmd_logged("bash", &[build_sh.to_str().unwrap()], Some(cp2k_dir.as_path()), &log_tx, &cancel_flag)?;
    } else {
        // Legacy make approach
        let arch_dir = "local";
        // Find arch file
        let arch_file = toolchain_dir.join("install").join("arch").join(format!("{}.psmp", arch_dir));
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

        run_cmd_logged("bash", &[build_sh.to_str().unwrap()], Some(cp2k_dir.as_path()), &log_tx, &cancel_flag)?;
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
        let size = std::fs::metadata(&cp2k_binary).map(|m| m.len()).unwrap_or(0);
        log(&format!("   ✅ CP2K built successfully: {}", cp2k_binary.display()), false);
        log(&format!("   Binary size: {} MB", size / 1_048_576), false);
        log("", false);
        log("   🎉 To use CP2K, add to your PATH:", false);
        log(&format!("      export PATH={}:$PATH", cp2k_binary.parent().unwrap().display()), false);
    } else {
        log("   ⚠️  CP2K binary not found at expected location. Check build output above.", true);
    }

    log("", false);
    log("✅ Native build completed!", false);
    Ok(())
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
                if stderr.contains("timeout") || stderr.contains("refused") || stderr.contains("no route") {
                    NetworkStatus::Blocked(stderr)
                } else {
                    NetworkStatus::Unknown(stderr)
                }
            }
        }
        Err(e) => NetworkStatus::Unknown(e.to_string()),
    }
}

/// Common Docker registry mirrors
const REGISTRY_MIRRORS: &[&str] = &[
    "https://docker.mirrors.ustc.edu.cn",
    "https://mirror.ccs.tencentyun.com",
    "https://2a59f68c.m.daocloud.io",
    "https://registry.docker-cn.com",
    "https://dockerhub.timeweb.cloud",
];

/// Try to find a working registry mirror
pub fn detect_best_mirror() -> Option<String> {
    for mirror in REGISTRY_MIRRORS {
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
                return s == "True";
            }
            Err(_) => return false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let test_url = format!("{}/v2/_catalog", url.trim_end_matches('/'));
        let output = Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "--connect-timeout", "5", &test_url])
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
        let content = std::fs::read_to_string(&config_path)
            .context("Failed to read Docker daemon config")?;
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

    let content = serde_json::to_string_pretty(&config)
        .context("Failed to serialize config")?;
    std::fs::write(&config_path, &content)
        .context("Failed to write Docker daemon config")?;

    Ok(())
}

/// Remove registry mirror configuration
pub fn remove_registry_mirror() -> Result<()> {
    let config_path = daemon_config_path();

    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)
        .context("Failed to read Docker daemon config")?;
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

    config.get("registry-mirrors")?
        .as_array()?
        .first()?
        .as_str()
        .map(|s| s.to_string())
}

// ============================================================
// Configuration Listing
// ============================================================

#[derive(Debug, Clone)]
pub struct ConfigInfo {
    pub method: String,
    pub version: String,
    pub mpi: String,
    pub cpu: String,
    pub cuda: String,
    pub variant: String,
    pub _dockerfile: String,
    pub base_image: String,
    pub description: String,
}

/// List all available pre-configured build configurations
pub fn list_available_configs() -> Vec<ConfigInfo> {
    vec![
        ConfigInfo {
            method: "spack".into(),
            version: "2026.1".into(),
            mpi: "mpich".into(),
            cpu: "x86_64".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2026.1_mpich_x86_64_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K v2026.1 with Spack + MPICH, x86_64 (recommended)".into(),
        },
        ConfigInfo {
            method: "spack".into(),
            version: "2026.1".into(),
            mpi: "mpich".into(),
            cpu: "cascadelake".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2026.1_mpich_cascadelake_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K v2026.1 with MPICH, optimized for Cascade Lake CPUs".into(),
        },
        ConfigInfo {
            method: "spack".into(),
            version: "2026.1".into(),
            mpi: "openmpi".into(),
            cpu: "cascadelake".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2026.1_openmpi_cascadelake_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K v2026.1 with OpenMPI, optimized for Cascade Lake CPUs".into(),
        },
        ConfigInfo {
            method: "toolchain".into(),
            version: "master".into(),
            mpi: "mpich".into(),
            cpu: "generic".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "master_mpich_generic_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K latest master branch (bleeding edge)".into(),
        },
        ConfigInfo {
            method: "spack".into(),
            version: "2025.2".into(),
            mpi: "mpich".into(),
            cpu: "x86_64".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2025.2_mpich_x86_64_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K 2025.2 with MPICH, x86_64 (stable)".into(),
        },
        ConfigInfo {
            method: "spack".into(),
            version: "2025.2".into(),
            mpi: "mpich".into(),
            cpu: "cascadelake".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2025.2_mpich_cascadelake_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K 2025.2 with MPICH, optimized for Cascade Lake CPUs".into(),
        },
        ConfigInfo {
            method: "spack".into(),
            version: "2025.2".into(),
            mpi: "openmpi".into(),
            cpu: "cascadelake".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2025.2_openmpi_cascadelake_psmp.Dockerfile".into(),
            base_image: "ubuntu:24.04".into(),
            description: "CP2K 2025.2 with OpenMPI, optimized for Cascade Lake CPUs".into(),
        },
        ConfigInfo {
            method: "toolchain".into(),
            version: "2023.2".into(),
            mpi: "mpich".into(),
            cpu: "generic".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            _dockerfile: "2023.2_mpich_generic_psmp.Dockerfile".into(),
            base_image: "ubuntu:22.04".into(),
            description: "CP2K 2023.2 with MPICH, generic x86_64 (LTS)".into(),
        },
        ConfigInfo {
            method: "toolchain".into(),
            version: "2023.2".into(),
            mpi: "mpich".into(),
            cpu: "generic".into(),
            cuda: "V100".into(),
            variant: "psmp".into(),
            _dockerfile: "2023.2_mpich_generic_cuda_V100_psmp.Dockerfile".into(),
            base_image: "nvidia/cuda:12.2.0-devel-ubuntu22.04".into(),
            description: "CP2K 2023.2 with CUDA V100 GPU acceleration".into(),
        },
        ConfigInfo {
            method: "toolchain".into(),
            version: "2023.2".into(),
            mpi: "mpich".into(),
            cpu: "generic".into(),
            cuda: "P100".into(),
            variant: "psmp".into(),
            _dockerfile: "2023.2_mpich_generic_cuda_P100_psmp.Dockerfile".into(),
            base_image: "nvidia/cuda:12.2.0-devel-ubuntu22.04".into(),
            description: "CP2K 2023.2 with CUDA P100 GPU acceleration".into(),
        },
    ]
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

    let mpi_setup = match mpi.as_str() {
        "openmpi" => format!(
            r#"RUN sed -e '/^\s*mpi:/i\      require: target="{cpu}"' \
    -e 's/- mpich/- openmpi/' \
    -e '/^\s*xpmem:/i\    openmpi:\n      require:\n        - +internal-hwloc' \
    -e '/^\s*- "mpich@/ s/^ /#/' \
    -e '/^#\s*- "openmpi@/ s/^#/ /' \
    -i /opt/cp2k/tools/spack/cp2k_deps_all_${{CP2K_VERSION}}.yaml"#,
            cpu = cpu
        ),
        _ => format!(
            r#"RUN sed -e '/^\s*mpi:/i\      require: target="{cpu}"' \
    -i /opt/cp2k/tools/spack/cp2k_deps_all_${{CP2K_VERSION}}.yaml"#,
            cpu = cpu
        ),
    };

    Ok(format!(
        r#"#
# Dockerfile generated by Forge2K
# Inspired by github.com/cp2k/cp2k-containers
#

FROM ubuntu:24.04 AS build_cp2k

RUN apt-get update -qq && apt-get install -qq --no-install-recommends \
    g++ gcc gfortran python3 autoconf automake bzip2 ca-certificates cmake git \
    less libncurses-dev libssh-dev libssl-dev libtool-bin lsb-release make \
    ninja-build openssh-client patch pkgconf python3-dev python3-pip \
    python3-venv unzip wget xxd xz-utils zlib1g-dev zstd \
    && rm -rf /var/lib/apt/lists/*

{git_clone}

ARG NUM_PROCS=16
ENV NUM_PROCS=${{NUM_PROCS}}
ARG SPACK_VERSION=1.0.0
ARG SPACK_PACKAGES_VERSION=2025.07.0
ENV SPACK_VERSION=${{SPACK_VERSION}}
ENV SPACK_PACKAGES_VERSION=${{SPACK_PACKAGES_VERSION}}

RUN mkdir -p /opt/spack-${{SPACK_VERSION}} && \
    wget -q https://github.com/spack/spack/archive/v${{SPACK_VERSION}}.tar.gz && \
    tar -xzf v${{SPACK_VERSION}}.tar.gz -C /opt && \
    rm -f v${{SPACK_VERSION}}.tar.gz && \
    mkdir -p /opt/spack-packages-${{SPACK_PACKAGES_VERSION}} && \
    wget -q https://github.com/spack/spack-packages/archive/v${{SPACK_PACKAGES_VERSION}}.tar.gz && \
    tar -xzf v${{SPACK_PACKAGES_VERSION}}.tar.gz -C /opt && \
    rm -f v${{SPACK_PACKAGES_VERSION}}.tar.gz

ENV PATH="/opt/spack-${{SPACK_VERSION}}/bin:${{PATH}}"
RUN spack repo add --scope site /opt/spack-packages-${{SPACK_PACKAGES_VERSION}}/repos/spack_repo/builtin
RUN spack compiler find
RUN spack external find --all --not-buildable

ARG CP2K_VERSION=psmp
ENV CP2K_VERSION=${{CP2K_VERSION}}

RUN cp -a /opt/cp2k/tools/spack/cp2k_dev_repo /opt/spack-packages-${{SPACK_PACKAGES_VERSION}}/repos/spack_repo && \
    spack repo add --scope site /opt/spack-packages-${{SPACK_PACKAGES_VERSION}}/repos/spack_repo/cp2k_dev_repo

{mpi_setup}

RUN cat /opt/cp2k/tools/spack/cp2k_deps_all_${{CP2K_VERSION}}.yaml && \
    spack env create myenv /opt/cp2k/tools/spack/cp2k_deps_all_${{CP2K_VERSION}}.yaml

RUN spack -e myenv concretize -f
ENV SPACK_ENV_VIEW="/opt/spack-${{SPACK_VERSION}}/var/spack/environments/myenv/spack-env/view"
RUN spack -e myenv env depfile -o spack_makefile && \
    make -j${{NUM_PROCS}} -f spack_makefile

WORKDIR /opt/cp2k
RUN cp /opt/cp2k/tools/spack/spack_env_relocate.sh . && \
    cp /opt/spack-packages-${{SPACK_PACKAGES_VERSION}}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/*.patch . && \
    cp /opt/spack-packages-${{SPACK_PACKAGES_VERSION}}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/*.sh . && \
    bash -c "source /opt/spack-${{SPACK_VERSION}}/share/spack/setup-env.sh && \
             spack env activate myenv && \
             spack build-env -- spack install --source cp2k@${{CP2K_VERSION}}"

FROM ubuntu:24.04 AS runtime
RUN apt-get update -qq && apt-get install -qq --no-install-recommends \
    g++ gcc gfortran ca-certificates libgomp1 libopenblas-dev \
    libmpich-dev libpython3-dev libstdc++-13-dev python3 python3-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build_cp2k /opt/cp2k/install /opt/cp2k
COPY --from=build_cp2k /opt/cp2k/exe /opt/cp2k/exe
COPY --from=build_cp2k /opt/cp2k/data /opt/cp2k/data
COPY --from=build_cp2k /opt/cp2k/tests /opt/cp2k/tests

RUN ln -sf /opt/cp2k/exe/local/cp2k.psmp /usr/local/bin/cp2k
ENV PATH="/opt/cp2k/exe/local:${{PATH}}"
ENV LD_LIBRARY_PATH="/opt/cp2k/lib:${{LD_LIBRARY_PATH}}"
WORKDIR /work
ENTRYPOINT ["cp2k"]
"#,
        mpi_setup = mpi_setup,
        git_clone = git_clone,
    ))
}

fn generate_toolchain_dockerfile(config: &BuildConfig) -> Result<String> {
    let cuda_enabled = config.cuda != "none";
    let gpu_ver = if config.cuda == "none" { "no" } else { &config.cuda };
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
        format!("RUN git clone --recursive -b support/v{} https://github.com/cp2k/cp2k.git /opt/cp2k", config.version)
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
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#.to_string()
    } else {
        // Tagged releases use old make approach with arch files
        format!(r#"WORKDIR /opt/cp2k
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
                arch = arch_dir)
    };

    let copy_step: String = if use_cmake {
        "COPY --from=build /opt/cp2k/install/ /opt/cp2k/install/\n\
         COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/\n\
         COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/".into()
    } else {
        format!("COPY --from=build /opt/cp2k/exe/{arch}/ /opt/cp2k/exe/{arch}/\n\
                 COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/\n\
                 COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/",
                arch = arch_dir)
    };

    let link_step: String = if use_cmake {
        r#"RUN ln -sf /opt/cp2k/install/bin/cp2k.psmp /usr/local/bin/cp2k && \
    ln -sf /opt/cp2k/install/bin/cp2k_shell.psmp /usr/local/bin/cp2k_shell && \
    ln -sf /opt/cp2k/install/bin/cp2k.popt /usr/local/bin/cp2k.popt
ENV PATH="/opt/cp2k/install/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/cp2k/install/lib:${LD_LIBRARY_PATH}""#.to_string()
    } else {
        format!(r#"RUN for binary in cp2k dumpdcd graph xyz2dcd; do \
        ln -sf /opt/cp2k/exe/{arch}/${{binary}}.psmp /usr/local/bin/${{binary}}; \
    done && \
    ln -sf /opt/cp2k/exe/{arch}/cp2k.psmp /usr/local/bin/cp2k_shell
ENV PATH="/opt/cp2k/exe/{arch}:${{PATH}}"
ENV LD_LIBRARY_PATH="/opt/cp2k/tools/toolchain/install/lib:${{LD_LIBRARY_PATH}}""#,
                arch = arch_dir)
    };

    Ok(format!(
        r#"#
# Dockerfile generated by Forge2K
# Inspired by github.com/cp2k/cp2k-containers
#

FROM {base_image} AS build

{cuda_env}

RUN apt-get update -qq && apt-get install -qq --no-install-recommends \
    autoconf autogen automake autotools-dev \
    bzip2 ca-certificates \
    g++ gcc gfortran git less libtool libtool-bin \
    libmpich-dev make mpich ninja-build openssh-client patch \
    pkg-config python3 python3-dev python3-pip \
    unzip wget xxd xz-utils zlib1g-dev

{git_clone}

WORKDIR /opt/cp2k/tools/toolchain
RUN ./install_cp2k_toolchain.sh -j ${{NUM_PROCS:-8}} \
    --install-all \
    --enable-cuda={cuda_flag} {cuda_extra} --with-deepmd=no \
    --target-cpu={cpu} \
    --with-cusolvermp=no \
    --with-gcc=system \
    --with-mpich=system

{build_step}

FROM {base_image} AS install
RUN apt-get update -qq && apt-get install -qq --no-install-recommends \
    g++ gcc gfortran libmpich-dev mpich openssh-client python3 \
    && rm -rf /var/lib/apt/lists/*

{copy_step}

{link_step}

WORKDIR /work
ENTRYPOINT ["cp2k"]
"#,
        base_image = base_image,
        cuda_env = if cuda_enabled {
            "ENV CUDA_PATH /usr/local/cuda\nENV LD_LIBRARY_PATH /usr/local/cuda/lib64\nENV CUDA_CACHE_DISABLE 1"
        } else {
            ""
        },
        cuda_flag = cuda_flag,
        cuda_extra = cuda_extra,
        cpu = config.cpu,
        git_clone = git_clone,
        build_step = build_step,
        copy_step = copy_step,
        link_step = link_step,
    ))
}
