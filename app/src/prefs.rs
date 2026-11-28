//! Small durable preferences: the active theme and other one-line settings.
//!
//! Kept separate from the workspace file: workspace state is per-directory
//! (sitting next to the exe), while preferences are per-user and live under
//! %APPDATA%\Zero, matching where the theme packs folder lives. A preference
//! that fails to load or save is never fatal; the app just falls back to the
//! default.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prefs {
    /// Active theme id; empty means the built-in default.
    #[serde(default)]
    pub theme: String,
    /// Version the user chose to skip. Empty means nothing was skipped.
    #[serde(default)]
    pub skipped_version: String,
    /// Whether to look for updates on startup. Defaults to on, so an older
    /// prefs.json that predates this field keeps the original behaviour.
    #[serde(default = "yes")]
    pub auto_update: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: String::new(),
            skipped_version: String::new(),
            auto_update: true,
        }
    }
}

/// The per-user data folder. The theme packs directory is created lazily by
/// the theme engine, so preferences must not depend on that folder existing.
///
/// `ZERO_DATA_DIR` overrides the location outright, which makes a portable
/// install possible and lets the self-test exercise the real read/write path
/// without touching the user's own settings.
pub fn data_dir() -> PathBuf {
    let dir = match std::env::var("ZERO_DATA_DIR") {
        Ok(custom) if !custom.trim().is_empty() => PathBuf::from(custom),
        _ => {
            let base = std::env::var("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            base.join("Zero")
        }
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn prefs_path() -> PathBuf {
    data_dir().join("prefs.json")
}

/// Load preferences. Any failure (missing file, corrupt JSON, unreadable
/// path) yields the defaults; the user must never be blocked by a settings
/// file.
pub fn load() -> Prefs {
    let path = prefs_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Prefs::default(),
    }
}

fn persist(prefs: &Prefs) {
    let path = prefs_path();
    if let Ok(text) = serde_json::to_string_pretty(prefs) {
        let _ = std::fs::write(path, text);
    }
}

/// Remember the active theme. Silently ignored on write failure: the theme
/// still applies for this session, it just may not survive a restart.
pub fn set_theme(id: &str) {
    let mut prefs = load();
    prefs.theme = id.to_string();
    persist(&prefs);
}

/// Remember (or clear, with an empty string) the version the user skipped.
pub fn set_skipped_version(version: &str) {
    let mut prefs = load();
    prefs.skipped_version = version.to_string();
    persist(&prefs);
}

/// Remember whether updates should be checked automatically on startup.
pub fn set_auto_update(enabled: bool) {
    let mut prefs = load();
    prefs.auto_update = enabled;
    persist(&prefs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_prefs_keep_auto_update_on() {
        let old: Prefs = serde_json::from_str(r#"{"theme":"neon"}"#).unwrap();
        assert_eq!(old.theme, "neon");
        assert!(old.auto_update, "缺失字段必须默认为开启");
        assert!(old.skipped_version.is_empty());
    }

    #[test]
    fn skipped_version_survives_round_trip() {
        let text = serde_json::to_string(&Prefs {
            theme: "ice".into(),
            skipped_version: "1.3.0".into(),
            auto_update: false,
        })
        .unwrap();
        let back: Prefs = serde_json::from_str(&text).unwrap();
        assert_eq!(back.skipped_version, "1.3.0");
        assert!(!back.auto_update);
    }
}
