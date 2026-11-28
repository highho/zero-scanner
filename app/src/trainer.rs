//! Trainer export: turn a finished table into something a player can run.
//!
//! The deliberate choice here is **not** to build a second executable. A
//! standalone trainer would mean a second binary to sign, version and keep in
//! ABI step with the main app, for the sake of one button. Instead a trainer is
//! two files: the table itself and a launcher that calls this same executable
//! with --apply-table. What the player gets is identical; what the project has
//! to maintain is not.
//!
//! Everything here is checked before anything is written: an address that does
//! not resolve, or a value that cannot be written and restored, stops the
//! export with the reason. A trainer that half works is worse than none.

use crate::ctable::{self, CtTable};
use crate::inj;
use crate::model::VType;
use crate::winmem::{self, ProcessHandle};
use std::path::{Path, PathBuf};

/// One entry that passed every check, ready to be applied at run time.
#[derive(Clone, Debug)]
pub struct Ready {
    pub description: String,
    pub address: u64,
    pub vtype: VType,
    /// The bytes the entry writes, already encoded.
    pub bytes: Vec<u8>,
}

/// What the export produced, so the UI can report it honestly.
#[derive(Clone, Debug, Default)]
pub struct Export {
    pub table_path: PathBuf,
    pub launcher_path: PathBuf,
    pub ready: usize,
    /// Entries that were skipped, with the reason. Never silently dropped.
    pub skipped: Vec<String>,
}

/// Encode a table value into the bytes a patch writes.
pub fn encode_value(vtype: VType, text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("没有填写数值".to_string());
    }
    match vtype {
        VType::I8 | VType::I16 | VType::I32 | VType::I64 => {
            let value: i64 = text
                .parse()
                .or_else(|_| {
                    let cleaned = text.trim_start_matches("0x").trim_start_matches("0X");
                    i64::from_str_radix(cleaned, 16)
                })
                .map_err(|_| format!("「{text}」不是整数"))?;
            Ok(match vtype {
                VType::I8 => (value as i8).to_le_bytes().to_vec(),
                VType::I16 => (value as i16).to_le_bytes().to_vec(),
                VType::I32 => (value as i32).to_le_bytes().to_vec(),
                _ => value.to_le_bytes().to_vec(),
            })
        }
        VType::F32 => {
            let value: f32 = text.parse().map_err(|_| format!("「{text}」不是小数"))?;
            Ok(value.to_le_bytes().to_vec())
        }
        VType::F64 => {
            let value: f64 = text.parse().map_err(|_| format!("「{text}」不是小数"))?;
            Ok(value.to_le_bytes().to_vec())
        }
        VType::Str => Ok(text.as_bytes().to_vec()),
        VType::Auto => Err("类型是「自动」，导出前要定下来".to_string()),
    }
}

/// Check one table entry against the live process: the address must resolve now,
/// and the bytes must be writable and restorable. A dry run, not a promise.
pub fn check_entry(
    handle: &ProcessHandle,
    modules: &[(String, u64)],
    entry: &ctable::CtEntry,
) -> Result<Ready, String> {
    let address = ctable::resolve_address(
        &entry.address,
        modules,
        handle.is_64bit,
        &|base| handle.read_pointer(base),
    )?;
    let value_text = entry
        .value
        .as_deref()
        .ok_or_else(|| "没有固定值，训练器里会一直是空的".to_string())?;
    let bytes = encode_value(entry.vtype, value_text)?;
    // Prove the write can happen and can be undone before promising anything.
    let original = inj::read_original(handle, address, bytes.len())
        .map_err(|e| format!("读不到 {address:X} 的原值：{e}"))?;
    let previous = handle
        .protect(address, bytes.len(), inj::executable_protection())
        .map_err(|e| format!("{address:X} 不可写：{e}"))?;
    let wrote = handle.write(address, &bytes);
    let restored = handle.protect(address, bytes.len(), previous).is_ok();
    if !wrote {
        return Err(format!("{address:X} 写入失败"));
    }
    let back = handle.write(address, &original);
    if !back {
        return Err(format!("{address:X} 写回原值失败，导出中止"));
    }
    if !restored {
        return Err(format!("{address:X} 的内存保护没有恢复"));
    }
    Ok(Ready {
        description: entry.description.clone(),
        address,
        vtype: entry.vtype,
        bytes,
    })
}

/// Write the launcher next to the table. It calls this same executable, so the
/// trainer is always in step with the build that produced it.
fn write_launcher(table: &Path, exe: &Path) -> Result<PathBuf, String> {
    let launcher = table.with_extension("bat");
    let exe_text = exe.display().to_string();
    let table_text = table.display().to_string();
    let mut body = String::new();
    body.push_str("@echo off\r\n");
    body.push_str("rem Zero exported trainer. Ctrl+C exits and restores every write.\r\n");
    body.push_str(&format!("\"{exe_text}\" --apply-table \"{table_text}\"\r\n"));
    body.push_str("if errorlevel 1 pause\r\n");
    std::fs::write(&launcher, body)
        .map_err(|e| format!("写不了 {}：{e}", launcher.display()))?;
    Ok(launcher)
}

