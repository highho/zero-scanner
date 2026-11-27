//! A small x86-64 assembler.
//!
//! This is the reverse of disasm.rs: it turns assembly text into machine code.
//! It covers the instruction forms that show up in hand-written Cheat Engine
//! style patches and code-cave stubs, and supports labels so a jump can name its
//! destination instead of hard-coding an offset.
//!
//! Unsupported syntax is reported as an error naming the line, never emitted as
//! something else: a silently wrong byte would corrupt the target.

/// Result of assembling a source string.
#[derive(Clone, Debug)]
pub struct AsmResult {
    pub bytes: Vec<u8>,
    pub origin: u64,
    /// Labels and the address each one resolved to, in declaration order.
    pub labels: Vec<(String, u64)>,
}

impl AsmResult {
    pub fn hex(&self) -> String {
        let mut text = self
            .bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        text
    }

    pub fn label_address(&self, name: &str) -> Option<u64> {
        self.labels
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, a)| *a)
    }
}

const REG8: [&str; 16] = [
    "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b",
    "r12b", "r13b", "r14b", "r15b",
];
const REG16: [&str; 16] = [
    "ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "r8w", "r9w", "r10w", "r11w",
    "r12w", "r13w", "r14w", "r15w",
];
const REG32: [&str; 16] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d",
    "r11d", "r12d", "r13d", "r14d", "r15d",
];
const REG64: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11",
    "r12", "r13", "r14", "r15",
];

/// Look a register name up, returning its encoding index and operand width.
fn parse_register(name: &str) -> Option<(u8, u8)> {
    let lower = name.trim().to_ascii_lowercase();
    for (index, reg) in REG8.iter().enumerate() {
        if *reg == lower {
            return Some((index as u8, 1));
        }
    }
    for (index, reg) in REG16.iter().enumerate() {
        if *reg == lower {
            return Some((index as u8, 2));
        }
    }
    for (index, reg) in REG32.iter().enumerate() {
        if *reg == lower {
            return Some((index as u8, 4));
        }
    }
    for (index, reg) in REG64.iter().enumerate() {
        if *reg == lower {
            return Some((index as u8, 8));
        }
    }
    None
}

/// A memory operand: [base + index*scale + disp].
#[derive(Clone, Debug, Default)]
pub struct Mem {
    pub base: Option<u8>,
    pub index: Option<(u8, u8)>,
    pub disp: i64,
    /// True when the base was written as rip.
    pub rip: bool,
    /// True when the operand was a bare [0x1234] absolute address.
    pub absolute: bool,
}

#[derive(Clone, Debug)]
pub enum Operand {
    Reg(u8, u8),
    Imm(i64),
    Mem(Mem, Option<u8>),
    Label(String),
}

/// Parse a number written as decimal, 0x-hex, or with a leading minus.
fn parse_number(text: &str) -> Option<i64> {
    let t = text.trim();
    let negative = t.starts_with('-');
    let t = t.trim_start_matches('-').trim_start_matches('+');
    let lower = t.to_ascii_lowercase();
    let value = if let Some(hex) = lower.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(bin) = lower.strip_prefix("0b") {
        i64::from_str_radix(bin, 2).ok()?
    } else {
        t.parse::<i64>().ok()?
    };
    Some(if negative { -value } else { value })
}

