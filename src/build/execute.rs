use super::{BuildConfig, BuildOutcome, LogLevel, LogLine};
use anyhow::{anyhow, Context, Result};
use chrono::Local;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

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

/// CUDA options the GUI offers for a build method — the single source of
/// truth for the method→CUDA support matrix, kept next to (and consistent
/// with) the Step 0 validation inside [`execute_native_build`] and with
/// the bundled dockerfiles/ layout:
///
/// - `"spack"`: bundled spack images are CPU-only → `none`
/// - `"toolchain"`: bundled CUDA variants exist for P100 and V100
/// - `"native"`: the host build accepts the same CUDA set as the bundled
///   toolchain matrix (CUDA produces the `local_cuda` arch, matching the
///   bundled CUDA Dockerfiles); `master` + CUDA is rejected by Step 0
///   (the master branch builds via cmake, which carries no CUDA flags)
/// - anything unknown: empty set, which the GUI renders as a disabled
///   control
pub fn cuda_options_for_method(method: &str) -> Vec<&'static str> {
    match method {
        "spack" => vec!["none"],
        "toolchain" | "native" => vec!["none", "P100", "V100"],
        _ => Vec::new(),
    }
}

// ============================================================
// Native Build — Pure Configuration Helpers (no filesystem/process I/O)
// ============================================================

/// Support-set fragment reused by the Step 0 rejection message: the native
/// path's 6 supported combinations.
const NATIVE_SUPPORTED_SET: &str =
    "cpu in {'generic', 'x86_64'}, mpi='mpich', variant='psmp', cuda in {'none', 'P100', 'V100'}";

/// Validate a native (host) build configuration against the supported
/// combination whitelist. Pure: no filesystem/process access, and the
/// version is an explicit parameter (not judged from call-site state), so
/// the version dimension is unit-testable.
///
/// Supported: mpi='mpich' && variant='psmp' && cpu in {'generic','x86_64'}
/// && cuda in {'none','P100','V100'} (6 combinations) for any release
/// version. `version='master'` additionally requires cuda='none': the
/// master branch builds via cmake in this path, which carries no CUDA
/// flags — an explicit rejection beats a silently misconfigured build.
pub fn validate_native_config(
    version: &str,
    cpu: &str,
    mpi: &str,
    variant: &str,
    cuda: &str,
) -> Result<(), String> {
    let combination_supported = mpi == "mpich"
        && variant == "psmp"
        && matches!(cpu, "generic" | "x86_64")
        && matches!(cuda, "none" | "P100" | "V100");
    if combination_supported && !(version == "master" && cuda != "none") {
        return Ok(());
    }
    Err(format!(
        "Unsupported native build configuration: version='{}', cpu='{}', mpi='{}', variant='{}', cuda='{}'. \
         The native method supports: {}. \
         'master' additionally requires cuda='none' (the master branch builds via cmake in this path). \
         Use the Spack (Docker) method for other combinations.",
        version, cpu, mpi, variant, cuda, NATIVE_SUPPORTED_SET
    ))
}

/// The `install_cp2k_toolchain.sh` argument vector (excluding the script
/// path) for a native configuration. The flag surface is aligned with the
/// bundled toolchain Dockerfiles — the reference for the flags the 2023.2
/// script accepts (`dockerfiles/toolchain/2023.2_mpich_generic_psmp.Dockerfile`
/// lines 23-29 and its `_cuda_P100_`/`_cuda_V100_` siblings, line 33):
///
/// - cuda='none'         → `--enable-cuda=no`
/// - cuda='P100'/'V100'  → `--enable-cuda=yes --gpu-ver=<cuda> --with-libtorch=no`
/// - `--target-cpu=<cpu>` from the configuration
///
/// `--with-deepmd=no` is deliberately absent: the 2023.2 toolchain script
/// rejects it with "Unknown flag" (runner probe run 38057079002), and the
/// bundled Dockerfiles never pass it either.
pub(crate) fn native_toolchain_args(config: &BuildConfig) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-j".to_string(),
        config.jobs.to_string(),
        "--install-all".to_string(),
    ];
    if config.cuda == "none" {
        args.push("--enable-cuda=no".to_string());
    } else {
        args.push("--enable-cuda=yes".to_string());
        args.push(format!("--gpu-ver={}", config.cuda));
        args.push("--with-libtorch=no".to_string());
    }
    args.push(format!("--target-cpu={}", config.cpu));
    args.push("--with-cusolvermp=no".to_string());
    args.push("--with-gcc=system".to_string());
    args.push("--with-mpich=system".to_string());
    args
}

