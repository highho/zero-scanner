//! 调试器：找出「是谁在改这个地址」。
//!
//! 现代作弊器查看内存的第一步是找值，第二步是找基址，第三步是看代码里哪条
//! 指令在改它。这一步 Zero 此前完全没有：1.3 用 Windows 调试接口补上。
//!
//! 实现方式：DebugActiveProcess 附加目标，把目标地址写进各线程的硬件调试
//! 寄存器（DR0-DR3），命中时 CPU 触发单步异常，事件循环里记下指令地址、
//! 反汇编文本、寄存器快照与写入后的值。
//!
//! 安全边界（硬约束）：
//! - 只对自己 attach 的进程设断点；
//! - 每个线程最多 4 个断点（DR0-DR3 槽位）；设断点前先读取并保存该线程的
//!   完整上下文，detach 时按原值写回，绝不硬编码猜值；
//! - 断点在会话结束 / 进程退出时全部清除，目标线程不留残留；
//! - 附加失败（已被调试 / 权限不足）明确报出系统错误码，不静默。

use crate::winmem::ProcessHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, NTSTATUS,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    ContinueDebugEvent, DebugActiveProcess, DebugActiveProcessStop, DebugSetProcessKillOnExit,
    GetThreadContext, SetThreadContext, WaitForDebugEvent, Wow64GetThreadContext, Wow64SetThreadContext,
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, DEBUG_EVENT, EXCEPTION_DEBUG_EVENT,
    EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, RIP_EVENT,
    UNLOAD_DLL_DEBUG_EVENT, WOW64_CONTEXT, CONTEXT_ALL_AMD64, WOW64_CONTEXT_ALL, CONTEXT,
    CONTEXT_DEBUG_REGISTERS_AMD64, WOW64_CONTEXT_DEBUG_REGISTERS,
};
use windows_sys::Win32::System::Threading::{OpenThread, THREAD_GET_CONTEXT, THREAD_SET_CONTEXT};

const EXCEPTION_SINGLE_STEP: i32 = -2147483644;
const ERROR_SEM_TIMEOUT: u32 = 121;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BpKind {
    Write,
    ReadWrite,
}

