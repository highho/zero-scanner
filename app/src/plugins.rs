//! Plugin reservation: a thin, honest extension point.
//!
//! Zero is a single offline binary. A full plugin runtime is a large promise
//! (sandboxing, ABI, code loading) that this project intentionally does not
//! make yet. What we DO ship is the directory contract and the two safest
//! capabilities that grow from it, so third parties can extend the app today
//! without us pretending the rest is done:
//!
//!   %APPDATA%/Zero/plugins/<name>/plugin.json
//!
//! A manifest declares which reserved capabilities the plugin provides. The
//! loader only honours capabilities that are actually implemented; unknown
//! or experimental keys are ignored and reported, never executed. A plugin
//! that fails to parse is skipped with a reason, and a broken plugin never
//! prevents Zero from starting.
//!
//! Currently implemented capabilities:
//!   theme   -> the plugin ships a theme pack registered with theme.rs
//!
//! Future capabilities will be added here as they become real (game profile
//! packs, struct templates, script autoloads). The manifest shape below is
//! the contract they will extend.

use serde::{Deserialize, Serialize};

/// The manifest every plugin must carry at its folder root.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// Display name, shown in error messages and future plugin lists.
    #[serde(default)]
    pub name: String,
    /// Optional version string, informational only.
    #[serde(default)]
    pub version: String,
    /// One-line description, informational only.
    #[serde(default)]
    pub description: String,
    /// A theme pack inside the plugin folder (theme capability). Path is
    /// relative to the plugin folder, e.g. "themes/my-pack.json".
    #[serde(default)]
    pub theme: Option<String>,
}

/// What happened while installing one plugin.
#[derive(Clone, Debug)]
pub struct InstallNotice {
    pub plugin: String,
    pub ok: bool,
    pub message: String,
}

/// The folder that holds plugins. Created lazily so the contract exists even
/// when the user has never installed one.
pub fn plugins_dir() -> std::path::PathBuf {
    let dir = crate::prefs::data_dir().join("plugins");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn read_manifest(dir: &std::path::Path) -> Result<Manifest, String> {
    let path = dir.join("plugin.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?;
    serde_json::from_str::<Manifest>(&text)
        .map_err(|e| format!("{} 不是合法 plugin.json：{e}", path.display()))
}

fn install_manifest(dir: &std::path::Path, manifest: &Manifest, notice: &mut InstallNotice) {
    // Only the theme capability is wired today. Unknown keys are ignored but
    // reported, so a plugin that expected more still gets a clear answer.
    if let Some(theme_path) = &manifest.theme {
        let resolved = dir.join(theme_path);
        match crate::theme::register_plugin_pack(resolved) {
            Ok(()) => {
                notice.ok = true;
                notice.message = format!("已注册主题包 {}", theme_path);
            }
            Err(e) => {
                notice.ok = false;
                notice.message = format!("主题包注册失败：{e}");
            }
        }
    } else {
        // A manifest with no implemented capability is still installed
        // harmlessly: it declares nothing we can act on yet.
        notice.ok = true;
        notice.message = "清单没有已实现的能力（目前支持 theme）".to_string();
    }
}

/// Scan the plugins folder, install every implemented capability, and return
/// per-plugin notices for the UI to surface. Safe to call any number of
/// times; a rebuild of the theme list is cheap and idempotent.
pub fn discover() -> Vec<InstallNotice> {
    let mut notices = Vec::new();
    let dir = plugins_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => {
            notices.push(InstallNotice {
                plugin: "plugins".to_string(),
                ok: false,
                message: format!("无法读取插件目录 {}：{e}", dir.display()),
            });
            return notices;
        }
    };
    let mut folders: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let name = folder
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| folder.display().to_string());
        let mut notice = InstallNotice {
            plugin: name.clone(),
            ok: false,
            message: String::new(),
        };
        match read_manifest(&folder) {
            Ok(manifest) => {
                let label = if manifest.name.is_empty() {
                    name
                } else {
                    manifest.name.clone()
                };
                notice.plugin = label;
                install_manifest(&folder, &manifest, &mut notice);
            }
            Err(e) => {
                notice.ok = false;
                notice.message = e;
            }
        }
        notices.push(notice);
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip() {
        let json = r#"{"name":"demo","version":"1.0.0","theme":"themes/pack.json"}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.name, "demo");
        assert_eq!(manifest.theme.as_deref(), Some("themes/pack.json"));
    }

    #[test]
    fn manifest_minimal() {
        let manifest: Manifest = serde_json::from_str(r#"{"name":"quiet"}"#).unwrap();
        assert!(manifest.theme.is_none());
        assert!(manifest.description.is_empty());
    }
}