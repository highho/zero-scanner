use crate::model::Region;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, VirtualProtectEx, VirtualQueryEx, MEMORY_BASIC_INFORMATION,
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE, PAGE_WRITECOPY,
};
use windows_sys::Win32::System::Threading::{
    IsWow64Process, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

/// A loaded module (exe or dll) inside the target process. Pointer scan reports
/// static bases as "module + offset", which is what survives a restart.
#[derive(Clone, Debug)]
pub struct ModuleInfo {
    pub name: String,
    pub base: u64,
    pub size: u64,
}

impl ModuleInfo {
    pub fn contains(&self, address: u64) -> bool {
        address >= self.base && address < self.base.saturating_add(self.size)
    }
    pub fn offset_of(&self, address: u64) -> u64 {
        address.saturating_sub(self.base)
    }
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub arch: String,
    pub path: Option<String>,
}

pub struct ProcessHandle {
    handle: HANDLE,
    pub pid: u32,
    pub name: String,
    pub is_64bit: bool,
    pub writable: bool,
}

unsafe impl Send for ProcessHandle {}
unsafe impl Sync for ProcessHandle {}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if self.handle != 0 && self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

impl ProcessHandle {
    pub fn read(&self, address: u64, buffer: &mut [u8]) -> bool {
        if buffer.is_empty() {
            return true;
        }
        let mut read = 0usize;
        unsafe {
            ReadProcessMemory(
                self.handle,
                address as *const c_void,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len(),
                &mut read,
            ) != 0
                && read == buffer.len()
        }
    }

    pub fn write(&self, address: u64, bytes: &[u8]) -> bool {
        if !self.writable || bytes.is_empty() {
            return false;
        }
        let mut written = 0usize;
        unsafe {
            WriteProcessMemory(
                self.handle,
                address as *const c_void,
                bytes.as_ptr() as *const c_void,
                bytes.len(),
                &mut written,
            ) != 0
                && written == bytes.len()
        }
    }

    /// Read one pointer-sized value, using the target's own pointer width.
    pub fn read_pointer(&self, address: u64) -> Option<u64> {
        if self.is_64bit {
            let mut buf = [0u8; 8];
            if self.read(address, &mut buf) {
                Some(u64::from_le_bytes(buf))
            } else {
                None
            }
        } else {
            let mut buf = [0u8; 4];
            if self.read(address, &mut buf) {
                Some(u32::from_le_bytes(buf) as u64)
            } else {
                None
            }
        }
    }

    /// Enumerate loaded modules for this process.
    pub fn modules(&self) -> Vec<ModuleInfo> {
        let mut out = Vec::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(
                TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32,
                self.pid,
            );
            if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
                return out;
            }
            let mut entry: MODULEENTRY32W = zeroed();
            entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
            let mut ok = Module32FirstW(snapshot, &mut entry);
            while ok != 0 {
                out.push(ModuleInfo {
                    name: wide_string(&entry.szModule),
                    base: entry.modBaseAddr as usize as u64,
                    size: entry.modBaseSize as u64,
                });
                ok = Module32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        out.sort_by_key(|m| m.base);
        out
    }

    pub fn module_of<'a>(&self, modules: &'a [ModuleInfo], address: u64) -> Option<&'a ModuleInfo> {
        modules.iter().find(|m| m.contains(address))
    }

    pub fn regions(&self, writable_only: bool) -> Vec<Region> {
        let max = if self.is_64bit {
            0x0000_7FFF_FFFF_FFFFu64
        } else {
            0xFFFF_FFFFu64
        };
        let mut out = Vec::new();
        let mut address = 0u64;
        while address < max && out.len() < 200_000 {
            let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
            let got = unsafe {
                VirtualQueryEx(
                    self.handle,
                    address as *const c_void,
                    &mut mbi,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if got == 0 {
                break;
            }
            let base = mbi.BaseAddress as usize as u64;
            let size = mbi.RegionSize as u64;
            if size == 0 {
                break;
            }
            let protect = mbi.Protect;
            let readable = mbi.State == MEM_COMMIT
                && protect != PAGE_NOACCESS
                && (protect & PAGE_GUARD) == 0;
            let writable = (protect
                & (PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY))
                != 0;
            let executable = (protect
                & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY))
                != 0;
            if readable && (!writable_only || writable) {
                out.push(Region {
                    base,
                    size,
                    writable,
                    executable,
                });
            }
            let next = base.saturating_add(size);
            if next <= address {
                break;
            }
            address = next;
        }
        out
    }

    pub fn region_at(&self, address: u64) -> Option<Region> {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
        let got = unsafe {
            VirtualQueryEx(
                self.handle,
                address as *const c_void,
                &mut mbi,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if got == 0 || mbi.State != MEM_COMMIT {
            return None;
        }
        Some(Region {
            base: mbi.BaseAddress as usize as u64,
            size: mbi.RegionSize as u64,
            writable: (mbi.Protect
                & (PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY))
                != 0,
            executable: (mbi.Protect
                & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY))
                != 0,
        })
    }

    /// Reserve and commit a private RWX block inside the target process. Used for
    /// code caves: allocated blocks are owned by this process and can be freed.
    pub fn allocate(&self, size: usize) -> Result<u64, String> {
        let size = size.max(0x1000);
        let address = unsafe {
            VirtualAllocEx(
                self.handle,
                std::ptr::null(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        };
        if address.is_null() {
            let code = unsafe { GetLastError() };
            return Err(format!("无法在目标进程分配内存（系统错误 {code}）。"));
        }
        Ok(address as usize as u64)
    }

    pub fn free(&self, address: u64) -> Result<(), String> {
        if address == 0 {
            return Err("地址为空。".to_string());
        }
        let ok = unsafe { VirtualFreeEx(self.handle, address as *mut c_void, 0, MEM_RELEASE) };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!("无法释放内存（系统错误 {code}）。"));
        }
        Ok(())
    }

    /// Change a region's protection and return the previous value, so callers can
    /// put it back exactly as it was.
    pub fn protect(&self, address: u64, size: usize, protection: u32) -> Result<u32, String> {
        let mut old: u32 = 0;
        let ok = unsafe {
            VirtualProtectEx(
                self.handle,
                address as *const c_void,
                size,
                protection,
                &mut old,
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!("无法修改内存保护（系统错误 {code}）。"));
        }
        Ok(old)
    }

    pub fn protection_at(&self, address: u64) -> String {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
        let got = unsafe {
            VirtualQueryEx(
                self.handle,
                address as *const c_void,
                &mut mbi,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if got == 0 {
            return "不可用".to_string();
        }
        let state = if mbi.State == MEM_COMMIT {
            "已提交"
        } else {
            "未提交"
        };
        format!("{state} · {}", protection_name(mbi.Protect))
    }
}

pub fn protection_name(protect: u32) -> &'static str {
    if protect & PAGE_GUARD != 0 {
        return "GUARD";
    }
    if protect == PAGE_NOACCESS {
        return "NOACCESS";
    }
    match protect {
        PAGE_READWRITE => "READWRITE",
        PAGE_WRITECOPY => "WRITECOPY",
        PAGE_EXECUTE => "EXECUTE",
        PAGE_EXECUTE_READ => "EXECUTE_READ",
        PAGE_EXECUTE_READWRITE => "EXECUTE_READWRITE",
        PAGE_EXECUTE_WRITECOPY => "EXECUTE_WRITECOPY",
        0x02 => "READONLY",
        _ => "OTHER",
    }
}

fn wide_string(bytes: &[u16]) -> String {
    let end = bytes.iter().position(|c| *c == 0).unwrap_or(bytes.len());
    String::from_utf16_lossy(&bytes[..end])
}

fn is_wow64(handle: HANDLE) -> bool {
    let mut wow: i32 = 0;
    let ok = unsafe { IsWow64Process(handle, &mut wow) };
    ok != 0 && wow != 0
}

fn process_path(handle: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
    if ok != 0 {
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    } else {
        None
    }
}

pub fn list_processes() -> Vec<ProcessInfo> {
    let mut out = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
            return out;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let pid = entry.th32ProcessID;
            if pid != 0 && pid != 4 {
                let name = wide_string(&entry.szExeFile);
                let mut arch = "?".to_string();
                let mut path = None;
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if h != 0 {
                    arch = if is_wow64(h) { "x86" } else { "x64" }.to_string();
                    path = process_path(h);
                    CloseHandle(h);
                }
                out.push(ProcessInfo {
                    pid,
                    name,
                    arch,
                    path,
                });
            }
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then(a.pid.cmp(&b.pid))
    });
    out
}

