//! Lua scripting for cheat tables.
//!
//! Cheat Engine tables carry Lua scripts that drive the table: read and write
//! memory, wait, walk pointer chains. This module embeds a real Lua runtime
//! (mlua) and exposes the small CE-flavoured API those scripts actually use,
//! so a shared table's logic runs instead of being ignored.
//!
//! Every host function that touches the target process goes through the same
//! ProcessHandle the rest of the app uses, and writes are recorded as patches
//! so they can be undone like any other change.

use crate::winmem::ProcessHandle;
use mlua::{Lua, MultiValue, Table, Value};
use std::sync::Arc;

/// What a script run produced.
#[derive(Clone, Debug)]
pub struct RunResult {
    /// Anything the script printed with print().
    pub output: Vec<String>,
    /// A value the script returned, when it returned something printable.
    pub returned: Option<String>,
    /// Patches the script applied with autoAssemble, already reversible.
    ///
    /// The caller owns them from here: they go into the same patch list as
    /// hand-written edits, so a script's writes can be undone like any other.
    pub patches: Vec<crate::inj::Patch>,
    /// Blocks the script allocated with allocateMemory. Freed when the script
    /// ends unless the script called freeMemory on them itself.
    pub allocations: Vec<u64>,
}

/// Parse a hex address string the way CE scripts write them:
/// "0x1234", "1234ABCD", or a decimal number string.
fn parse_address_text(text: &str) -> Option<u64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).ok();
    }
    if lower.chars().all(|c| c.is_ascii_hexdigit()) && lower.chars().any(|c| c.is_ascii_alphabetic()) {
        return u64::from_str_radix(&lower, 16).ok();
    }
    t.parse::<u64>()
        .ok()
        .or_else(|| u64::from_str_radix(&lower, 16).ok())
}

/// Read `count` bytes and return them as a Lua table of numbers.
fn bytes_to_table<'lua>(lua: &'lua Lua, bytes: &[u8]) -> mlua::Result<Table<'lua>> {
    let table = lua.create_table()?;
    for (index, byte) in bytes.iter().enumerate() {
        table.set(index + 1, *byte)?;
    }
    Ok(table)
}

/// Turn a Lua value into bytes for a write: a table of numbers, or a hex string.
fn value_to_bytes(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Table(table) => {
            let mut out = Vec::new();
            for item in table.clone().sequence_values::<i64>() {
                let number = item.map_err(|e| format!("写入表里有非数字：{e}"))?;
                if !(0..=255).contains(&number) {
                    return Err(format!("字节值 {number} 超出 0-255。"));
                }
                out.push(number as u8);
            }
            Ok(out)
        }
        Value::String(text) => {
            let bytes = text.as_bytes().to_vec();
            Ok(bytes)
        }
        Value::Integer(number) => Ok(vec![*number as u8]),
        Value::Number(number) => Ok(vec![*number as u8]),
        _ => Err("写入需要一个字节表或字符串。".to_string()),
    }
}

