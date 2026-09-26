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
