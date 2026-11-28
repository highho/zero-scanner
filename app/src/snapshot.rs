//! Scan snapshots: keep one scan result around so the next scan can be
//! compared against it.
//!
//! After a restart, or after the game reloads a level, the useful question is
//! not "what did the scanner find" but "what changed since last time": which
//! addresses survived, which disappeared, which are new. Saving the hit list
//! answers that without keeping a process handle alive.

use crate::model::ScanHit;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How many snapshots are kept before the oldest one is dropped.
pub const MAX: usize = 12;

/// One address inside a saved snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub address: u64,
    pub vtype: String,
    pub value: String,
}

/// A named, saved copy of one scan result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    /// Program the scan ran against. Empty when the scan predates a known
    /// process, which is why it is defaulted rather than required.
    #[serde(default)]
    pub process_name: String,
    /// What kind of scan produced it, e.g. "首次 · 精确值".
    #[serde(default)]
    pub kind: String,
    /// Unix seconds. Zero when the clock was unavailable at save time.
    #[serde(default)]
    pub taken_at: u64,
    pub rows: Vec<Row>,
}

impl Snapshot {
    /// Human line for a list row: "1,204 个地址 · 首次 · 精确值 · 3 小时前".
    pub fn describe(&self, now: u64) -> String {
        let mut text = format!("{} 个地址", self.rows.len());
        if !self.kind.is_empty() {
            text.push_str(" · ");
            text.push_str(&self.kind);
        }
        let age = age_text(self.taken_at, now);
        if !age.is_empty() {
            text.push_str(" · ");
            text.push_str(&age);
        }
        text
    }
}