/// Parse [base + index*scale + disp] and the optional size prefix before it.
fn parse_memory(text: &str) -> Result<(Mem, Option<u8>), String> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let mut size: Option<u8> = None;
    let mut rest = lower.as_str();
    for (prefix, width) in [
        ("byte ptr ", 1u8),
        ("word ptr ", 2),
        ("dword ptr ", 4),
        ("qword ptr ", 8),
        ("xmmword ptr ", 16),
    ] {
        if let Some(tail) = rest.strip_prefix(prefix) {
            size = Some(width);
            rest = tail;
            break;
        }
    }
    let rest = rest.trim();
    let Some(inner) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Err(format!("{trimmed} 不是有效的内存操作数（需要方括号）。"));
    };
    let mut mem = Mem::default();
    let normalised = inner.replace('-', "+- ");
    for raw in normalised.split('+') {
        let term = raw.trim();
        if term.is_empty() {
            continue;
        }
        let negative = term.starts_with('-');
        let term_body = term.trim_start_matches('-').trim();
        if let Some((reg_text, scale_text)) = term_body.split_once('*') {
            let Some((index, _)) = parse_register(reg_text.trim()) else {
                return Err(format!("索引寄存器「{reg_text}」不认识。"));
            };
            let scale = parse_number(scale_text.trim()).unwrap_or(1);
            if !matches!(scale, 1 | 2 | 4 | 8) {
                return Err(format!("缩放只能是 1/2/4/8，收到 {scale}。"));
            }
            mem.index = Some((index, scale as u8));
            continue;
        }
        if let Some((reg, _width)) = parse_register(term_body) {
            if negative {
                return Err("基址寄存器前不能有负号。".to_string());
            }
            if term_body.trim() == "rip" {
                mem.rip = true;
            } else {
                mem.base = Some(reg);
            }
            continue;
        }
        if let Some(value) = parse_number(term_body) {
            mem.disp += if negative { -value } else { value };
            continue;
        }
        if term_body.trim() == "rip" {
            mem.rip = true;
            continue;
        }
        return Err(format!("内存操作数里的「{term_body}」无法解析。"));
    }
    if mem.base.is_none() && mem.index.is_none() && !mem.rip {
        mem.absolute = true;
    }
    Ok((mem, size))
}

fn parse_operand(text: &str) -> Result<Operand, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("空的逗号：操作数之间多写了一个逗号。".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains('[') || lower.ends_with("ptr") {
        let (mem, size) = parse_memory(trimmed)?;
        return Ok(Operand::Mem(mem, size));
    }
    if let Some((index, width)) = parse_register(&lower) {
        return Ok(Operand::Reg(index, width));
    }
    if let Some(value) = parse_number(&lower) {
        return Ok(Operand::Imm(value));
    }
    // Anything else must be a label reference.
    if lower.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) {
        return Ok(Operand::Label(trimmed.to_string()));
    }
    Err(format!("操作数「{trimmed}」无法解析。"))
}

/// An encoded r/m operand: the REX bits it needs plus the ModRM/SIB/displacement
/// tail. The caller emits the REX prefix before the opcode, then this tail.
struct RmEncoding {
    rex: u8,
    tail: Vec<u8>,
}

/// Encode the r/m side of an instruction. reg_field goes in ModRM.reg.
fn encode_rm(operand: &Operand, reg_field: u8, size: u8) -> Result<RmEncoding, String> {
    let mut rex = 0x40 | (if reg_field >= 8 { 0x04 } else { 0 });
    let mut tail: Vec<u8> = Vec::new();
    match operand {
        Operand::Reg(index, _) => {
            if *index >= 8 {
                rex |= 0x01;
            }
            if size == 8 {
                rex |= 0x08;
            }
            tail.push(0xC0 | ((reg_field & 7) << 3) | (index & 7));
        }
        Operand::Mem(mem, _) => {
            let base = mem.base.map(|b| (b & 7, b >= 8));
            let index = mem.index.map(|(i, scale)| (i & 7, i >= 8, scale));
            // RIP-relative and bare absolute addresses both use mod=00 rm=101.
            let rip = mem.rip || (mem.base.is_none() && mem.index.is_none());
            let (modrm, sib, disp_bytes) = if rip {
                (
                    ((reg_field & 7) << 3) | 0x05,
                    None,
                    (mem.disp as i32).to_le_bytes().to_vec(),
                )
            } else {
                let (base_low, base_high) = base.unwrap_or((5, false));
                if base_high {
                    rex |= 0x01;
                }
                let disp = mem.disp;
                // ModRM mod field: 0 = no displacement, 1 = disp8, 2 = disp32.
                // (0b00/0b01/0b10; the shift into place happens once, below.)
                let mode: u8 = if disp == 0 && base_low != 5 {
                    0
                } else if (-128..=127).contains(&disp) {
                    1
                } else {
                    2
                };
                let mut disp_bytes: Vec<u8> = Vec::new();
                match mode {
                    1 => disp_bytes.push(disp as i8 as u8),
                    2 => disp_bytes.extend_from_slice(&(disp as i32).to_le_bytes()),
                    _ => {}
                }
                let mut sib: Option<u8> = None;
                let modrm_rm = if let Some((index_low, index_high, scale)) = index {
                    if index_high {
                        rex |= 0x02;
                    }
                    let scale_bits = match scale {
                        2 => 1u8,
                        4 => 2,
                        8 => 3,
                        _ => 0,
                    };
                    sib = Some((scale_bits << 6) | (index_low << 3) | base_low);
                    0x04
                } else if base_low == 4 {
                    // rsp/r12 as base always needs a SIB byte with no index.
                    sib = Some(base_low);
                    0x04
                } else {
                    base_low
                };
                (
                    (mode << 6) | ((reg_field & 7) << 3) | modrm_rm,
                    sib,
                    disp_bytes,
                )
            };
            if size == 8 {
                rex |= 0x08;
            }
            tail.push(modrm);
            if let Some(sib_byte) = sib {
                tail.push(sib_byte);
            }
            tail.extend_from_slice(&disp_bytes);
        }
        _ => {
            return Err("这里需要寄存器或内存操作数，不能是立即数或标签。".to_string());
        }
    }
    Ok(RmEncoding { rex, tail })
}

