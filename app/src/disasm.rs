//! A small x86-64 disassembler.
//!
//! Cheat Engine shows code as instructions. This module decodes the integer and
//! SSE subset that shows up in real game code well enough to give a readable
//! listing with correct instruction lengths, which is what an address view and an
//! eventual assembler need.
//!
//! Unknown encodings are reported as data bytes rather than guessed at, so the
//! listing never lies about how much space an instruction occupies.

/// What an instruction does to the instruction pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    None,
    /// A call to this absolute address.
    Call(u64),
    /// An unconditional jump to this absolute address.
    Jump(u64),
    /// A conditional jump; the mnemonic already says the condition.
    CondJump(u64),
    /// ret / syscall: leaves this block.
    Return,
}

#[derive(Clone, Debug)]
pub struct Insn {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub length: usize,
    pub text: String,
    pub flow: Flow,
}

impl Insn {
    pub fn hex(&self) -> String {
        self.bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

const REG8_REX: [&str; 16] = [
    "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b",
    "r13b", "r14b", "r15b",
];
const REG8_LEGACY: [&str; 8] = ["al", "cl", "dl", "bl", "ah", "ch", "dh", "bh"];
const REG16: [&str; 16] = [
    "ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "r8w", "r9w", "r10w", "r11w", "r12w", "r13w",
    "r14w", "r15w",
];
const REG32: [&str; 16] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d",
    "r13d", "r14d", "r15d",
];
const REG64: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];
const SREG: [&str; 8] = ["es", "cs", "ss", "ds", "fs", "gs", "?", "?"];

fn reg_name(index: u8, size: u8, has_rex: bool) -> String {
    let index = (index & 0x0F) as usize;
    match size {
        1 => {
            if has_rex || index >= 8 {
                REG8_REX[index].to_string()
            } else {
                REG8_LEGACY[index].to_string()
            }
        }
        2 => REG16[index].to_string(),
        4 => REG32[index].to_string(),
        8 => REG64[index].to_string(),
        16 => format!("xmm{index}"),
        _ => format!("r{index}"),
    }
}

struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
    address: u64,
    is64: bool,
    rex: u8,
    opsize16: bool,
    addr32: bool,
    rep: Option<&'static str>,
    seg: Option<&'static str>,
}

impl<'a> Decoder<'a> {
    fn new(buf: &'a [u8], address: u64, is64: bool) -> Self {
        Self {
            buf,
            pos: 0,
            address,
            is64,
            rex: 0,
            opsize16: false,
            addr32: false,
            rep: None,
            seg: None,
        }
    }

    fn taken(&self) -> usize {
        self.pos
    }

    fn has_rex(&self) -> bool {
        self.rex != 0
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.buf.get(self.pos + offset).copied()
    }

    fn byte(&mut self) -> Option<u8> {
        let value = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(value)
    }

    fn word(&mut self) -> Option<u64> {
        let low = self.byte()? as u64;
        let high = self.byte()? as u64;
        Some(low | (high << 8))
    }

    fn dword(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in 0..4 {
            value |= (self.byte()? as u64) << (shift * 8);
        }
        Some(value)
    }

    fn qword(&mut self) -> Option<u64> {
        let low = self.dword()?;
        let high = self.dword()?;
        Some(low | (high << 32))
    }

    /// An immediate of the given byte size, sign-extended.
    fn immediate(&mut self, size: usize) -> Option<i64> {
        match size {
            1 => Some(self.byte()? as i8 as i64),
            2 => Some(self.word()? as u16 as i16 as i64),
            4 => Some(self.dword()? as u32 as i32 as i64),
            8 => Some(self.qword()? as i64),
            _ => None,
        }
    }

    /// Size of a full-size operand, honoring REX.W and the 0x66 prefix.
    fn opsize(&self) -> u8 {
        if self.is64 && self.rex & 0x08 != 0 {
            8
        } else if self.opsize16 {
            2
        } else {
            4
        }
    }

    fn addr_size(&self) -> u8 {
        if self.is64 && !self.addr32 {
            8
        } else {
            4
        }
    }

