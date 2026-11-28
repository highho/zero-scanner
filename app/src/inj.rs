//! Code patching: write bytes into a target, and do it reversibly.
//!
//! Every change this module makes is recorded together with the bytes it
//! replaced, so anything applied can be undone. A patch is either restored from
//! its backup or explicitly kept; nothing here is silent.

use crate::winmem::ProcessHandle;

const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_READ: u32 = 0x20;

/// A block of memory this process allocated inside the target.
#[derive(Clone, Debug)]
pub struct CodeCave {
    pub address: u64,
    pub size: usize,
}

/// One reversible edit: the address, the bytes that were there, the bytes that
/// are there now, and whether the page protection was put back.
#[derive(Clone, Debug)]
pub struct Patch {
    pub address: u64,
    pub original: Vec<u8>,
    pub written: Vec<u8>,
    pub label: String,
    pub active: bool,
    pub protection_changed: bool,
}

impl Patch {
    pub fn size(&self) -> usize {
        self.written.len()
    }

    /// Preview of the bytes that replaced the original.
    pub fn written_hex(&self) -> String {
        preview(&self.written)
    }

    pub fn original_hex(&self) -> String {
        preview(&self.original)
    }
}

fn preview(bytes: &[u8]) -> String {
    const MAX: usize = 12;
    let mut text = bytes
        .iter()
        .take(MAX)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > MAX {
        text.push_str(" ...");
    }
    text
}

/// The x86 instructions needed to reach a code cave and come back.
///
/// entry is the jump written over the original bytes, stolen is how many
/// original bytes that jump displaced (the cave must replay them before it
/// returns).
#[derive(Clone, Debug)]
pub struct Trampoline {
    pub entry: Vec<u8>,
    pub stolen: usize,
}

impl Trampoline {
    /// Build a jump-to-cave trampoline that steals whole instructions.
    ///
    /// instruction_lengths must be the lengths of the instructions starting at
    /// from, in order, so the jump never lands in the middle of an instruction.
    pub fn jump_to(from: u64, cave: u64, instruction_lengths: &[usize]) -> Result<Self, String> {
        if instruction_lengths.is_empty() {
            return Err("需要至少一条要替换的指令。".to_string());
        }
        let mut stolen = 0usize;
        for length in instruction_lengths {
            stolen += *length;
            if stolen >= 5 {
                break;
            }
        }
        if stolen < 5 {
            return Err("要替换的指令太短，凑不够一个 5 字节跳转。".to_string());
        }
        let displacement = cave as i128 - (from as i128 + 5);
        if displacement < i32::MIN as i128 || displacement > i32::MAX as i128 {
            return Err("代码洞距离太远，超出了 5 字节跳转的范围。".to_string());
        }
        let mut entry = Vec::with_capacity(stolen);
        entry.push(0xE9);
        entry.extend_from_slice(&(displacement as i32).to_le_bytes());
        while entry.len() < stolen {
            entry.push(0x90);
        }
        Ok(Self { entry, stolen })
    }

    /// Bytes that jump from the end of the cave back to resume.
    pub fn exit_jump(piece_offset: u64, resume: u64) -> Result<Vec<u8>, String> {
        let displacement = resume as i128 - (piece_offset as i128 + 5);
        if displacement < i32::MIN as i128 || displacement > i32::MAX as i128 {
            return Err("代码洞与返回地址相距太远。".to_string());
        }
        let mut bytes = vec![0xE9];
        bytes.extend_from_slice(&(displacement as i32).to_le_bytes());
        Ok(bytes)
    }
}

/// Read the bytes a patch would overwrite, so the same patch can be undone later.
pub fn read_original(handle: &ProcessHandle, address: u64, size: usize) -> Result<Vec<u8>, String> {
    if size == 0 {
        return Err("补丁长度不能为 0。".to_string());
    }
    let mut buffer = vec![0u8; size];
    if let Err(code) = handle.read_code(address, &mut buffer) {
        let diagnosis = crate::diag::for_read_memory(code);
        return Err(format!("无法读取 {address:X} 处的原始字节：{}", diagnosis.line()));
    }
    Ok(buffer)
}

/// Apply a patch, remembering the original bytes.
///
/// The page protection is raised for the write and restored afterwards; if the
/// write fails nothing is left half-written.
pub fn apply(
    handle: &ProcessHandle,
    address: u64,
    bytes: &[u8],
    label: &str,
) -> Result<Patch, String> {
    if bytes.is_empty() {
        return Err("补丁内容为空。".to_string());
    }
    let original = read_original(handle, address, bytes.len())?;
    let previous = handle.protect(address, bytes.len(), PAGE_EXECUTE_READWRITE)?;
    let wrote = handle.write_code(address, bytes);
    let restored = handle.protect(address, bytes.len(), previous).is_ok();
    if let Err(code) = wrote {
        let diagnosis = crate::diag::for_write_memory(code);
        return Err(format!("写入 {address:X} 失败（原字节未被修改）：{}", diagnosis.line()));
    }
    Ok(Patch {
        address,
        original,
        written: bytes.to_vec(),
        label: label.to_string(),
        active: true,
        protection_changed: !restored,
    })
}

/// Put the original bytes back and mark the patch inactive.
pub fn revert(handle: &ProcessHandle, patch: &mut Patch) -> Result<(), String> {
    if !patch.active {
        return Err("这个补丁已经还原过了。".to_string());
    }
    let previous = handle.protect(patch.address, patch.original.len(), PAGE_EXECUTE_READWRITE)?;
    let wrote = handle.write_code(patch.address, &patch.original);
    let _ = handle.protect(patch.address, patch.original.len(), previous);
    if let Err(code) = wrote {
        let diagnosis = crate::diag::for_write_memory(code);
        return Err(format!("无法还原 {:X} 处的原始字节：{}", patch.address, diagnosis.line()));
    }
    patch.active = false;
    Ok(())
}

/// True when the bytes in memory are the ones the patch wrote.
pub fn verify(handle: &ProcessHandle, patch: &Patch) -> bool {
    let mut buffer = vec![0u8; patch.written.len()];
    if !handle.read(patch.address, &mut buffer) {
        return false;
    }
    buffer == patch.written
}

/// Allocate a code cave and return it only if it is usable memory.
pub fn create_cave(handle: &ProcessHandle, size: usize) -> Result<CodeCave, String> {
    let size = size.max(0x1000);
    let address = handle.allocate(size)?;
    let mut probe = vec![0u8; size.min(16)];
    if !handle.read(address, &mut probe) {
        let _ = handle.free(address);
        return Err("新分配的代码洞无法读取，已回收。".to_string());
    }
    Ok(CodeCave { address, size })
}

pub fn destroy_cave(handle: &ProcessHandle, cave: &CodeCave) -> Result<(), String> {
    handle.free(cave.address)
}

/// Write raw bytes into a cave without touching executable code.
pub fn fill_cave(handle: &ProcessHandle, cave: &CodeCave, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > cave.size {
        return Err(format!(
            "内容 {} 字节，超过代码洞容量 {} 字节。",
            bytes.len(),
            cave.size
        ));
    }
    if !handle.write(cave.address, bytes) {
        return Err("写入代码洞失败。".to_string());
    }
    Ok(())
}

/// A protection value that keeps a page executable.
pub fn executable_protection() -> u32 {
    PAGE_EXECUTE_READ
}