//! End-to-end verification against a real, running process.
//!
//! Everything here runs against an actual target process: real memory regions,
//! real addresses read back from that process. It walks the whole Zero flow
//! once so the product is proven end to end, not just unit by unit.
//!
//! Each stage prints a PASS/FAIL line and appends to e2e.log; the exit code is
//! the overall verdict.

use crate::aob;
use crate::assembler;
use crate::ctable;
use crate::disasm;
use crate::inj;
use crate::lua as script;
use crate::model::{ScanType, VType};
use crate::scan::ScanConfig;
use crate::pointerscan;
use crate::scan;
use crate::winmem::ProcessHandle;
use std::sync::Arc;

struct Report {
    log: String,
    ok: bool,
}

impl Report {
    fn new() -> Self {
        Self { log: String::new(), ok: true }
    }

    fn check(&mut self, condition: bool, label: &str, detail: impl FnOnce() -> String) {
        if condition {
            self.log.push_str(&format!("PASS  {label}\n"));
        } else {
            self.ok = false;
            self.log.push_str(&format!("FAIL  {label}\n      -> {}\n", detail()));
        }
        // Flush after every check: a long pointer scan must not hide the stages
        // that already passed if the run is interrupted.
        std::fs::write("e2e.log", &self.log).ok();
    }

    /// Mark a stage as started, so a hang can be localised.
    fn stage(&mut self, label: &str) {
        self.log.push_str(&format!("....  {label}\n"));
        std::fs::write("e2e.log", &self.log).ok();
    }

    fn finish(&mut self) -> bool {
        self.log.push_str(&format!("\nRESULT: {}\n", if self.ok { "PASS" } else { "FAIL" }));
        std::fs::write("e2e.log", &self.log).ok();
        self.ok
    }
}

fn info_value(path: &str, key: &str) -> Option<u64> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        if k == key {
            return u64::from_str_radix(v.trim_start_matches("0x").trim(), 16).ok();
        }
    }
    None
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

