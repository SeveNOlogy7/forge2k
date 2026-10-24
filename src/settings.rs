//! Persistent GUI settings stored at `<home>/.forge2k/settings.json`.
//!
//! The [`ForgeSettings`] fields mirror the user-editable inputs that must
//! survive a restart: the Settings tab's mirror draft (`mirror_input`,
//! `render_settings_tab` in gui.rs) and the Build tab's advanced options
//! (`no_cache` / `force` / `features` / `dockerfile`,
//! `render_advanced_options` in gui.rs) — nothing more, nothing invented.
//! Every field carries `#[serde(default)]`, so a settings.json written by
//! an older forge2k (carrying only `mirror_input`) loads without error and
//! the absent keys fall back to `Default`. Any problem while loading
//! (missing file or directory, unreadable or corrupt JSON, unresolvable
//! home directory) degrades to `Default`; saving problems are returned as
//! `Err` so the GUI can show them as a red settings message instead of
//! panicking.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The persisted GUI inputs: the Settings tab mirror draft plus the Build
/// tab's advanced options (CLI parity: `--no-cache`, `--force`,
/// `--features`, `-f/--dockerfile`).
///
/// (`current_mirror` is deliberately NOT here — it is Docker-daemon state
/// owned by daemon.json via `build::get_registry_mirror`; duplicating it
/// would create a second source of truth. `settings_message` and the
/// other transient UI feedback are equally not persisted.)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForgeSettings {
    /// Draft content of the "Mirror URL" text input (Settings tab).
    #[serde(default)]
    pub mirror_input: String,
    /// Build tab: pass `--no-cache` to `docker build`.
    #[serde(default)]
    pub no_cache: bool,
    /// Build tab: start the build even when the Docker pre-flight check
    /// would refuse (CLI `--force` parity).
    #[serde(default)]
    pub force: bool,
    /// Build tab: selected feature presets. The GUI only ever writes values
    /// from `build::KNOWN_FEATURES`; unknown values are filtered out when
    /// loading so a hand-edited settings.json cannot inject them.
    #[serde(default)]
    pub features: Vec<String>,
    /// Build tab: raw draft of the custom Dockerfile path; empty = none.
    /// Stored verbatim; existence is validated by the GUI on edit-commit.
    #[serde(default)]
    pub dockerfile: String,
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
            ..Default::default()
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

    #[test]
    fn load_old_settings_without_advanced_fields_degrades_to_defaults() {
        // Backward compat: a settings.json written before the advanced
        // Build-tab options existed carries only `mirror_input`; every
        // `#[serde(default)]` field must fall back silently instead of
        // failing the whole file.
        let dir = scratch("backward_compat");
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{ "mirror_input": "https://docker.mirrors.ustc.edu.cn" }"#,
        )
        .expect("write old-style settings");
        let loaded = ForgeSettings::load_from_path(&path).expect("old settings must load");
        assert_eq!(loaded.mirror_input, "https://docker.mirrors.ustc.edu.cn");
        assert!(!loaded.no_cache);
        assert!(!loaded.force);
        assert!(loaded.features.is_empty());
        assert!(loaded.dockerfile.is_empty());
    }

    #[test]
    fn saved_json_uses_stable_field_names() {
        // Forward-compat guard: the on-disk key is part of the public
        // format. A silent field rename would orphan every existing user's
        // settings.json (serde would fall back to the `#[serde(default)]`
        // empty string), so the key name itself is pinned here.
        let dir = scratch("field_names");
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let path = dir.join("settings.json");
        ForgeSettings {
            mirror_input: "https://mirror.example.com".into(),
            ..Default::default()
        }
        .save_to_path(&path)
        .expect("save must succeed");
        let raw = std::fs::read_to_string(&path).expect("read saved settings");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("saved file must be JSON");
        assert_eq!(
            value.get("mirror_input").and_then(|v| v.as_str()),
            Some("https://mirror.example.com"),
            "on-disk key 'mirror_input' must not be renamed (would orphan existing settings)"
        );
    }
}
