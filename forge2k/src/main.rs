// Windows subsystem: no black console window on desktop launch.
// CLI mode still works via AttachConsole to parent cmd process.
#![windows_subsystem = "windows"]

mod build;
mod gui;

use anyhow::Result;
use build::{check_docker, print_docker_install_guide, DockerStatus};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::sync::{Arc, Mutex};

// ============================================================
// CLI Definition
// ============================================================

#[derive(Parser)]
#[command(
    name = "forge2k",
    version = "1.0.0",
    about = "🔥 Forge2K - One-click CP2K Docker Image Builder",
    long_about = "Forge2K: A beautiful GUI+CLI tool for building CP2K Docker images.\n\
                   Supports Spack-based (v2025.2+), Toolchain-based (v2023.2+), and\n\
                   master branch builds. Auto-detects Docker engine, configures registry\n\
                   mirrors, and more.\n\n\
                   GitHub: github.com/cp2k/cp2k-containers",
    styles = clap::builder::Styles::styled()
        .header(clap::builder::styling::AnsiColor::Yellow.on_default())
        .usage(clap::builder::styling::AnsiColor::Green.on_default())
        .literal(clap::builder::styling::AnsiColor::Cyan.on_default()),
    after_help = "💡 Tip: Run 'forge2k gui' for the graphical interface!"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a CP2K Docker image (CLI mode)
    #[command(visible_alias = "b")]
    Build {
        /// Build method: spack (Docker, ubuntu:24.04), toolchain (Docker, ubuntu:22.04), native (direct on host)
        #[arg(short = 'm', long = "method", default_value = "spack", value_parser = clap::builder::PossibleValuesParser::new(["spack", "toolchain", "native"]))]
        method: String,

        /// CP2K version: 2026.1, 2025.2, 2023.2, or 'master' for latest HEAD
        #[arg(short = 'v', long = "version", default_value = "2026.1")]
        version: String,

        /// MPI implementation: mpich or openmpi
        #[arg(long = "mpi", default_value = "mpich", value_parser = clap::builder::PossibleValuesParser::new(["mpich", "openmpi"]))]
        mpi: String,

        /// CPU target: x86_64, generic, cascadelake, haswell, skylake-avx512
        #[arg(long = "cpu", default_value = "x86_64")]
        cpu: String,

        /// CUDA GPU: none, P100, V100
        #[arg(long = "cuda", default_value = "none", value_parser = clap::builder::PossibleValuesParser::new(["none", "P100", "V100"]))]
        cuda: String,

        /// CP2K binary variant: psmp, ssmp, pdbg, sdbg
        #[arg(long = "variant", default_value = "psmp", value_parser = clap::builder::PossibleValuesParser::new(["psmp", "ssmp", "pdbg", "sdbg"]))]
        variant: String,

        /// Number of parallel build jobs (default: auto-detect)
        #[arg(short = 'j', long = "jobs", default_value = "0")]
        jobs: u32,

        /// Custom Docker image tag
        #[arg(short = 't', long = "tag")]
        tag: Option<String>,

        /// Disable Docker build cache
        #[arg(long = "no-cache")]
        no_cache: bool,

        /// Path to custom Dockerfile (overrides automatic selection)
        #[arg(short = 'f', long = "dockerfile")]
        dockerfile: Option<String>,

        /// Shared memory size for Docker build
        #[arg(long = "shm-size", default_value = "1g")]
        shm_size: String,

        /// Output format: docker (local image) or image (tarball)
        #[arg(short = 'o', long = "output", default_value = "docker")]
        output: String,

        /// Skip Docker engine check
        #[arg(long = "force")]
        force: bool,
    },

    /// List all available pre-configured build configurations
    #[command(visible_alias = "l")]
    List {
        /// Filter by build method
        #[arg(long = "method", value_parser = clap::builder::PossibleValuesParser::new(["spack", "toolchain"]))]
        method: Option<String>,

        /// Filter by CP2K version
        #[arg(long = "version")]
        version: Option<String>,
    },

    /// Launch the graphical user interface
    #[command(visible_alias = "g")]
    Gui,

    /// Check system requirements (Docker, network, etc.)
    #[command(visible_alias = "c")]
    Check,

    /// Configure Docker registry mirror (for network issues)
    #[command(visible_alias = "m")]
    Mirror {
        /// Registry mirror URL (leave empty for auto-detect)
        url: Option<String>,

        /// Remove registry mirror configuration
        #[arg(long = "remove", short = 'r')]
        remove: bool,

        /// Restart Docker daemon after configuration
        #[arg(long = "restart")]
        restart: bool,
    },
}