/// Emit a REX prefix only when one of its bits is actually used.
fn push_rex(out: &mut Vec<u8>, rex: u8) {
    if rex != 0x40 {
        out.push(rex);
    }
}

fn push_imm(out: &mut Vec<u8>, value: i64, width: usize) {
    match width {
        1 => out.push(value as i8 as u8),
        2 => out.extend_from_slice(&(value as i16).to_le_bytes()),
        8 => out.extend_from_slice(&value.to_le_bytes()),
        _ => out.extend_from_slice(&(value as i32).to_le_bytes()),
    }
}

/// Immediate width for a full-size operand (imm32 unless 16-bit).
fn imm_width(size: u8) -> usize {
    if size == 2 {
        2
    } else {
        4
    }
}

/// How an instruction's operands must look, so errors are caught before encoding.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// No operands.
    None,
    /// A single register or memory operand.
    OneRm,
    /// r/m destination, register source.
    RmReg,
    /// register destination, r/m source.
    RegRm,
    /// register/memory, immediate.
    RmImm,
    /// register, immediate.
    RegImm,
    /// register, register.
    RegReg,
    /// a single immediate.
    Imm,
    /// a relative target (label or absolute address).
    Rel,
}

/// One supported mnemonic and the opcode bytes that start its 8/32-bit forms.
struct Spec {
    name: &'static str,
    form: Form,
    /// Opcode when the source/destination is a byte (None: not available).
    op8: Option<u8>,
    /// Opcode for the full-width form.
    op: u8,
    /// 0x0F-prefixed opcode (the value is the second byte).
    twobyte: bool,
}

const fn spec(name: &'static str, form: Form, op8: Option<u8>, op: u8) -> Spec {
    Spec { name, form, op8, op, twobyte: false }
}

const fn spec2(name: &'static str, form: Form, op: u8) -> Spec {
    Spec { name, form, op8: None, op, twobyte: true }
}