pub fn run(pid: u32, info_path: &str) -> bool {
    let mut report = Report::new();
    let handle = match crate::winmem::open_process(pid, "e2e-target") {
        Ok(h) => h,
        Err(e) => {
            report.log.push_str(&format!("FAIL  cannot attach to pid {pid}: {e}\n"));
            report.log.push_str("RESULT: FAIL\n");
            std::fs::write("e2e.log", &report.log).ok();
            return false;
        }
    };

    report.stage("enumerate modules");
    let modules = handle.modules();
    report.check(!modules.is_empty(), "enumerate modules of a live process",
        || format!("got {} modules", modules.len()));
    report.check(handle.is_64bit, "target is a 64-bit process",
        || format!("is_64bit={}", handle.is_64bit));

    let coins = info_value(info_path, "coins");
    let coins_ptr = info_value(info_path, "coins_ptr");
    let marker = info_value(info_path, "marker");
    report.check(coins.is_some(), "target advertised its addresses",
        || format!("coins={coins:?}"));

    if let Some(address) = coins {
        let mut buffer = [0u8; 4];
        let readable = handle.read(address, &mut buffer);
        report.check(readable && u32::from_le_bytes(buffer) == 9900,
            "read a known value from the target's memory",
            || format!("read {} ({}) at {address:X}", u32::from_le_bytes(buffer), readable));

        report.stage("value scan");
        let regions = handle.regions(false);
        report.check(!regions.is_empty(), "target has readable memory regions",
            || format!("{} regions", regions.len()));
        match scan::start_first_scan(handle.clone(), regions, ScanConfig {
            vtype: VType::Auto,
            scan_type: ScanType::Exact,
            value1: "9900".to_string(),
            value2: String::new(),
            fast: false,
            max_results: 500,
        }) {
            Ok(scan_handle) => {
                let done = scan_handle.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap_or_else(|_| scan::ScanDone {
                    hits: Vec::new(), elapsed: std::time::Duration::ZERO,
                    truncated: false, cancelled: true, error: Some("timeout".to_string()),
                });
                report.check(done.hits.iter().any(|h| h.address == address),
                    "value scan finds the target's address",
                    || format!("{} hits, target at {address:X} not among them", done.hits.len()));
            }
            Err(e) => report.check(false, "value scan starts", || e),
        }

        // 9900 = 0x26AC -> AC 26 00 00 ; 9904 = 0x26B0 -> B0 26 00 00
        let write_ok = handle.write(address, &[0xB0, 0x26, 0x00, 0x00]);
        let mut after = [0u8; 4];
        handle.read(address, &mut after);
        report.check(write_ok && u32::from_le_bytes(after) == 9904,
            "write into the target and read the new value back",
            || format!("read {} back", u32::from_le_bytes(after)));
        handle.write(address, &[0xAC, 0x26, 0x00, 0x00]);
        let mut restored = [0u8; 4];
        handle.read(address, &mut restored);
        report.check(u32::from_le_bytes(restored) == 9900,
            "value restored to the original",
            || format!("read {} back", u32::from_le_bytes(restored)));
    }

    if let Some(pointer) = coins_ptr {
        report.stage("pointer scan");
        let regions = handle.regions(false);
        if !regions.is_empty() {
            let writable: Vec<_> = regions.into_iter().filter(|r| r.writable).collect();
            match pointerscan::start_scan(handle.clone(), writable, modules.clone(), coins.unwrap_or(0), 2, 0x1000, 200) {
                Ok(ptr_handle) => {
                    let done = ptr_handle.rx.recv_timeout(std::time::Duration::from_secs(180)).unwrap_or_else(|_| pointerscan::PtrDone {
                        paths: Vec::new(), elapsed: std::time::Duration::ZERO,
                        cancelled: true, truncated: false, scanned_pointers: 0,
                    });
                    // A path reaches the target; its head is a static base that
                    // lives inside a module. Any path whose final field is the
                    // coins address proves the chain was resolved.
                    report.check(
                        done.paths.iter().any(|p| p.final_address == coins.unwrap_or(0)),
                        "pointer scan resolves a chain to the target's address",
                        || format!("{} paths, none ends at {coins:?}", done.paths.len()));
                }
                Err(e) => report.check(false, "pointer scan starts", || e),
            }
        }
    }

    if let Some(address) = marker {
        report.stage("aob scan");
        let regions = handle.regions(false);
        let pattern = aob::AobPattern::from_bytes(&[0x5A, 0x5A, 0x5A, 0x5A, 0xC3]);
        if !regions.is_empty() {
            match aob::start_scan(handle.clone(), regions, pattern, 200) {
                Ok(scan_handle) => {
                    let done = scan_handle.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap_or_else(|_| aob::AobDone {
                        hits: Vec::new(), elapsed: std::time::Duration::ZERO,
                        cancelled: true, truncated: false,
                    });
                    report.check(done.hits.iter().any(|h| *h == address),
                        "AOB scan finds the known signature",
                        || format!("{} hits, signature at {address:X} not among them", done.hits.len()));
                }
                Err(e) => report.check(false, "AOB scan starts", || e),
            }
        }
    }

    report.stage("symbols");
    if let Some(module) = modules.iter().find(|m| m.name.ends_with(".exe")) {
        match crate::syms::parse(&handle, module) {
            Ok(table) => report.check(true, "symbol table parsed for a live module",
                || format!("imports {}", table.import_count())),
            Err(e) => report.check(false, "symbol table parses", || e),
        }
    }

    report.stage("disassembly");
    if let Some(address) = coins {
        let mut window = [0u8; 64];
        if handle.read(address, &mut window) {
            let insns = disasm::disassemble(&window, address, true, 16);
            report.check(!insns.is_empty(), "disassembly decodes a real target window",
                || format!("{} instructions", insns.len()));
        }
    }


    report.stage("patch");
    match handle.allocate(0x2000) {
        Ok(address) => {
            let mut filler = [0u8; 16];
            handle.read(address, &mut filler);
            let payload = vec![0x48, 0x31, 0xC0];
            match inj::apply(&handle, address, &payload, "e2e") {
                Ok(mut patch) => {
                    let original_snapshot = patch.original.clone();
                    report.check(original_snapshot.len() == 3,
                        "patch recorded the original bytes",
                        || format!("original {}", patch.original_hex()));
                    report.check(inj::verify(&handle, &patch),
                        "patched bytes verified in the target", || "verify failed".to_string());
                    match inj::revert(&handle, &mut patch) {
                        Ok(()) => {
                            let mut back = [0u8; 3];
                            handle.read(address, &mut back);
                            report.check(back[..] == original_snapshot[..],
                                "patched region restored to the original",
                                || format!("read {:02X} {:02X} {:02X}, expected {}", back[0], back[1], back[2], hex(&original_snapshot)));
                        }
                        Err(e) => report.check(false, "patch reverts", || e),
                    }
                }
                Err(e) => report.check(false, "patch applies to the target", || e),
            }
            let _ = handle.free(address);
        }
        Err(e) => report.check(false, "allocate a scratch region in the target", || e),
    }

    report.stage("assembler");
    match assembler::assemble("mov rax, rbx\nnop\nret", 0) {
        Ok(result) => {
            if let Ok(address) = handle.allocate(0x2000) {
                handle.write(address, &vec![0x22u8; 16]);
                let mut before = [0u8; 16];
                handle.read(address, &mut before);
                match inj::apply(&handle, address, &result.bytes, "assembled") {
                    Ok(mut patch) => {
                        let mut now = [0u8; 16];
                        handle.read(address, &mut now);
                        report.check(now[..result.bytes.len()] == result.bytes[..],
                            "assembled bytes land in the target",
                            || format!("{} != {}", hex(&now[..result.bytes.len()]), hex(&result.bytes)));
                        let back = inj::revert(&handle, &mut patch);
                        let mut restored = [0u8; 16];
                        handle.read(address, &mut restored);
                        report.check(back.is_ok() && restored == before,
                            "assembled patch restored to the original",
                            || format!("before {}, after {}", hex(&before), hex(&restored)));
                    }
                    Err(e) => report.check(false, "assembled bytes apply to the target", || e),
                }
                let _ = handle.free(address);
            } else {
                report.check(false, "assemble into the target", || "could not allocate".to_string());
            }
        }
        Err(e) => report.check(false, "assemble a snippet for the target", || e),
    }

    report.stage("ct table");
    match ctable::parse(REAL_TABLE) {
        Ok(table) => {
            report.check(table.entries.len() == 2, "real CT document parsed",
                || format!("{} entries", table.entries.len()));
            let pairs: Vec<(String, u64)> = modules.iter().map(|m| (m.name.clone(), m.base)).collect();
            let probe = modules
                .first()
                .map(|m| format!("{}+1A2B", m.name))
                .unwrap_or_default();
            report.check(ctable::resolve_address(&probe, &pairs, handle.is_64bit, &|_| None).is_ok(),
                "CT address resolves against the target's modules",
                || format!("{probe} did not resolve"));
        }
        Err(e) => report.check(false, "real CT document parses", || e),
    }

    // Watch list: rank candidates, sample them, change the target, and confirm
    // the one that moved is identified.
    report.stage("watch list");
    if let Some(address) = coins {
        let hits = vec![
            crate::model::ScanHit {
                address,
                vtype: VType::I32,
                value: crate::model::Value::Int(9900),
                previous: Some(crate::model::Value::Int(9800)),
            },
            crate::model::ScanHit {
                address: address.wrapping_add(0x100),
                vtype: VType::I32,
                value: crate::model::Value::Int(9900),
                previous: Some(crate::model::Value::Int(9900)),
            },
        ];
        let region_hints: Vec<crate::triage::RegionHint> = handle
            .regions(false)
            .into_iter()
            .map(|r| crate::triage::RegionHint {
                base: r.base,
                size: r.size,
                writable: r.writable,
                executable: r.executable,
            })
            .collect();
        let ranked = crate::triage::rank(&hits, &region_hints, &[]);
        let mut watches = crate::focus::from_candidates(&ranked, 4);
        report.check(!watches.is_empty(), "watch list built from ranked candidates",
            || "no watches produced".to_string());
        crate::focus::sample(&handle, &mut watches);
        handle.write(address, &[0xB0, 0x26, 0x00, 0x00]);
        for _ in 0..3 {
            crate::focus::sample(&handle, &mut watches);
        }
        let best = crate::focus::best(&watches).map(|w| w.address);
        report.check(best == Some(address),
            "the watch list identifies the address that actually changed",
            || format!("best={best:?} expected {address:X}"));
        handle.write(address, &[0xAC, 0x26, 0x00, 0x00]);
    }

    report.stage("lua");
    let lua_source = format!(
        "writeInteger(0x{:X}, 42, 4) print(readInteger(0x{:X}, 4))",
        coins.unwrap_or(0), coins.unwrap_or(0)
    );
    match script::run_script(&lua_source, Some(handle.clone())) {
        Ok(result) => {
            report.check(result.output == vec!["42".to_string()],
                "Lua script wrote and read back real target memory",
                || format!("{:?}", result.output));
            let mut back = [0u8; 4];
            handle.read(coins.unwrap_or(0), &mut back);
            report.check(u32::from_le_bytes(back) == 42,
                "the Lua write is visible outside the script",
                || format!("read {} back", u32::from_le_bytes(back)));
            handle.write(coins.unwrap_or(0), &[0xAC, 0x26, 0x00, 0x00]);
        }
        Err(e) => report.check(false, "Lua script runs against the target", || e),
    }

    report.finish()
}

const REAL_TABLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable>
  <CheatEntries>
    <CheatEntry>
      <Description>"coins"</Description>
      <VariableType>4 Bytes</VariableType>
      <Address>game.exe+1A2B</Address>
      <Value>9900</Value>
    </CheatEntry>
    <CheatEntry>
      <Description>"nested"</Description>
      <VariableType>Float</VariableType>
      <Address>[[game.exe+8C]+18]+30</Address>
    </CheatEntry>
  </CheatEntries>
</CheatTable>"#;