impl BpKind {
    fn rw_bits(self) -> u64 {
        match self {
            BpKind::Write => 0b01,
            BpKind::ReadWrite => 0b11,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            BpKind::Write => "写入",
            BpKind::ReadWrite => "读写",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot(pub i32);

impl Slot {
    pub fn valid(self) -> bool {
        (0..4).contains(&self.0)
    }
    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug)]
pub struct SlotInfo {
    pub address: u64,
    pub kind: BpKind,
    pub hits: u64,
}

#[derive(Clone, Debug)]
pub struct RegSnapshot {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl RegSnapshot {
    pub fn rows(&self) -> Vec<(String, String)> {
        [
            ("RAX", self.rax), ("RBX", self.rbx), ("RCX", self.rcx), ("RDX", self.rdx),
            ("RSI", self.rsi), ("RDI", self.rdi), ("RBP", self.rbp), ("RSP", self.rsp),
            ("R8", self.r8), ("R9", self.r9), ("R10", self.r10), ("R11", self.r11),
            ("R12", self.r12), ("R13", self.r13), ("R14", self.r14), ("R15", self.r15),
            ("RIP", self.rip), ("RFLAGS", self.rflags),
        ]
        .iter()
        .map(|(name, value)| (name.to_string(), format!("{value:016X}")))
        .collect()
    }
}

#[derive(Clone, Debug)]
pub struct HitRecord {
    pub slot: Slot,
    pub address: u64,
    pub thread: u32,
    pub rip: u64,
    pub asm: String,
    pub bytes: Vec<u8>,
    pub written: Option<u64>,
    pub regs: RegSnapshot,
    pub at: Instant,
}

#[derive(Clone, Debug)]
pub struct DebugStatus {
    pub attached: bool,
    pub pid: u32,
    pub is_64bit: bool,
    pub slots: [Option<SlotInfo>; 4],
    pub hits: usize,
    pub error: Option<String>,
}

enum Request {
    Set { slot: Slot, address: u64, kind: BpKind },
    Clear { slot: Slot },
    ClearAll,
}

struct Shared {
    slots: [Option<SlotInfo>; 4],
    hits: Vec<HitRecord>,
    requests: Vec<Request>,
    error: Option<String>,
    attached: bool,
    pid: u32,
    is_64bit: bool,
}

#[derive(Clone)]
enum SavedCtx {
    X64(CONTEXT),
    X86(WOW64_CONTEXT),
}

struct ThreadState {
    id: u32,
    original: Option<SavedCtx>,
}

pub struct DebugSession {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    pub done: Option<mpsc::Receiver<()>>,
    _handle: Option<std::thread::JoinHandle<()>>,
}

impl DebugSession {
    pub fn start(pid: u32, target: &Arc<ProcessHandle>) -> Result<Arc<Self>, String> {
        if pid == std::process::id() {
            return Err("不能调试 Zero 自己：断点会用单步异常钉住自身线程。".to_string());
        }
        if unsafe { DebugActiveProcess(pid) } == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!(
                "附加调试失败（系统错误 {code}：{}）",
                describe_error(code)
            ));
        }
        unsafe { DebugSetProcessKillOnExit(0) };

        let shared = Arc::new(Mutex::new(Shared {
            slots: [None, None, None, None],
            hits: Vec::new(),
            requests: Vec::new(),
            error: None,
            attached: true,
            pid,
            is_64bit: target.is_64bit,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let shared_2 = shared.clone();
        let stop_2 = stop.clone();
        let target_2 = target.clone();
        let handle = std::thread::Builder::new()
            .name("zero-debugger".to_string())
            .spawn(move || {
                let code = debug_loop(&shared_2, &stop_2, &target_2);
                if let Err(e) = code {
                    if let Ok(mut s) = shared_2.lock() {
                        s.error = Some(e);
                        s.attached = false;
                    }
                }
                let _ = done_tx.send(());
            })
            .map_err(|e| format!("无法启动调试线程：{e}"))?;

        Ok(Arc::new(DebugSession {
            shared,
            stop,
            done: Some(done_rx),
            _handle: Some(handle),
        }))
    }

    pub fn set_breakpoint(&self, slot: Slot, address: u64, kind: BpKind) -> Result<(), String> {
        if !slot.valid() {
            return Err("断点槽位必须在 0-3 之间".to_string());
        }
        if let Ok(mut s) = self.shared.lock() {
            if s.slots[slot.index()].is_some() {
                return Err(format!("槽位 {} 已被占用", slot.0));
            }
            s.requests.push(Request::Set { slot, address, kind });
            Ok(())
        } else {
            Err("调试器状态锁被占用".to_string())
        }
    }

    pub fn clear_breakpoint(&self, slot: Slot) -> Result<(), String> {
        if !slot.valid() {
            return Err("断点槽位必须在 0-3 之间".to_string());
        }
        if let Ok(mut s) = self.shared.lock() {
            s.requests.push(Request::Clear { slot });
            Ok(())
        } else {
            Err("调试器状态锁被占用".to_string())
        }
    }

    pub fn clear_all(&self) {
        if let Ok(mut s) = self.shared.lock() {
            s.requests.push(Request::ClearAll);
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.clear_all();
    }

    pub fn status(&self) -> DebugStatus {
        self.shared.lock().map(|s| s.clone_status()).unwrap_or_else(|_| DebugStatus {
            attached: false,
            pid: 0,
            is_64bit: true,
            slots: [None, None, None, None],
            hits: 0,
            error: Some("调试器状态锁被占用".to_string()),
        })
    }

    pub fn hits(&self) -> Vec<HitRecord> {
        self.shared.lock().map(|s| s.hits.clone()).unwrap_or_default()
    }
}

impl Shared {
    fn clone_status(&self) -> DebugStatus {
        DebugStatus {
            attached: self.attached,
            pid: self.pid,
            is_64bit: self.is_64bit,
            slots: self.slots.clone(),
            hits: self.hits.len(),
            error: self.error.clone(),
        }
    }
}

fn debug_loop(
    shared: &Arc<Mutex<Shared>>,
    stop: &Arc<AtomicBool>,
    target: &Arc<ProcessHandle>,
) -> Result<(), String> {
    let mut threads: Vec<ThreadState> = Vec::new();
    let mut exit_requested = false;
    let mut event: DEBUG_EVENT = unsafe { std::mem::zeroed() };

    while !exit_requested && !stop.load(Ordering::SeqCst) {
        drain_requests(shared, target, &mut threads)?;
        let got = unsafe { WaitForDebugEvent(&mut event, 50) };
        if got == 0 {
            if unsafe { GetLastError() } == ERROR_SEM_TIMEOUT {
                continue;
            }
            return Err(format!(
                "WaitForDebugEvent 失败（系统错误 {}）",
                unsafe { GetLastError() }
            ));
        }

        let proc = event.dwProcessId;
        let thr = event.dwThreadId;
        match event.dwDebugEventCode as u32 {
            CREATE_PROCESS_DEBUG_EVENT => {
                thread_enter(&mut threads, thr);
                drain_requests(shared, target, &mut threads)?;
                close_debug_handles(&event);
                continue_debug(proc, thr, DBG_CONTINUE);
            }
            CREATE_THREAD_DEBUG_EVENT => {
                thread_enter(&mut threads, thr);
                apply_slots_thread(shared, target, &mut threads, thr)?;
                close_debug_handles(&event);
                continue_debug(proc, thr, DBG_CONTINUE);
            }
            EXIT_THREAD_DEBUG_EVENT => {
                threads.retain(|t| t.id != thr);
                continue_debug(proc, thr, DBG_CONTINUE);
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                exit_requested = true;
                threads.clear();
                continue_debug(proc, thr, DBG_CONTINUE);
            }
            LOAD_DLL_DEBUG_EVENT => {
                close_debug_handles(&event);
                continue_debug(proc, thr, DBG_CONTINUE);
            }
            UNLOAD_DLL_DEBUG_EVENT | RIP_EVENT => {
                continue_debug(proc, thr, DBG_CONTINUE);
            }
            EXCEPTION_DEBUG_EVENT => {
                let record = unsafe { event.u.Exception.ExceptionRecord };
                let code = record.ExceptionCode;
                if code == EXCEPTION_SINGLE_STEP {
                    handle_hit(shared, target, thr, &mut threads);
                    continue_debug(proc, thr, DBG_CONTINUE);
                } else {
                    continue_debug(proc, thr, DBG_EXCEPTION_NOT_HANDLED);
                }
            }
            _ => {
                continue_debug(proc, thr, DBG_CONTINUE);
            }
        }
    }

    restore_all(target, &mut threads);
    let pid = shared.lock().map(|s| s.pid).unwrap_or(0);
    if pid != 0 {
        unsafe { DebugActiveProcessStop(pid) };
    }
    if let Ok(mut s) = shared.lock() {
        s.attached = false;
    }
    Ok(())
}

fn thread_enter(threads: &mut Vec<ThreadState>, id: u32) {
    if !threads.iter().any(|t| t.id == id) {
        threads.push(ThreadState { id, original: None });
    }
}

fn close_debug_handles(event: &DEBUG_EVENT) {
    unsafe {
        match event.dwDebugEventCode as u32 {
            CREATE_PROCESS_DEBUG_EVENT => {
                if event.u.CreateProcessInfo.hProcess != 0 {
                    CloseHandle(event.u.CreateProcessInfo.hProcess);
                }
                if event.u.CreateProcessInfo.hThread != 0 {
                    CloseHandle(event.u.CreateProcessInfo.hThread);
                }
            }
            CREATE_THREAD_DEBUG_EVENT => {
                if event.u.CreateThread.hThread != 0 {
                    CloseHandle(event.u.CreateThread.hThread);
                }
            }
            LOAD_DLL_DEBUG_EVENT => {
                if event.u.LoadDll.hFile != 0 {
                    CloseHandle(event.u.LoadDll.hFile);
                }
            }
            _ => {}
        }
    }
}

fn drain_requests(
    shared: &Arc<Mutex<Shared>>,
    target: &Arc<ProcessHandle>,
    threads: &mut Vec<ThreadState>,
) -> Result<(), String> {
    let requests: Vec<Request> = {
        let mut s = shared.lock().map_err(|_| "调试器状态锁被占用".to_string())?;
        s.requests.drain(..).collect()
    };
    for request in requests {
        match request {
            Request::Set { slot, address, kind } => {
                if let Ok(mut s) = shared.lock() {
                    if s.slots[slot.index()].is_some() {
                        continue;
                    }
                    s.slots[slot.index()] = Some(SlotInfo { address, kind, hits: 0 });
                }
                apply_slot_to_all(target, threads, slot.index(), address, kind)?;
            }
            Request::Clear { slot } => {
                clear_slot_from_all(target, threads, slot.index())?;
                if let Ok(mut s) = shared.lock() {
                    s.slots[slot.index()] = None;
                }
            }
            Request::ClearAll => {
                for index in 0..4 {
                    clear_slot_from_all(target, threads, index)?;
                }
                if let Ok(mut s) = shared.lock() {
                    s.slots = [None, None, None, None];
                }
            }
        }
    }
    Ok(())
}

fn apply_slots_thread(
    shared: &Arc<Mutex<Shared>>,
    target: &Arc<ProcessHandle>,
    _threads: &mut Vec<ThreadState>,
    thread_id: u32,
) -> Result<(), String> {
    let slots: [Option<(u64, BpKind)>; 4] = {
        let s = shared.lock().map_err(|_| "调试器状态锁被占用".to_string())?;
        std::array::from_fn(|i| s.slots[i].as_ref().map(|info| (info.address, info.kind)))
    };
    if slots.iter().all(|s| s.is_none()) {
        return Ok(());
    }
    let mut ctx = read_thread_context(target, thread_id).ok_or_else(|| {
        format!("无法读取线程 {thread_id} 上下文（系统错误 {}）", unsafe { GetLastError() })
    })?;
    ctx.set_dr7(0);
    for (index, slot) in slots.iter().enumerate().take(4) {
        if let Some((address, kind)) = slot {
            set_slot_dr(&mut ctx, index, *address, *kind);
        }
    }
    if !write_thread_context(target, thread_id, &ctx) {
        return Err(format!("无法写入线程 {thread_id} 上下文（系统错误 {}）", unsafe { GetLastError() }));
    }
    Ok(())
}

fn apply_slot_to_all(
    target: &Arc<ProcessHandle>,
    threads: &mut Vec<ThreadState>,
    index: usize,
    address: u64,
    kind: BpKind,
) -> Result<(), String> {
    for thread in threads.iter_mut() {
        let mut ctx = read_thread_context(target, thread.id).ok_or_else(|| {
            format!("无法读取线程 {} 上下文（系统错误 {}）", thread.id, unsafe { GetLastError() })
        })?;
        if thread.original.is_none() {
            thread.original = Some(ctx.clone());
        }
        set_slot_dr(&mut ctx, index, address, kind);
        if !write_thread_context(target, thread.id, &ctx) {
            return Err(format!("无法写入线程 {} 上下文（系统错误 {}）", thread.id, unsafe { GetLastError() }));
        }
    }
    Ok(())
}

fn clear_slot_from_all(
    target: &Arc<ProcessHandle>,
    threads: &mut Vec<ThreadState>,
    index: usize,
) -> Result<(), String> {
    for thread in threads.iter_mut() {
        if let Some(mut ctx) = read_thread_context(target, thread.id) {
            ctx.set_dr_slot(index, 0);
            let dr7 = ctx.dr7() & !slot_dr7_mask(index);
            ctx.set_dr7(dr7);
            if !write_thread_context(target, thread.id, &ctx) {
                return Err(format!("无法清除线程 {} 断点（系统错误 {}）", thread.id, unsafe { GetLastError() }));
            }
        }
    }
    Ok(())
}

fn restore_all(target: &Arc<ProcessHandle>, threads: &mut [ThreadState]) {
    for thread in threads.iter_mut() {
        if let Some(original) = &thread.original {
            let _ = write_thread_context_saved(thread.id, original);
        } else if let Some(mut ctx) = read_thread_context(target, thread.id) {
            ctx.set_dr7(0);
            ctx.set_dr_slot(0, 0);
            ctx.set_dr_slot(1, 0);
            ctx.set_dr_slot(2, 0);
            ctx.set_dr_slot(3, 0);
            let _ = write_thread_context(target, thread.id, &ctx);
        }
    }
}

fn continue_debug(proc: u32, thread: u32, status: NTSTATUS) {
    unsafe {
        ContinueDebugEvent(proc, thread, status);
    }
}

fn read_thread_context(target: &Arc<ProcessHandle>, thread_id: u32) -> Option<SavedCtx> {
    let handle = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, thread_id) };
    if handle == 0 {
        return None;
    }
    let ctx = if target.is_64bit {
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.ContextFlags = CONTEXT_ALL_AMD64;
        let ok = unsafe { GetThreadContext(handle, &mut context) };
        unsafe { CloseHandle(handle) };
        if ok == 0 { None } else { Some(SavedCtx::X64(context)) }
    } else {
        let mut context: WOW64_CONTEXT = unsafe { std::mem::zeroed() };
        context.ContextFlags = WOW64_CONTEXT_ALL;
        let ok = unsafe { Wow64GetThreadContext(handle, &mut context) };
        unsafe { CloseHandle(handle) };
        if ok == 0 { None } else { Some(SavedCtx::X86(context)) }
    };
    ctx
}

/// 写入线程上下文，只提交调试寄存器子集（Dr0-DR3/Dr6/Dr7）。
///
/// 绝不整块写回 CONTEXT_ALL：那样会把运行中线程的 RIP/RSP/RFLAGS 改成保存时的
/// 旧值，等于把目标线程拽回过去。这里把 ContextFlags 限定为调试寄存器，Windows
/// 只应用 Dr 相关字段，目标的位置与栈不受影响。
fn write_thread_context(_target: &Arc<ProcessHandle>, thread_id: u32, ctx: &SavedCtx) -> bool {
    let handle = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, thread_id) };
    if handle == 0 {
        return false;
    }
    let ok = match ctx {
        SavedCtx::X64(context) => {
            let mut copy = *context;
            copy.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
            unsafe { SetThreadContext(handle, &copy) }
        }
        SavedCtx::X86(context) => {
            let mut copy = *context;
            copy.ContextFlags = WOW64_CONTEXT_DEBUG_REGISTERS;
            unsafe { Wow64SetThreadContext(handle, &copy) }
        }
    };
    unsafe { CloseHandle(handle) };
    ok != 0
}

fn write_thread_context_saved(thread_id: u32, original: &SavedCtx) -> bool {
    let handle = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, thread_id) };
    if handle == 0 {
        return false;
    }
    let ok = match original {
        SavedCtx::X64(context) => {
            let mut copy = *context;
            copy.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
            unsafe { SetThreadContext(handle, &copy) }
        }
        SavedCtx::X86(context) => {
            let mut copy = *context;
            copy.ContextFlags = WOW64_CONTEXT_DEBUG_REGISTERS;
            unsafe { Wow64SetThreadContext(handle, &copy) }
        }
    };
    unsafe { CloseHandle(handle) };
    ok != 0
}