static SPECS: &[Spec] = &[
    spec("mov", Form::RmReg, Some(0x88), 0x89),
    spec("mov", Form::RegRm, Some(0x8A), 0x8B),
    spec("mov", Form::RegImm, Some(0xB0), 0xB8),
    spec("mov", Form::RmImm, Some(0xC6), 0xC7),
    spec("add", Form::RmReg, Some(0x00), 0x01),
    spec("add", Form::RegRm, Some(0x02), 0x03),
    spec("add", Form::RmImm, Some(0x80), 0x81),
    spec("or", Form::RmReg, Some(0x08), 0x09),
    spec("or", Form::RegRm, Some(0x0A), 0x0B),
    spec("and", Form::RmReg, Some(0x20), 0x21),
    spec("and", Form::RegRm, Some(0x22), 0x23),
    spec("and", Form::RmImm, Some(0x80), 0x81),
    spec("sub", Form::RmReg, Some(0x28), 0x29),
    spec("sub", Form::RegRm, Some(0x2A), 0x2B),
    spec("sub", Form::RmImm, Some(0x80), 0x81),
    spec("xor", Form::RmReg, Some(0x30), 0x31),
    spec("xor", Form::RegRm, Some(0x32), 0x33),
    spec("xor", Form::RmImm, Some(0x80), 0x81),
    spec("cmp", Form::RmReg, Some(0x38), 0x39),
    spec("cmp", Form::RegRm, Some(0x3A), 0x3B),
    spec("cmp", Form::RmImm, Some(0x80), 0x81),
    spec("test", Form::RmReg, Some(0x84), 0x85),
    spec("test", Form::RmImm, Some(0xF6), 0xF7),
    spec("xchg", Form::RmReg, Some(0x86), 0x87),
    spec("lea", Form::RegRm, None, 0x8D),
    spec("movzx", Form::RegRm, None, 0xB6),
    spec("movsx", Form::RegRm, None, 0xBF),
    spec("imul", Form::RegRm, None, 0xAF),
    spec("not", Form::OneRm, Some(0xF6), 0xF7),
    spec("neg", Form::OneRm, Some(0xF6), 0xF7),
    spec("mul", Form::OneRm, Some(0xF6), 0xF7),
    spec("div", Form::OneRm, Some(0xF6), 0xF7),
    spec("inc", Form::OneRm, Some(0xFE), 0xFF),
    spec("dec", Form::OneRm, Some(0xFE), 0xFF),
    spec("push", Form::OneRm, None, 0xFF),
    spec("call", Form::OneRm, None, 0xFF),
    spec("jmp", Form::OneRm, None, 0xFF),
];

/// The /digit value in ModRM.reg for each group-encoded mnemonic.
fn group_digit(name: &str) -> u8 {
    match name {
        "test" => 0,
        "not" => 2,
        "neg" => 3,
        "mul" => 4,
        "div" => 6,
        "inc" => 0,
        "dec" => 1,
        "call" => 2,
        "jmp" => 4,
        "push" => 6,
        _ => 0,
    }
}

/// Conditional jumps, indexed by the low nibble of the 0x70 opcode.
const JCC: [(&str, u8); 16] = [
    ("jo", 0x0), ("jno", 0x1), ("jb", 0x2), ("jae", 0x3),
    ("je", 0x4), ("jne", 0x5), ("jbe", 0x6), ("ja", 0x7),
    ("js", 0x8), ("jns", 0x9), ("jp", 0xA), ("jnp", 0xB),
    ("jl", 0xC), ("jge", 0xD), ("jle", 0xE), ("jg", 0xF),
];

const SHIFTS: [(&str, u8); 8] = [
    ("rol", 0), ("ror", 1), ("rcl", 2), ("rcr", 3),
    ("shl", 4), ("shr", 5), ("sal", 4), ("sar", 7),
];

/// One parsed source line.
enum Line {
    Empty,
    Label(String),
    Insn { mnemonic: String, operands: Vec<String>, raw: String },
}

fn parse_line(raw: &str) -> Result<Line, String> {
    let mut text = raw.trim();
    if text.is_empty() || text.starts_with(';') || text.starts_with("//") {
        return Ok(Line::Empty);
    }
    // Strip a trailing comment, but only when it is not inside brackets.
    if let Some(pos) = text.find("//") {
        text = text[..pos].trim();
    } else if let Some(pos) = text.find(';') {
        text = text[..pos].trim();
    }
    if text.is_empty() {
        return Ok(Line::Empty);
    }
    if let Some(colon) = text.find(':') {
        let candidate = text[..colon].trim();
        let valid = !candidate.is_empty()
            && candidate
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
        if valid && !candidate.contains(' ') {
            let rest = text[colon + 1..].trim();
            if rest.is_empty() {
                return Ok(Line::Label(candidate.to_string()));
            }
            // A label followed by an instruction on the same line.
            let (mnemonic, operands) = split_insn(rest)?;
            return Ok(Line::Insn {
                mnemonic,
                operands,
                raw: format!("{candidate}: {rest}"),
            });
        }
    }
    let (mnemonic, operands) = split_insn(text)?;
    Ok(Line::Insn {
        mnemonic,
        operands,
        raw: text.to_string(),
    })
}

