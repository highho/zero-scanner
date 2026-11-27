//! Speed control: the classic speed hack.
//!
//! A process cannot be sped up from outside without changing its code, so the
//! trick is to stop all of its threads and let them run again on a schedule.
//! Stopping them for longer than real time makes the game slow down; stopping
//! them for less makes it speed up.
//!
//! This module owns the dangerous part (suspend/resume) and guarantees that
//! every thread it stops is released again: a stop that leaked a suspended
//! thread would leave the target frozen forever.

use std::mem::size_of;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME,
};

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
        // A 40 ms window keeps the stutter under ~25 fps worth of granularity.
        let period = 40u64;
        let factor = self.factor.clamp(0.01, 16.0);
        let run = ((period as f64) * factor / (1.0 + factor)).round() as u64;
        let run = run.clamp(1, period - 1);
        (period, run)
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

/// Suspend every thread of the process. Returns the threads actually stopped,
/// so a later resume only touches those.
///
/// Suspending our own process would stop the very thread doing the suspending:
/// the UI would freeze and nothing could ever resume it. That is a hard error,
/// not a warning, because the process cannot recover from it by itself.
pub fn suspend_all(pid: u32) -> Result<Vec<TargetThread>, String> {
    if pid == std::process::id() {
        return Err("不能挂起 Zero 自己：那样会把执行挂起的线程一起停掉。".to_string());
    }
    let mut stopped = Vec::new();
    for id in thread_ids(pid) {
        unsafe {
            let handle = OpenThread(THREAD_SUSPEND_RESUME, 0, id);
            if handle == 0 {
                continue;
            }
            // A negative return means the call failed.
            if SuspendThread(handle) != u32::MAX {
                stopped.push(TargetThread {
                    id,
                    suspend_count: 1,
                });
            }
            CloseHandle(handle);
        }
    }
    Ok(stopped)
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
        Self {
            stop_ms: period - run,
            run_ms: run,
        }
    }
}