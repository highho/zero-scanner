//! Symbol resolution: parse a loaded module's PE export and import tables.
//!
//! Cheat Engine names addresses through symbols (exports and imports) instead of
//! raw numbers. Everything here reads the target process's own mapped image, so
//! there is no file parsing and no extra privilege beyond the open handle.

use crate::winmem::{ModuleInfo, ProcessHandle};

const MAX_NAME: usize = 512;
const MAX_EXPORTS: u32 = 500_000;
const MAX_IMPORT_LIBS: u64 = 4096;
const MAX_IMPORTS_PER_LIB: u64 = 100_000;

/// One exported or imported function.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    /// Virtual address in the target process. Forwarded exports have no local
    /// address, so this is 0 and `forwarded` carries the real target.
    pub address: u64,
    pub ordinal: u32,
    /// "other.dll!Function" when the export is a forwarder.
    pub forwarded: Option<String>,
}

impl Symbol {
    pub fn display_name(&self) -> String {
        match &self.forwarded {
            Some(target) => format!("{} -> {}", self.name, target),
            None => self.name.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImportLib {
    pub dll: String,
    pub symbols: Vec<Symbol>,
}

#[derive(Clone, Debug)]
pub struct SymbolTable {
    pub module: String,
    pub base: u64,
    pub exports: Vec<Symbol>,
    pub imports: Vec<ImportLib>,
}

impl SymbolTable {
    pub fn import_count(&self) -> usize {
        self.imports.iter().map(|lib| lib.symbols.len()).sum()
    }

    /// Name a virtual address through this module's exports, when it lands on one.
    pub fn resolve(&self, address: u64) -> Option<String> {
        self.exports
            .iter()
            .find(|sym| sym.address == address)
            .map(|sym| format!("{}!{}", self.module, sym.name))
    }

    /// Closest export at or below `address`: "module!Name+0x12" style symbolization.
    pub fn nearest(&self, address: u64) -> Option<String> {
        let mut best: Option<&Symbol> = None;
        for sym in &self.exports {
            if sym.address == 0 || sym.address > address {
                continue;
            }
            match best {
                Some(current) if current.address >= sym.address => {}
                _ => best = Some(sym),
            }
        }
        best.map(|sym| {
            let offset = address.saturating_sub(sym.address);
            if offset == 0 {
                format!("{}!{}", self.module, sym.name)
            } else {
                format!("{}!{}+{:X}", self.module, sym.name, offset)
            }
        })
    }
}

fn read_bytes(handle: &ProcessHandle, address: u64, len: usize) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; len];
    if len == 0 || handle.read(address, &mut buffer) {
        Some(buffer)
    } else {
        None
    }
}

fn read_u16(handle: &ProcessHandle, address: u64) -> Option<u16> {
    let bytes = read_bytes(handle, address, 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(handle: &ProcessHandle, address: u64) -> Option<u32> {
    let bytes = read_bytes(handle, address, 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(handle: &ProcessHandle, address: u64) -> Option<u64> {
    let bytes = read_bytes(handle, address, 8)?;
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[..8]);
    Some(u64::from_le_bytes(raw))
}

/// Read a NUL-terminated ASCII string from the target process.
fn read_cstr(handle: &ProcessHandle, address: u64) -> Option<String> {
    let mut out: Vec<u8> = Vec::new();
    let mut cursor = address;
    loop {
        let chunk = read_bytes(handle, cursor, 64)?;
        for byte in chunk {
            if byte == 0 {
                return String::from_utf8(out).ok();
            }
            out.push(byte);
            if out.len() > MAX_NAME {
                return None;
            }
        }
        cursor = cursor.checked_add(64)?;
    }
}

struct OptionalHeader {
    /// Absolute address of the first data directory entry (optional header start
    /// plus the format's data directory offset).
    dir_address: u64,
    dir_count: usize,
}

fn optional_header(handle: &ProcessHandle, image_base: u64) -> Result<OptionalHeader, String> {
    let dos = read_bytes(handle, image_base, 0x40)
        .ok_or_else(|| "无法读取模块的 DOS 头。".to_string())?;
    if &dos[0..2] != b"MZ" {
        return Err("模块开头不是 MZ，可能不是 PE 映像。".to_string());
    }
    let mut lfanew = [0u8; 4];
    lfanew.copy_from_slice(&dos[0x3C..0x40]);
    let nt = image_base
        .checked_add(u32::from_le_bytes(lfanew) as u64)
        .ok_or_else(|| "DOS 头的 PE 偏移无效。".to_string())?;
    let signature = read_bytes(handle, nt, 4).ok_or_else(|| "无法读取 PE 签名。".to_string())?;
    if signature != b"PE\0\0" {
        return Err("PE 签名不匹配。".to_string());
    }
    let coff = read_bytes(handle, nt + 4, 20).ok_or_else(|| "无法读取 COFF 头。".to_string())?;
    let size_optional = u16::from_le_bytes([coff[16], coff[17]]) as usize;
    if size_optional < 0x40 || size_optional > 0x1000 {
        return Err("可选头大小异常。".to_string());
    }
    let optional = read_bytes(handle, nt + 24, size_optional)
        .ok_or_else(|| "无法读取可选头。".to_string())?;
    let magic = u16::from_le_bytes([optional[0], optional[1]]);
    let (dir_offset, dir_count_offset) = match magic {
        0x10B => (96usize, 92usize),  // PE32
        0x20B => (112usize, 108usize), // PE32+
        other => return Err(format!("未知的可选头格式 0x{other:X}。")),
    };
    if optional.len() < dir_offset {
        return Err("可选头不足以容纳数据目录。".to_string());
    }
    let optional_address = nt
        .checked_add(24)
        .ok_or_else(|| "PE 头偏移无效。".to_string())?;
    let dir_address = optional_address
        .checked_add(dir_offset as u64)
        .ok_or_else(|| "可选头偏移无效。".to_string())?;
    let mut count_bytes = [0u8; 4];
    count_bytes.copy_from_slice(&optional[dir_count_offset..dir_count_offset + 4]);
    Ok(OptionalHeader {
        dir_address,
        dir_count: u32::from_le_bytes(count_bytes) as usize,
    })
}

fn data_directory(handle: &ProcessHandle, header: &OptionalHeader, index: usize) -> Option<(u32, u32)> {
    if index >= header.dir_count || index > 15 {
        return None;
    }
    let entry = header.dir_address + (index * 8) as u64;
    let rva = read_u32(handle, entry)?;
    let size = read_u32(handle, entry + 4)?;
    Some((rva, size))
}

fn parse_exports(
    handle: &ProcessHandle,
    base: u64,
    dir_rva: u32,
    dir_size: u32,
) -> Result<Vec<Symbol>, String> {
    let dir_address = base + dir_rva as u64;
    let header = read_bytes(handle, dir_address, 40)
        .ok_or_else(|| "无法读取导出目录。".to_string())?;
    let read_dir_u32 = |offset: usize| -> u32 {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(&header[offset..offset + 4]);
        u32::from_le_bytes(raw)
    };
    let ordinal_base = read_dir_u32(16);
    let number_of_functions = read_dir_u32(20);
    let number_of_names = read_dir_u32(24).min(MAX_EXPORTS);
    let functions_rva = read_dir_u32(28);
    let names_rva = read_dir_u32(32);
    let ordinals_rva = read_dir_u32(36);
    let mut out = Vec::new();
    for index in 0..number_of_names {
        let name_rva = match read_u32(handle, base + names_rva as u64 + index as u64 * 4) {
            Some(value) => value,
            None => continue,
        };
        let ordinal_index = match read_u16(handle, base + ordinals_rva as u64 + index as u64 * 2) {
            Some(value) => value,
            None => continue,
        };
        if ordinal_index >= number_of_functions as u16 && number_of_functions > 0 && ordinal_index as u32 >= number_of_functions {
            continue;
        }
        let function_rva = match read_u32(handle, base + functions_rva as u64 + ordinal_index as u64 * 4) {
            Some(value) => value,
            None => continue,
        };
        let name = match read_cstr(handle, base + name_rva as u64) {
            Some(value) => value,
            None => continue,
        };
        let is_forwarder = dir_size > 0
            && function_rva >= dir_rva
            && function_rva < dir_rva.saturating_add(dir_size);
        let forwarded = if is_forwarder {
            read_cstr(handle, base + function_rva as u64)
        } else {
            None
        };
        out.push(Symbol {
            name,
            address: if is_forwarder { 0 } else { base + function_rva as u64 },
            ordinal: ordinal_base.saturating_add(ordinal_index as u32),
            forwarded,
        });
    }
    out.sort_by(|a, b| a.address.cmp(&b.address).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn read_thunk(handle: &ProcessHandle, address: u64, width: u32) -> Option<(u64, bool)> {
    if width == 8 {
        let value = read_u64(handle, address)?;
        if value == 0 {
            return Some((0, false));
        }
        let is_ordinal = value & 0x8000_0000_0000_0000 != 0;
        Some((value & 0x7FFF_FFFF_FFFF_FFFF, is_ordinal))
    } else {
        let value = read_u32(handle, address)? as u64;
        if value == 0 {
            return Some((0, false));
        }
        let is_ordinal = value & 0x8000_0000 != 0;
        Some((value & 0x7FFF_FFFF, is_ordinal))
    }
}

fn parse_imports(handle: &ProcessHandle, base: u64, dir_rva: u32) -> Result<Vec<ImportLib>, String> {
    let width: u32 = if handle.is_64bit { 8 } else { 4 };
    let mut out = Vec::new();
    let mut descriptor_index: u64 = 0;
    loop {
        if descriptor_index >= MAX_IMPORT_LIBS {
            break;
        }
        let descriptor = base + dir_rva as u64 + descriptor_index * 20;
        let raw = match read_bytes(handle, descriptor, 20) {
            Some(value) => value,
            None => break,
        };
        if raw.iter().all(|byte| *byte == 0) {
            break;
        }
        let mut u32_field = |offset: usize| -> u32 {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&raw[offset..offset + 4]);
            u32::from_le_bytes(bytes)
        };
        let original_thunk = u32_field(0);
        let name_rva = u32_field(12);
        let first_thunk = u32_field(16);
        descriptor_index += 1;
        let dll = match read_cstr(handle, base + name_rva as u64) {
            Some(value) => value,
            None => continue,
        };
        let thunk_rva = if original_thunk != 0 {
            original_thunk
        } else {
            first_thunk
        };
        let mut symbols = Vec::new();
        let mut index: u64 = 0;
        while index < MAX_IMPORTS_PER_LIB {
            let Some((value, is_ordinal)) =
                read_thunk(handle, base + thunk_rva as u64 + index * width as u64, width)
            else {
                break;
            };
            if value == 0 {
                break;
            }
            let name = if is_ordinal {
                format!("Ordinal_{value}")
            } else {
                // IMAGE_IMPORT_BY_NAME: a u16 hint followed by the name.
                match read_cstr(handle, base + value + 2) {
                    Some(value) => value,
                    None => break,
                }
            };
            symbols.push(Symbol {
                name,
                address: base + first_thunk as u64 + index * width as u64,
                ordinal: 0,
                forwarded: None,
            });
            index += 1;
        }
        if !symbols.is_empty() {
            out.push(ImportLib { dll, symbols });
        }
    }
    out.sort_by(|a, b| a.dll.to_ascii_lowercase().cmp(&b.dll.to_ascii_lowercase()));
    Ok(out)
}

/// Parse both symbol tables of one loaded module.
pub fn parse(handle: &ProcessHandle, module: &ModuleInfo) -> Result<SymbolTable, String> {
    let base = module.base;
    if base == 0 {
        return Err("模块基址为空。".to_string());
    }
    let header = optional_header(handle, base)?;
    let mut exports = Vec::new();
    if let Some((rva, size)) = data_directory(handle, &header, 0) {
        if rva != 0 {
            exports = parse_exports(handle, base, rva, size)?;
        }
    }
    let mut imports = Vec::new();
    if let Some((rva, _)) = data_directory(handle, &header, 1) {
        if rva != 0 {
            imports = parse_imports(handle, base, rva)?;
        }
    }
    Ok(SymbolTable {
        module: module.name.clone(),
        base,
        exports,
        imports,
    })
}

/// The module list is what the pointer scan already caches; keep this helper so
/// callers do not have to re-scan Toolhelp just to find a name.
pub fn find_module(modules: &[ModuleInfo], name: &str) -> Option<ModuleInfo> {
    modules
        .iter()
        .find(|module| module.name.eq_ignore_ascii_case(name))
        .cloned()
}

/// Diagnostics for the headless self-check: dump the PE optional header and the
/// first two data directories of one module, so a failed parse can be traced.
pub fn diagnostics(handle: &ProcessHandle, module: &ModuleInfo) -> String {
    let mut out = String::new();
    match optional_header(handle, module.base) {
        Ok(header) => {
            out.push_str(&format!("dirs={} ", header.dir_count));
            for index in 0..2 {
                match data_directory(handle, &header, index) {
                    Some((rva, size)) => {
                        out.push_str(&format!("dir{index}=({rva:X},{size:X}) "))
                    }
                    None => out.push_str(&format!("dir{index}=none ")),
                }
            }
        }
        Err(error) => out.push_str(&format!("header error: {error}")),
    }
    out
}

