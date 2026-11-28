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
    /// Struct layout files (struct_templates capability). Each path is
    /// relative to the plugin folder and holds one template JSON.
    #[serde(default)]
    pub struct_templates: Vec<String>,
    /// Game profile files (game_profiles capability). Each path is relative
    /// to the plugin folder and holds one profile JSON.
    #[serde(default)]
    pub game_profiles: Vec<String>,
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

/// One struct-template file as shipped by a plugin.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TemplateFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Module or symbol name the layout belongs to, e.g. "Player".
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub fields: Vec<TemplateFieldFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TemplateFieldFile {
    #[serde(default)]
    pub name: String,
    /// Offset as text: "0x1C" or "28".
    #[serde(default)]
    pub offset: String,
    /// Kind as text: "float", "int32", "pointer", ...
    #[serde(default)]
    pub r#type: String,
}

/// One game-profile file as shipped by a plugin.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Process image name this profile belongs to, e.g. "game.exe".
    #[serde(default)]
    pub process: String,
    #[serde(default)]
    pub scan: Option<ProfileScan>,
    /// Hotkeys the profile asks for. Keys the app cannot honour are reported
    /// as unsupported instead of silently doing nothing.
    #[serde(default)]
    pub hotkeys: Vec<ProfileHotkey>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileScan {
    #[serde(default)]
    pub value_type: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileHotkey {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub key: String,
}

fn install_manifest(dir: &std::path::Path, manifest: &Manifest, notice: &mut InstallNotice) {
    let mut done: Vec<String> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut unsupported: Vec<String> = Vec::new();

    if let Some(theme_path) = &manifest.theme {
        let resolved = dir.join(theme_path);
        match crate::theme::register_plugin_pack(resolved) {
            Ok(()) => done.push(format!("主题包 {}", theme_path)),
            Err(e) => problems.push(format!("主题包注册失败：{e}")),
        }
    }

    for relative in &manifest.struct_templates {
        match load_template(dir, relative) {
            Ok(template) => {
                let label = template.name.clone();
                match crate::structs::register_template(template) {
                    Ok(()) => done.push(format!("结构体模板 {label}")),
                    Err(e) => problems.push(e),
                }
            }
            Err(e) => problems.push(e),
        }
    }

    for relative in &manifest.game_profiles {
        match load_profile(dir, relative) {
            Ok(profile) => {
                let label = profile.name.clone();
                // Anything the profile asks for that this build does not do
                // is reported, never faked into looking supported.
                for key in profile.unsupported_keys() {
                    unsupported.push(format!("{label}: {key}"));
                }
                match crate::smart::register_profile_preset(profile.to_preset()) {
                    Ok(()) => done.push(format!("游戏配置 {label}")),
                    Err(e) => problems.push(e),
                }
            }
            Err(e) => problems.push(e),
        }
    }

    if done.is_empty() && problems.is_empty() {
        // A manifest with no implemented capability is still installed
        // harmlessly: it declares nothing we can act on yet.
        notice.ok = true;
        notice.message = "清单没有已实现的能力（支持 theme / struct_templates / game_profiles）"
            .to_string();
        return;
    }
    let mut message = if done.is_empty() {
        "没有可用能力".to_string()
    } else {
        format!("已注册 {}", done.join("、"))
    };
    if !unsupported.is_empty() {
        message.push_str(&format!("；未支持的键：{}", unsupported.join("、")));
    }
    if !problems.is_empty() {
        message.push_str(&format!("；失败：{}", problems.join("；")));
    }
    notice.ok = problems.is_empty();
    notice.message = message;
}

pub(crate) fn load_template(dir: &std::path::Path, relative: &str) -> Result<crate::structs::StructTemplate, String> {
    let path = dir.join(relative);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?;
    let file: TemplateFile = serde_json::from_str(&text)
        .map_err(|e| format!("{} 不是合法的结构体模板：{e}", path.display()))?;
    if file.name.trim().is_empty() {
        return Err(format!("{} 缺少 name。", path.display()));
    }
    let mut fields = Vec::with_capacity(file.fields.len());
    for field in &file.fields {
        let offset = crate::model::parse_address(&field.offset)
            .ok_or_else(|| format!("字段「{}」的偏移「{}」读不出来。", field.name, field.offset))?;
        let kind = crate::structs::kind_from_name(&field.r#type)
            .ok_or_else(|| format!("字段「{}」的类型「{}」不认识。", field.name, field.r#type))?;
        fields.push(crate::structs::TemplateField {
            name: field.name.clone(),
            offset: offset as usize,
            kind,
        });
    }
    Ok(crate::structs::StructTemplate {
        name: file.name.clone(),
        signature: if file.signature.is_empty() {
            file.name.clone()
        } else {
            file.signature.clone()
        },
        fields,
    })
}

pub(crate) fn load_profile(dir: &std::path::Path, relative: &str) -> Result<ProfileFile, String> {
    let path = dir.join(relative);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?;
    let file: ProfileFile = serde_json::from_str(&text)
        .map_err(|e| format!("{} 不是合法的游戏配置：{e}", path.display()))?;
    if file.name.trim().is_empty() {
        return Err(format!("{} 缺少 name。", path.display()));
    }
    Ok(file)
}

impl ProfileFile {
    /// Hotkey actions this build does not implement. Kept explicit so the UI
    /// can say "declared but not supported" instead of pretending.
    pub fn unsupported_keys(&self) -> Vec<String> {
        self.hotkeys
            .iter()
            .filter(|h| !crate::hotkey::is_supported_action(&h.action))
            .map(|h| format!("hotkey {}={}", h.action, h.key))
            .collect()
    }

    /// Turn the profile into a preset the scan page can apply.
    pub fn to_preset(&self) -> crate::smart::Preset {
        let scan = self.scan.clone().unwrap_or_default();
        let label = if scan.label.is_empty() {
            self.name.clone()
        } else {
            scan.label.clone()
        };
        crate::smart::Preset {
            key: Box::leak(self.name.clone().into_boxed_str()),
            label: Box::leak(label.into_boxed_str()),
            hint: Box::leak(self.description.clone().into_boxed_str()),
            vtype: crate::session::vtype_from_name(&scan.value_type),
            value: Box::leak(scan.value.into_boxed_str()),
        }
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