//! A toggleable MCP (Model Context Protocol) service.
//!
//! Zero is a windowed app, so it cannot be launched as a stdio MCP server the
//! way most clients expect. Instead this runs a small HTTP server on localhost
//! that speaks MCP: press the toggle to start it, point an AI agent at the URL,
//! press it again to stop. Nothing listens while it is off.
//!
//! The server is deliberately self-contained. It owns its own engine session
//! (its own process handle, its own scan state) rather than reaching into the
//! GUI: sharing mutable state between a network thread and the UI thread is the
//! kind of thing that works until it does not.
//!
//! Transport: MCP over HTTP with JSON-RPC 2.0. Requests are POSTed to /mcp and
//! answered with a JSON body. That covers what local agent clients need without
//! pulling in an async runtime.

use crate::ctable;
use crate::debugger::{BpKind, DebugSession, Slot};
use crate::inj;
use crate::model::{ScanHit, ScanType, VType};
use crate::scan::{self, ScanConfig};
use crate::triage;
use crate::winmem::{self, ProcessHandle};
use serde_json::{json, Value as Json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The running service's handle to its thread.
pub struct Service {
    pub port: u16,
    pub token: String,
    stop: Arc<AtomicBool>,
    /// Set once the listener is bound, so the UI can report a real port.
    pub bound: Arc<AtomicBool>,
}

impl Service {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp?token={}", self.port, self.token)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // Nudge the accept loop: a connecting socket wakes it out of accept().
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// Per-session engine state owned by the MCP service.
struct Engine {
    handle: Option<Arc<ProcessHandle>>,
    hits: Vec<ScanHit>,
    last_kind: String,
    addresses: Vec<(String, u64, VType)>,
    feedback: Vec<triage::RejectedTrait>,
    /// A watch session that outlives one request. An agent has to be able to
    /// start watching, go change the value in the game, then come back and ask
    /// what moved; a single blocking call cannot do that.
    watch: Option<WatchSession>,
    /// Every write this session made, kept so it can be undone. The GUI has a
    /// patch list for exactly this reason; an MCP client that can write without
    /// being able to restore is worse than one that cannot write at all.
    patches: Vec<inj::Patch>,
    /// 会话式调试器：zero_dbg_start 附加，zero_dbg_hits 读命中。
    dbg: Option<Arc<DebugSession>>,
}

/// A background watch: its own thread keeps sampling while the agent works.
struct WatchSession {
    watches: Arc<Mutex<Vec<crate::focus::Watch>>>,
    stop: Arc<AtomicBool>,
    started: std::time::Instant,
}

impl Engine {
    fn new() -> Self {
        Self {
            handle: None,
            hits: Vec::new(),
            last_kind: String::new(),
            addresses: Vec::new(),
            feedback: Vec::new(),
            watch: None,
            patches: Vec::new(),
            dbg: None,
        }
    }

    fn dbg_active(&self) -> bool {
        self.dbg
            .as_ref()
            .map(|d| d.status().attached)
            .unwrap_or(false)
    }
}

/// Start the service on a port. Returns as soon as the listener is bound.
pub fn start(port: u16) -> Result<Service, String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("端口 {port} 无法绑定：{e}"))?;
    let actual = listener
        .local_addr()
        .map(|a| a.port())
        .unwrap_or(port);
    let token = make_token();
    let stop = Arc::new(AtomicBool::new(false));
    let bound = Arc::new(AtomicBool::new(true));
    let engine = Arc::new(Mutex::new(Engine::new()));

    let stop_thread = stop.clone();
    let token_thread = token.clone();
    std::thread::Builder::new()
        .name("zero-mcp".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let engine = engine.clone();
                let token = token_thread.clone();
                // One thread per request: an agent may pipeline calls, and a
                // slow scan must not block the next request.
                std::thread::spawn(move || {
                    let _ = handle_connection(stream, &engine, &token);
                });
            }
        })
        .map_err(|e| format!("无法启动 MCP 线程：{e}"))?;

    Ok(Service {
        port: actual,
        token,
        stop,
        bound,
    })
}

/// A token that is unguessable enough for a loopback service.
///
/// Always available, not only in debug builds: the --selftest entry point is a
/// runtime check, so it has to compile into release builds too.
pub fn make_token_for_test() -> String {
    make_token()
}

pub fn tool_definitions_for_test() -> Vec<Json> {
    tool_definitions()
}

fn make_token() -> String {
    // A rotl on a 128-bit value drops the high bits, which left the first
    // thirteen hex digits zero in an earlier version. Mix with a proper
    // avalanche instead: xor-fold the shifted halves, then run a splitmix
    // style finaliser twice.
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let seed = nanos ^ (pid << 64) ^ 0x9E37_79B9_7F4A_7C15_1234_5678_9ABC_DEF1u128;

    let mut state = (seed ^ (seed >> 64)) as u64;
    let mut out = String::with_capacity(32);
    for _ in 0..2 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.push_str(&format!("{z:016x}"));
    }
    out
}

