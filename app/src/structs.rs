//! Dissect: read the bytes behind an address as a structure.
//!
//! Game state is rarely a single scalar. The variable the user found often
//! sits inside a bigger block - a player struct with health, ammo, position,
//! states, pointers to other objects. This module reads the block behind an
//! address and guesses the layout: every few bytes become a candidate field
//! with its offset, size, hex, integer, float and ASCII interpretation.
//!
//! The guess is deliberately conservative and transparent: it is a reading
//! aid, not a type system. Every field carries the raw bytes so the user can
//! verify the interpretation before acting on it.

use crate::model::{format_float, VType};
use crate::winmem::{ModuleInfo, ProcessHandle};

/// How the dissector interpreted one span of bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    /// Looks like a readable pointer into the process.
    Pointer,
    /// A 4-byte whole number or on-disk byte field.
    I32,
    /// A 4-byte IEEE float.
    F32,
    /// An 8-byte whole number.
    I64,
    /// An 8-byte IEEE float.
    F64,
    /// A 2-byte whole number.
    I16,
    /// A single byte.
    I8,
    /// NUL-terminated ASCII text.
    Str,
    /// Everything else: raw padding.
    Bytes,
}

impl FieldKind {
    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Pointer => "指针",
            FieldKind::I32 => "I32",
            FieldKind::F32 => "F32",
            FieldKind::I64 => "I64",
            FieldKind::F64 => "F64",
            FieldKind::I16 => "I16",
            FieldKind::I8 => "I8",
            FieldKind::Str => "文本",
            FieldKind::Bytes => "字节",
        }
    }
    pub fn vtype(self) -> Option<VType> {
        match self {
            FieldKind::I32 => Some(VType::I32),
            FieldKind::F32 => Some(VType::F32),
            FieldKind::I64 => Some(VType::I64),
            FieldKind::F64 => Some(VType::F64),
            FieldKind::I16 => Some(VType::I16),
            FieldKind::I8 => Some(VType::I8),
            FieldKind::Pointer => Some(VType::I64),
            _ => None,
        }
    }
}

/// One interpreted span inside the dissected block.
#[derive(Clone, Debug)]
pub struct Field {
    /// Byte offset from the block start.
    pub offset: usize,
    pub kind: FieldKind,
    pub bytes: Vec<u8>,
    pub unsigned: u64,
    pub signed: i64,
    pub float: Option<f64>,
    pub ascii: String,
    /// A human hint: pointer target, string content, or an obvious pattern.
    pub note: String,
}

impl Field {
    pub fn hex(&self) -> String {
        self.bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
    }
    pub fn value_text(&self) -> String {
        match self.kind {
            FieldKind::Pointer => format!("0x{:X}", self.unsigned),
            FieldKind::F32 | FieldKind::F64 => self.float.map(format_float).unwrap_or_default(),
            FieldKind::I8 | FieldKind::I16 | FieldKind::I32 | FieldKind::I64 => self.signed.to_string(),
            FieldKind::Str => self.ascii.clone(),
            FieldKind::Bytes => String::new(),
        }
    }
    /// A short annotation prepended to the value, like a guessed field name.
    pub fn summary(&self) -> String {
        if self.note.is_empty() {
            format!("{} · {}", self.kind.label(), self.value_text())
        } else {
            format!("{} · {} · {}", self.note, self.kind.label(), self.value_text())
        }
    }
}