fn handle_hit(
    shared: &Arc<Mutex<Shared>>,
    target: &Arc<ProcessHandle>,
    thread: u32,
    threads: &mut Vec<ThreadState>,
) {
    let ctx = match read_thread_context(target, thread) {
        Some(ctx) => ctx,
        None => return,
    };
    let dr6 = ctx.dr6();
    let slot_index = match dr6 & 0xF {
        0x1 => 0usize,
        0x2 => 1usize,
        0x4 => 2usize,
        0x8 => 3usize,
        _ => return,
    };

    let (address, _kind) = match shared.lock().map(|s| {
        s.slots[slot_index].as_ref().map(|info| (info.address, info.kind))
    }) {
        Ok(Some(slot)) => slot,
        _ => return,
    };

    let rip = ctx.rip();
    let mut insn_bytes = vec![0u8; 15];
    let read_len = if target.read(rip, &mut insn_bytes) { 15 } else { 0 };
    insn_bytes.truncate(read_len);
    let listing = crate::disasm::disassemble(&insn_bytes, rip, true, 1);
    let (asm, bytes) = if let Some(insn) = listing.first() {
        (insn.text.clone(), insn.bytes.clone())
    } else {
        (format!("无法反汇编 @ {rip:X}"), Vec::new())
    };

    let written = if target.is_64bit {
        target.read_pointer(address)
    } else {
        let mut buf = [0u8; 4];
        if target.read(address, &mut buf) {
            Some(u32::from_le_bytes(buf) as u64)
        } else {
            None
        }
    };

    let regs = ctx.snapshot();

    if let Ok(mut s) = shared.lock() {
        if let Some(info) = &mut s.slots[slot_index] {
            info.hits += 1;
        }
        s.hits.push(HitRecord {
            slot: Slot(slot_index as i32),
            address,
            thread,
            rip,
            asm,
            bytes,
            written,
            regs,
            at: Instant::now(),
        });
        if s.hits.len() > 500 {
            let excess = s.hits.len() - 500;
            s.hits.drain(..excess);
        }
    }

    if let Some(state) = threads.iter().find(|t| t.id == thread) {
        clear_dr6(target, thread, state);
    }
}