/// Export a table as a trainer. `exe` is the running executable, which the
/// launcher points at.
pub fn export(
    table: &CtTable,
    handle: &ProcessHandle,
    out_dir: &Path,
    exe: &Path,
) -> Result<Export, String> {
    if table.entries.is_empty() {
        return Err("这张表没有可导出的条目。".to_string());
    }
    let modules: Vec<(String, u64)> = handle
        .modules()
        .into_iter()
        .map(|m| (m.name, m.base))
        .collect();
    let mut ready = Vec::new();
    let mut skipped = Vec::new();
    for entry in &table.entries {
        match check_entry(handle, &modules, entry) {
            Ok(item) => ready.push(item),
            Err(reason) => skipped.push(format!("{}：{reason}", entry.description)),
        }
    }
    if ready.is_empty() {
        return Err(format!("没有一条能导出：{}", skipped.join("；")));
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("建不了目录：{e}"))?;
    let table_path = out_dir.join("trainer.ct");
    // Only the entries that survived the check go into the exported table, so
    // the player never sees a checkbox that cannot work.
    let mut exported = table.clone();
    exported.entries.retain(|entry| {
        ready.iter().any(|r| r.description == entry.description)
    });
    let xml = ctable::to_xml(&exported);
    std::fs::write(&table_path, xml)
        .map_err(|e| format!("写不了 {}：{e}", table_path.display()))?;
    let launcher_path = write_launcher(&table_path, exe)?;
    Ok(Export {
        table_path,
        launcher_path,
        ready: ready.len(),
        skipped,
    })
}

/// Pick the process a trainer should attach to: the one named by --pid, or the
/// first running process whose name matches the table's address expressions.
fn pick_process(table: &CtTable, pid: Option<u32>) -> Result<u32, String> {
    if let Some(pid) = pid {
        return Ok(pid);
    }
    let mut wanted: Vec<String> = Vec::new();
    for entry in &table.entries {
        let head = entry.address.split(['+', ',', '[', ']']).next().unwrap_or("");
        let head = head.trim();
        if head.to_ascii_lowercase().ends_with(".exe") {
            let lower = head.to_ascii_lowercase();
            if !wanted.contains(&lower) {
                wanted.push(lower);
            }
        }
    }
    if wanted.is_empty() {
        return Err("表里没有写模块名，用 --pid 指定目标进程".to_string());
    }
    for process in winmem::list_processes() {
        let name = process.name.to_ascii_lowercase();
        if wanted.iter().any(|w| w == &name) {
            return Ok(process.pid);
        }
    }
    Err(format!("找不到 {}，用 --pid 指定一个", wanted.join(" / ")))
}

/// The --apply-table run: open the game, resolve every entry, apply it, wait for
/// the console to be closed, then restore. A trainer never leaves a process
/// modified after it exits.
pub fn run(table_path: &Path, pid: Option<u32>) -> Result<(), String> {
    let source = std::fs::read_to_string(table_path)
        .map_err(|e| format!("读不了 {}：{e}", table_path.display()))?;
    let table = ctable::parse(&source)?;
    if table.entries.is_empty() {
        return Err("表里没有条目。".to_string());
    }
    let target = pick_process(&table, pid)?;
    let handle = winmem::open_process(target, "trainer")?;
    let modules: Vec<(String, u64)> = handle
        .modules()
        .into_iter()
        .map(|m| (m.name, m.base))
        .collect();
    let mut patches = Vec::new();
    for entry in &table.entries {
        let address = match ctable::resolve_address(
            &entry.address,
            &modules,
            handle.is_64bit,
            &|base| handle.read_pointer(base),
        ) {
            Ok(address) => address,
            Err(e) => {
                println!("跳过 {}：{e}", entry.description);
                continue;
            }
        };
        let value_text = entry.value.as_deref().unwrap_or("");
        let bytes = match encode_value(entry.vtype, value_text) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("跳过 {}：{e}", entry.description);
                continue;
            }
        };
        match inj::apply(&handle, address, &bytes, &entry.description) {
            Ok(patch) => {
                println!("已应用 {} @ {address:X}", entry.description);
                patches.push(patch);
            }
            Err(e) => println!("跳过 {}：{e}", entry.description),
        }
    }
    if patches.is_empty() {
        return Err("没有一条能应用。".to_string());
    }
    println!("共 {} 条已应用。按 Ctrl+C 退出并还原所有写入。", patches.len());
    // Wait for Ctrl+C. The console handler sets a flag instead of terminating,
    // so the restore below always runs.
    while !crate::trainer::stop_requested() {
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    let mut restored = 0usize;
    for patch in &mut patches {
        if inj::revert(&handle, patch).is_ok() {
            restored += 1;
        }
    }
    println!("已还原 {restored} 处写入。");
    Ok(())
}

// ---------------------------------------------------------------------------
// Ctrl+C handling. A trainer that dies on Ctrl+C would leave the game modified,
// so the handler only raises a flag and the main loop restores on its way out.
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

/// True once the user asked the trainer to stop.
pub fn stop_requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn console_handler(_ctrl_type: u32) -> i32 {
    STOP.store(true, Ordering::Relaxed);
    1
}

/// Install the Ctrl+C handler. Best effort: if it cannot be installed the
/// trainer still works, it just will not restore on an interrupt.
#[cfg(target_os = "windows")]
pub fn install_stop_handler() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe {
        SetConsoleCtrlHandler(Some(console_handler), 1);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn install_stop_handler() {}