fn handle_connection(
    mut stream: TcpStream,
    engine: &Arc<Mutex<Engine>>,
    token: &str,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    // Read headers first, then exactly Content-Length bytes.
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buffer, b"\r\n\r\n") {
            break pos + 4;
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 1_000_000 {
            return write_response(&mut stream, 413, "text/plain", "request too large");
        }
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let request_line = header_text.lines().next().unwrap_or("").to_string();
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    // Every request must carry the token: this is the only thing standing
    // between a local port and any other program on the machine.
    if !request_line.contains(token) && !header_text.contains(token) {
        return write_response(&mut stream, 401, "text/plain", "unauthorized");
    }

    if request_line.starts_with("GET ") {
        // Health check: lets a client verify the service is up without a token
        // in the body.
        let body = json!({
            "service": "zero-scanner",
            "protocol": "mcp",
            "transport": "http",
        })
        .to_string();
        return write_response(&mut stream, 200, "application/json", &body);
    }
    if !request_line.starts_with("POST ") {
        return write_response(&mut stream, 405, "text/plain", "method not allowed");
    }

    let text = String::from_utf8_lossy(&body).to_string();
    let Ok(request) = serde_json::from_str::<Json>(&text) else {
        let body = json!({
            "jsonrpc": "2.0",
            "id": Json::Null,
            "error": { "code": -32700, "message": "parse error" }
        })
        .to_string();
        return write_response(&mut stream, 200, "application/json", &body);
    };

    match dispatch(&request, engine) {
        Some(response) => write_response(&mut stream, 200, "application/json", &response.to_string()),
        // Notifications have no reply.
        None => write_response(&mut stream, 202, "text/plain", ""),
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Handle one JSON-RPC message. Returns None for notifications.
fn dispatch(request: &Json, engine: &Arc<Mutex<Engine>>) -> Option<Json> {
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = request.get("id").cloned();
    let params = request.get("params").cloned().unwrap_or(Json::Null);

    // Anything without an id is a notification and must not be answered.
    let id = id?;

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "zero-scanner", "version": env!("CARGO_PKG_VERSION") }
        })),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(name, &args, engine) {
                Ok(text) => Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                })),
                Err(e) => Ok(json!({
                    "content": [{ "type": "text", "text": e }],
                    "isError": true
                })),
            }
        }
        "ping" => Ok(json!({})),
        other => Err(format!("unknown method: {other}")),
    };

    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": message }
        }),
    })
}