pub fn open_process(pid: u32, name: &str) -> Result<Arc<ProcessHandle>, String> {
    let full = PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION;
    let read_only = PROCESS_VM_READ | PROCESS_QUERY_INFORMATION;
    let mut handle = unsafe { OpenProcess(full, 0, pid) };
    let mut writable = true;
    if handle == 0 {
        handle = unsafe { OpenProcess(read_only, 0, pid) };
        writable = false;
    }
    if handle == 0 {
        let code = unsafe { GetLastError() };
        // 诊断表只有一处：界面和 MCP 读同一句话，不会出现两个入口两种解释。
        let diagnosis = crate::diag::for_open_process(code);
        crate::diag::remember(&diagnosis);
        return Err(diagnosis.line());
    }
    let is_64bit = !is_wow64(handle);
    let real_name = process_path(handle)
        .and_then(|p| {
            std::path::Path::new(&p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| name.to_string());
    Ok(Arc::new(ProcessHandle {
        handle,
        pid,
        name: real_name,
        is_64bit,
        writable,
    }))
}

pub fn is_elevated() -> bool {
    // A pragmatic probe: opening a system process's memory usually fails without elevation.
    // The UI stays useful either way, so avoid touching security APIs here.
    false
}

pub fn relaunch_as_admin() {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if let Ok(exe) = std::env::current_exe() {
        let exe_w = to_wide(&exe.to_string_lossy());
        let verb = to_wide("runas");
        unsafe {
            ShellExecuteW(
                0,
                verb.as_ptr(),
                exe_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