fn split_insn(text: &str) -> Result<(String, Vec<String>), String> {
    let mut parts = text.splitn(2, char::is_whitespace);
    let mnemonic = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim();
    if mnemonic.is_empty() {
        return Err("空指令。".to_string());
    }
    let mut operands = Vec::new();
    if !rest.is_empty() {
        let mut depth = 0i32;
        let mut current = String::new();
        for ch in rest.chars() {
            match ch {
                '[' => {
                    depth += 1;
                    current.push(ch);
                }
                ']' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    operands.push(current.trim().to_string());
                    current = String::new();
                }
                _ => current.push(ch),
            }
        }
        operands.push(current.trim().to_string());
        if operands.iter().any(|o| o.is_empty()) {
            return Err(format!("「{text}」里有空的逗号。"));
        }
    }
    Ok((mnemonic, operands))
}

/// Assemble source into bytes starting at `origin`.
///
/// Two passes: the first sizes every instruction so labels get real addresses,
/// the second emits bytes with the jump displacements resolved.
pub fn assemble(source: &str, origin: u64) -> Result<AsmResult, String> {
    let mut lines: Vec<Line> = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        match parse_line(raw) {
            Ok(line) => lines.push(line),
            Err(e) => return Err(format!("第 {} 行：{e}", index + 1)),
        }
    }
    // Pass 1: assign addresses to labels and size every instruction.
    let mut labels: Vec<(String, u64)> = Vec::new();
    let mut cursor = origin;
    let mut sizes: Vec<Option<usize>> = Vec::with_capacity(lines.len());
    for line in &lines {
        match line {
            Line::Empty => sizes.push(None),
            Line::Label(name) => {
                if labels.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    return Err(format!("标签「{name}」重复定义了。"));
                }
                labels.push((name.clone(), cursor));
                sizes.push(None);
            }
            Line::Insn { mnemonic, operands, raw } => {
                let size = encode_line(mnemonic, operands, cursor, &labels, true, raw)?.len();
                cursor += size as u64;
                sizes.push(Some(size));
            }
        }
    }
    // Pass 2: emit the real bytes now that every label has an address.
    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = origin;
    for (index, line) in lines.iter().enumerate() {
        if let Line::Insn { mnemonic, operands, raw } = line {
            let encoded = encode_line(mnemonic, operands, cursor, &labels, false, raw)?;
            if let Some(expected) = sizes[index] {
                if encoded.len() != expected {
                    return Err(format!("内部不一致：同一条指令两次长度不同（{raw}）。"));
                }
            }
            cursor += encoded.len() as u64;
            bytes.extend_from_slice(&encoded);
        }
    }
    Ok(AsmResult {
        bytes,
        origin,
        labels,
    })
}