fn tool_definitions() -> Vec<Json> {
    vec![
        json!({
            "name": "zero_attach",
            "description": "连接到一个正在运行的进程，按 PID 或进程名。之后所有工具都作用于它。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pid": { "type": "integer", "description": "进程 PID" },
                    "name": { "type": "string", "description": "进程名，例如 game.exe" }
                }
            }
        }),
        json!({
            "name": "zero_status",
            "description": "查看当前连接状态、候选数量与已保存地址。",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "zero_scan",
            "description": "首次扫描内存里的数值。返回候选数量和最可能的几个地址。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "value": { "type": "string", "description": "要查找的数值" },
                    "vtype": { "type": "string", "description": "i8/i16/i32/i64/f32/f64/str，默认 i32" },
                    "mode": { "type": "string", "description": "exact/bigger/smaller/unknown，默认 exact" }
                },
                "required": ["value"]
            }
        }),
        json!({
            "name": "zero_next_scan",
            "description": "在已有结果上再次扫描，用变化方向缩小范围。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "value": { "type": "string", "description": "新数值（mode 为 increased/decreased/changed/unchanged 时可不填）" },
                    "mode": { "type": "string", "description": "exact/increased/decreased/changed/unchanged，默认 exact" }
                }
            }
        }),
        json!({
            "name": "zero_candidates",
            "description": "列出按可信度排序的候选地址，含打分理由。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "最多返回几条，默认 10" }
                }
            }
        }),
        json!({
            "name": "zero_read_memory",
            "description": "读取目标进程内存。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "十六进制地址，例如 7FF6A1B2C010" },
                    "size": { "type": "integer", "description": "字节数，默认 16" }
                },
                "required": ["address"]
            }
        }),
        json!({
            "name": "zero_write_memory",
            "description": "写入目标进程内存，可撤销（返回 undo 句柄）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "十六进制地址" },
                    "bytes": { "type": "string", "description": "字节，空格分隔的十六进制，例如 90 90 C3" }
                },
                "required": ["address", "bytes"]
            }
        }),
        json!({
            "name": "zero_pointer_scan",
            "description": "为一个地址反推稳定指针链（模块基址 + 若干偏移），重启后仍然有效。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "目标地址，十六进制" },
                    "depth": { "type": "integer", "description": "最大层数，1-6，默认 3" },
                    "max_offset": { "type": "integer", "description": "单级偏移上限，默认 4096" },
                    "limit": { "type": "integer", "description": "最多返回几条链，默认 10" }
                },
                "required": ["address"]
            }
        }),
        json!({
            "name": "zero_disassemble",
            "description": "反汇编目标进程里的一段代码，返回地址、字节与助记符。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "起始地址，十六进制" },
                    "count": { "type": "integer", "description": "指令条数，默认 20" }
                },
                "required": ["address"]
            }
        }),
        json!({
            "name": "zero_watch",
            "description": "观察队列（后台会话）：action=start 开始连续采样并立即返回，去游戏里改数值，再用 action=status 查谁跟着动，action=stop 结束。单次阻塞调用无法在采样期间改值，所以用会话。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "start / status / stop，默认 start" },
                    "limit": { "type": "integer", "description": "观察几个地址，默认 5" },
                    "seconds": { "type": "number", "description": "仅 action=status 显示的采样时长，默认 0" }
                }
            }
        }),
        json!({
            "name": "zero_ct_table",
            "description": "Cheat Engine 的 .ct 表格。action=read 解析文件并列出条目（可解析指针链地址）；action=write 把条目写成 .ct 文件；action=build 直接从当前地址列表生成表格内容返回。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "read / write / build，默认 read" },
                    "path": { "type": "string", "description": "action=read/write 时的 .ct 文件路径" },
                    "resolve": { "type": "boolean", "description": "action=read 时是否把地址解析成真实地址，默认 false" },
                    "overwrite": { "type": "boolean", "description": "action=write 时允许覆盖已存在的文件，默认 false" }
                }
            }
        }),
        json!({
            "name": "zero_lua",
            "description": "执行 Cheat Engine 风格的 Lua 脚本，可直接读写目标进程。可用函数：print / readInteger / writeInteger / readBytes / writeBytes / getAddress。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Lua 脚本内容" }
                },
                "required": ["source"]
            }
        }),
        json!({
            "name": "zero_undo",
            "description": "还原本会话做过的内存写入。不带 address 时还原全部；带上只还原该地址。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "可选，十六进制地址；省略则还原全部" }
                }
            }
        }),
        json!({
            "name": "zero_add_address",
            "description": "把一个地址加入地址列表。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "十六进制地址" },
                    "description": { "type": "string", "description": "描述" },
                    "vtype": { "type": "string", "description": "数值类型，默认 i32" }
                },
                "required": ["address"]
            }
        }),
        json!({
            "name": "zero_aliases",
            "description": "管理地址别名：给地址起个名字，之后所有地址框都能用这个名字（含指针链 game.exe+0x10,0x18）。action=list 列出、set 新增或改名、remove 删除。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "list（默认）/ set / remove" },
                    "name": { "type": "string", "description": "别名，字母或下划线开头" },
                    "expression": { "type": "string", "description": "别名指向的地址表达式" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "zero_symbols",
            "description": "列出已加载模块的导出/导入符号，或把一个地址反查成最近的导出名（module!Name+0x12）。module 省略时查所有模块的导出。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "module": { "type": "string", "description": "模块名，例如 game.exe；省略则汇总所有模块" },
                    "address": { "type": "string", "description": "反查这个地址最近的导出符号（十六进制）" },
                    "limit": { "type": "integer", "description": "最多返回多少条，默认 40" }
                }
            }
        }),        json!({
            "name": "zero_dbg_start",
            "description": "对当前进程设硬件写入/读写断点，回答「谁在改这个地址」。会话式：start 之后去游戏里改值，再调 zero_dbg_hits 读命中。再次 start 会先停掉上一个。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "十六进制地址（game.exe+偏移 或 0x 前缀均可）" },
                    "kind": { "type": "string", "description": "write（默认）或 readwrite" }
                },
                "required": ["address"]
            }
        }),
        json!({
            "name": "zero_dbg_hits",
            "description": "读取硬件断点命中：命中指令、寄存器、写后值。clear=true 时读取后清空并停止调试。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clear": { "type": "boolean", "description": "读取后停止调试并清空（默认 false）" }
                }
            }
        })
    ]
}

fn parse_hex_address(text: &str) -> Option<u64> {
    let t = text.trim().trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(t, 16).ok()
}

fn vtype_of(text: &str) -> VType {
    match text.trim().to_ascii_lowercase().as_str() {
        "i8" | "byte" => VType::I8,
        "i16" | "word" => VType::I16,
        "i64" | "qword" => VType::I64,
        "f32" | "float" => VType::F32,
        "f64" | "double" => VType::F64,
        "str" | "string" => VType::Str,
        "auto" => VType::Auto,
        _ => VType::I32,
    }
}