fn clear_dr6(target: &Arc<ProcessHandle>, thread_id: u32, _state: &ThreadState) -> bool {
    if let Some(mut ctx) = read_thread_context(target, thread_id) {
        let dr6 = ctx.dr6() & !0xF;
        ctx.set_dr6(dr6);
        write_thread_context(target, thread_id, &ctx)
    } else {
        false
    }
}

impl SavedCtx {
    fn dr6(&self) -> u64 {
        match self {
            SavedCtx::X64(c) => c.Dr6,
            SavedCtx::X86(c) => c.Dr6 as u64,
        }
    }
    fn dr7(&self) -> u64 {
        match self {
            SavedCtx::X64(c) => c.Dr7,
            SavedCtx::X86(c) => c.Dr7 as u64,
        }
    }
    fn set_dr7(&mut self, value: u64) {
        match self {
            SavedCtx::X64(c) => c.Dr7 = value,
            SavedCtx::X86(c) => c.Dr7 = value as u32,
        }
    }
    fn set_dr_slot(&mut self, index: usize, value: u64) {
        match self {
            SavedCtx::X64(c) => match index {
                0 => c.Dr0 = value,
                1 => c.Dr1 = value,
                2 => c.Dr2 = value,
                _ => c.Dr3 = value,
            },
            SavedCtx::X86(c) => match index {
                0 => c.Dr0 = value as u32,
                1 => c.Dr1 = value as u32,
                2 => c.Dr2 = value as u32,
                _ => c.Dr3 = value as u32,
            },
        }
    }
    fn set_dr6(&mut self, value: u64) {
        match self {
            SavedCtx::X64(c) => c.Dr6 = value,
            SavedCtx::X86(c) => c.Dr6 = value as u32,
        }
    }
    fn rip(&self) -> u64 {
        match self {
            SavedCtx::X64(c) => c.Rip,
            SavedCtx::X86(c) => c.Eip as u64,
        }
    }
    fn snapshot(&self) -> RegSnapshot {
        match self {
            SavedCtx::X64(c) => RegSnapshot {
                rax: c.Rax, rbx: c.Rbx, rcx: c.Rcx, rdx: c.Rdx,
                rsi: c.Rsi, rdi: c.Rdi, rbp: c.Rbp, rsp: c.Rsp,
                r8: c.R8, r9: c.R9, r10: c.R10, r11: c.R11,
                r12: c.R12, r13: c.R13, r14: c.R14, r15: c.R15,
                rip: c.Rip, rflags: c.EFlags as u64,
            },
            SavedCtx::X86(c) => RegSnapshot {
                rax: c.Eax as u64, rbx: c.Ebx as u64, rcx: c.Ecx as u64, rdx: c.Edx as u64,
                rsi: c.Esi as u64, rdi: c.Edi as u64, rbp: c.Ebp as u64, rsp: c.Esp as u64,
                r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
                rip: c.Eip as u64, rflags: c.EFlags as u64,
            },
        }
    }
}

fn slot_dr7_mask(index: usize) -> u64 {
    let enable = 0b11u64 << (index * 2);
    let control = 0b1111u64 << (16 + index * 4);
    enable | control
}

fn set_slot_dr(ctx: &mut SavedCtx, index: usize, address: u64, kind: BpKind) {
    ctx.set_dr_slot(index, address);
    let rw = kind.rw_bits() << (16 + index * 4);
    let len = 0u64 << (18 + index * 4);
    let local_enable = 1u64 << (index * 2);
    let mut dr7 = ctx.dr7();
    dr7 &= !slot_dr7_mask(index);
    dr7 |= local_enable | rw | len;
    ctx.set_dr7(dr7);
}

fn describe_error(code: u32) -> &'static str {
    match code {
        5 => "拒绝访问：目标以更高权限运行或受保护，请以管理员身份运行 Zero",
        87 => "进程已退出或参数无效",
        299 => "进程已被其它调试器附加（只允许一个调试器）",
        31 => "目标可能刚崩溃或已退出",
        _ => "未知错误",
    }
}