    fn imm_size(&self, size: u8) -> usize {
        if size == 2 {
            2
        } else {
            4
        }
    }

    /// Decode ModRM plus any SIB and displacement. Returns the r/m operand text
    /// and the register index from the ModRM.reg field.
    fn modrm(&mut self, size: u8) -> Option<(String, u8)> {
        let modrm = self.byte()?;
        let mode = modrm >> 6;
        let reg = ((modrm >> 3) & 7) | ((self.rex & 0x04) << 1);
        let rm_low = modrm & 7;
        let mut rm = rm_low | ((self.rex & 0x01) << 3);
        if mode == 3 {
            // Register-direct form: the r/m field is itself a register.
            return Some((reg_name(rm, size, self.has_rex()), reg));
        }

        let mut base_name: Option<String> = None;
        let mut index_name: Option<String> = None;
        let mut scale = 1u64;
        let mut disp: i64 = 0;
        let mut have_disp = false;

        if rm_low == 4 {
            let sib = self.byte()?;
            scale = 1u64 << (sib >> 6);
            let index_low = (sib >> 3) & 7;
            let base_low = sib & 7;
            let index = index_low | ((self.rex & 0x02) << 2);
            if index_low != 4 || (self.rex & 0x02) != 0 {
                index_name = Some(reg_name(index, self.addr_size(), self.has_rex()));
            }
            if base_low == 5 && mode == 0 {
                disp = self.dword()? as u32 as i32 as i64;
                have_disp = true;
            } else {
                rm = base_low | ((self.rex & 0x01) << 3);
                base_name = Some(reg_name(rm, self.addr_size(), self.has_rex()));
            }
        } else if mode == 0 && rm_low == 5 && self.is64 && !self.addr32 {
            // RIP-relative: the displacement is resolved against the next
            // instruction, which is not known until the whole insn is decoded.
            disp = self.dword()? as u32 as i32 as i64;
            have_disp = true;
            base_name = Some("rip".to_string());
        } else {
            base_name = Some(reg_name(rm, self.addr_size(), self.has_rex()));
        }

        match mode {
            1 => {
                disp = self.byte()? as i8 as i64;
                have_disp = true;
            }
            2 => {
                disp = self.dword()? as u32 as i32 as i64;
                have_disp = true;
            }
            _ => {}
        }

        let mut parts: Vec<String> = Vec::new();
        if let Some(base) = base_name {
            parts.push(base);
        }
        if let Some(index) = index_name {
            if scale > 1 {
                parts.push(format!("{index}*{scale}"));
            } else {
                parts.push(index);
            }
        }
        if have_disp {
            if disp < 0 {
                parts.push(format!("-0x{:X}", -(disp as i128) as i64));
            } else {
                parts.push(format!("0x{disp:X}"));
            }
        }
        let mut text = String::new();
        if let Some(seg) = self.seg {
            text.push_str(seg);
            text.push(':');
        }
        text.push('[');
        text.push_str(&parts.join("+"));
        text.push(']');
        let text = match size {
            1 => format!("byte ptr {text}"),
            2 => format!("word ptr {text}"),
            8 => format!("qword ptr {text}"),
            16 => format!("xmmword ptr {text}"),
            _ => text,
        };
        Some((text, reg))
    }
}

/// Relative-branch target: the displacement is relative to the next instruction.
fn branch_target(address: u64, taken: usize, displacement: i64) -> u64 {
    (address as i128 + taken as i128 + displacement as i128) as u64
}

const ALU: [&str; 8] = ["add", "or", "adc", "sbb", "and", "sub", "xor", "cmp"];
const SHIFTS: [&str; 8] = ["rol", "ror", "rcl", "rcr", "shl", "shr", "sal", "sar"];
const CONDITIONS: [&str; 16] = [
    "o", "no", "b", "ae", "e", "ne", "be", "a", "s", "ns", "p", "np", "l", "ge", "le", "g",
];