// ============================================================
// Main Entry Point
// ============================================================

/// On Windows, attach to parent console for CLI output.
/// When double-clicked in Explorer (no parent console), fails silently → no console window.
/// When launched from cmd/powershell, attaches to existing console → CLI output works.
#[cfg(windows)]
fn attach_parent_console() {
    extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
        fn SetStdHandle(nStdHandle: u32, h: isize) -> i32;
    }
    use std::os::windows::io::IntoRawHandle;

    const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5;
    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4;

    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return; // No parent console → running from Explorer, just return silently
        }
    }

    // After AttachConsole, refresh stdout/stderr handles so println! works
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
        let handle = file.into_raw_handle() as isize;
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }
    }
}

fn main() -> Result<()> {
    // On Windows (release), try to attach parent console so CLI output works.
    // When double-clicked (Explorer), no parent → no console → GUI mode clean.
    #[cfg(windows)]
    attach_parent_console();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Gui);
    match command {
        Commands::Build {
            method,
            version,
            mpi,
            cpu,
            cuda,
            variant,
            jobs,
            tag,
            no_cache,
            dockerfile,
            shm_size,
            output,
            force,
        } => cmd_build(
            method, version, mpi, cpu, cuda, variant,
            jobs, tag, no_cache, dockerfile, shm_size, output, force,
        ),
        Commands::List { method, version } => cmd_list(method, version),
        Commands::Gui => cmd_gui(),
        Commands::Check => cmd_check(),
        Commands::Mirror { url, remove, restart } => cmd_mirror(url, remove, restart),
    }
}

// ============================================================
// Command Handlers
// ============================================================

