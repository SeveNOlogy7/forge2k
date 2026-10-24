use anyhow::Result;
use colored::Colorize;
use std::process::Command;

#[cfg(target_os = "linux")]
use std::path::Path;

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
// System Diagnostics: BuildKit & Docker Storage
// (shared by CLI `check` and the GUI System tab)
// ============================================================

/// Result of probing the Docker BuildKit (buildx) plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildKitStatus {
    /// `docker buildx version` answered; carries the trimmed stdout
    /// (a single line like `github.com/docker/buildx v0.17.1 ...`).
    Available { version: String },
    /// buildx missing, `docker` spawn failed, or the command failed.
    NotDetected,
}

/// Probe `docker buildx version` — the same command the CLI check's
/// BuildKit section runs. Every failure degrades to
/// [`BuildKitStatus::NotDetected`] (the caller renders guidance either
/// way); this never panics.
pub fn check_buildkit() -> BuildKitStatus {
    let output = Command::new("docker").args(["buildx", "version"]).output();
    match output {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if version.is_empty() {
                BuildKitStatus::NotDetected
            } else {
                BuildKitStatus::Available { version }
            }
        }
        _ => BuildKitStatus::NotDetected,
    }
}

/// One probed volume/filesystem entry of a disk-space check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEntry {
    /// Drive letter (`C:`) on Windows, device path (`/dev/sda1`) on Unix.
    pub volume: String,
    /// Human-readable total size ("118.0 GB"); empty when not reported.
    pub total: String,
    /// Human-readable used space; empty when not reported.
    pub used: String,
    /// Human-readable free space.
    pub free: String,
}

impl DiskEntry {
    /// The check-style summary line, e.g.
    /// `Available: 42.3 GB (used: 75.7 GB of 118.0 GB)`; without totals
    /// it degrades to `Available: <free>`.
    pub fn summary(&self) -> String {
        if self.used.is_empty() || self.total.is_empty() {
            format!("Available: {}", self.free)
        } else {
            format!(
                "Available: {} (used: {} of {})",
                self.free, self.used, self.total
            )
        }
    }
}

/// Outcome of the disk-space probe that accompanies `DockerRootDir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskSpaceStatus {
    /// At least one volume was probed successfully.
    Ok(Vec<DiskEntry>),
    /// The probe could not run or produced nothing parsable; carries a
    /// human-readable reason for display.
    Unknown(String),
}

/// Storage facts of the local Docker daemon: its root directory plus the
/// disk-space verdict for the volume hosting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerStorageStatus {
    /// `docker info --format {{.DockerRootDir}}`; `None` when the daemon
    /// is unreachable. The disk probe then falls back to every fixed
    /// volume (Windows) or degrades to `Unknown` (Unix, nothing to stat).
    pub root_dir: Option<String>,
    pub disk: DiskSpaceStatus,
}

