//! Small durable preferences: the active theme and other one-line settings.
//!
//! Kept separate from the workspace file: workspace state is per-directory
//! (sitting next to the exe), while preferences are per-user and live under
//! %APPDATA%\Zero, matching where the theme packs folder lives. A preference
//! that fails to load or save is never fatal; the app just falls back to the
//! default.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Prefs {
    /// Active theme id; empty means the built-in default.
    #[serde(default)]
    pub theme: String,
}

/// The per-user data folder. The theme packs directory is created lazily by
/// the theme engine, so preferences must not depend on that folder existing.
pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("Zero");
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