fn cmd_build(
    method: String,
    version: String,
    mpi: String,
    cpu: String,
    cuda: String,
    variant: String,
    jobs: u32,
    tag: Option<String>,
    no_cache: bool,
    dockerfile: Option<String>,
    shm_size: String,
    output: String,
    force: bool,
) -> Result<()> {
    print_banner();

    // Check Docker
    if !force {
        match check_docker() {
            DockerStatus::Installed { version: ver, running: true } => {
                println!("{} Docker Engine: {} (running)", "✓".green(), ver.cyan());
            }
            DockerStatus::Installed { running: false, .. } => {
                println!("{} Docker Engine is installed but not running!", "✗".red());
                println!("  Please start Docker Desktop or the Docker daemon.");
                println!("  Run 'forge2k check' for more details.\n");
                if !ask_continue("Continue anyway?") {
                    return Ok(());
                }
            }
            DockerStatus::NotInstalled => {
                println!("{} Docker Engine is not installed!", "✗".red());
                print_docker_install_guide();
                return Ok(());
            }
            DockerStatus::NotRunning => {
                println!("{} Docker daemon is not running!", "✗".red());
                println!("  Please start Docker Desktop or restart the Docker daemon.\n");
                if !ask_continue("Continue anyway?") {
                    return Ok(());
                }
            }
        }
    }

    let num_jobs = if jobs == 0 {
        num_cpus() as u32
    } else {
        jobs
    };

    let config = build::BuildConfig {
        method,
        version,
        mpi,
        cpu,
        cuda,
        variant,
        jobs: num_jobs,
        tag: tag.unwrap_or_default(),
        no_cache,
        dockerfile: dockerfile.map(std::path::PathBuf::from),
        shm_size,
        _output: output,
    };

    println!("\n{}", "Build Configuration:".yellow().bold());
    println!("  Method:     {}", config.method.cyan());
    println!("  Version:    {}", config.version.cyan());
    println!("  MPI:        {}", config.mpi.cyan());
    println!("  CPU Target: {}", config.cpu.cyan());
    println!("  CUDA:       {}", config.cuda.cyan());
    println!("  Variant:    {}", config.variant.cyan());
    println!("  Jobs:       {}", config.jobs.to_string().cyan());
    println!("  Image Tag:  {}", config.default_tag().cyan());
    println!();

    if !ask_continue("Start build?") {
        println!("{} Build cancelled.", "⏹".yellow());
        return Ok(());
    }

    println!("\n{} Starting build...\n", "🚀".bold());

    let (tx, rx) = std::sync::mpsc::channel::<build::LogLine>();
    let cancel_flag: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let cancel_clone = cancel_flag.clone();

    let is_native = config.method == "native";
    let config_clone = config.clone();
    let build_thread = std::thread::spawn(move || {
        if is_native {
            build::execute_native_build(&config_clone, tx, cancel_clone)
        } else {
            build::execute_build(&config_clone, tx, cancel_clone)
        }
    });

    // Print log lines as they arrive
    for line in rx {
        let prefix = if line.is_error {
            format!("[{}]", line.timestamp).red().to_string()
        } else {
            format!("[{}]", line.timestamp).dimmed().to_string()
        };
        if line.is_error {
            println!("{} {}", prefix, line.text.red());
        } else {
            println!("{} {}", prefix, line.text);
        }
    }

    match build_thread.join() {
        Ok(Ok(())) => {
            println!("\n{} Build process finished.", "✅".bold());
            Ok(())
        }
        Ok(Err(e)) => {
            println!("\n{} Build failed: {}", "❌".red().bold(), e.to_string().red());
            std::process::exit(1);
        }
        Err(e) => {
            println!("\n{} Build thread panicked: {:?}", "💥".red().bold(), e);
            std::process::exit(1);
        }
    }
}

fn cmd_list(method_filter: Option<String>, version_filter: Option<String>) -> Result<()> {
    let configs = build::list_available_configs();

    let filtered: Vec<_> = configs
        .into_iter()
                .filter(|c| {
            method_filter.as_ref().map_or(true, |m| c.method.as_str() == m.as_str())
                && version_filter.as_ref().map_or(true, |v| c.version.as_str() == v.as_str())
        })
        .collect();

    if filtered.is_empty() {
        println!("{} No configurations match the given filters.", "ℹ".yellow());
        return Ok(());
    }

    println!();
    println!("{}", "╔══════════════════════════════════════════════════════════════════════════════════╗".bright_black());
    println!("{}", "║                           Available Build Configurations                        ║".yellow().bold());
    println!("{}", "╚══════════════════════════════════════════════════════════════════════════════════╝".bright_black());
    println!();

    for (i, config) in filtered.iter().enumerate() {
        let num = format!("#{}", i + 1);
        println!("  {}  {}", num.yellow().bold(), config.description.cyan());
        println!("      {} {}/{} ({})",
            "Method:".dimmed(),
            config.method,
            config.version,
            config.base_image,
        );
        println!("      {} {} / {} / {} / {}",
            "Specs:".dimmed(),
            format!("MPI={}", config.mpi).bright_blue(),
            format!("CPU={}", config.cpu).bright_blue(),
            format!("CUDA={}", config.cuda).bright_blue(),
            format!("Variant={}", config.variant).bright_blue(),
        );
        println!("      {} forge2k build -m {} -v {} --mpi {} --cpu {} --cuda {} --variant {}",
            "Build:".dimmed(),
            config.method,
            config.version,
            config.mpi,
            config.cpu,
            config.cuda,
            config.variant,
        );
        println!();
    }

    println!("  {} Run 'forge2k gui' for the interactive builder!", "💡".yellow());
    println!();
    Ok(())
}

