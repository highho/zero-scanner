//! Speed control: the classic speed hack.
//!
//! A process cannot be sped up from outside without changing its code, so the
//! trick is to stop some of its threads and let them run again on a schedule.
//! Stopping them for longer than real time makes the game slow down; stopping
//! them for less makes it speed up.
//!
//! Stopping *every* thread is the blunt version: it freezes the renderer and
//! the audio mixer along with the game logic, so the picture stutters and the
//! sound crackles. 1.5 narrows the target to the threads that actually drive
//! game time (the ones running code inside the game's own modules) and leaves
//! the graphics and audio plumbing running.
//!
//! This module owns the dangerous part (suspend/resume) and guarantees that
//! every thread it stops is released again: a stop that leaked a suspended
//! thread would leave the target frozen forever.

use std::mem::size_of;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows_sys::Win32::System::Threading::{
    GetThreadPriority, OpenThread, ResumeThread, SuspendThread, THREAD_QUERY_INFORMATION,
    THREAD_SUSPEND_RESUME,
};

use crate::winmem::{ModuleInfo, ProcessHandle};
/// One thread of the target, tracked so it can always be resumed.
#[derive(Clone, Debug)]
pub struct TargetThread {
    pub id: u32,
    /// How many times this thread has been suspended by us. Guests that were
    /// already suspended for their own reasons must not be resumed blindly,
    /// so the count is tracked per thread.
    pub suspend_count: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpeedState {
    /// Threads run normally.
    Off,
    /// Stopping them for `period`, letting them run for `run_for`.
    Active,
}

/// Which threads the speed change is allowed to touch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetMode {
    /// Every thread, including the renderer and the audio mixer.
    All,
    /// Only threads executing code inside the game's own modules.
    GameLogic,
}

impl TargetMode {
    pub fn label(self) -> &'static str {
        match self {
            TargetMode::All => "全部线程",
            TargetMode::GameLogic => "只停游戏逻辑",
        }
    }
}

/// Configuration of a speed change.
#[derive(Clone, Copy, Debug)]
pub struct SpeedPlan {
    /// Desired multiplier: 0.5 = half speed, 2.0 = double speed.
    pub factor: f64,
}

impl SpeedPlan {
    /// Total period in milliseconds and the slice the threads are allowed to
    /// run. The ratio run/period equals the speed factor.
    pub fn timing(&self) -> (u64, u64) {
        self.timing_for(40)
    }

    /// Same maths against a period the caller picked. The period is the
    /// granularity of the effect: too short and the sleep calls themselves
    /// eat the budget, too long and the game visibly hitches. When only a few
    /// threads are being juggled the scheduler can hold a shorter window, so
    /// `period_for` scales it with the target count.
    pub fn timing_for(&self, period: u64) -> (u64, u64) {
        let period = period.max(4);
        let factor = self.factor.clamp(0.01, 16.0);
        let run = ((period as f64) * factor / (1.0 + factor)).round() as u64;
        let run = run.clamp(1, period - 1);
        (period, run)
    }
}

/// Pick the cycle length for a given number of target threads.
///
/// Every thread costs two context switches per cycle, so a process with a few
/// hundred threads cannot be flipped at 25 Hz without the flips dominating the
/// wall clock. The window widens as the count grows; the ratio stays the same,
/// so the requested speed is unaffected.
pub fn period_for(threads: usize) -> u64 {
    match threads {
        0..=8 => 20,
        9..=32 => 30,
        33..=96 => 40,
        _ => 60,
    }
}