/// Arch name of the toolchain-generated arch file, which also names the
/// `exe/<arch>/` output directory: cuda='none' → `local`, any CUDA value →
/// `local_cuda`. Same names the bundled Dockerfiles copy (generic
/// Dockerfile line 38 vs CUDA Dockerfile lines 46-48); the name does not
/// depend on the CPU target.
pub(crate) fn native_arch_name(cuda: &str) -> &'static str {
    if cuda == "none" {
        "local"
    } else {
        "local_cuda"
    }
}

// ============================================================
// Docker Build Execution
// ============================================================

/// Execute a docker build and send log lines through the channel
pub fn execute_build(
    config: &BuildConfig,
    log_tx: mpsc::Sender<LogLine>,
    cancel_flag: Arc<Mutex<bool>>,
) -> Result<BuildOutcome> {
    let dockerfile = config.resolve_dockerfile()?;

    let log = |text: &str, level: LogLevel| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            level,
        });
    };

    log(
        &format!("🔨 Forge2K Build Engine v{}", env!("CARGO_PKG_VERSION")),
        LogLevel::Info,
    );
    log(&format!("   Method:    {}", config.method), LogLevel::Info);
    if config.dockerfile.is_none() && config.find_bundled_dockerfile().is_none() {
        log("ℹ️ No bundled Dockerfile matches this configuration; using a SYNTHESIZED Dockerfile generated at runtime.", LogLevel::Info);
    }
    log(&format!("   Version:   {}", config.version), LogLevel::Info);
    log(&format!("   MPI:       {}", config.mpi), LogLevel::Info);
    log(&format!("   CPU:       {}", config.cpu), LogLevel::Info);
    log(&format!("   CUDA:      {}", config.cuda), LogLevel::Info);
    log(&format!("   Variant:   {}", config.variant), LogLevel::Info);
    log(&format!("   Jobs:      {}", config.jobs), LogLevel::Info);
    log(
        &format!("   Tag:       {}", config.default_tag()),
        LogLevel::Info,
    );
    log(
        &format!("   Dockerfile: {}", dockerfile.display()),
        LogLevel::Info,
    );
    log("", LogLevel::Info);
    log(
        "🚀 Starting build (this may take 1-3 hours)...",
        LogLevel::Info,
    );
    log("", LogLevel::Info);

    let args = config.build_docker_args(&dockerfile);
    log(&format!("$ docker {}", args.join(" ")), LogLevel::Info);
    log("", LogLevel::Info);

    let mut child = Command::new("docker")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to launch docker build: {}", e))?;

    // Invariant: spawn() above used Stdio::piped() for both pipes on a
    // fresh child, so take() cannot return None here (unrecoverable).
    let stdout = child
        .stdout
        .take()
        .expect("child stdout was piped at spawn");
    let stderr = child
        .stderr
        .take()
        .expect("child stderr was piped at spawn");

    // Read stdout in a thread
    let tx_stdout = log_tx.clone();
    let cancel_stdout = cancel_flag.clone();
    let stdout_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if *cancel_stdout.lock().unwrap_or_else(PoisonError::into_inner) {
                break;
            }
            if let Ok(line) = line {
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx_stdout.send(LogLine {
                    timestamp: ts,
                    text: line,
                    level: LogLevel::Info,
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
            if *cancel_stderr.lock().unwrap_or_else(PoisonError::into_inner) {
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
                    level: if is_err {
                        LogLevel::Error
                    } else {
                        LogLevel::Info
                    },
                });
            }
        }
    });

    // Wait for completion or cancellation
    loop {
        if *cancel_flag.lock().unwrap_or_else(PoisonError::into_inner) {
            let _ = child.kill();
            log("⛔ Build cancelled by user.", LogLevel::Error);
            return Ok(BuildOutcome::Cancelled);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                drop(stdout_thread);
                drop(stderr_thread);
                log("", LogLevel::Info);
                if status.success() {
                    log("✅ Build completed successfully!", LogLevel::Info);
                    log(
                        &format!("   Image: {}", config.default_tag()),
                        LogLevel::Info,
                    );
                    log("", LogLevel::Info);
                    log("💡 Run with:", LogLevel::Info);
                    log(
                        &format!(
                            "   docker run --rm -v $(pwd):/work {} cp2k --help",
                            config.default_tag()
                        ),
                        LogLevel::Info,
                    );
                    return Ok(BuildOutcome::Success);
                } else {
                    log(
                        &format!("❌ Build failed with exit code: {:?}", status.code()),
                        LogLevel::Error,
                    );
                    return Ok(BuildOutcome::Failed);
                }
            }
            Ok(None) => {
                // Still running, check cancel flag periodically
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                log(
                    &format!("⚠️ Error waiting for build: {}", e),
                    LogLevel::Error,
                );
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
pub(crate) enum CmdStatus {
    Completed,
    Cancelled,
}

pub(crate) fn run_cmd_logged<S: AsRef<str> + std::fmt::Display>(
    cmd: &str,
    args: &[S],
    workdir: Option<&Path>,
    log_tx: &mpsc::Sender<LogLine>,
    cancel_flag: &Arc<Mutex<bool>>,
) -> Result<CmdStatus> {
    let log = |text: &str, level: LogLevel| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            level,
        });
    };

    let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
    log(&format!("$ {} {}", cmd, args_str.join(" ")), LogLevel::Info);

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

    // Invariant: spawn() above used Stdio::piped() for both pipes on a
    // fresh child, so take() cannot return None here (unrecoverable).
    let stdout = child
        .stdout
        .take()
        .expect("child stdout was piped at spawn");
    let stderr = child
        .stderr
        .take()
        .expect("child stderr was piped at spawn");

    let tx1 = log_tx.clone();
    let cancel1 = cancel_flag.clone();
    let stdout_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if *cancel1.lock().unwrap_or_else(PoisonError::into_inner) {
                break;
            }
            if let Ok(l) = line {
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx1.send(LogLine {
                    timestamp: ts,
                    text: l,
                    level: LogLevel::Info,
                });
            }
        }
    });

    let tx2 = log_tx.clone();
    let cancel2 = cancel_flag.clone();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if *cancel2.lock().unwrap_or_else(PoisonError::into_inner) {
                break;
            }
            if let Ok(l) = line {
                let is_err =
                    l.to_lowercase().contains("error") || l.to_lowercase().contains("failed");
                let ts = Local::now().format("%H:%M:%S").to_string();
                let _ = tx2.send(LogLine {
                    timestamp: ts,
                    text: l,
                    level: if is_err {
                        LogLevel::Error
                    } else {
                        LogLevel::Info
                    },
                });
            }
        }
    });

    // Poll with try_wait so cancellation can kill the child promptly;
    // log-reader threads keep draining pipes until EOF or cancel.
    loop {
        if *cancel_flag.lock().unwrap_or_else(PoisonError::into_inner) {
            let _ = child.kill();
            // Detach readers: pipes EOF when the child dies, but a
            // grandchild could hold them open, so never join here.
            drop(stdout_thread);
            drop(stderr_thread);
            log(&format!("⛔ '{}' cancelled by user.", cmd), LogLevel::Error);
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

/// Probe-only tool list for Step 1: each entry is checked with
/// `<tool> --version`. `mpicc` is probed because the toolchain runs with
/// `--with-mpich=system`; it must NOT be pasted into an apt install batch
/// (it is not an Ubuntu package name — see [`prereq_package_for`]).
pub(crate) const NATIVE_REQUIRED_TOOLS: &[&str] = &[
    "gcc", "g++", "gfortran", "git", "make", "cmake", "wget", "bunzip2", "mpicc",
];

/// Fixed extra apt packages installed alongside whatever the missing-tool
/// mapping resolves to. `mpich`/`libmpich-dev` mirror the bundled
/// toolchain Dockerfile package line (generic Dockerfile lines 13-14) so a
/// bare host gets the same MPI supply as the images; `libmpich-dev` is
/// also the provider of the probed `mpicc`.
pub(crate) const NATIVE_APT_EXTRAS: &[&str] = &[
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
    "mpich",
    "libmpich-dev",
];

/// Map a probe tool name to the Ubuntu package that provides it. Tools
/// whose name already is the package name map to themselves; `mpicc` is
/// provided by `libmpich-dev` and must never reach `apt-get install` as
/// the raw tool name.
pub(crate) fn prereq_package_for(tool: &str) -> &str {
    match tool {
        "mpicc" => "libmpich-dev",
        other => other,
    }
}

/// Build the apt package batch for the missing probe tools: each tool is
/// mapped to its provider package, the fixed extras are appended, and the
/// batch is deduplicated while preserving order. Pure (no I/O).
pub(crate) fn apt_install_packages(missing: &[&str]) -> Vec<String> {
    let mut pkgs: Vec<String> = Vec::new();
    for &tool in missing {
        let pkg = prereq_package_for(tool);
        if !pkgs.iter().any(|p| p == pkg) {
            pkgs.push(pkg.to_string());
        }
    }
    for &extra in NATIVE_APT_EXTRAS {
        if !pkgs.iter().any(|p| p == extra) {
            pkgs.push(extra.to_string());
        }
    }
    pkgs
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
    let log = |text: &str, level: LogLevel| {
        let ts = Local::now().format("%H:%M:%S").to_string();
        let _ = log_tx.send(LogLine {
            timestamp: ts,
            text: text.to_string(),
            level,
        });
    };

    log(
        &format!("🔨 Forge2K Native Build v{}", env!("CARGO_PKG_VERSION")),
        LogLevel::Info,
    );
    log("   Method:    native", LogLevel::Info);

    // ── Step 0: Validate the configuration combination ──
    // The native path supports a 6-combination whitelist (see
    // [`validate_native_config`]); anything else must fail explicitly
    // instead of silently ignoring the settings.
    if let Err(msg) = validate_native_config(
        &config.version,
        &config.cpu,
        &config.mpi,
        &config.variant,
        &config.cuda,
    ) {
        log(&format!("✗ {}", msg), LogLevel::Error);
        return Err(anyhow::anyhow!(msg));
    }
    log(
        "✓ Configuration combination supported by native build (6 combinations: cpu generic|x86_64, mpi mpich, variant psmp, cuda none|P100|V100; master requires cuda=none)",
        LogLevel::Info,
    );
    log(&format!("   Version:   {}", config.version), LogLevel::Info);
    log(&format!("   MPI:       {}", config.mpi), LogLevel::Info);
    log(&format!("   CPU:       {}", config.cpu), LogLevel::Info);
    log(&format!("   CUDA:      {}", config.cuda), LogLevel::Info);
    log(&format!("   Variant:   {}", config.variant), LogLevel::Info);
    log(&format!("   Jobs:      {}", config.jobs), LogLevel::Info);
    log("", LogLevel::Info);

    // ── Step 1: Check prerequisites ──
    log(
        "📋 Step 1/6: Checking system prerequisites...",
        LogLevel::Info,
    );
    let mut missing: Vec<&str> = Vec::new();
    for &tool in NATIVE_REQUIRED_TOOLS {
        if !check_prereq(tool) {
            missing.push(tool);
        }
    }
    if !missing.is_empty() {
        log(
            &format!("   Missing: {}", missing.join(", ")),
            LogLevel::Error,
        );
        log(
            "   Attempting to install missing packages...",
            LogLevel::Info,
        );
        // 'apt-get update' needs root on most hosts (CI runners included);
        // mirror the install fallback below instead of aborting the build
        // on a permission error.
        match run_cmd_logged("apt-get", &["update", "-qq"], None, &log_tx, &cancel_flag) {
            Ok(CmdStatus::Completed) => {}
            Ok(CmdStatus::Cancelled) => return Ok(BuildOutcome::Cancelled),
            Err(_) => {
                log(
                    "   ⚠️  'apt-get update' failed. Trying with sudo...",
                    LogLevel::Error,
                );
                if let Ok(CmdStatus::Cancelled) = run_cmd_logged(
                    "sudo",
                    &["apt-get", "update", "-qq"],
                    None,
                    &log_tx,
                    &cancel_flag,
                ) {
                    return Ok(BuildOutcome::Cancelled);
                }
            }
        }
        // Probe tool names are mapped to their provider packages first
        // (e.g. 'mpicc' -> 'libmpich-dev'), so a raw tool name can never
        // reach the apt batch.
        let pkgs = apt_install_packages(&missing);
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
                    LogLevel::Error,
                );
                // Same mapped, deduplicated package batch, one argument per
                // package (a joined single string would be one bogus name).
                let mut sudo_args: Vec<String> = vec![
                    "apt-get".into(),
                    "install".into(),
                    "-qq".into(),
                    "-y".into(),
                ];
                sudo_args.extend(pkgs.iter().cloned());
                if let Ok(CmdStatus::Cancelled) =
                    run_cmd_logged("sudo", &sudo_args, None, &log_tx, &cancel_flag)
                {
                    return Ok(BuildOutcome::Cancelled);
                }
            }
        }
    } else {
        log("   ✅ All required tools found", LogLevel::Info);
    }
    log("", LogLevel::Info);

    // ── Step 2: Create working directory ──
    log(
        "📋 Step 2/6: Setting up working directory...",
        LogLevel::Info,
    );
    let work_dir = PathBuf::from("/opt/cp2k_build");
    std::fs::create_dir_all(&work_dir).context("Failed to create /opt/cp2k_build")?;
    log(
        &format!("   Work dir: {}", work_dir.display()),
        LogLevel::Info,
    );
    log("", LogLevel::Info);

    // ── Step 3: Clone CP2K ──
    log("📋 Step 3/6: Cloning CP2K source...", LogLevel::Info);
    let cp2k_dir = work_dir.join("cp2k");
    if cp2k_dir.exists() {
        log(
            "   CP2K directory already exists, pulling latest...",
            LogLevel::Info,
        );
        run_step!(
            "git",
            // Hardcoded ASCII work-dir path: to_str() is always Some here.
            &[
                "-C",
                cp2k_dir
                    .to_str()
                    .expect("hardcoded ASCII work-dir is valid UTF-8"),
                "pull",
            ],
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
        git_args.push(
            cp2k_dir
                .to_str()
                .expect("hardcoded ASCII work-dir is valid UTF-8")
                .to_string(),
        );
        run_step!("git", &git_args, None, &log_tx, &cancel_flag);
    }
    log("", LogLevel::Info);

    // ── Step 4: Install toolchain dependencies ──
    log(
        "📋 Step 4/6: Installing CP2K toolchain dependencies...",
        LogLevel::Info,
    );
    log(
        "   This will download and compile many libraries (30-60 min)...",
        LogLevel::Info,
    );
    log("", LogLevel::Info);

    let toolchain_dir = cp2k_dir.join("tools").join("toolchain");
    let toolchain_script = toolchain_dir.join("install_cp2k_toolchain.sh");
    if !toolchain_script.exists() {
        return Err(anyhow!(
            "Toolchain script not found at {}",
            toolchain_script.display()
        ));
    }

    let tc_script = toolchain_dir.join("install_cp2k_toolchain.sh");
    let mut tc_args: Vec<String> = vec![tc_script.to_string_lossy().into_owned()];
    tc_args.extend(native_toolchain_args(config));
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
    log("", LogLevel::Info);

    // ── Step 5: Build CP2K ──
    log("📋 Step 5/6: Building CP2K...", LogLevel::Info);
    log("", LogLevel::Info);

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
            // Built below from the hardcoded ASCII work-dir: always UTF-8.
            &[build_sh
                .to_str()
                .expect("ASCII build-script path is valid UTF-8")],
            Some(cp2k_dir.as_path()),
            &log_tx,
            &cancel_flag
        );
    } else {
        // Legacy make approach
        let arch_dir = native_arch_name(&config.cuda);
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
            // Built below from the hardcoded ASCII work-dir: always UTF-8.
            &[build_sh
                .to_str()
                .expect("ASCII build-script path is valid UTF-8")],
            Some(cp2k_dir.as_path()),
            &log_tx,
            &cancel_flag
        );
    }
    log("", LogLevel::Info);

    // ── Step 6: Verify installation ──
    log("📋 Step 6/6: Verifying installation...", LogLevel::Info);
    let cp2k_binary = if use_cmake {
        PathBuf::from("/opt/cp2k/install/bin/cp2k.psmp")
    } else {
        cp2k_dir
            .join("exe")
            .join(native_arch_name(&config.cuda))
            .join("cp2k.psmp")
    };

    if cp2k_binary.exists() {
        let size = std::fs::metadata(&cp2k_binary)
            .map(|m| m.len())
            .unwrap_or(0);
        log(
            &format!("   ✅ CP2K built successfully: {}", cp2k_binary.display()),
            LogLevel::Info,
        );
        log(
            &format!("   Binary size: {} MB", size / 1_048_576),
            LogLevel::Info,
        );
        log("", LogLevel::Info);
        log("   🎉 To use CP2K, add to your PATH:", LogLevel::Info);
        log(
            &format!(
                "      export PATH={}:$PATH",
                // Absolute binary path always has a parent directory.
                cp2k_binary
                    .parent()
                    .expect("binary path has a parent directory")
                    .display()
            ),
            LogLevel::Info,
        );
    } else {
        log(
            "   ⚠️  CP2K binary not found at expected location. Check build output above.",
            LogLevel::Error,
        );
    }

    log("", LogLevel::Info);
    log("✅ Native build completed!", LogLevel::Info);
    Ok(BuildOutcome::Success)
}
