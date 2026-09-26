use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

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