/// Run a CE-style script against a target process.
///
/// `read` and `write` are supplied so the caller keeps control of how memory is
/// reached and how writes are recorded.
pub fn run_script(
    source: &str,
    handle: Option<Arc<ProcessHandle>>,
) -> Result<RunResult, String> {
    let lua = Lua::new();
    let output = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    // Writes a script makes are collected here, then handed back to the caller
    // so they join the normal patch list and stay individually undoable.
    let patches = Arc::new(std::sync::Mutex::new(Vec::<crate::inj::Patch>::new()));
    // Blocks allocated by the script; anything still here when it ends is freed.
    let allocations = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
    // Symbols registered by registerSymbol: name -> address expression.
    let symbols = Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::<String, u64>::new(),
    ));

    // print(): CE scripts use it freely; collect instead of writing to stdout,
    // which the GUI build suppresses.
    {
        let sink = output.clone();
        let print = lua
            .create_function(move |_, args: MultiValue| {
                let mut line = String::new();
                for (index, value) in args.iter().enumerate() {
                    if index > 0 {
                        line.push('\t');
                    }
                    line.push_str(&format_value(value));
                }
                if let Ok(mut sink) = sink.lock() {
                    sink.push(line);
                }
                Ok(())
            })
            .map_err(|e| format!("无法创建 print：{e}"))?;
        lua.globals()
            .set("print", print)
            .map_err(|e| format!("无法注册 print：{e}"))?;
    }

    // readInteger / writeInteger / readBytes / writeBytes and friends.
    let globals = lua.globals();
    let read_handle = handle.clone();
    let read_integer = lua
        .create_function(move |_, (address, size): (Value, Option<i64>)| {
            let address = value_to_address(&address).ok_or_else(|| {
                mlua::Error::RuntimeError("readInteger 需要地址".to_string())
            })?;
            let size = size.unwrap_or(4);
            let Some(handle) = read_handle.as_ref() else {
                return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
            };
            let mut buffer = vec![0u8; size.clamp(1, 8) as usize];
            if !handle.read(address, &mut buffer) {
                return Err(mlua::Error::RuntimeError(format!(
                    "无法读取 {address:X}"
                )));
            }
            let mut value: i64 = 0;
            for (index, byte) in buffer.iter().enumerate() {
                value |= (*byte as i64) << (index * 8);
            }
            Ok(value)
        })
        .map_err(|e| format!("无法创建 readInteger：{e}"))?;
    globals
        .set("readInteger", read_integer)
        .map_err(|e| format!("无法注册 readInteger：{e}"))?;

    let write_handle = handle.clone();
    let write_integer = lua
        .create_function(move |_, (address, value, size): (Value, i64, Option<i64>)| {
            let address = value_to_address(&address).ok_or_else(|| {
                mlua::Error::RuntimeError("writeInteger 需要地址".to_string())
            })?;
            let size = size.unwrap_or(4).clamp(1, 8) as usize;
            let Some(handle) = write_handle.as_ref() else {
                return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
            };
            let bytes: Vec<u8> = (0..size).map(|i| ((value >> (i * 8)) & 0xFF) as u8).collect();
            if !handle.write(address, &bytes) {
                return Err(mlua::Error::RuntimeError(format!(
                    "无法写入 {address:X}（只读或权限不足）"
                )));
            }
            Ok(())
        })
        .map_err(|e| format!("无法创建 writeInteger：{e}"))?;
    globals
        .set("writeInteger", write_integer)
        .map_err(|e| format!("无法注册 writeInteger：{e}"))?;

    let read_bytes_handle = handle.clone();
    let read_bytes = lua
        .create_function(move |lua, (address, count): (Value, i64)| {
            let address = value_to_address(&address).ok_or_else(|| {
                mlua::Error::RuntimeError("readBytes 需要地址".to_string())
            })?;
            let count = count.clamp(1, 4096) as usize;
            let Some(handle) = read_bytes_handle.as_ref() else {
                return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
            };
            let mut buffer = vec![0u8; count];
            if !handle.read(address, &mut buffer) {
                return Err(mlua::Error::RuntimeError(format!(
                    "无法读取 {address:X}"
                )));
            }
            bytes_to_table(lua, &buffer)
        })
        .map_err(|e| format!("无法创建 readBytes：{e}"))?;
    globals
        .set("readBytes", read_bytes)
        .map_err(|e| format!("无法注册 readBytes：{e}"))?;

    let write_bytes_handle = handle.clone();
    let write_bytes = lua
        .create_function(move |_, (address, bytes): (Value, Value)| {
            let address = value_to_address(&address).ok_or_else(|| {
                mlua::Error::RuntimeError("writeBytes 需要地址".to_string())
            })?;
            let payload = value_to_bytes(&bytes).map_err(mlua::Error::RuntimeError)?;
            let Some(handle) = write_bytes_handle.as_ref() else {
                return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
            };
            if !handle.write(address, &payload) {
                return Err(mlua::Error::RuntimeError(format!(
                    "无法写入 {address:X}（只读或权限不足）"
                )));
            }
            Ok(())
        })
        .map_err(|e| format!("无法创建 writeBytes：{e}"))?;
    globals
        .set("writeBytes", write_bytes)
        .map_err(|e| format!("无法注册 writeBytes：{e}"))?;

    // getAddress: "module+offset" or a comma pointer chain, plus any symbol a
    // script registered with registerSymbol.
    let addr_handle = handle.clone();
    let addr_symbols = symbols.clone();
    let get_address = lua
        .create_function(move |_, text: String| {
            let Some(handle) = addr_handle.as_ref() else {
                return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
            };
            let expanded = expand_symbol(&addr_symbols, &text);
            let modules = handle.modules();
            match crate::addr::resolve(Some(handle), &modules, &expanded) {
                Some(address) => Ok(address),
                None => Err(mlua::Error::RuntimeError(format!(
                    "无法解析地址「{text}」。"
                ))),
            }
        })
        .map_err(|e| format!("无法创建 getAddress：{e}"))?;
    globals
        .set("getAddress", get_address)
        .map_err(|e| format!("无法注册 getAddress：{e}"))?;

    // registerSymbol(name, address): a script-local symbol, resolved before
    // module names so a table can name its own anchors.
    {
        let sym_table = symbols.clone();
        let register_symbol = lua
            .create_function(move |_, (name, address): (String, Value)| {
                let value = value_to_address(&address).ok_or_else(|| {
                    mlua::Error::RuntimeError("registerSymbol 需要地址".to_string())
                })?;
                if let Ok(mut table) = sym_table.lock() {
                    table.insert(name.to_ascii_lowercase(), value);
                }
                Ok(())
            })
            .map_err(|e| format!("无法创建 registerSymbol：{e}"))?;
        globals
            .set("registerSymbol", register_symbol)
            .map_err(|e| format!("无法注册 registerSymbol：{e}"))?;
    }

    // getSymbol(name): look one back up.
    {
        let sym_table = symbols.clone();
        let get_symbol = lua
            .create_function(move |_, name: String| {
                let table = sym_table
                    .lock()
                    .map_err(|_| mlua::Error::RuntimeError("符号表不可用".to_string()))?;
                match table.get(&name.to_ascii_lowercase()) {
                    Some(address) => Ok(*address),
                    None => Err(mlua::Error::RuntimeError(format!("没有符号「{name}」。"))),
                }
            })
            .map_err(|e| format!("无法创建 getSymbol：{e}"))?;
        globals
            .set("getSymbol", get_symbol)
            .map_err(|e| format!("无法注册 getSymbol：{e}"))?;
    }

    // readPointer(address[, size]): one dereference at the target's width.
    {
        let ptr_handle = handle.clone();
        let read_pointer = lua
            .create_function(move |_, (address, size): (Value, Option<i64>)| {
                let address = value_to_address(&address).ok_or_else(|| {
                    mlua::Error::RuntimeError("readPointer 需要地址".to_string())
                })?;
                let Some(handle) = ptr_handle.as_ref() else {
                    return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
                };
                match size {
                    None => handle.read_pointer(address).ok_or_else(|| {
                        mlua::Error::RuntimeError(format!("无法读取 {address:X} 处的指针"))
                    }),
                    Some(width) => {
                        let width = width.clamp(1, 8) as usize;
                        let mut buffer = vec![0u8; width];
                        if !handle.read(address, &mut buffer) {
                            return Err(mlua::Error::RuntimeError(format!(
                                "无法读取 {address:X}"
                            )));
                        }
                        let mut value: u64 = 0;
                        for (index, byte) in buffer.iter().enumerate() {
                            value |= (*byte as u64) << (index * 8);
                        }
                        Ok(value)
                    }
                }
            })
            .map_err(|e| format!("无法创建 readPointer：{e}"))?;
        globals
            .set("readPointer", read_pointer)
            .map_err(|e| format!("无法注册 readPointer：{e}"))?;
    }

    // writePointer(address, value[, size])
    {
        let ptr_handle = handle.clone();
        let write_pointer = lua
            .create_function(move |_, (address, value, size): (Value, u64, Option<i64>)| {
                let address = value_to_address(&address).ok_or_else(|| {
                    mlua::Error::RuntimeError("writePointer 需要地址".to_string())
                })?;
                let Some(handle) = ptr_handle.as_ref() else {
                    return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
                };
                let width = match size {
                    Some(w) => w.clamp(1, 8) as usize,
                    None => {
                        if handle.is_64bit {
                            8
                        } else {
                            4
                        }
                    }
                };
                let bytes: Vec<u8> = (0..width).map(|i| ((value >> (i * 8)) & 0xFF) as u8).collect();
                if !handle.write(address, &bytes) {
                    return Err(mlua::Error::RuntimeError(format!(
                        "无法写入 {address:X}（只读或权限不足）"
                    )));
                }
                Ok(())
            })
            .map_err(|e| format!("无法创建 writePointer：{e}"))?;
        globals
            .set("writePointer", write_pointer)
            .map_err(|e| format!("无法注册 writePointer：{e}"))?;
    }

    // AOBScan(pattern[, from, to]): returns every hit as a table of addresses.
    {
        let aob_handle = handle.clone();
        let aob_scan = lua
            .create_function(move |lua, (pattern, from, to): (String, Option<Value>, Option<Value>)| {
                let Some(handle) = aob_handle.as_ref() else {
                    return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
                };
                let pattern = crate::aob::AobPattern::parse(&pattern)
                    .map_err(mlua::Error::RuntimeError)?;
                let mut regions = handle.regions(false);
                if let Some(from) = from.as_ref().and_then(value_to_address) {
                    regions.retain(|r| r.base + r.size > from);
                }
                if let Some(to) = to.as_ref().and_then(value_to_address) {
                    regions.retain(|r| r.base < to);
                }
                let scan = crate::aob::start_scan(handle.clone(), regions, pattern, 4096)
                    .map_err(mlua::Error::RuntimeError)?;
                let done = scan
                    .rx
                    .recv()
                    .map_err(|_| mlua::Error::RuntimeError("扫描线程没有返回结果".to_string()))?;
                let table = lua.create_table()?;
                for (index, address) in done.hits.iter().enumerate() {
                    table.set(index + 1, *address)?;
                }
                Ok(table)
            })
            .map_err(|e| format!("无法创建 AOBScan：{e}"))?;
        globals
            .set("AOBScan", aob_scan)
            .map_err(|e| format!("无法注册 AOBScan：{e}"))?;
    }

    // allocateMemory(size): a private block inside the target, freed when the
    // script ends unless freeMemory takes it first.
    {
        let alloc_handle = handle.clone();
        let alloc_store = allocations.clone();
        let allocate_memory = lua
            .create_function(move |_, size: i64| {
                let Some(handle) = alloc_handle.as_ref() else {
                    return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
                };
                let size = (size.max(0x1000)) as usize;
                let address = handle.allocate(size).map_err(mlua::Error::RuntimeError)?;
                if let Ok(mut store) = alloc_store.lock() {
                    store.push(address);
                }
                Ok(address)
            })
            .map_err(|e| format!("无法创建 allocateMemory：{e}"))?;
        globals
            .set("allocateMemory", allocate_memory)
            .map_err(|e| format!("无法注册 allocateMemory：{e}"))?;
    }
    {
        let free_handle = handle.clone();
        let alloc_store = allocations.clone();
        let free_memory = lua
            .create_function(move |_, address: Value| {
                let address = value_to_address(&address).ok_or_else(|| {
                    mlua::Error::RuntimeError("freeMemory 需要地址".to_string())
                })?;
                let Some(handle) = free_handle.as_ref() else {
                    return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
                };
                handle.free(address).map_err(mlua::Error::RuntimeError)?;
                if let Ok(mut store) = alloc_store.lock() {
                    store.retain(|a| *a != address);
                }
                Ok(())
            })
            .map_err(|e| format!("无法创建 freeMemory：{e}"))?;
        globals
            .set("freeMemory", free_memory)
            .map_err(|e| format!("无法注册 freeMemory：{e}"))?;
    }

    // sleep(ms): a real pause, capped so a runaway script cannot hang the UI.
    {
        let sleep_fn = lua
            .create_function(move |_, ms: i64| {
                let ms = ms.clamp(0, 5000) as u64;
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(())
            })
            .map_err(|e| format!("无法创建 sleep：{e}"))?;
        globals
            .set("sleep", sleep_fn)
            .map_err(|e| format!("无法注册 sleep：{e}"))?;
    }

    // autoAssemble(source): assemble here, then write it as a reversible patch.
    //
    // CE scripts write either "address:" lines or a bare block. We assemble at
    // the address named by the first line when it looks like one, otherwise at
    // a freshly allocated code cave; either way the write is recorded.
    {
        let asm_handle = handle.clone();
        let patch_store = patches.clone();
        let auto_assemble = lua
            .create_function(move |_, source: String| {
                let Some(handle) = asm_handle.as_ref() else {
                    return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
                };
                let (origin, body) = split_asm_target(&source);
                let origin = match origin {
                    Some(text) => {
                        let modules = handle.modules();
                        crate::addr::resolve(Some(handle), &modules, &text).ok_or_else(|| {
                            mlua::Error::RuntimeError(format!("无法解析地址「{text}」。"))
                        })?
                    }
                    None => handle
                        .allocate(0x1000)
                        .map_err(mlua::Error::RuntimeError)?,
                };
                let assembled =
                    crate::assembler::assemble(&body, origin).map_err(mlua::Error::RuntimeError)?;
                let label = format!("autoAssemble@{origin:X}");
                let patch = crate::inj::apply(handle, origin, &assembled.bytes, &label)
                    .map_err(mlua::Error::RuntimeError)?;
                if let Ok(mut store) = patch_store.lock() {
                    store.push(patch);
                }
                Ok(origin)
            })
            .map_err(|e| format!("无法创建 autoAssemble：{e}"))?;
        globals
            .set("autoAssemble", auto_assemble)
            .map_err(|e| format!("无法注册 autoAssemble：{e}"))?;
    }

    // createTimer(intervalMs, callback): CE's recurring script tick.
    //
    // The callback is queued here and run after the main body, on this same
    // thread, for a bounded number of ticks. Nothing is left running once
    // run_script returns, so a table cannot leak a timer into the next run.
    let timers: Arc<std::sync::Mutex<Vec<(u64, mlua::RegistryKey)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    {
        let timer_store = timers.clone();
        let create_timer = lua
            .create_function(move |lua, (interval, callback): (i64, mlua::Function)| {
                let interval = interval.clamp(10, 10_000) as u64;
                let key = lua.create_registry_value(callback)?;
                if let Ok(mut store) = timer_store.lock() {
                    store.push((interval, key));
                }
                Ok(())
            })
            .map_err(|e| format!("无法创建 createTimer：{e}"))?;
        globals
            .set("createTimer", create_timer)
            .map_err(|e| format!("无法注册 createTimer：{e}"))?;
    }

    let result = lua
        .load(source)
        .set_name("cheat script")
        .exec();

    // Timers registered by the script get their ticks now, after the body, so
    // they observe whatever state the body left behind.
    if result.is_ok() {
        let queued: Vec<(u64, mlua::RegistryKey)> = timers
            .lock()
            .map(|mut store| std::mem::take(&mut *store))
            .unwrap_or_default();
        for (interval, key) in queued {
            let Ok(callback) = lua.registry_value::<mlua::Function>(&key) else {
                continue;
            };
            // Bounded: 100 ticks or 2 seconds of wall clock, whichever is first.
            let started = std::time::Instant::now();
            for tick in 0..100u64 {
                if started.elapsed() > std::time::Duration::from_secs(2) {
                    break;
                }
                if callback.call::<_, ()>(tick).is_err() {
                    break;
                }
                if tick + 1 < 100 {
                    std::thread::sleep(std::time::Duration::from_millis(interval.min(50)));
                }
            }
        }
    }

    let collected = output.lock().map(|o| o.clone()).unwrap_or_default();
    // Anything the script allocated is released now unless it freed it itself:
    // a script cannot leak memory into a process the user cannot see.
    let mut allocated = allocations
        .lock()
        .map(|mut store| std::mem::take(&mut *store))
        .unwrap_or_default();
    let mut leftover = Vec::new();
    if let Some(handle) = handle.as_ref() {
        for address in allocated.drain(..) {
            if handle.free(address).is_err() {
                leftover.push(address);
            }
        }
    }
    let applied = patches
        .lock()
        .map(|mut store| std::mem::take(&mut *store))
        .unwrap_or_default();
    match result {
        Ok(()) => Ok(RunResult {
            output: std::mem::take(&mut collected.clone()),
            returned: None,
            patches: applied,
            allocations: leftover,
        }),
        Err(e) => Err(format!("{e}")),
    }
}

