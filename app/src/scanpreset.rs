//! Reusable scan recipes.
//!
//! Picking the type, the comparison and the region filters is the same three
//! clicks every session: 4-byte health, float coordinates, unknown-start. This
//! module stores those recipes by name so they can be re-applied in one click
//! and reached from MCP as well.
//!
//! A preset describes *how* to search, never *what* to search for: it carries no
//! value, so applying one can never overwrite the number the user just typed.

use crate::model::{ScanType, VType};
use crate::session::vtype_from_name;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn yes() -> bool {
    true
}

/// One saved search recipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanPreset {
    pub name: String,
    /// One line saying when this recipe is the right one.
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub vtype: String,
    #[serde(default)]
    pub scan_type: String,
    #[serde(default)]
    pub fast: bool,
    #[serde(default = "yes")]
    pub writable_only: bool,
    /// Restrict the scan to pages the heat map has seen change.
    #[serde(default)]
    pub hot_only: bool,
    /// 0 means the app default.
    #[serde(default)]
    pub max_results: usize,
}

impl ScanPreset {
    pub fn vtype(&self) -> VType {
        vtype_from_name(&self.vtype)
    }

    pub fn scan_type(&self) -> ScanType {
        scan_type_from_name(&self.scan_type)
    }

    /// The same one-liner the UI tag and the MCP listing show.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = vec![
            self.vtype().label().to_string(),
            self.scan_type().label().to_string(),
        ];
        if self.writable_only {
            parts.push("仅可写内存".to_string());
        }
        if self.fast {
            parts.push("快速扫描".to_string());
        }
        if self.hot_only {
            parts.push("只扫热区".to_string());
        }
        if self.max_results > 0 {
            parts.push(format!("上限 {}", self.max_results));
        }
        parts.join(" · ")
    }

    pub fn with_note(mut self, note: &str) -> Self {
        self.note = note.to_string();
        self
    }
}

/// Three recipes that cover most first sessions. They are the fallback when no
/// file exists yet, so a fresh install still has something to click.
pub fn builtins() -> Vec<ScanPreset> {
    vec![
        ScanPreset {
            name: "精确值 · 4 字节".to_string(),
            note: "知道当前数值时用这个，最常见的一条路".to_string(),
            vtype: "i32".to_string(),
            scan_type: "exact".to_string(),
            fast: false,
            writable_only: true,
            hot_only: false,
            max_results: 0,
        },
        ScanPreset {
            name: "未知初值起步".to_string(),
            note: "不知道当前值，先拍一张快照再收窄".to_string(),
            vtype: "auto".to_string(),
            scan_type: "unknown".to_string(),
            fast: false,
            writable_only: true,
            hot_only: false,
            max_results: 0,
        },
        ScanPreset {
            name: "浮点坐标".to_string(),
            note: "位置、朝向、镜头距离这类连续变化的值".to_string(),
            vtype: "f32".to_string(),
            scan_type: "exact".to_string(),
            fast: false,
            writable_only: true,
            hot_only: false,
            max_results: 0,
        },
    ]
}

pub fn scan_type_from_name(name: &str) -> ScanType {
    match name.trim().to_ascii_lowercase().as_str() {
        "bigger" => ScanType::Bigger,
        "smaller" => ScanType::Smaller,
        "between" => ScanType::Between,
        "unknown" => ScanType::Unknown,
        "increased" => ScanType::Increased,
        "decreased" => ScanType::Decreased,
        "changed" => ScanType::Changed,
        "unchanged" => ScanType::Unchanged,
        _ => ScanType::Exact,
    }
}

/// Stable name for storage; the inverse of scan_type_from_name.
pub fn scan_type_name(scan_type: ScanType) -> &'static str {
    match scan_type {
        ScanType::Exact => "exact",
        ScanType::Bigger => "bigger",
        ScanType::Smaller => "smaller",
        ScanType::Between => "between",
        ScanType::Unknown => "unknown",
        ScanType::Increased => "increased",
        ScanType::Decreased => "decreased",
        ScanType::Changed => "changed",
        ScanType::Unchanged => "unchanged",
    }
}