/// Relative age, deliberately coarse: the exact second never matters here.
pub fn age_text(taken_at: u64, now: u64) -> String {
    if taken_at == 0 || now < taken_at {
        return String::new();
    }
    let secs = now - taken_at;
    if secs < 60 {
        "刚刚".to_string()
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86_400 {
        format!("{} 小时前", secs / 3600)
    } else {
        format!("{} 天前", secs / 86_400)
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Capture the current hit list under a name.
pub fn take(name: &str, process_name: &str, kind: &str, hits: &[ScanHit]) -> Snapshot {
    Snapshot {
        name: name.trim().to_string(),
        process_name: process_name.to_string(),
        kind: kind.to_string(),
        taken_at: now_secs(),
        rows: hits
            .iter()
            .map(|hit| Row {
                address: hit.address,
                vtype: hit.vtype.short().to_string(),
                value: hit.value.display(),
            })
            .collect(),
    }
}

/// Result of comparing a snapshot against the current candidates.
#[derive(Clone, Debug)]
pub struct Diff {
    pub name: String,
    /// Addresses present in both.
    pub kept: Vec<u64>,
    /// Addresses the snapshot had that are gone now.
    pub gone: Vec<u64>,
    /// Addresses that appeared since the snapshot was taken.
    pub added: Vec<u64>,
}

impl Diff {
    pub fn summary(&self) -> String {
        format!(
            "{} 个仍在 · {} 个消失 · {} 个新出现",
            self.kept.len(),
            self.gone.len(),
            self.added.len()
        )
    }

    /// One short line per bucket, for a toast or an MCP answer.
    pub fn report(&self, limit: usize) -> String {
        let mut text = format!("对比「{}」：{}", self.name, self.summary());
        for (label, list) in [
            ("仍在", &self.kept),
            ("消失", &self.gone),
            ("新出现", &self.added),
        ] {
            if list.is_empty() {
                continue;
            }
            let shown = list
                .iter()
                .take(limit)
                .map(|a| format!("{a:016X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let more = if list.len() > limit {
                format!(" …共 {} 个", list.len())
            } else {
                String::new()
            };
            text.push_str(&format!("\n{label}：{shown}{more}"));
        }
        text
    }
}

/// Compare a snapshot against the current hit list.
///
/// Identity is the address alone. Requiring the value type to match as well
/// would report a widened type as "gone + new", which reads like the address
/// moved when it did not.
pub fn diff(snapshot: &Snapshot, current: &[ScanHit]) -> Diff {
    let mut kept = Vec::new();
    let mut gone = Vec::new();
    for row in &snapshot.rows {
        if current.iter().any(|hit| hit.address == row.address) {
            kept.push(row.address);
        } else {
            gone.push(row.address);
        }
    }
    let mut added: Vec<u64> = current
        .iter()
        .map(|hit| hit.address)
        .filter(|address| !snapshot.rows.iter().any(|row| row.address == *address))
        .collect();
    added.sort_unstable();
    added.dedup();
    Diff {
        name: snapshot.name.clone(),
        kept,
        gone,
        added,
    }
}

pub fn path() -> PathBuf {
    crate::prefs::data_dir().join("snapshots.json")
}

/// Saved snapshots, newest first. A missing or corrupt file is an empty list:
/// a lost comparison is an annoyance, a crash on launch is not.
pub fn load() -> Vec<Snapshot> {
    let file = path();
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<Snapshot>>(&text) {
        Ok(mut list) => {
            list.retain(|snapshot| !snapshot.name.trim().is_empty());
            list
        }
        Err(_) => Vec::new(),
    }
}

pub fn save(list: &[Snapshot]) -> Result<(), String> {
    let file = path();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建快照目录：{e}"))?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| format!("无法序列化快照：{e}"))?;
    std::fs::write(&file, text).map_err(|e| format!("无法写入快照文件：{e}"))
}

/// Insert or replace by name. New snapshots go to the front so the list reads
/// newest-first, and the oldest beyond MAX is dropped.
pub fn upsert(list: &mut Vec<Snapshot>, snapshot: Snapshot) -> bool {
    let replaced = if let Some(index) = list
        .iter()
        .position(|item| item.name.eq_ignore_ascii_case(&snapshot.name))
    {
        list.remove(index);
        true
    } else {
        false
    };
    list.insert(0, snapshot);
    if list.len() > MAX {
        list.truncate(MAX);
    }
    replaced
}

pub fn find<'a>(list: &'a [Snapshot], name: &str) -> Option<&'a Snapshot> {
    list.iter()
        .find(|item| item.name.eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ScanHit, VType, Value};

    fn hit(address: u64, value: i64) -> ScanHit {
        ScanHit {
            address,
            vtype: VType::I32,
            value: Value::Int(value),
            previous: None,
        }
    }

    fn snap(name: &str, addresses: &[u64]) -> Snapshot {
        Snapshot {
            name: name.to_string(),
            process_name: "game.exe".to_string(),
            kind: "首次 · 精确值".to_string(),
            taken_at: 1_000,
            rows: addresses
                .iter()
                .map(|a| Row {
                    address: *a,
                    vtype: "I32".to_string(),
                    value: "100".to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_snapshot_keeps_the_addresses_it_was_taken_from() {
        let hits = vec![hit(0x1000, 42), hit(0x2000, 43)];
        let snapshot = take("第一次", "game.exe", "首次 · 精确值", &hits);
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(snapshot.rows[0].address, 0x1000);
        assert_eq!(snapshot.rows[0].value, "42");
        assert_eq!(snapshot.process_name, "game.exe");
    }

    #[test]
    fn diff_finds_survivors_new_and_gone() {
        let snapshot = snap("a", &[0x1000, 0x2000, 0x3000]);
        let current = vec![hit(0x2000, 1), hit(0x3000, 2), hit(0x9000, 3)];
        let d = diff(&snapshot, &current);
        assert_eq!(d.kept, vec![0x2000, 0x3000]);
        assert_eq!(d.gone, vec![0x1000]);
        assert_eq!(d.added, vec![0x9000]);
        assert!(d.summary().contains("2 个仍在"));
    }

    #[test]
    fn diff_of_an_empty_current_list_loses_everything() {
        let snapshot = snap("a", &[0x1000, 0x2000]);
        let d = diff(&snapshot, &[]);
        assert!(d.kept.is_empty());
        assert_eq!(d.gone.len(), 2);
        assert!(d.added.is_empty());
    }

    #[test]
    fn diff_does_not_count_the_same_new_address_twice() {
        let snapshot = snap("a", &[0x1000]);
        let current = vec![hit(0x5000, 1), hit(0x5000, 2)];
        let d = diff(&snapshot, &current);
        assert_eq!(d.added, vec![0x5000]);
    }

    #[test]
    fn saving_the_same_name_replaces_it_and_stays_newest_first() {
        let mut list = vec![snap("old", &[1])];
        let replaced = upsert(&mut list, snap("old", &[1, 2]));
        assert!(replaced);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].rows.len(), 2);
        upsert(&mut list, snap("newer", &[3]));
        assert_eq!(list[0].name, "newer");
    }

    #[test]
    fn the_oldest_snapshot_is_dropped_past_the_cap() {
        let mut list = Vec::new();
        for i in 0..MAX + 3 {
            upsert(&mut list, snap(&format!("s{i}"), &[i as u64]));
        }
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0].name, format!("s{}", MAX + 2));
    }

    #[test]
    fn names_match_without_case_and_around_whitespace() {
        let list = vec![snap("Level 1", &[1])];
        assert!(find(&list, " level 1 ").is_some());
        assert!(find(&list, "level 2").is_none());
    }

    #[test]
    fn a_snapshot_survives_a_round_trip() {
        let list = vec![snap("a", &[0x10, 0x20])];
        let text = serde_json::to_string(&list).unwrap();
        let back: Vec<Snapshot> = serde_json::from_str(&text).unwrap();
        assert_eq!(back[0].rows.len(), 2);
        assert_eq!(back[0].rows[1].address, 0x20);
    }

    #[test]
    fn an_old_snapshot_file_without_metadata_still_loads() {
        let text = r#" [{"name":"legacy","rows":[{"address":4096,"vtype":"I32","value":"7"}]}] "#;
        let back: Vec<Snapshot> = serde_json::from_str(text).unwrap();
        assert_eq!(back[0].name, "legacy");
        assert!(back[0].kind.is_empty());
        assert_eq!(back[0].rows[0].address, 4096);
    }

    #[test]
    fn age_is_coarse_but_never_negative() {
        assert_eq!(age_text(100, 130), "刚刚");
        assert_eq!(age_text(100, 100 + 120), "2 分钟前");
        assert_eq!(age_text(100, 100 + 7200), "2 小时前");
        assert_eq!(age_text(100, 100 + 172_800), "2 天前");
        assert_eq!(age_text(0, 999), "");
        assert_eq!(age_text(500, 100), "");
    }
}