fn scan_type_of(text: &str) -> ScanType {
    match text.trim().to_ascii_lowercase().as_str() {
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

fn engine_handle(engine: &Arc<Mutex<Engine>>) -> Result<Arc<ProcessHandle>, String> {
    let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
    guard
        .handle
        .clone()
        .ok_or_else(|| "还没有连接进程，先调用 zero_attach".to_string())
}

fn call_tool(name: &str, args: &Json, engine: &Arc<Mutex<Engine>>) -> Result<String, String> {
    match name {
        "zero_attach" => {
            // Clients send numbers inconsistently: some serialise a numeric id
            // as a JSON number, some as a string. Accept both rather than
            // failing an attach over a formatting difference.
            let pid = args.get("pid").and_then(|p| {
                p.as_u64()
                    .map(|n| n as u32)
                    .or_else(|| p.as_str().and_then(|s| s.trim().parse::<u32>().ok()))
            });
            let resolved = match pid {
                Some(pid) => Some((pid, format!("pid {pid}"))),
                None => {
                    let wanted = args
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if wanted.is_empty() {
                        return Err("需要 pid 或 name".to_string());
                    }
                    winmem::list_processes()
                        .into_iter()
                        .find(|p| p.name.to_ascii_lowercase().contains(&wanted))
                        .map(|p| (p.pid, p.name))
                }
            };
            let Some((pid, label)) = resolved else {
                return Err("没有找到这个进程".to_string());
            };
            let handle = winmem::open_process(pid, &label).map_err(|e| {
                // The same diagnosis the picker shows, so a failure reported over
                // MCP is actionable without re-explaining the error code.
                match crate::diag::last() {
                    Some(d) => format!("{e}\n下一步：{}", d.action),
                    None => e,
                }
            })?;
            crate::diag::clear_last();
            let summary = format!(
                "已连接 {}（PID {}，{}，{}）",
                handle.name,
                handle.pid,
                if handle.is_64bit { "x64" } else { "x86" },
                if handle.writable { "可写" } else { "只读" }
            );
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            guard.handle = Some(handle);
            guard.hits.clear();
            guard.addresses.clear();
            guard.feedback.clear();
            if let Some(old) = guard.dbg.take() {
                old.stop();
            }
            Ok(summary)
        }
        "zero_status" => {
            let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            let attached = guard
                .handle
                .as_ref()
                .map(|h| format!("{}（PID {}）", h.name, h.pid))
                .unwrap_or_else(|| "未连接".to_string());
            let mut text = format!(
                "进程：{attached}\n候选：{} 个\n已保存地址：{} 个",
                guard.hits.len(),
                guard.addresses.len()
            );
            if !guard.addresses.is_empty() {
                text.push_str("\n\n地址列表：");
                for (desc, address, vtype) in &guard.addresses {
                    text.push_str(&format!("\n  {address:016X}  {}  {}", vtype.short(), desc));
                }
            }
            Ok(text)
        }
        "zero_scan" => {
            let handle = engine_handle(engine)?;
            let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let vtype = vtype_of(args.get("vtype").and_then(|v| v.as_str()).unwrap_or("i32"));
            let mode = scan_type_of(args.get("mode").and_then(|v| v.as_str()).unwrap_or("exact"));
            let regions = handle.regions(false);
            if regions.is_empty() {
                return Err("目标进程没有可读内存区".to_string());
            }
            let cfg = ScanConfig {
                vtype,
                scan_type: mode,
                value1: value.clone(),
                value2: String::new(),
                fast: false,
                max_results: 400_000,
            };
            let job = scan::start_first_scan(handle.clone(), regions, cfg)?;
            let done = job
                .rx
                .recv_timeout(Duration::from_secs(120))
                .map_err(|_| "扫描超时".to_string())?;
            if let Some(error) = done.error {
                return Err(error);
            }
            let count = done.hits.len();
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            guard.hits = done.hits;
            guard.last_kind = format!("{} {}", mode.label(), value);
            guard.feedback.clear();
            let hits = guard.hits.clone();
            drop(guard);
            Ok(format!(
                "扫描完成：{count} 个候选（{} {}）。\n\n{}",
                mode.label(),
                value,
                summarize_candidates(&handle, &hits)
            ))
        }
        "zero_next_scan" => {
            let handle = engine_handle(engine)?;
            let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mode = scan_type_of(args.get("mode").and_then(|v| v.as_str()).unwrap_or("exact"));
            let previous = {
                let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                guard.hits.clone()
            };
            if previous.is_empty() {
                return Err("还没有首次扫描结果，先调用 zero_scan".to_string());
            }
            let cfg = ScanConfig {
                vtype: VType::Auto,
                scan_type: mode,
                value1: value.clone(),
                value2: String::new(),
                fast: false,
                max_results: 400_000,
            };
            let job = scan::start_next_scan(handle.clone(), previous, cfg)?;
            let done = job
                .rx
                .recv_timeout(Duration::from_secs(120))
                .map_err(|_| "扫描超时".to_string())?;
            if let Some(error) = done.error {
                return Err(error);
            }
            let count = done.hits.len();
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            guard.hits = done.hits;
            let hits = guard.hits.clone();
            drop(guard);
            Ok(format!(
                "再次扫描完成：{count} 个候选。\n\n{}",
                summarize_candidates(&handle, &hits)
            ))
        }
        "zero_candidates" => {
            let handle = engine_handle(engine)?;
            let limit = args
                .get("limit")
                .and_then(|l| l.as_u64())
                .unwrap_or(10)
                .clamp(1, 100) as usize;
            let hits = {
                let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                guard.hits.clone()
            };
            if hits.is_empty() {
                return Err("还没有扫描结果".to_string());
            }
            let regions: Vec<triage::RegionHint> = handle
                .regions(false)
                .into_iter()
                .map(|r| triage::RegionHint {
                    base: r.base,
                    size: r.size,
                    writable: r.writable,
                    executable: r.executable,
                })
                .collect();
            let modules: Vec<(String, u64, u64)> = handle
                .modules()
                .iter()
                .map(|m| (m.name.clone(), m.base, m.size))
                .collect();
            let ranked = triage::rank(&hits, &regions, &modules);
            let mut text = String::new();
            for candidate in ranked.iter().take(limit) {
                text.push_str(&format!(
                    "{:016X}  {}  {} 分\n    理由：{}\n",
                    candidate.hit.address,
                    candidate.hit.value.display(),
                    candidate.score,
                    candidate.headline()
                ));
            }
            Ok(text)
        }
        "zero_read_memory" => {
            let handle = engine_handle(engine)?;
            let address = args
                .get("address")
                .and_then(|a| a.as_str())
                .and_then(parse_hex_address)
                .ok_or_else(|| "地址格式不对".to_string())?;
            let size = args
                .get("size")
                .and_then(|s| s.as_u64())
                .unwrap_or(16)
                .clamp(1, 4096) as usize;
            let mut buffer = vec![0u8; size];
            if !handle.read(address, &mut buffer) {
                return Err(format!("无法读取 {address:016X}"));
            }
            let hex: Vec<String> = buffer.iter().map(|b| format!("{b:02X}")).collect();
            Ok(format!("{address:016X}: {}", hex.join(" ")))
        }
        "zero_write_memory" => {
            let handle = engine_handle(engine)?;
            let address = args
                .get("address")
                .and_then(|a| a.as_str())
                .and_then(parse_hex_address)
                .ok_or_else(|| "地址格式不对".to_string())?;
            let raw = args
                .get("bytes")
                .and_then(|b| b.as_str())
                .ok_or_else(|| "需要 bytes".to_string())?;
            let mut payload = Vec::new();
            for token in raw.split_whitespace() {
                let t = token.trim_start_matches("0x");
                if t.len() != 2 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("「{token}」不是有效字节"));
                }
                payload.push(u8::from_str_radix(t, 16).map_err(|_| "字节解析失败".to_string())?);
            }
            if payload.is_empty() {
                return Err("要写入的字节为空".to_string());
            }
            let patch = inj::apply(&handle, address, &payload, "mcp")?;
            let summary = format!(
                "已写入 {} 字节到 {address:016X}。原字节：{}",
                patch.size(),
                patch.original_hex()
            );
            // Record it before reporting success: a write the caller believes is
            // undoable must actually be undoable.
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            guard.patches.push(patch);
            let total = guard.patches.iter().filter(|p| p.active).count();
            Ok(format!("{summary}（当前 {} 个写入可用 zero_undo 还原）", total))
        }
        "zero_ct_table" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("read");
            // Every entry the table should contain: either the live address list
            // or whatever a previous read parsed.
            let mut table = ctable::CtTable::default();
            {
                let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                if action != "read" {
                    if guard.addresses.is_empty() {
                        return Err("地址列表是空的，先用 zero_add_address 添加".to_string());
                    }
                    for (description, address, vtype) in &guard.addresses {
                        table.entries.push(ctable::CtEntry {
                            description: description.clone(),
                            vtype: *vtype,
                            address: format!("{address:X}"),
                            value: None,
                            active: false,
                        });
                    }
                }
            }
            if action == "build" {
                let xml = ctable::to_xml(&table);
                return Ok(format!(
                    "由地址列表生成 {} 个条目：\n\n{xml}",
                    table.entries.len()
                ));
            }
            let path = args
                .get("path")
                .and_then(|p| p.as_str())
                .ok_or_else(|| "需要 path".to_string())?;
            if action == "write" {
                // Refuse to clobber an existing table unless asked: a .ct can
                // hold work that took a long time to narrow down.
                if std::path::Path::new(path).exists()
                    && !args
                        .get("overwrite")
                        .and_then(|o| o.as_bool())
                        .unwrap_or(false)
                {
                    return Err(format!(
                        "{path} 已存在。要覆盖请传 overwrite: true"
                    ));
                }
                let xml = ctable::to_xml(&table);
                std::fs::write(path, &xml).map_err(|e| format!("写不到 {path}：{e}"))?;
                return Ok(format!(
                    "已写入 {path}：{} 个条目，{} 字节",
                    table.entries.len(),
                    xml.len()
                ));
            }
            let source = std::fs::read_to_string(path)
                .map_err(|e| format!("读不到 {path}：{e}"))?;
            let table = ctable::parse(&source)?;
            if table.is_empty() {
                return Ok(format!("{path} 里没有条目。"));
            }
            let mut text = format!("{path}：{} 个条目\n", table.entries.len());
            for entry in &table.entries {
                text.push_str(&format!(
                    "  [{}] {}  {}  {}\n",
                    if entry.active { "x" } else { " " },
                    entry.description,
                    ctable::vtype_to_ct(entry.vtype),
                    entry.address
                ));
            }
            Ok(text)
        }
        "zero_lua" => {
            let source = args
                .get("source")
                .and_then(|s| s.as_str())
                .ok_or_else(|| "需要 source".to_string())?;
            let handle = engine_handle(engine)?;
            let result = crate::lua::run_script(source, Some(handle))?;
            let patched = result.patches.len();
            if patched > 0 {
                if let Ok(mut guard) = engine.lock() {
                    guard.patches.extend(result.patches);
                }
            }
            let mut text = String::new();
            for line in &result.output {
                text.push_str(line);
                text.push('\n');
            }
            if patched > 0 {
                text.push_str(&format!("（脚本留下 {patched} 个补丁，可用 zero_undo 撤销）\n"));
            }
            if let Some(value) = &result.returned {
                text.push_str(&format!("=> {value}\n"));
            }
            if text.is_empty() {
                text.push_str("脚本执行完毕，没有输出。");
            }
            Ok(text)
        }
        "zero_undo" => {
            let handle = engine_handle(engine)?;
            let which = args.get("address").and_then(|a| a.as_str());
            let target = match which {
                Some(text) => parse_hex_address(text)
                    .ok_or_else(|| "地址格式不对".to_string())?,
                None => 0,
            };
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            let active: Vec<usize> = guard
                .patches
                .iter()
                .enumerate()
                .filter(|(_, p)| p.active)
                .filter(|(_, p)| target == 0 || p.address == target)
                .map(|(index, _)| index)
                .collect();
            if active.is_empty() {
                return Ok(if target == 0 {
                    "没有需要还原的写入。".to_string()
                } else {
                    format!("{target:016X} 没有可还原的写入。")
                });
            }
            let mut restored = Vec::new();
            let mut failed = Vec::new();
            for index in active {
                let patch = &mut guard.patches[index];
                let address = patch.address;
                match inj::revert(&handle, patch) {
                    Ok(()) => restored.push(address),
                    Err(e) => failed.push(format!("{address:016X}: {e}")),
                }
            }
            let mut text = format!("已还原 {} 处写入。", restored.len());
            if !failed.is_empty() {
                text.push_str(&format!("\n失败 {} 处：\n  {}", failed.len(), failed.join("\n  ")));
            }
            Ok(text)
        }
        "zero_add_address" => {
            let address = args
                .get("address")
                .and_then(|a| a.as_str())
                .and_then(parse_hex_address)
                .ok_or_else(|| "地址格式不对".to_string())?;
            let description = args
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("MCP 添加")
                .to_string();
            let vtype = vtype_of(args.get("vtype").and_then(|v| v.as_str()).unwrap_or("i32"));
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            guard.addresses.push((description.clone(), address, vtype));
            Ok(format!("已添加 {address:016X}（{description}）"))
        }
        "zero_aliases" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("list");
            match action {
                "list" => {
                    let list = crate::aliases::list();
                    if list.is_empty() {
                        return Ok("还没有别名。用 action=set 加一个。".to_string());
                    }
                    let mut text = String::new();
                    for alias in list {
                        text.push_str(&format!("{} = {}\n", alias.name, alias.expression));
                    }
                    Ok(text)
                }
                "set" => {
                    let name = args
                        .get("name")
                        .and_then(|n| n.as_str())
                        .ok_or_else(|| "set 需要 name".to_string())?;
                    let expression = args
                        .get("expression")
                        .and_then(|e| e.as_str())
                        .ok_or_else(|| "set 需要 expression".to_string())?;
                    crate::aliases::add(name, expression)?;
                    Ok(format!("别名已记下：{name} = {expression}"))
                }
                "remove" => {
                    let name = args
                        .get("name")
                        .and_then(|n| n.as_str())
                        .ok_or_else(|| "remove 需要 name".to_string())?;
                    if crate::aliases::remove(name) {
                        Ok(format!("已删除别名 {name}"))
                    } else {
                        Err(format!("没有别名 {name}"))
                    }
                }
                other => Err(format!("未知 action：{other}（list / set / remove）")),
            }
        }
        "zero_symbols" => {
            let handle = engine_handle(engine)?;
            let modules = handle.modules();
            // Reverse lookup first: one address, one answer.
            if let Some(text) = args.get("address").and_then(|a| a.as_str()) {
                let address = crate::addr::resolve(Some(&handle), &modules, text)
                    .ok_or_else(|| format!("无法解析地址「{text}」"))?;
                let mut best: Option<String> = None;
                for module in &modules {
                    if !module.contains(address) {
                        continue;
                    }
                    if let Ok(table) = crate::syms::parse(&handle, module) {
                        if let Some(name) = table.nearest(address) {
                            best = Some(name);
                        }
                    }
                }
                return Ok(match best {
                    Some(name) => format!("{address:016X} 最近导出：{name}"),
                    None => format!("{address:016X} 没有落在任何导出上（或模块没有导出表）。"),
                });
            }
            let limit = args
                .get("limit")
                .and_then(|l| l.as_u64())
                .unwrap_or(40)
                .clamp(1, 200) as usize;
            let wanted = args
                .get("module")
                .and_then(|m| m.as_str())
                .map(|m| m.to_ascii_lowercase());
            let mut text = String::new();
            let mut shown = 0usize;
            for module in &modules {
                if let Some(name) = &wanted {
                    if module.name.to_ascii_lowercase() != *name {
                        continue;
                    }
                }
                let Ok(table) = crate::syms::parse(&handle, module) else {
                    continue;
                };
                if table.exports.is_empty() && table.imports.is_empty() {
                    continue;
                }
                text.push_str(&format!(
                    "{} ({} 导出, {} 导入)\n",
                    table.module,
                    table.exports.len(),
                    table.import_count()
                ));
                for sym in table.exports.iter().take(limit.saturating_sub(shown)) {
                    text.push_str(&format!("  {:016X}  {}\n", sym.address, sym.name));
                    shown += 1;
                }
                if shown >= limit {
                    break;
                }
            }
            if text.is_empty() {
                return Ok("没有找到符号：模块可能没有导出表，或名字没匹配上。".to_string());
            }
            Ok(text)
        }
        "zero_dbg_start" => {
            let handle = engine_handle(engine)?;
            let address = args
                .get("address")
                .and_then(|a| a.as_str())
                .and_then(parse_hex_address)
                .ok_or_else(|| "地址格式不对，需要十六进制".to_string())?;
            let kind = match args.get("kind").and_then(|k| k.as_str()).unwrap_or("write") {
                "readwrite" | "rw" => BpKind::ReadWrite,
                _ => BpKind::Write,
            };
            let pid = handle.pid;
            // 上一个会话先停。
            if let Some(old) = &engine.lock().ok().and_then(|g| g.dbg.clone()) {
                old.stop();
            }
            let session = DebugSession::start(pid, &handle)?;
            // 附加是异步的：轮询等它，再落断点。
            for _ in 0..50 {
                if session.status().attached || session.status().error.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            let status = session.status();
            if let Some(err) = &status.error {
                return Err(format!("附加失败：{err}"));
            }
            if !status.attached {
                return Err("附加超时".to_string());
            }
            session.set_breakpoint(Slot(0), address, kind)?;
            let kind_label = match kind {
                BpKind::Write => "写入",
                BpKind::ReadWrite => "读写",
            };
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            guard.dbg = Some(session);
            Ok(format!(
                "已对 {address:016X} 设{kind_label}断点（PID {pid}）。去游戏里改这个值，然后调 zero_dbg_hits。"
            ))
        }
        "zero_dbg_hits" => {
            let session = {
                let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                guard.dbg.clone().ok_or_else(|| "还没有调试会话，先 zero_dbg_start".to_string())?
            };
            let clear = args.get("clear").and_then(|c| c.as_bool()).unwrap_or(false);
            let hits = session.hits();
            if hits.is_empty() {
                return Ok("还没有命中。确认游戏里已经改过那个值。".to_string());
            }
            let mut text = format!("共 {} 次命中：", hits.len());
            for rec in hits.iter().rev().take(10) {
                let written = rec
                    .written
                    .map(|v| format!("{v:#x}"))
                    .unwrap_or_else(|| "?".to_string());
                text.push_str(&format!(
                    "\n  @{:016X}  {}  线程 {}  写后 {}",
                    rec.rip, rec.asm, rec.thread, written
                ));
            }
            if hits.len() > 10 {
                text.push_str(&format!("\n  （只显示最近 10 条，共 {} 条）", hits.len()));
            }
            if clear {
                session.stop();
                let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                guard.dbg = None;
                text.push_str("\n调试已停止，断点已清除。");
            }
            Ok(text)
        }
        "zero_pointer_scan" => {
            let handle = engine_handle(engine)?;
            let address = args
                .get("address")
                .and_then(|a| a.as_str())
                .and_then(parse_hex_address)
                .ok_or_else(|| "地址格式不对".to_string())?;
            let depth = args
                .get("depth")
                .and_then(|d| d.as_u64())
                .unwrap_or(3)
                .clamp(1, 6) as usize;
            let max_offset = args
                .get("max_offset")
                .and_then(|m| m.as_u64())
                .unwrap_or(0x1000)
                .clamp(0x10, 0x10000);
            let limit = args
                .get("limit")
                .and_then(|l| l.as_u64())
                .unwrap_or(10)
                .clamp(1, 50) as usize;
            // Pointer scanning over every readable region is expensive; restrict
            // it to writable memory, which is where pointer fields actually live.
            let regions: Vec<_> = handle
                .regions(false)
                .into_iter()
                .filter(|r| r.writable)
                .collect();
            if regions.is_empty() {
                return Err("目标进程没有可写内存区".to_string());
            }
            let modules = handle.modules();
            let job = crate::pointerscan::start_scan(
                handle.clone(),
                regions,
                modules,
                address,
                depth,
                max_offset,
                2000,
            )?;
            let done = job
                .rx
                .recv_timeout(Duration::from_secs(180))
                .map_err(|_| "指针扫描超时".to_string())?;
            if done.paths.is_empty() {
                return Ok(format!(
                    "没有找到通向 {address:016X} 的指针链。可以把 depth 调大，或确认这个地址确实由指针引用。"
                ));
            }
            let mut text = format!(
                "找到 {} 条指针链（只显示前 {limit} 条）：\n",
                done.paths.len()
            );
            for path in done.paths.iter().take(limit) {
                text.push_str(&format!("  {}\n", path.display()));
            }
            Ok(text)
        }
        "zero_disassemble" => {
            let handle = engine_handle(engine)?;
            let address = args
                .get("address")
                .and_then(|a| a.as_str())
                .and_then(parse_hex_address)
                .ok_or_else(|| "地址格式不对".to_string())?;
            let count = args
                .get("count")
                .and_then(|c| c.as_u64())
                .unwrap_or(20)
                .clamp(1, 200) as usize;
            // Read more than count*15 bytes so long instructions still fit.
            let window = (count * 15).clamp(64, 4096);
            let mut buffer = vec![0u8; window];
            if !handle.read(address, &mut buffer) {
                return Err(format!("无法读取 {address:016X}"));
            }
            let insns = crate::disasm::disassemble(&buffer, address, handle.is_64bit, count);
            if insns.is_empty() {
                return Err("这段内存无法解码成指令".to_string());
            }
            let mut text = String::new();
            for insn in &insns {
                text.push_str(&format!(
                    "{:016X}  {:<24}  {}\n",
                    insn.address,
                    insn.hex(),
                    insn.text
                ));
            }
            Ok(text)
        }
        "zero_watch" => {
            // action=start begins a background session and returns immediately;
            // action=status reads it; action=stop ends it. Blocking the call for
            // the whole duration made the tool useless: the agent could not
            // change the value in the game while it was sampling, so every
            // address looked static.
            let action = args
                .get("action")
                .and_then(|a| a.as_str())
                .unwrap_or("start");
            if action == "stop" {
                let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                if let Some(session) = guard.watch.take() {
                    session.stop.store(true, Ordering::Relaxed);
                    return Ok("已停止观察。".to_string());
                }
                return Ok("当前没有在观察。".to_string());
            }
            if action == "status" {
                let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                let Some(session) = &guard.watch else {
                    return Ok("当前没有在观察。用 action=start 开始。".to_string());
                };
                let watches = session.watches.lock().map_err(|_| "观察状态不可用".to_string())?;
                let elapsed = session.started.elapsed().as_secs_f32();
                let mut text = format!("已观察 {elapsed:.1} 秒：\n");
                for watch in watches.iter() {
                    text.push_str(&format!(
                        "  {:016X}  当前 {}  变化 {} 次  {}\n",
                        watch.address,
                        watch
                            .current
                            .as_ref()
                            .map(|v| v.display())
                            .unwrap_or_else(|| "?".to_string()),
                        watch.changes,
                        watch.behaviour()
                    ));
                }
                match crate::focus::best(&watches) {
                    Some(best) => text.push_str(&format!(
                        "\n最像是它：{:016X}（{} 次变化，{}）",
                        best.address,
                        best.changes,
                        best.behaviour()
                    )),
                    None => text.push_str(
                        "\n还没有任何候选发生变化。去游戏里改一下数值，再查一次。",
                    ),
                }
                return Ok(text);
            }

            let handle = engine_handle(engine)?;
            let limit = args
                .get("limit")
                .and_then(|l| l.as_u64())
                .unwrap_or(5)
                .clamp(1, 16) as usize;
            let hits = {
                let guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
                guard.hits.clone()
            };
            if hits.is_empty() {
                return Err("还没有扫描结果，先调用 zero_scan".to_string());
            }
            let regions: Vec<triage::RegionHint> = handle
                .regions(false)
                .into_iter()
                .map(|r| triage::RegionHint {
                    base: r.base,
                    size: r.size,
                    writable: r.writable,
                    executable: r.executable,
                })
                .collect();
            let modules: Vec<(String, u64, u64)> = handle
                .modules()
                .iter()
                .map(|m| (m.name.clone(), m.base, m.size))
                .collect();
            let ranked = triage::rank(&hits, &regions, &modules);
            let mut watches = crate::focus::from_candidates(&ranked, limit);
            if watches.is_empty() {
                return Err("没有可观察的候选".to_string());
            }
            // Take one baseline sample before handing the list to the thread.
            crate::focus::sample(&handle, &mut watches);
            let shared = Arc::new(Mutex::new(watches));
            let stop_flag = Arc::new(AtomicBool::new(false));
            let count = {
                let guard = shared.lock().map_err(|_| "观察状态不可用".to_string())?;
                guard.len()
            };
            // The sampling thread owns the list and never touches the engine
            // mutex, so a long watch cannot block other MCP calls.
            {
                let shared_thread = shared.clone();
                let stop_thread = stop_flag.clone();
                let handle_thread = handle.clone();
                std::thread::Builder::new()
                    .name("zero-mcp-watch".to_string())
                    .spawn(move || {
                        while !stop_thread.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(200));
                            if let Ok(mut list) = shared_thread.lock() {
                                crate::focus::sample(&handle_thread, &mut list);
                            }
                        }
                    })
                    .map_err(|e| format!("无法启动观察线程：{e}"))?;
            }
            let mut guard = engine.lock().map_err(|_| "引擎状态不可用".to_string())?;
            if let Some(previous) = guard.watch.take() {
                previous.stop.store(true, Ordering::Relaxed);
            }
            guard.watch = Some(WatchSession {
                watches: shared,
                stop: stop_flag,
                started: std::time::Instant::now(),
            });
            Ok(format!(
                "开始观察 {count} 个最可能的地址（{} 分档）。现在去游戏里改数值，然后用 action=status 回来查看谁跟着动了。",
                ranked.first().map(|c| c.score).unwrap_or(0)
            ))
        }
        other => Err(format!("未知工具：{other}")),
    }
}

fn summarize_candidates(handle: &ProcessHandle, hits: &[ScanHit]) -> String {
    if hits.is_empty() {
        return "没有匹配。可以让数值变化后再次扫描，或换一种数值类型。".to_string();
    }
    let regions: Vec<triage::RegionHint> = handle
        .regions(false)
        .into_iter()
        .map(|r| triage::RegionHint {
            base: r.base,
            size: r.size,
            writable: r.writable,
            executable: r.executable,
        })
        .collect();
    let modules: Vec<(String, u64, u64)> = handle
        .modules()
        .iter()
        .map(|m| (m.name.clone(), m.base, m.size))
        .collect();
    let ranked = triage::rank(hits, &regions, &modules);
    let strong = triage::strong_count(&ranked);
    let mut text = format!("其中 {strong} 个很可能。最靠前的几个：\n");
    for candidate in ranked.iter().take(5) {
        text.push_str(&format!(
            "  {:016X}  {}  {} 分  {}\n",
            candidate.hit.address,
            candidate.hit.value.display(),
            candidate.score,
            candidate.headline()
        ));
    }
    text
}