/// Split an autoAssemble source into an optional target address and the body.
///
/// CE scripts put the destination on its own line as "address:" or
/// "address: instruction". A leading comment block is ignored.
fn split_asm_target(source: &str) -> (Option<String>, String) {
    let mut lines = source.lines().peekable();
    let mut body = String::new();
    let mut origin = None;
    let mut first = true;
    for line in lines.by_ref() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if first {
            first = false;
            // "game.exe+1234:" or "0x7FF...:" — a target line has a colon and
            // nothing that looks like an instruction before it.
            if let Some(colon) = trimmed.find(':') {
                let head = trimmed[..colon].trim();
                let tail = trimmed[colon + 1..].trim();
                let looks_like_target = !head.contains(' ')
                    && !head.is_empty()
                    && (head.contains('.') || head.contains('+') || head.contains('[')
                        || head.starts_with("0x") || head.chars().all(|c| c.is_ascii_hexdigit()));
                if looks_like_target {
                    origin = Some(head.to_string());
                    if !tail.is_empty() {
                        body.push_str(tail);
                        body.push('\n');
                    }
                    continue;
                }
            }
        }
        body.push_str(line);
        body.push('\n');
    }
    (origin, body)
}

/// Replace any registered symbol name inside an address expression.
///
/// Longest name first so "player" does not eat "player_base".
fn expand_symbol(
    symbols: &Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    text: &str,
) -> String {
    let Ok(table) = symbols.lock() else {
        return text.to_string();
    };
    if table.is_empty() {
        return text.to_string();
    }
    let mut names: Vec<&String> = table.keys().collect();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    let mut out = text.to_string();
    for name in names {
        let Some(address) = table.get(name) else {
            continue;
        };
        let lower = out.to_ascii_lowercase();
        if !lower.contains(name.as_str()) {
            continue;
        }
        // Only whole-token matches: a symbol must not rewrite a longer name.
        let mut rebuilt = String::new();
        let mut rest = out.as_str();
        while let Some(pos) = rest.to_ascii_lowercase().find(name.as_str()) {
            let before_ok = pos == 0
                || !rest[..pos]
                    .chars()
                    .next_back()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);
            let after_index = pos + name.len();
            let after_ok = after_index >= rest.len()
                || !rest[after_index..]
                    .chars()
                    .next()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);
            if before_ok && after_ok {
                rebuilt.push_str(&rest[..pos]);
                rebuilt.push_str(&format!("{address:X}"));
                rest = &rest[after_index..];
            } else {
                rebuilt.push_str(&rest[..after_index]);
                rest = &rest[after_index..];
            }
        }
        rebuilt.push_str(rest);
        out = rebuilt;
    }
    out
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => {
            if n.fract() == 0.0 {
                format!("{n:.0}")
            } else {
                format!("{n}")
            }
        }
        Value::String(s) => s.to_string_lossy().to_string(),
        other => format!("{other:?}"),
    }
}

/// Accept a number or a hex string as an address.
fn value_to_address(value: &Value) -> Option<u64> {
    match value {
        Value::Integer(i) => Some(*i as u64),
        Value::Number(n) => Some(*n as u64),
        Value::String(s) => parse_address_text(&s.to_string_lossy()),
        _ => None,
    }
}