/// Inspect Docker's storage location and the free disk space behind it.
/// Shared by CLI `check` and the GUI System tab. Never panics: every
/// spawn/parse failure degrades to a [`DiskSpaceStatus::Unknown`] reason.
pub fn check_docker_storage() -> DockerStorageStatus {
    let root_dir = Command::new("docker")
        .args(["info", "--format", "{{.DockerRootDir}}"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|root| !root.is_empty());

    let disk = probe_disk_space(root_dir.as_deref());
    DockerStorageStatus { root_dir, disk }
}

/// Disk-space probe behind [`check_docker_storage`], split out so the
/// fallback behavior for a missing/unusable root dir stays reviewable and
/// testable.
fn probe_disk_space(root_dir: Option<&str>) -> DiskSpaceStatus {
    #[cfg(target_os = "windows")]
    {
        // `wmic logicaldisk` is gone on Windows 11 24H2+ (build 26100+):
        // probe via PowerShell CIM instead (same `powershell -NoProfile`
        // precedent as registry.rs's mirror probe). Prefer the volume that
        // hosts root_dir; fall back to every fixed drive (DriveType=3) —
        // Docker Desktop's WSL backend reports a `/var/lib/docker` root
        // with no drive letter, which must still show something.
        let filter = match root_dir.and_then(drive_letter_of) {
            Some(drive) => format!("DeviceID='{}'", drive),
            None => "DriveType=3".to_string(),
        };
        let script = format!(
            "Get-CimInstance Win32_LogicalDisk -Filter \"{}\" | \
             ForEach-Object {{ \"$($_.DeviceID)|$($_.Size)|$($_.FreeSpace)\" }}",
            filter
        );
        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let entries = parse_cim_disk_lines(&String::from_utf8_lossy(&out.stdout));
                if entries.is_empty() {
                    DiskSpaceStatus::Unknown(
                        "PowerShell Get-CimInstance returned no fixed-drive space".into(),
                    )
                } else {
                    DiskSpaceStatus::Ok(entries)
                }
            }
            Ok(out) => DiskSpaceStatus::Unknown(format!(
                "PowerShell probe failed (exit {:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => DiskSpaceStatus::Unknown(format!("Could not run PowerShell: {}", e)),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let Some(root) = root_dir else {
            return DiskSpaceStatus::Unknown(
                "Docker root directory unknown (daemon not reachable); cannot check disk space"
                    .into(),
            );
        };
        let output = Command::new("df").args(["-h", root]).output();
        match output {
            Ok(out) if out.status.success() => {
                let entries = parse_df_output(&String::from_utf8_lossy(&out.stdout));
                if entries.is_empty() {
                    DiskSpaceStatus::Unknown(format!("df -h {} produced no parsable output", root))
                } else {
                    DiskSpaceStatus::Ok(entries)
                }
            }
            Ok(out) => DiskSpaceStatus::Unknown(format!(
                "df -h {} failed (exit {:?}): {}",
                root,
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => DiskSpaceStatus::Unknown(format!("Could not run df: {}", e)),
        }
    }
}

/// Extract the drive (`C:`) from a Windows-style root path. `None` for
/// anything else — including WSL-style `/var/lib/docker` roots.
#[cfg(target_os = "windows")]
pub(crate) fn drive_letter_of(root: &str) -> Option<String> {
    let bytes = root.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        Some(root[..2].to_ascii_uppercase())
    } else {
        None
    }
}

/// Parse the `DeviceID|Size|FreeSpace` lines the Windows CIM probe emits
/// (e.g. `C:|999871418368|326829953024`). Blank rows and rows without a
/// parsable free-space value are skipped; a missing/invalid size degrades
/// that entry's total/used to empty.
// Pure string parser that also runs on non-Windows (test support), where
// the CIM probe branch is compiled out.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn parse_cim_disk_lines(stdout: &str) -> Vec<DiskEntry> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(3, '|');
            let volume = parts.next()?.trim().to_string();
            let size = parts.next()?.trim();
            let free = parts.next()?.trim();
            if volume.is_empty() {
                return None;
            }
            let free_bytes: u64 = free.parse().ok()?;
            let (total, used) = match size.parse::<u64>() {
                Ok(size_bytes) if size_bytes >= free_bytes => (
                    format_bytes(size_bytes),
                    format_bytes(size_bytes - free_bytes),
                ),
                _ => (String::new(), String::new()),
            };
            Some(DiskEntry {
                volume,
                total,
                used,
                free: format_bytes(free_bytes),
            })
        })
        .collect()
}

/// Parse `df -h` output into entries (skips the header line). Column
/// order mirrors the CLI check's original inline parsing:
/// Filesystem / Size / Used / Avail.
// Pure string parser that also runs on Windows (test support), where the
// `df` probe branch is compiled out.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn parse_df_output(stdout: &str) -> Vec<DiskEntry> {
    stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                return None;
            }
            Some(DiskEntry {
                volume: parts[0].to_string(),
                total: parts[1].to_string(),
                used: parts[2].to_string(),
                free: parts[3].to_string(),
            })
        })
        .collect()
}

/// Format a byte count as a compact human-readable size ("42.3 GB").
pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Restart the Docker engine (CLI `forge2k mirror --restart`).
///
/// Long-blocking by design: on Windows this force-quits Docker Desktop,
/// relaunches it and polls up to 60s for the engine to answer; on other
/// platforms it shells out to `sudo systemctl restart docker` (falling back
/// to rootless `systemctl --user restart docker`). Consequences for
/// callers:
/// - never call this on a UI thread — the GUI runs it on a worker thread;
/// - a GUI process has no tty, so the `sudo` password prompt can never be
///   answered — non-Windows GUIs must show this as a manual command instead
///   of executing it.
pub fn restart_docker() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        println!("  Stopping Docker Desktop...");
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "Docker Desktop.exe"])
            .output();
        std::thread::sleep(std::time::Duration::from_secs(3));

        println!("  Starting Docker Desktop...");
        // "start" is a cmd builtin, not an executable: spawning it directly
        // always fails with a program-not-found OS error. Wrap it in
        // `cmd /C` and check the result so success/failure is visible.
        let exe = std::path::Path::new(r"C:\Program Files\Docker\Docker\Docker Desktop.exe");
        let launch = if exe.exists() {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", exe.to_str().unwrap_or("Docker Desktop")])
                .output()
        } else {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", "Docker Desktop"])
                .output()
        };
        match launch {
            Ok(out) if out.status.success() => {
                println!("  Docker Desktop launch requested.");
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(anyhow::anyhow!(
                    "Failed to launch Docker Desktop (exit code {:?}{}). \
                     Start it manually from the Start menu or system tray, then re-run this command.",
                    out.status.code(),
                    if stderr.is_empty() { String::new() } else { format!(": {}", stderr) }
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to launch Docker Desktop: {}. \
                     Start it manually from the Start menu or system tray, then re-run this command.",
                    e
                ));
            }
        }

        println!("  Waiting for Docker engine to come up...");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let up = std::process::Command::new("docker")
                .arg("version")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("Server:"))
                .unwrap_or(false);
            if up {
                println!("  Docker engine is up.");
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "Docker engine did not come up within 60s after launching Docker Desktop. \
                     Check that Docker Desktop is running and the engine has started, then re-run this command."
                ));
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
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