/// Decode one instruction at the given address.
pub fn decode(buf: &[u8], address: u64, is64: bool) -> Option<Insn> {
    let mut d = Decoder::new(buf, address, is64);

    // Prefixes. REX must come last, so stop scanning once it is seen.
    loop {
        let next = d.peek(0)?;
        match next {
            0x66 => {
                d.opsize16 = true;
                d.pos += 1;
            }
            0x67 => {
                d.addr32 = true;
                d.pos += 1;
            }
            0xF0 => d.pos += 1,
            0xF2 => {
                d.rep = Some("repne ");
                d.pos += 1;
            }
            0xF3 => {
                d.rep = Some("rep ");
                d.pos += 1;
            }
            0x26 => {
                d.seg = Some("es");
                d.pos += 1;
            }
            0x2E => {
                d.seg = Some("cs");
                d.pos += 1;
            }
            0x36 => {
                d.seg = Some("ss");
                d.pos += 1;
            }
            0x3E => {
                d.seg = Some("ds");
                d.pos += 1;
            }
            0x64 => {
                d.seg = Some("fs");
                d.pos += 1;
            }
            0x65 => {
                d.seg = Some("gs");
                d.pos += 1;
            }
            0x40..=0x4F if is64 => {
                d.rex = next;
                d.pos += 1;
            }
            _ => break,
        }
    }

    let opcode = d.byte()?;
    let mut mnemonic = String::new();
    let mut operands: Vec<String> = Vec::new();
    let mut flow = Flow::None;

    match opcode {
        0x00..=0x3B if opcode & 7 <= 3 => {
            let alu = (opcode >> 3) as usize;
            mnemonic = ALU[alu].to_string();
            let byte = opcode & 1 == 0;
            let size = if byte { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            if opcode & 2 == 0 {
                operands.push(rm);
                operands.push(reg_name(reg, size, d.has_rex()));
            } else {
                operands.push(reg_name(reg, size, d.has_rex()));
                operands.push(rm);
            }
        }
        0x04 | 0x05 | 0x0C | 0x0D | 0x14 | 0x15 | 0x1C | 0x1D | 0x24 | 0x25 | 0x2C | 0x2D
        | 0x34 | 0x35 | 0x3C | 0x3D => {
            let alu = (opcode >> 3) as usize;
            mnemonic = ALU[alu].to_string();
            let byte = opcode & 1 == 0;
            let size = if byte { 1 } else { d.opsize() };
            operands.push(reg_name(0, size, d.has_rex()));
            let imm = if byte { 1 } else { d.imm_size(size) };
            operands.push(format!("0x{:X}", d.immediate(imm)?));
        }
        0x50..=0x57 => {
            mnemonic = "push".to_string();
            let reg = (opcode & 7) | ((d.rex & 0x01) << 3);
            operands.push(reg_name(reg, if is64 { 8 } else { d.opsize() }, d.has_rex()));
        }
        0x58..=0x5F => {
            mnemonic = "pop".to_string();
            let reg = (opcode & 7) | ((d.rex & 0x01) << 3);
            operands.push(reg_name(reg, if is64 { 8 } else { d.opsize() }, d.has_rex()));
        }
        0x63 => {
            mnemonic = "movsxd".to_string();
            let size = d.opsize();
            let (rm, reg) = d.modrm(4)?;
            operands.push(reg_name(reg, if size == 8 { 8 } else { 4 }, d.has_rex()));
            operands.push(rm);
        }
        0x68 => {
            mnemonic = "push".to_string();
            operands.push(format!("0x{:X}", d.immediate(d.imm_size(d.opsize()))?));
        }
        0x6A => {
            mnemonic = "push".to_string();
            operands.push(format!("0x{:X}", d.immediate(1)?));
        }
        0x69 | 0x6B => {
            mnemonic = "imul".to_string();
            let size = d.opsize();
            let (rm, reg) = d.modrm(size)?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
            let imm = if opcode == 0x6B { 1 } else { d.imm_size(size) };
            operands.push(format!("0x{:X}", d.immediate(imm)?));
        }
        0x70..=0x7F => {
            mnemonic = format!("j{}", CONDITIONS[(opcode & 0x0F) as usize]);
            let displacement = d.immediate(1)?;
            let target = branch_target(address, d.taken(), displacement);
            operands.push(format!("0x{target:X}"));
            flow = Flow::CondJump(target);
        }
        0x80 | 0x81 | 0x83 => {
            let size = if opcode == 0x80 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            mnemonic = ALU[(reg & 7) as usize].to_string();
            operands.push(rm);
            let imm = if opcode == 0x83 { 1 } else { d.imm_size(size) };
            operands.push(format!("0x{:X}", d.immediate(imm)?));
        }
        0x84 | 0x85 => {
            mnemonic = "test".to_string();
            let size = if opcode == 0x84 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            operands.push(rm);
            operands.push(reg_name(reg, size, d.has_rex()));
        }
        0x86 | 0x87 => {
            mnemonic = "xchg".to_string();
            let size = if opcode == 0x86 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            operands.push(rm);
            operands.push(reg_name(reg, size, d.has_rex()));
        }
        0x88 | 0x89 => {
            mnemonic = "mov".to_string();
            let size = if opcode == 0x88 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            operands.push(rm);
            operands.push(reg_name(reg, size, d.has_rex()));
        }
        0x8A | 0x8B => {
            mnemonic = "mov".to_string();
            let size = if opcode == 0x8A { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
        }
        0x8D => {
            mnemonic = "lea".to_string();
            let size = d.opsize();
            let (rm, reg) = d.modrm(size)?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
        }
        0x8E => {
            mnemonic = "mov".to_string();
            let (rm, reg) = d.modrm(2)?;
            operands.push(SREG[(reg & 7) as usize].to_string());
            operands.push(rm);
        }
        0x8F => {
            mnemonic = "pop".to_string();
            let (rm, _) = d.modrm(if is64 { 8 } else { 2 })?;
            operands.push(rm);
        }
        0x90 => mnemonic = "nop".to_string(),
        0x91..=0x97 => {
            mnemonic = "xchg".to_string();
            let size = d.opsize();
            let reg = (opcode & 7) | ((d.rex & 0x01) << 3);
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(reg_name(0, size, d.has_rex()));
        }
        0x98 => {
            mnemonic = match d.opsize() {
                8 => "cdqe",
                2 => "cbw",
                _ => "cwde",
            }
            .to_string();
        }
        0x99 => {
            mnemonic = match d.opsize() {
                8 => "cqo",
                2 => "cwd",
                _ => "cdq",
            }
            .to_string();
        }
        0x9C => mnemonic = "pushfq".to_string(),
        0x9D => mnemonic = "popfq".to_string(),
        0xA8 | 0xA9 => {
            mnemonic = "test".to_string();
            let size = if opcode == 0xA8 { 1 } else { d.opsize() };
            operands.push(reg_name(0, size, d.has_rex()));
            let imm = if size == 1 { 1 } else { d.imm_size(size) };
            operands.push(format!("0x{:X}", d.immediate(imm)?));
        }
        0xA4..=0xAF => {
            let suffix = match d.opsize() {
                8 => "q",
                4 => "d",
                2 => "w",
                _ => "b",
            };
            let base = match opcode {
                0xA4 | 0xA5 => "movs",
                0xA6 | 0xA7 => "cmps",
                0xAA | 0xAB => "stos",
                0xAC | 0xAD => "lods",
                _ => "scas",
            };
            mnemonic = format!("{base}{suffix}");
        }
        0xB0..=0xB7 => {
            mnemonic = "mov".to_string();
            let reg = (opcode & 7) | ((d.rex & 0x01) << 3);
            operands.push(reg_name(reg, 1, d.has_rex()));
            operands.push(format!("0x{:X}", d.immediate(1)?));
        }
        0xB8..=0xBF => {
            mnemonic = "mov".to_string();
            let size = d.opsize();
            let reg = (opcode & 7) | ((d.rex & 0x01) << 3);
            let imm = if size == 8 { 8 } else { d.imm_size(size) };
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(format!("0x{:X}", d.immediate(imm)?));
        }
        0xC0 | 0xC1 | 0xD0 | 0xD1 | 0xD2 | 0xD3 => {
            let size = if opcode & 1 == 0 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            mnemonic = SHIFTS[(reg & 7) as usize].to_string();
            operands.push(rm);
            match opcode {
                0xC0 | 0xC1 => operands.push(format!("0x{:X}", d.immediate(1)?)),
                0xD0 | 0xD1 => operands.push("1".to_string()),
                _ => operands.push("cl".to_string()),
            }
        }
        0xC2 => {
            mnemonic = "ret".to_string();
            operands.push(format!("0x{:X}", d.immediate(2)?));
        }
        0xC3 => {
            mnemonic = "ret".to_string();
            flow = Flow::Return;
        }
        0xC6 | 0xC7 => {
            mnemonic = "mov".to_string();
            let size = if opcode == 0xC6 { 1 } else { d.opsize() };
            let (rm, _) = d.modrm(size)?;
            operands.push(rm);
            let imm = if size == 1 { 1 } else { d.imm_size(size) };
            operands.push(format!("0x{:X}", d.immediate(imm)?));
        }
        0xC9 => mnemonic = "leave".to_string(),
        0xCC => mnemonic = "int3".to_string(),
        0xE8 => {
            mnemonic = "call".to_string();
            let displacement = d.immediate(4)?;
            let target = branch_target(address, d.taken(), displacement);
            operands.push(format!("0x{target:X}"));
            flow = Flow::Call(target);
        }
        0xE9 => {
            mnemonic = "jmp".to_string();
            let displacement = d.immediate(4)?;
            let target = branch_target(address, d.taken(), displacement);
            operands.push(format!("0x{target:X}"));
            flow = Flow::Jump(target);
        }
        0xEB => {
            mnemonic = "jmp".to_string();
            let displacement = d.immediate(1)?;
            let target = branch_target(address, d.taken(), displacement);
            operands.push(format!("0x{target:X}"));
            flow = Flow::Jump(target);
        }
        0xF6 | 0xF7 => {
            let size = if opcode == 0xF6 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            let names = ["test", "test", "not", "neg", "mul", "imul", "div", "idiv"];
            mnemonic = names[(reg & 7) as usize].to_string();
            operands.push(rm);
            if reg & 7 < 2 {
                let imm = if size == 1 { 1 } else { d.imm_size(size) };
                operands.push(format!("0x{:X}", d.immediate(imm)?));
            }
        }
        0xF8 => mnemonic = "clc".to_string(),
        0xF9 => mnemonic = "stc".to_string(),
        0xFA => mnemonic = "cli".to_string(),
        0xFB => mnemonic = "sti".to_string(),
        0xFC => mnemonic = "cld".to_string(),
        0xFD => mnemonic = "std".to_string(),
        0xFE | 0xFF => {
            let size = if opcode == 0xFE { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            mnemonic = match reg & 7 {
                0 => "inc",
                1 => "dec",
                2 => "call",
                3 => "call far",
                4 => "jmp",
                5 => "jmp far",
                6 => "push",
                _ => "?",
            }
            .to_string();
            operands.push(rm);
        }
        0x0F => {
            let second = d.byte()?;
            mnemonic = decode_0f(&mut d, second, &mut operands, address, &mut flow)?;
        }
        _ => {
            let text = format!("db {opcode:02X}");
            let length = d.taken();
            return Some(Insn {
                address,
                bytes: buf[..length].to_vec(),
                length,
                text,
                flow: Flow::None,
            });
        }
    }

    let length = d.taken();
    if length == 0 || length > buf.len() {
        return None;
    }
    let mut text = String::new();
    if let Some(rep) = d.rep {
        text.push_str(rep);
    }
    text.push_str(&mnemonic);
    if !operands.is_empty() {
        text.push(' ');
        text.push_str(&operands.join(", "));
    }
    Some(Insn {
        address,
        bytes: buf[..length].to_vec(),
        length,
        text,
        flow,
    })
}

/// The 0F escape map: two-byte integer opcodes plus SSE.
fn decode_0f(
    d: &mut Decoder,
    second: u8,
    operands: &mut Vec<String>,
    address: u64,
    flow: &mut Flow,
) -> Option<String> {
    match second {
        0x05 => Some("syscall".to_string()),
        0x0B => Some("ud2".to_string()),
        0x1F => {
            let (rm, _) = d.modrm(d.opsize())?;
            operands.push(rm);
            Some("nop".to_string())
        }
        0x30 => Some("wrmsr".to_string()),
        0x31 => Some("rdtsc".to_string()),
        0x32 => Some("rdmsr".to_string()),
        0xA2 => Some("cpuid".to_string()),
        0x40..=0x4F => {
            let size = d.opsize();
            let (rm, reg) = d.modrm(size)?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
            Some(format!("cmov{}", CONDITIONS[(second & 0x0F) as usize]))
        }
        0x80..=0x8F => {
            let displacement = d.immediate(4)?;
            let target = branch_target(address, d.taken(), displacement);
            operands.push(format!("0x{target:X}"));
            *flow = Flow::CondJump(target);
            Some(format!("j{}", CONDITIONS[(second & 0x0F) as usize]))
        }
        0x90..=0x9F => {
            let (rm, _) = d.modrm(1)?;
            operands.push(rm);
            Some(format!("set{}", CONDITIONS[(second & 0x0F) as usize]))
        }
        0xA3 | 0xAB | 0xB3 | 0xBB => {
            let name = match second {
                0xA3 => "bt",
                0xAB => "bts",
                0xB3 => "btr",
                _ => "btc",
            };
            let size = d.opsize();
            let (rm, reg) = d.modrm(size)?;
            operands.push(rm);
            operands.push(reg_name(reg, size, d.has_rex()));
            Some(name.to_string())
        }
        0xAF => {
            let size = d.opsize();
            let (rm, reg) = d.modrm(size)?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
            Some("imul".to_string())
        }
        0xB0 | 0xB1 => {
            let size = if second == 0xB0 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            operands.push(rm);
            operands.push(reg_name(reg, size, d.has_rex()));
            Some("cmpxchg".to_string())
        }
        0xB6 | 0xB7 => {
            let size = d.opsize();
            let (rm, reg) = d.modrm(if second == 0xB6 { 1 } else { 2 })?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
            Some("movzx".to_string())
        }
        0xBE | 0xBF => {
            let size = d.opsize();
            let (rm, reg) = d.modrm(if second == 0xBE { 1 } else { 2 })?;
            operands.push(reg_name(reg, size, d.has_rex()));
            operands.push(rm);
            Some("movsx".to_string())
        }
        0xC0 | 0xC1 => {
            let size = if second == 0xC0 { 1 } else { d.opsize() };
            let (rm, reg) = d.modrm(size)?;
            operands.push(rm);
            operands.push(reg_name(reg, size, d.has_rex()));
            Some("xadd".to_string())
        }
        0xC8..=0xCF => {
            let reg = (second & 7) | ((d.rex & 0x01) << 3);
            operands.push(reg_name(reg, 4, d.has_rex()));
            Some("bswap".to_string())
        }
        _ => decode_sse(d, second, operands),
    }
}

/// SSE and SSE2 mnemonics. The prefix picks the column: none / 66 / F3 / F2.
fn decode_sse(d: &mut Decoder, second: u8, operands: &mut Vec<String>) -> Option<String> {
    let names: [&str; 4] = match second {
        0x10 | 0x11 => ["movups", "movupd", "movss", "movsd"],
        0x12 => ["movlps", "movlpd", "movsldup", "movddup"],
        0x13 => ["movlps", "movlpd", "movlps", "movlps"],
        0x14 => ["unpcklps", "unpcklpd", "unpcklps", "unpcklpd"],
        0x15 => ["unpckhps", "unpckhpd", "unpckhps", "unpckhpd"],
        0x16 => ["movhps", "movhpd", "movshdup", "movhps"],
        0x17 => ["movhps", "movhpd", "movhps", "movhps"],
        0x28 | 0x29 => ["movaps", "movapd", "movaps", "movapd"],
        0x2A => ["cvtpi2ps", "cvtpi2pd", "cvtsi2ss", "cvtsi2sd"],
        0x2B => ["movntps", "movntpd", "movntps", "movntpd"],
        0x2C => ["cvttps2pi", "cvttpd2pi", "cvttss2si", "cvttsd2si"],
        0x2D => ["cvtps2pi", "cvtpd2pi", "cvtss2si", "cvtsd2si"],
        0x2E => ["ucomiss", "ucomisd", "ucomiss", "ucomisd"],
        0x2F => ["comiss", "comisd", "comiss", "comisd"],
        0x50 => ["movmskps", "movmskpd", "movmskps", "movmskpd"],
        0x51 => ["sqrtps", "sqrtpd", "sqrtss", "sqrtsd"],
        0x52 => ["rsqrtps", "rsqrtps", "rsqrtss", "rsqrtps"],
        0x53 => ["rcpps", "rcpps", "rcpss", "rcpps"],
        0x54 => ["andps", "andpd", "andps", "andpd"],
        0x55 => ["andnps", "andnpd", "andnps", "andnpd"],
        0x56 => ["orps", "orpd", "orps", "orpd"],
        0x57 => ["xorps", "xorpd", "xorps", "xorpd"],
        0x58 => ["addps", "addpd", "addss", "addsd"],
        0x59 => ["mulps", "mulpd", "mulss", "mulsd"],
        0x5A => ["cvtps2pd", "cvtpd2ps", "cvtss2sd", "cvtsd2ss"],
        0x5B => ["cvtdq2ps", "cvtps2dq", "cvttps2dq", "cvtdq2ps"],
        0x5C => ["subps", "subpd", "subss", "subsd"],
        0x5D => ["minps", "minpd", "minss", "minsd"],
        0x5E => ["divps", "divpd", "divss", "divsd"],
        0x5F => ["maxps", "maxpd", "maxss", "maxsd"],
        0x6E => ["movd", "movd", "movd", "movd"],
        0x6F => ["movq", "movdqa", "movdqu", "movdqa"],
        0x70 => ["pshufw", "pshufd", "pshufhw", "pshuflw"],
        0x74 => ["pcmpeqb", "pcmpeqb", "pcmpeqb", "pcmpeqb"],
        0x75 => ["pcmpeqw", "pcmpeqw", "pcmpeqw", "pcmpeqw"],
        0x76 => ["pcmpeqd", "pcmpeqd", "pcmpeqd", "pcmpeqd"],
        0x7E => ["movd", "movd", "movq", "movd"],
        0x7F => ["movq", "movdqa", "movdqu", "movdqa"],
        0xC2 => ["cmpps", "cmppd", "cmpss", "cmpsd"],
        0xC6 => ["shufps", "shufpd", "shufps", "shufpd"],
        _ => {
            operands.clear();
            return Some(format!("db 0F{second:02X}"));
        }
    };
    let column = match d.rep {
        Some("rep ") => 2,
        Some("repne ") => 3,
        _ => {
            if d.opsize16 {
                1
            } else {
                0
            }
        }
    };
    let mnemonic = names[column].to_string();
    let has_immediate = matches!(second, 0x70 | 0xC2 | 0xC6);
    // Conversions from a general-purpose register take its real width.
    let gp_source = matches!(second, 0x2A | 0x2C | 0x2D) && column >= 2;
    let gp_move = matches!(second, 0x6E | 0x7E) && column == 0;
    if gp_source || gp_move {
        let size = d.opsize();
        let (rm, reg) = d.modrm(size)?;
        operands.push(reg_name(reg, 16, d.has_rex()));
        operands.push(rm);
    } else {
        let (rm, reg) = d.modrm(16)?;
        operands.push(reg_name(reg, 16, d.has_rex()));
        operands.push(rm);
    }
    if has_immediate {
        operands.push(format!("0x{:X}", d.immediate(1)?));
    }
    Some(mnemonic)
}

/// Decode a window of code into a listing of at most max_insns instructions.
pub fn disassemble(bytes: &[u8], address: u64, is64: bool, max_insns: usize) -> Vec<Insn> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() && out.len() < max_insns {
        let Some(insn) = decode(&bytes[offset..], address + offset as u64, is64) else {
            break;
        };
        if insn.length == 0 {
            break;
        }
        offset += insn.length;
        out.push(insn);
    }
    out
}