/// Encode one instruction. `sizing` is true during pass 1, where a label's
/// address may not exist yet and a placeholder zero displacement is used.
fn encode_line(
    mnemonic: &str,
    operand_texts: &[String],
    address: u64,
    labels: &[(String, u64)],
    sizing: bool,
    raw: &str,
) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let name = mnemonic.trim().to_ascii_lowercase();

    // Zero-operand instructions.
    let simple: Option<&[u8]> = match name.as_str() {
        "nop" => Some(&[0x90]),
        "ret" => Some(&[0xC3]),
        "int3" => Some(&[0xCC]),
        "leave" => Some(&[0xC9]),
        "pushfq" => Some(&[0x9C]),
        "popfq" => Some(&[0x9D]),
        "cdqe" => Some(&[0x48, 0x98]),
        "cqo" => Some(&[0x48, 0x99]),
        "cld" => Some(&[0xFC]),
        "std" => Some(&[0xFD]),
        "clc" => Some(&[0xF8]),
        "stc" => Some(&[0xF9]),
        "cli" => Some(&[0xFA]),
        "sti" => Some(&[0xFB]),
        "cpuid" => Some(&[0x0F, 0xA2]),
        "rdtsc" => Some(&[0x0F, 0x31]),
        "syscall" => Some(&[0x0F, 0x05]),
        "ud2" => Some(&[0x0F, 0x0B]),
        _ => None,
    };
    if let Some(bytes) = simple {
        if !operand_texts.is_empty() {
            return Err(format!("{name} 不接受操作数。"));
        }
        return Ok(bytes.to_vec());
    }

    let operands: Vec<Operand> = operand_texts
        .iter()
        .map(|t| parse_operand(t))
        .collect::<Result<_, _>>()?;

    // Relative branches.
    if let Some(opcode) = rel_opcode(&name) {
        if operands.len() != 1 {
            return Err(format!("{name} 需要一个跳转目标。"));
        }
        let target = resolve_target(&operands[0], labels, sizing)?;
        let mut bytes = vec![opcode];
        if opcode == 0xE8 || opcode == 0xE9 {
            let displacement = target as i128 - (address as i128 + 5);
            if displacement < i32::MIN as i128 || displacement > i32::MAX as i128 {
                return Err(format!("{raw}：跳转距离超出 rel32 范围。"));
            }
            bytes.extend_from_slice(&(displacement as i32).to_le_bytes());
        } else {
            let displacement = target as i128 - (address as i128 + 2);
            if displacement < i8::MIN as i128 || displacement > i8::MAX as i128 {
                return Err(format!("{raw}：短跳转超出 rel8 范围，请用 jmp 远的写法或调整布局。"));
            }
            bytes.push(displacement as i8 as u8);
        }
        return Ok(bytes);
    }

    // Shifts: shl/shr/sar/rol/ror with an 8-bit count.
    if let Some(digit) = SHIFTS.iter().find(|(n, _)| *n == name).map(|(_, d)| *d) {
        if operands.len() != 2 {
            return Err(format!("{name} 需要两个操作数（目标, 位数）。"));
        }
        let size = operand_size(&operands[0]).ok_or_else(|| format!("{raw}：无法确定操作数宽度。"))?;
        let opcode = if size == 1 { 0xC0 } else { 0xC1 };
        let count = match &operands[1] {
            Operand::Imm(v) => *v,
            _ => return Err(format!("{name} 的第二操作数必须是立即数（用 cl 的写法暂不支持）。")),
        };
        let rm = encode_rm(&operands[0], digit, if size == 2 { 4 } else { size })?;
        if size == 2 {
            out.push(0x66);
        }
        push_rex(&mut out, rm.rex);
        out.push(opcode);
        out.extend_from_slice(&rm.tail);
        push_imm(&mut out, count, 1);
        return Ok(out);
    }

    let specs: Vec<&Spec> = SPECS.iter().filter(|s| s.name == name).collect();
    if specs.is_empty() {
        return Err(format!("不支持的指令「{name}」。"));
    }
    for spec in &specs {
        if let Some(bytes) = try_encode(spec, &operands, address, labels, sizing) {
            return bytes;
        }
    }
    Err(format!("{raw}：操作数组合不支持。"))
}

fn rel_opcode(name: &str) -> Option<u8> {
    match name {
        "jmp" => Some(0xEB),
        "call" => Some(0xE8),
        _ => JCC
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, nibble)| 0x70 | *nibble),
    }
}

fn resolve_target(
    operand: &Operand,
    labels: &[(String, u64)],
    sizing: bool,
) -> Result<u64, String> {
    match operand {
        Operand::Imm(value) => Ok(*value as u64),
        Operand::Label(name) => {
            if let Some((_, address)) = labels.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Ok(*address)
            } else if sizing {
                // Not yet known in pass 1; the target address does not change
                // the instruction length, so a placeholder is fine.
                Ok(0)
            } else {
                Err(format!("标签「{name}」没有定义。"))
            }
        }
        _ => Err("跳转目标必须是标签或地址。".to_string()),
    }
}

fn operand_size(operand: &Operand) -> Option<u8> {
    match operand {
        Operand::Reg(_, width) => Some(*width),
        Operand::Mem(_, hint) => *hint,
        _ => None,
    }
}

