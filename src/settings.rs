//! Persistent GUI settings stored at `<home>/.forge2k/settings.json`.
//!
//! The [`ForgeSettings`] fields mirror the user-editable items of the GUI
//! Settings tab (`render_settings_tab` in gui.rs) — nothing more, nothing
//! invented. Any problem while loading (missing file or directory,
//! unreadable or corrupt JSON, unresolvable home directory) degrades to
//! `Default`; saving problems are returned as `Err` so the GUI can show
//! them as a red settings message instead of panicking.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The one user-editable setting on the GUI Settings tab: the draft text
/// of the "Mirror URL" input box.
///
/// (`current_mirror` is deliberately NOT here — it is Docker-daemon state
/// owned by daemon.json via `build::get_registry_mirror`; duplicating it
/// would create a second source of truth. `settings_message` is transient
/// UI feedback and is equally not persisted.)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForgeSettings {
    /// Draft content of the "Mirror URL" text input (Settings tab).
    #[serde(default)]
    pub mirror_input: String,
}

/// Resolve the settings file location: `<home>/.forge2k/settings.json`,
/// with the home directory taken from USERPROFILE first, then HOME
/// (std::env only — no external path crates).
pub fn settings_path() -> Result<PathBuf> {
    let home = home_dir()
        .context("Cannot resolve the user home directory (USERPROFILE and HOME are both unset)")?;
    Ok(home.join(".forge2k").join("settings.json"))
}

/// Home directory via USERPROFILE, falling back to HOME. Empty values are
/// treated as unset.
fn home_dir() -> Result<PathBuf> {
    for var in ["USERPROFILE", "HOME"] {
        if let Ok(value) = std::env::var(var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed));
            }
        }
    }
    anyhow::bail!("Neither USERPROFILE nor HOME is set")
}

impl ForgeSettings {
    /// Load settings from the default location. Any problem (missing
    /// file/directory, unreadable or corrupt JSON, no home directory)
    /// degrades to `Default` — callers never see an error and never panic.
    pub fn load() -> Self {
        match settings_path() {
            Ok(path) => Self::load_from_path(&path).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Load from an explicit path. Missing or corrupt files are reported
    /// as `Err` (the caller decides how to degrade).
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read settings file {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("Corrupt settings file {}", path.display()))
    }

    /// Save settings to the default location, creating the `.forge2k`
    /// directory when missing. Failures (including an unresolvable home
    /// directory) are returned as `Err`.
    pub fn save(&self) -> Result<()> {
        let path = settings_path()?;
        self.save_to_path(&path)
    }

    /// Save to an explicit path, creating parent directories as needed.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Cannot create settings directory {}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("Cannot serialize settings")?;
        std::fs::write(path, json + "\n")
            .with_context(|| format!("Cannot write settings file {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test scratch directory (pid-suffixed; tests use distinct
    /// file names so they never collide even in parallel).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("forge2k_settings_test_{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_then_load_roundtrip_is_lossless() {
        let path = scratch("roundtrip").join("settings.json");
        let original = ForgeSettings {
            mirror_input: "https://docker.mirrors.ustc.edu.cn".into(),
        };
        original.save_to_path(&path).expect("save must succeed");
        let loaded = ForgeSettings::load_from_path(&path).expect("load must succeed");
        assert_eq!(
            loaded, original,
            "roundtrip must preserve every persisted field"
        );
    }

    #[test]
    fn load_missing_file_reports_error_so_callers_can_default() {
        let path = scratch("missing").join("settings.json");
        let result = ForgeSettings::load_from_path(&path);
        assert!(result.is_err(), "missing file must be Err, got {result:?}");
        // The public contract: load() degrades it to Default.
        // (load() itself resolves the real home; the degrade path is
        // `unwrap_or_default()` applied to this Err.)
        assert_eq!(result.unwrap_or_default(), ForgeSettings::default());
    }

    #[test]
    fn load_corrupt_json_reports_error_instead_of_panic() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not valid json !!").expect("write corrupt file");
        let result = ForgeSettings::load_from_path(&path);
        assert!(result.is_err(), "corrupt JSON must be Err, got {result:?}");
        assert_eq!(result.unwrap_or_default(), ForgeSettings::default());
    }

    #[test]
    fn save_creates_missing_directory() {
        let dir = scratch("mkdirs");
        assert!(!dir.exists(), "scratch dir must not pre-exist");
        let path = dir.join("nested").join("settings.json");
        ForgeSettings::default()
            .save_to_path(&path)
            .expect("save must create parent dirs");
        assert!(path.is_file(), "settings file must exist after save");
    }
}
