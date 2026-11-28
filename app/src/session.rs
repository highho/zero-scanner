//! Workspace persistence: keep the useful state across restarts.
//!
//! Closing Zero used to throw away everything: the address list narrowed over
//! minutes, the patches already applied, the settings that worked. This module
//! saves that to a small JSON file and restores it on launch.
//!
//! Only durable, user-authored state is saved. Live things (process handles,
//! running scans, results tied to a still-attached process) are deliberately
//! excluded: restoring a stale process handle would be a lie.
//!
//! A corrupt or unreadable file never stops Zero from starting; every failure
//! falls back to an empty workspace.

use crate::ctable::CtEntry;
use crate::model::VType;
use serde::{Deserialize, Serialize};

/// One saved address-list row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedEntry {
    pub description: String,
    pub address: u64,
    pub vtype: String,
    pub frozen: bool,
    pub value_text: String,
}

impl SavedEntry {
    pub fn vtype(&self) -> VType {
        vtype_from_name(&self.vtype)
    }
}

/// One saved patch record, so the user can still undo it after a restart.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedPatch {
    pub address: u64,
    pub original: Vec<u8>,
    pub written: Vec<u8>,
    pub label: String,
    pub active: bool,
}

/// A saved watch-list row, so a restart does not lose the narrowed candidates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedWatch {
    pub address: u64,
    pub vtype: String,
    pub description: String,
    pub score: i32,
}

impl SavedWatch {
    pub fn vtype(&self) -> VType {
        vtype_from_name(&self.vtype)
    }
}

/// A saved CT entry, flattened because the in-memory form has no serde derive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedTableEntry {
    pub description: String,
    pub address: String,
    pub vtype: String,
    pub value: Option<String>,
}

/// One saved pointer map: the target that was scanned plus every chain found.
///
/// Chains are stored with a module *name* instead of the live module base, so
/// the same map can be re-pointed at a fresh process after a restart.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedPointerMap {
    pub target: u64,
    pub depth: usize,
    pub paths: Vec<SavedPointerPath>,
}

/// A pointer chain flattened for storage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedPointerPath {
    pub module: String,
    pub module_offset: u64,
    pub offsets: Vec<u64>,
    /// Final address in the process that produced the map; informational only,
    /// it is recomputed by rebasing after the map is loaded.
    pub final_address: u64,
}
impl SavedTableEntry {
    pub fn from_ct_entry(entry: &CtEntry) -> Self {
        Self {
            description: entry.description.clone(),
            address: entry.address.clone(),
            vtype: vtype_name(entry.vtype).to_string(),
            value: entry.value.clone(),
        }
    }

    pub fn to_ct_entry(&self) -> CtEntry {
        CtEntry {
            description: self.description.clone(),
            vtype: vtype_from_name(&self.vtype),
            address: self.address.clone(),
            value: self.value.clone(),
            active: self.value.is_some(),
        }
    }
}

/// The whole saved workspace.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub version: u32,
    pub entries: Vec<SavedEntry>,
    pub patches: Vec<SavedPatch>,
    pub table_entries: Vec<SavedTableEntry>,
    pub watches: Vec<SavedWatch>,
    pub pointer_maps: Vec<SavedPointerMap>,
    pub table_path: String,
    pub script_source: String,
    pub asm_source: String,
    pub aob_text: String,
    pub scan_value: String,
    pub last_tab: String,
}

impl Workspace {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.patches.iter().all(|p| !p.active)
            && self.table_entries.is_empty()
            && self.watches.is_empty()
            && self.pointer_maps.iter().all(|m| m.paths.is_empty())
            && self.script_source.trim().is_empty()
            && self.asm_source.trim().is_empty()
    }

    /// Patches that were still applied when the workspace was saved. These are
    /// the ones that matter after a restart: the target may still be modified.
    pub fn active_patches(&self) -> usize {
        self.patches.iter().filter(|p| p.active).count()
    }
}

fn vtype_name(vtype: VType) -> &'static str {
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

pub fn vtype_from_name(name: &str) -> VType {
    match name {
        "i8" => VType::I8,
        "i16" => VType::I16,
        "i32" => VType::I32,
        "i64" => VType::I64,
        "f32" => VType::F32,
        "f64" => VType::F64,
        "str" => VType::Str,
        _ => VType::Auto,
    }
}

pub fn to_string(workspace: &Workspace) -> Result<String, String> {
    serde_json::to_string_pretty(workspace).map_err(|e| format!("无法序列化工作区：{e}"))
}

pub fn from_str(text: &str) -> Result<Workspace, String> {
    serde_json::from_str(text).map_err(|e| format!("工作区文件格式不对：{e}"))
}

pub fn save(path: &str, workspace: &Workspace) -> Result<(), String> {
    let text = to_string(workspace)?;
    std::fs::write(path, text).map_err(|e| format!("无法写入 {path}：{e}"))
}

/// Load a workspace, falling back to an empty one on any problem.
pub fn load(path: &str) -> Workspace {
    match std::fs::read_to_string(path) {
        Ok(text) => from_str(&text).unwrap_or_default(),
        Err(_) => Workspace::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_patches() {
        let mut workspace = Workspace::default();
        workspace.patches.push(SavedPatch {
            address: 0x7FF6_A1B2_C010,
            original: vec![0x55, 0x48, 0x89],
            written: vec![0x31, 0xC0, 0xC3],
            label: "test".to_string(),
            active: true,
        });
        workspace.entries.push(SavedEntry {
            description: "玩家金币".to_string(),
            address: 0x1000,
            vtype: "i32".to_string(),
            frozen: true,
            value_text: "9900".to_string(),
        });
        let text = to_string(&workspace).expect("serialise");
        let back = from_str(&text).expect("parse");
        assert_eq!(back.patches.len(), 1);
        assert_eq!(back.patches[0].original, vec![0x55, 0x48, 0x89]);
        assert!(back.patches[0].active);
        assert_eq!(back.entries[0].description, "玩家金币");
        assert_eq!(back.entries[0].vtype(), VType::I32);
    }

    #[test]
    fn corrupt_file_yields_empty_workspace() {
        let workspace = from_str("{not json");
        assert!(workspace.is_err());
        // load() must not panic on garbage.
        let loaded = load("this-file-does-not-exist-12345.json");
        assert!(loaded.is_empty());
    }
}