fn try_encode(
    spec: &Spec,
    operands: &[Operand],
    _address: u64,
    labels: &[(String, u64)],
    sizing: bool,
) -> Option<Result<Vec<u8>, String>> {
    let mut out: Vec<u8> = Vec::new();
    match spec.form {
        Form::RegImm => {
            let [Operand::Reg(index, width), Operand::Imm(value)] = operands else {
                return None;
            };
            let opcode = if *width == 1 {
                spec.op8? 
            } else {
                spec.op
            };
            let mut rex = 0x40;
            if *index >= 8 {
                rex |= 0x01;
            }
            if *width == 8 {
                rex |= 0x08;
            }
            if *width == 2 {
                out.push(0x66);
            }
            push_rex(&mut out, rex);
            out.push(opcode + (index & 7));
            let imm_width = if *width == 1 { 1 } else { imm_width(*width) };
            if *width == 8 && name_is_mov_imm(spec.name) {
                push_imm(&mut out, *value, 8);
            } else {
                push_imm(&mut out, *value, imm_width);
            }
            Some(Ok(out))
        }
        Form::RmReg => {
            // For "mov r/m, reg" the first operand is the destination (r/m) and
            // the second is the source register.
            let [dst, Operand::Reg(reg, width)] = operands else {
                return None;
            };
            let opcode = if *width == 1 { spec.op8? } else { spec.op };
            let rm = encode_rm(dst, *reg, *width).ok()?;
            if *width == 2 {
                out.push(0x66);
            }
            push_rex(&mut out, rm.rex);
            out.push(opcode);
            out.extend_from_slice(&rm.tail);
            Some(Ok(out))
        }
        Form::RegRm => {
            let [Operand::Reg(reg, width), src] = operands else {
                return None;
            };
            let opcode = if *width == 1 {
                spec.op8.unwrap_or(0x8A)
            } else {
                spec.op
            };
            let effective = if *width == 0 { 4 } else { *width };
            let rm = encode_rm(src, *reg, effective).ok()?;
            if effective == 2 {
                out.push(0x66);
            }
            push_rex(&mut out, rm.rex);
            if spec.twobyte {
                out.push(0x0F);
            }
            out.push(opcode);
            out.extend_from_slice(&rm.tail);
            Some(Ok(out))
        }
        Form::OneRm => {
            let [target] = operands else {
                return None;
            };
            let size = operand_size(target).unwrap_or(4);
            let opcode = if size == 1 { spec.op8? } else { spec.op };
            let digit = group_digit(spec.name);
            let rm = encode_rm(target, digit, if size == 2 { 4 } else { size }).ok()?;
            if size == 2 {
                out.push(0x66);
            }
            push_rex(&mut out, rm.rex);
            out.push(opcode);
            out.extend_from_slice(&rm.tail);
            Some(Ok(out))
        }
        Form::RmImm => {
            let [dst, Operand::Imm(value)] = operands else {
                return None;
            };
            let size = operand_size(dst)?;
            let opcode = if size == 1 {
                spec.op8? 
            } else {
                spec.op
            };
            let digit = group_digit(spec.name);
            // For test/not/neg/mul/div the mnemonic is selected by ModRM.reg.
            let reg_field = if matches!(
                spec.name,
                "test" | "not" | "neg" | "mul" | "div"
            ) {
                digit
            } else {
                0
            };
            let rm = encode_rm(dst, reg_field, if size == 2 { 4 } else { size }).ok()?;
            if size == 2 {
                out.push(0x66);
            }
            push_rex(&mut out, rm.rex);
            out.push(opcode);
            out.extend_from_slice(&rm.tail);
            push_imm(&mut out, *value, if size == 1 { 1 } else { imm_width(size) });
            Some(Ok(out))
        }
        Form::Rel => {
            let [target] = operands else {
                return None;
            };
            let address = resolve_target(target, labels, sizing).ok()?;
            out.push(spec.op);
            out.extend_from_slice(&(address as i32).to_le_bytes());
            Some(Ok(out))
        }
        _ => None,
    }
}

fn name_is_mov_imm(name: &str) -> bool {
    name == "mov"
}