/// Enumerate the thread ids of a process.
pub fn thread_ids(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
            return out;
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut ok = Thread32First(snapshot, &mut entry);
        while ok != 0 {
            if entry.th32OwnerProcessID == pid {
                out.push(entry.th32ThreadID);
            }
            ok = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    out.sort_unstable();
    out
}

// ---- Where does a thread start? -------------------------------------------
//
// There is no documented Win32 call for "the start address of an existing
// thread", but ntdll has had one forever and every debugger and profiler uses
// it. It is loaded by name instead of linked so a machine without it (or a
// future signature change) degrades to "cannot tell" rather than failing to
// start. NtQueryInformationThread(ThreadQuerySetWin32StartAddress = 9).

type QueryFn = unsafe extern "system" fn(isize, u32, *mut core::ffi::c_void, u32, *mut u32) -> i32;

fn query_fn() -> Option<QueryFn> {
    static CACHED: OnceLock<Option<usize>> = OnceLock::new();
    let raw = *CACHED.get_or_init(|| unsafe {
        let module = LoadLibraryA(c"ntdll.dll".as_ptr() as *const u8);
        if module == 0 {
            return None;
        }
        let proc = GetProcAddress(module, c"NtQueryInformationThread".as_ptr() as *const u8);
        if proc.is_none() {
            return None;
        }
        Some(proc.unwrap() as usize)
    });
    let raw = raw?;
    // SAFETY: the pointer came from GetProcAddress on the documented export
    // and the signature above is the one ntdll has kept since Windows 2000.
    Some(unsafe { std::mem::transmute::<usize, QueryFn>(raw) })
}

/// The address a thread was created at, when the system will tell us.
pub fn thread_start_address(thread_id: u32) -> Option<u64> {
    let query = query_fn()?;
    unsafe {
        let handle = OpenThread(THREAD_QUERY_INFORMATION, 0, thread_id);
        if handle == 0 {
            return None;
        }
        let mut start: usize = 0;
        let mut written: u32 = 0;
        let status = query(
            handle,
            9, // ThreadQuerySetWin32StartAddress
            &mut start as *mut usize as *mut core::ffi::c_void,
            size_of::<usize>() as u32,
            &mut written,
        );
        CloseHandle(handle);
        if status < 0 || start == 0 {
            return None;
        }
        Some(start as u64)
    }
}

/// Is this address inside one of the process's own modules? Those are the
/// threads running game code; system DLLs host the renderer, the audio mixer
/// and the OS plumbing, which must keep running or the picture stutters.
fn inside_game_module(modules: &[ModuleInfo], address: u64) -> bool {
    modules.iter().any(|m| !is_system_module(&m.name) && m.contains(address))
}

/// A DLL that belongs to Windows, a graphics driver, or an audio stack. Kept
/// as a short prefix list: the point is to recognise the handful that own the
/// render and sound threads, not to classify every DLL in the process.
fn is_system_module(name: &str) -> bool {
    const PREFIXES: [&str; 48] = [
        "kernel", "ntdll", "user32", "gdi32", "win32u", "dxgi", "d3d", "opengl",
        "vulkan", "nvoglv", "atio6axx", "igc", "amdxc", "xaudio", "dsound", "winmm",
        "mmdevapi", "audio", "mfplat", "mfcore", "mfreadwrite", "wmcodecdsp", "msacm",
        "combase", "ole32", "rpcrt4", "ucrtbase", "vcruntime", "msvcp", "msvcrt",
        "winhttp", "ws2_", "sechost", "advapi", "bcrypt", "crypt", "shcore",
        "shlwapi", "shell32", "comctl", "imm32", "dwmapi", "uxtheme", "dbghelp",
        "psapi", "iphlpapi", "dnsapi", "wininet",
    ];
    let lower = name.to_ascii_lowercase();
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// One thread we decided to touch, plus why it was picked (for the UI).
#[derive(Clone, Debug)]
pub struct ThreadChoice {
    pub id: u32,
    pub start: Option<u64>,
    pub inside_module: bool,
}

/// Decide which threads a speed change should touch.
///
/// With TargetMode::All every thread is a candidate. With TargetMode::GameLogic
/// only threads whose start address lies inside a non-system module qualify.
/// Threads whose start address cannot be read are left alone in GameLogic mode:
/// guessing wrong here means freezing the renderer, which is exactly what this
/// mode exists to avoid.
pub fn plan_threads(pid: u32, modules: &[ModuleInfo], mode: TargetMode) -> Vec<ThreadChoice> {
    let ids = thread_ids(pid);
    if mode == TargetMode::All {
        return ids
            .into_iter()
            .map(|id| ThreadChoice { id, start: None, inside_module: true })
            .collect();
    }
    ids.into_iter()
        .filter_map(|id| {
            let start = thread_start_address(id);
            let inside = start.map(|a| inside_game_module(modules, a)).unwrap_or(false);
            if inside {
                Some(ThreadChoice { id, start, inside_module: true })
            } else {
                None
            }
        })
        .collect()
}

/// Suspend exactly these threads. Returns the ones actually stopped, so a
/// later resume only touches those.
///
/// Suspending our own process would stop the very thread doing the suspending:
/// the UI would freeze and nothing could ever resume it. That is a hard error,
/// not a warning, because the process cannot recover from it by itself.
pub fn suspend_threads(pid: u32, choices: &[ThreadChoice]) -> Result<Vec<TargetThread>, String> {
    if pid == std::process::id() {
        return Err("不能挂起 Zero 自己：那样会把执行挂起的线程一起停掉。".to_string());
    }
    let mut stopped = Vec::new();
    for choice in choices {
        unsafe {
            let handle = OpenThread(THREAD_SUSPEND_RESUME, 0, choice.id);
            if handle == 0 {
                continue;
            }
            // A negative return means the call failed.
            if SuspendThread(handle) != u32::MAX {
                stopped.push(TargetThread { id: choice.id, suspend_count: 1 });
            }
            CloseHandle(handle);
        }
    }
    Ok(stopped)
}

/// Suspend every thread of the process.
pub fn suspend_all(pid: u32) -> Result<Vec<TargetThread>, String> {
    let ids = thread_ids(pid);
    let choices: Vec<ThreadChoice> = ids
        .into_iter()
        .map(|id| ThreadChoice { id, start: None, inside_module: true })
        .collect();
    suspend_threads(pid, &choices)
}

/// Resume exactly the threads in `threads`, never more.
pub fn resume_all(threads: &[TargetThread]) {
    for thread in threads {
        for _ in 0..thread.suspend_count {
            unsafe {
                let handle = OpenThread(THREAD_SUSPEND_RESUME, 0, thread.id);
                if handle == 0 {
                    continue;
                }
                ResumeThread(handle);
                CloseHandle(handle);
            }
        }
    }
}

/// One step of the speed loop: how long to stop, how long to run.
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    pub stop_ms: u64,
    pub run_ms: u64,
}

impl Tick {
    pub fn from_plan(plan: SpeedPlan) -> Self {
        let (period, run) = plan.timing();
        Self { stop_ms: period - run, run_ms: run }
    }

    /// Same, against a period chosen for the current thread count.
    pub fn from_plan_for(plan: SpeedPlan, threads: usize) -> Self {
        let (period, run) = plan.timing_for(period_for(threads));
        Self { stop_ms: period - run, run_ms: run }
    }
}

/// Read the priority of a thread, for the UI hint only.
pub fn thread_priority(thread_id: u32) -> Option<i32> {
    unsafe {
        let handle = OpenThread(THREAD_QUERY_INFORMATION, 0, thread_id);
        if handle == 0 {
            return None;
        }
        let priority = GetThreadPriority(handle);
        CloseHandle(handle);
        Some(priority)
    }
}

/// Convenience: the modules of a live process, or an empty list.
pub fn modules_of(handle: &ProcessHandle) -> Vec<ModuleInfo> {
    handle.modules()
}