/// The whole dissected block.
#[derive(Clone, Debug)]
pub struct Dissect {
    pub address: u64,
    /// Module-relative annotation of the block start, when known.
    pub annotation: String,
    pub size: usize,
    pub fields: Vec<Field>,
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes.get(offset..offset + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes)
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes.get(offset..offset + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
}
fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes.get(offset..offset + 8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
}

fn ascii_of(bytes: &[u8]) -> String {
    let printable: String = bytes
        .iter()
        .map(|b| if (0x20..0x7f).contains(b) { *b as char } else { '.' })
        .collect();
    let trimmed = printable.trim_end_matches('.').to_string();
    if trimmed.len() >= 2 { trimmed } else { String::new() }
}

/// Is this pointer-looking value actually inside a readable region?
/// `read_pointer` fails on unmapped addresses, which is the cheap check.
fn is_readable_pointer(handle: &ProcessHandle, pointer: u64) -> bool {
    if pointer == 0 || pointer > 0x0000_7fff_ffff_ffff {
        return false;
    }
    let mut probe = [0u8; 8];
    handle.read(pointer, &mut probe)
}

/// Read a block and interpret it into candidate fields.
///
/// Cost is bounded: the caller picks the size, and here we peek a few bytes
/// at each candidate span to decide the kind. No writes ever happen.
pub fn dissect(
    handle: &ProcessHandle,
    modules: &[ModuleInfo],
    address: u64,
    size: usize,
) -> Option<Dissect> {
    let size = size.clamp(16, 4096);
    let mut bytes = vec![0u8; size];
    if !handle.read(address, &mut bytes) {
        return None;
    }
    let annotation = crate::addr::annotate(modules, address);
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < size {
        let remaining = size - offset;
        let field = interpret_field(handle, &bytes, offset, remaining);
        let step = field.kind.size();
        fields.push(field);
        offset += step;
    }
    Some(Dissect { address, annotation, size, fields })
}

impl FieldKind {
    pub fn size(self) -> usize {
        match self {
            FieldKind::Pointer | FieldKind::I64 | FieldKind::F64 => 8,
            FieldKind::I32 | FieldKind::F32 | FieldKind::Str => 4,
            FieldKind::I16 => 2,
            FieldKind::I8 | FieldKind::Bytes => 1,
        }
    }
}

fn interpret_field(handle: &ProcessHandle, bytes: &[u8], offset: usize, remaining: usize) -> Field {
    let make = |kind: FieldKind, step: usize| -> Field {
        let span = &bytes[offset..offset + step.min(remaining)];
        let mut field = Field {
            offset,
            kind,
            bytes: span.to_vec(),
            unsigned: 0,
            signed: 0,
            float: None,
            ascii: String::new(),
            note: String::new(),
        };
        match kind {
            FieldKind::Pointer => {
                field.unsigned = u64_at(bytes, offset).unwrap_or(0);
                field.signed = field.unsigned as i64;
                field.note = format!("→ 0x{:X}", field.unsigned);
            }
            FieldKind::I64 => {
                field.unsigned = u64_at(bytes, offset).unwrap_or(0);
                field.signed = field.unsigned as i64;
            }
            FieldKind::F64 => {
                field.unsigned = u64_at(bytes, offset).unwrap_or(0);
                field.signed = field.unsigned as i64;
                field.float = Some(f64::from_le_bytes(field.bytes[0..8].try_into().unwrap_or([0u8; 8])));
            }
            FieldKind::I32 => {
                field.unsigned = u32_at(bytes, offset).unwrap_or(0) as u64;
                field.signed = field.unsigned as i32 as i64;
            }
            FieldKind::F32 => {
                field.unsigned = u32_at(bytes, offset).unwrap_or(0) as u64;
                field.signed = field.unsigned as i32 as i64;
                field.float = Some(f32::from_le_bytes(field.bytes[0..4].try_into().unwrap_or([0u8; 4])) as f64);
            }
            FieldKind::I16 => {
                field.unsigned = u16_at(bytes, offset).unwrap_or(0) as u64;
                field.signed = field.unsigned as i16 as i64;
            }
            FieldKind::I8 => {
                field.unsigned = bytes[offset] as u64;
                field.signed = bytes[offset] as i8 as i64;
            }
            FieldKind::Str => {
                field.ascii = ascii_of(&bytes[offset..offset + step.min(remaining)]);
                field.note = format!("\"{}\"", field.ascii);
            }
            FieldKind::Bytes => {
                field.unsigned = bytes[offset] as u64;
                field.signed = bytes[offset] as i8 as i64;
            }
        }
        field
    };

    if remaining >= 8 {
        let pointer = u64_at(bytes, offset).unwrap_or(0);
        if pointer != 0 && is_readable_pointer(handle, pointer) {
            return make(FieldKind::Pointer, 8);
        }
        // 8-byte floats are rarer than 8-byte ints; only label F64 when the
        // reading is a plausible small float, otherwise keep the signed int.
        let f = f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap_or([0u8; 8]));
        if f.is_finite() && f.fract() != 0.0 && f.abs() < 1e9 {
            return make(FieldKind::F64, 8);
        }
        return make(FieldKind::I64, 8);
    }
    if remaining >= 4 {
        let raw = u32_at(bytes, offset).unwrap_or(0);
        // A NUL-terminated ASCII string of meaningful length beats a number.
        let ascii = ascii_of(&bytes[offset..offset + 4]);
        if !ascii.is_empty() && raw <= 0x7f_7f_7f_7f {
            return make(FieldKind::Str, 4);
        }
        let f = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0u8; 4]));
        // A value with a genuine fractional part reads as a float; anything
        // else (even a float whose integer part is exact) reads as I32.
        let float_plausible = f.is_finite() && f.fract() != 0.0 && f.abs() < 1_000_000.0;
        if float_plausible {
            return make(FieldKind::F32, 4);
        }
        return make(FieldKind::I32, 4);
    }
    if remaining >= 2 {
        return make(FieldKind::I16, 2);
    }
    make(FieldKind::I8, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_of_filters() {
        assert_eq!(ascii_of(b"ABC\0\0"), "ABC");
        assert_eq!(ascii_of(b"\xff\x00\x00"), "");
    }

    #[test]
    fn kind_size_sane() {
        assert_eq!(FieldKind::I32.size(), 4);
        assert_eq!(FieldKind::Pointer.size(), 8);
        assert_eq!(FieldKind::I8.size(), 1);
    }
}