pub fn vtype_name(vtype: VType) -> &'static str {
    match vtype {
        VType::Auto => "auto",
        VType::I8 => "i8",
        VType::I16 => "i16",
        VType::I32 => "i32",
        VType::I64 => "i64",
        VType::F32 => "f32",
        VType::F64 => "f64",
        VType::Str => "str",
    }
}

/// Presets are a user habit, not a per-directory fact, so they live with the
/// other preferences under the per-user data folder.
pub fn path() -> PathBuf {
    crate::prefs::data_dir().join("scan-presets.json")
}

/// Load the saved recipes. A missing or corrupt file is not an error: the
/// built-ins take over so the scan page always has something to offer.
pub fn load() -> Vec<ScanPreset> {
    let file = path();
    let Ok(text) = std::fs::read_to_string(&file) else {
        return builtins();
    };
    match serde_json::from_str::<Vec<ScanPreset>>(&text) {
        Ok(mut list) => {
            list.retain(|p| !p.name.trim().is_empty());
            if list.is_empty() {
                builtins()
            } else {
                list
            }
        }
        Err(_) => builtins(),
    }
}

pub fn save(list: &[ScanPreset]) -> Result<(), String> {
    let file = path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(list)
        .map_err(|e| format!("无法序列化扫描预设：{e}"))?;
    std::fs::write(&file, text)
        .map_err(|e| format!("无法写入 {}：{e}", file.display()))
}

/// Case-insensitive lookup, because a name typed into MCP is not a stable id.
pub fn find<'a>(list: &'a [ScanPreset], name: &str) -> Option<&'a ScanPreset> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return None;
    }
    list.iter()
        .find(|p| p.name.eq_ignore_ascii_case(wanted))
}

/// Insert or replace by name, keeping the list order stable.
pub fn upsert(list: &mut Vec<ScanPreset>, preset: ScanPreset) -> bool {
    if let Some(slot) = list
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(&preset.name))
    {
        *slot = preset;
        true
    } else {
        list.push(preset);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_distinct_and_named() {
        let list = builtins();
        assert_eq!(list.len(), 3);
        for p in &list {
            assert!(!p.name.trim().is_empty());
            assert!(!p.note.trim().is_empty());
        }
        assert!(find(&list, "浮点坐标").is_some());
        assert!(find(&list, "浮点坐标").unwrap().vtype() == VType::F32);
    }

    #[test]
    fn a_preset_describes_itself() {
        let p = ScanPreset {
            name: "x".to_string(),
            note: String::new(),
            vtype: "i32".to_string(),
            scan_type: "bigger".to_string(),
            fast: true,
            writable_only: true,
            hot_only: true,
            max_results: 5000,
        };
        let line = p.describe();
        assert!(line.contains("4 字节"), "{line}");
        assert!(line.contains("大于"), "{line}");
        assert!(line.contains("只扫热区"), "{line}");
        assert!(line.contains("上限 5000"), "{line}");
    }

    #[test]
    fn an_unknown_name_is_not_guessed() {
        let list = builtins();
        assert!(find(&list, "不存在的搜法").is_none());
        assert!(find(&list, "   ").is_none());
    }

    #[test]
    fn upsert_replaces_instead_of_duplicating() {
        let mut list = builtins();
        let mut edited = list[0].clone();
        edited.fast = true;
        let replaced = upsert(&mut list, edited);
        assert!(replaced);
        assert_eq!(list.len(), 3);
        assert!(list[0].fast);
    }

    #[test]
    fn names_round_trip_through_storage() {
        for ty in [
            ScanType::Exact,
            ScanType::Unknown,
            ScanType::Between,
            ScanType::Unchanged,
        ] {
            assert_eq!(scan_type_from_name(scan_type_name(ty)), ty);
        }
        for ty in [VType::I32, VType::F32, VType::Auto, VType::Str] {
            assert_eq!(vtype_from_name(vtype_name(ty)), ty);
        }
    }

    #[test]
    fn a_preset_carries_no_scanned_value() {
        // Applying a preset must never overwrite the number the user typed,
        // so the struct has no value field at all. This test pins that down.
        let p = builtins().remove(0);
        let text = serde_json::to_string(&p).unwrap();
        assert!(!text.contains("\"value\""), "{text}");
    }
}