fn cmd_gui() -> Result<()> {
    gui::run_gui()
}

fn cmd_check() -> Result<()> {
    println!();
    println!("{}", "╔══════════════════════════════════════════════════════════╗".bright_black());
    println!("{}", "║              Forge2K System Diagnostics                  ║".yellow().bold());
    println!("{}", "╚══════════════════════════════════════════════════════════╝".bright_black());
    println!();

    // Check Docker
    println!("{}", "── Docker Engine ──".cyan().bold());
    match check_docker() {
        DockerStatus::Installed { version, running: true } => {
            println!("  {} Docker is installed and running", "✓".green());
            println!("    Version: {}", version.cyan());
        }
        DockerStatus::Installed { running: false, .. } => {
            println!("  {} Docker is installed but NOT running", "✗".red());
            println!("    Start Docker Desktop or: sudo systemctl start docker");
        }
        DockerStatus::NotInstalled => {
            println!("  {} Docker is NOT installed", "✗".red());
            print_docker_install_guide();
        }
        DockerStatus::NotRunning => {
            println!("  {} Docker daemon is not reachable", "✗".red());
            println!("    Check if Docker Desktop is started or the daemon is running");
        }
    }
    println!();

    // Check Docker buildx
    println!("{}", "── Docker BuildKit ──".cyan().bold());
    let buildx = std::process::Command::new("docker")
        .args(["buildx", "version"])
        .output();
    match buildx {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            println!("  {} BuildKit available: {}", "✓".green(), ver.cyan());
        }
        _ => {
            println!("  {} BuildKit not detected (using legacy builder)", "ℹ".yellow());
        }
    }
    println!();

    // Check network & registry
    println!("{}", "── Docker Registry ──".cyan().bold());
    match build::check_registry() {
        build::NetworkStatus::Good => {
            println!("  {} Registry access is good (fast)", "✓".green());
        }
        build::NetworkStatus::Slow(t) => {
            println!("  {} Registry access is slow ({})", "⚠".yellow(), t.yellow());
        }
        build::NetworkStatus::Blocked(reason) => {
            println!("  {} Registry access appears blocked", "✗".red());
            println!("    Reason: {}", reason.dimmed());
            println!("    Suggested action: forge2k mirror --detect");
        }
        build::NetworkStatus::Unknown(reason) => {
            println!("  {} Registry status unknown: {}", "?".yellow(), reason.dimmed());
        }
    }

    // Check current mirror
    if let Some(mirror) = build::get_registry_mirror() {
        println!("  Current mirror: {}", mirror.cyan());
    }
    println!();

    // Check storage
    let docker_info = std::process::Command::new("docker")
        .args(["info", "--format", "{{.DockerRootDir}}"])
        .output();
    if let Ok(out) = docker_info {
        if out.status.success() {
            let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
            println!("{}", "── Docker Storage ──".cyan().bold());
            println!("  Root directory: {}", root.cyan());

            // Check disk space (platform-specific)
            #[cfg(target_os = "windows")]
            {
                let df = std::process::Command::new("wmic")
                    .args(["logicaldisk", "where", "drivetype=3", "get", "deviceid,freespace"])
                    .output();
                if let Ok(df) = df {
                    println!("  {}", String::from_utf8_lossy(&df.stdout).trim());
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let df = std::process::Command::new("df")
                    .args(["-h", &root])
                    .output();
                if let Ok(df) = df {
                    let out = String::from_utf8_lossy(&df.stdout);
                    for line in out.lines().skip(1) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 4 {
                            println!("  Available: {} (used: {} of {})",
                                parts[3].cyan(), parts[2], parts[1]);
                        }
                    }
                }
            }
            println!();
        }
    }

    // Summary
    println!("{}", "── Summary ──".cyan().bold());
    match check_docker() {
        DockerStatus::Installed { running: true, .. } => {
            println!("  {} System is ready for CP2K builds!", "✓".green());
            println!("  Run 'forge2k build' or 'forge2k gui' to get started.");
        }
        _ => {
            println!("  {} Please resolve the issues above before building.", "ℹ".yellow());
        }
    }
    println!();

    Ok(())
}

