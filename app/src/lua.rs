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

    // getAddress: "module+offset" against the target's modules.
    let addr_handle = handle.clone();
    let get_address = lua
        .create_function(move |_, text: String| {
            let Some(handle) = addr_handle.as_ref() else {
                return Err(mlua::Error::RuntimeError("没有连接进程".to_string()));
            };
            let modules = handle.modules();
            let pairs: Vec<(String, u64)> = modules.iter().map(|m| (m.name.clone(), m.base)).collect();
            match crate::ctable::resolve_address(&text, &pairs, handle.is_64bit, &|a| {
                handle.read_pointer(a)
            }) {
                Ok(address) => Ok(address),
                Err(e) => Err(mlua::Error::RuntimeError(e)),
            }
        })
        .map_err(|e| format!("无法创建 getAddress：{e}"))?;
    globals
        .set("getAddress", get_address)
        .map_err(|e| format!("无法注册 getAddress：{e}"))?;

    let result = lua
        .load(source)
        .set_name("cheat script")
        .exec();

    let mut collected = output.lock().map(|o| o.clone()).unwrap_or_default();
    match result {
        Ok(()) => Ok(RunResult {
            output: std::mem::take(&mut collected),
            returned: None,
        }),
        Err(e) => Err(format!("{e}")),
    }
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