fn cmd_mirror(url: Option<String>, remove: bool, restart: bool) -> Result<()> {
    println!();

    if remove {
        println!("{} Removing registry mirror configuration...", "🗑".yellow());
        build::remove_registry_mirror()?;
        println!("{} Mirror configuration removed.", "✓".green());
        println!("  Restart Docker to apply changes.\n");
        return Ok(());
    }

    // Use provided URL or auto-detect
    let mirror_url = if let Some(url) = url {
        url
    } else {
        print!("{} Detecting best registry mirror... ", "🔍".bold());
        std::io::Write::flush(&mut std::io::stdout())?;
        match build::detect_best_mirror() {
            Some(url) => {
                println!("found: {}", url.cyan());
                url
            }
            None => {
                println!("{} no working mirror found.", "✗".red());
                println!("  Please provide a mirror URL manually.");
                println!("  Example: forge2k mirror https://docker.mirrors.ustc.edu.cn\n");
                println!("{} Known mirrors:", "ℹ".yellow());
                println!("  • https://docker.mirrors.ustc.edu.cn");
                println!("  • https://mirror.ccs.tencentyun.com");
                println!("  • https://2a59f68c.m.daocloud.io");
                println!("  • https://registry.docker-cn.com");
                return Ok(());
            }
        }
    };

    // Validate URL format
    if !mirror_url.starts_with("http://") && !mirror_url.starts_with("https://") {
        println!("{} Invalid mirror URL. Must start with http:// or https://", "✗".red());
        return Ok(());
    }

    println!("{} Configuring registry mirror: {}", "📝".bold(), mirror_url.cyan());
    build::set_registry_mirror(&mirror_url)?;
    println!("{} Configuration saved to: {}", "✓".green(), build::daemon_config_path().to_string_lossy().cyan());

    if restart {
        println!("{} Restarting Docker...", "🔄".bold());
        restart_docker()?;
        println!("{} Docker restarted successfully.", "✓".green());
    } else {
        println!("\n{} Restart Docker to apply changes:", "💡".yellow());
        #[cfg(target_os = "windows")]
        println!("  Restart Docker Desktop from the system tray");
        #[cfg(target_os = "linux")]
        println!("  sudo systemctl restart docker");
        println!();
    }

    Ok(())
}

// ============================================================
// Helper Functions
// ============================================================

fn print_banner() {
    println!();
    println!("{}", "╔══════════════════════════════════════╗".yellow());
    println!("{}", "║          Forge2K  v1.0.0            ║".yellow().bold());
    println!("{}", "║   CP2K Docker Image Builder         ║".yellow());
    println!("{}", "╚══════════════════════════════════════╝".yellow());
    println!();
}

fn ask_continue(prompt: &str) -> bool {
    println!("{} {} [Y/n] ", "?".yellow(), prompt);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn restart_docker() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        println!("  Stopping Docker Desktop...");
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "Docker Desktop.exe"])
            .output();
        std::thread::sleep(std::time::Duration::from_secs(3));
        println!("  Starting Docker Desktop...");
        let _ = std::process::Command::new("start")
            .arg("Docker Desktop")
            .spawn();
        println!("  Waiting for Docker to start...");
        std::thread::sleep(std::time::Duration::from_secs(10));
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = std::process::Command::new("sudo")
            .args(["systemctl", "restart", "docker"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to restart Docker: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            // Try without sudo (for rootless Docker)
            let output = std::process::Command::new("systemctl")
                .args(["--user", "restart", "docker"])
                .output()?;
            if output.status.success() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "Could not restart Docker. Please restart manually.\n  stderr: {}",
                    String::from_utf8_lossy(&output.stderr)
                ))
            }
        }
    }
}
