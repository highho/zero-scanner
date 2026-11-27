use std::fmt;

/// Value widths understood by the scanner. `Auto` is resolved per hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum VType {
    Auto,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Str,
}

impl VType {
    pub const SELECTABLE: [VType; 8] = [
        VType::Auto,
        VType::I32,
        VType::I64,
        VType::F32,
        VType::F64,
        VType::I16,
        VType::I8,
        VType::Str,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VType::Auto => "自动",
            VType::I8 => "字节",
            VType::I16 => "2 字节",
            VType::I32 => "4 字节",
            VType::I64 => "8 字节",
            VType::F32 => "浮点",
            VType::F64 => "双精度",
            VType::Str => "字符串",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            VType::Auto => "AUTO",
            VType::I8 => "I8",
            VType::I16 => "I16",
            VType::I32 => "I32",
            VType::I64 => "I64",
            VType::F32 => "F32",
            VType::F64 => "F64",
            VType::Str => "TXT",
        }
    }

    pub fn size(self) -> usize {
        match self {
            VType::Auto => 0,
            VType::I8 | VType::Str => 1,
            VType::I16 => 2,
            VType::I32 | VType::F32 => 4,
            VType::I64 | VType::F64 => 8,
        }
    }

    pub fn concrete(self) -> &'static [VType] {
        match self {
            VType::Auto => &[VType::I32, VType::I64, VType::F32, VType::F64],
            VType::I8 => &[VType::I8],
            VType::I16 => &[VType::I16],
            VType::I32 => &[VType::I32],
            VType::I64 => &[VType::I64],
            VType::F32 => &[VType::F32],
            VType::F64 => &[VType::F64],
            VType::Str => &[VType::Str],
        }
    }

    pub fn parse(self, text: &str) -> Option<Value> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        match self {
            VType::Auto | VType::Str => Some(Value::Text(text.to_string())),
            VType::I8 => parse_int(text).map(|v| Value::Int(v as i8 as i64)),
            VType::I16 => parse_int(text).map(|v| Value::Int(v as i16 as i64)),
            VType::I32 => parse_int(text).map(|v| Value::Int(v as i32 as i64)),
            VType::I64 => parse_int(text).map(Value::Int),
            VType::F32 => text.parse::<f32>().ok().map(|v| Value::Float(v as f64)),
            VType::F64 => text.parse::<f64>().ok().map(Value::Float),
        }
    }

    pub fn decode(self, bytes: &[u8]) -> Option<Value> {
        match self {
            VType::Auto => None,
            VType::I8 => Some(Value::Int(i8::from_le_bytes([*bytes.first()?]) as i64)),
            VType::I16 => Some(Value::Int(i16::from_le_bytes(
                bytes.get(0..2)?.try_into().ok()?,
            ) as i64)),
            VType::I32 => Some(Value::Int(i32::from_le_bytes(
                bytes.get(0..4)?.try_into().ok()?,
            ) as i64)),
            VType::I64 => Some(Value::Int(i64::from_le_bytes(
                bytes.get(0..8)?.try_into().ok()?,
            ))),
            VType::F32 => Some(Value::Float(f32::from_le_bytes(
                bytes.get(0..4)?.try_into().ok()?,
            ) as f64)),
            VType::F64 => Some(Value::Float(f64::from_le_bytes(
                bytes.get(0..8)?.try_into().ok()?,
            ))),
            VType::Str => None,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, VType::F32 | VType::F64)
    }
}

fn parse_int(text: &str) -> Option<i64> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("0x") {
        i64::from_str_radix(&lower[2..], 16).ok()
    } else if lower.starts_with("-0x") {
        i64::from_str_radix(&lower[3..], 16).ok().map(|v| -v)
    } else {
        t.parse::<i64>().ok()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Text(String),
}

impl Value {
    pub fn display(&self) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Float(v) => format_float(*v),
            Value::Text(s) => {
                if s.chars().count() > 42 {
                    let cut: String = s.chars().take(40).collect();
                    format!("{cut}…")
                } else {
                    s.clone()
                }
            }
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Float(v) => Some(*v),
            Value::Text(_) => None,
        }
    }

    pub fn to_bytes(&self, ty: VType) -> Option<Vec<u8>> {
        match ty {
            VType::I8 => match self {
                Value::Int(v) => Some(vec![*v as i8 as u8]),
                _ => None,
            },
            VType::I16 => match self {
                Value::Int(v) => Some((*v as i16).to_le_bytes().to_vec()),
                _ => None,
            },
            VType::I32 => match self {
                Value::Int(v) => Some((*v as i32).to_le_bytes().to_vec()),
                _ => None,
            },
            VType::I64 => match self {
                Value::Int(v) => Some(v.to_le_bytes().to_vec()),
                _ => None,
            },
            VType::F32 => match self {
                Value::Float(v) => Some((*v as f32).to_le_bytes().to_vec()),
                _ => None,
            },
            VType::F64 => match self {
                Value::Float(v) => Some(v.to_le_bytes().to_vec()),
                _ => None,
            },
            VType::Str => match self {
                Value::Text(s) => Some(s.as_bytes().to_vec()),
                _ => None,
            },
            VType::Auto => None,
        }
    }

    pub fn approx_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => {
                let scale = a.abs().max(b.abs()).max(1.0);
                (a - b).abs() <= 0.0001 * scale
            }
            (Value::Float(a), Value::Int(b)) => (*a - *b as f64).abs() <= 0.0001,
            (Value::Int(a), Value::Float(b)) => (*a as f64 - *b).abs() <= 0.0001,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

pub fn format_float(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v != 0.0 && (v.abs() >= 1.0e9 || v.abs() < 0.0001) {
        let s = format!("{v:.5e}");
        return s;
    }
    let mut s = format!("{v:.4}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanType {
    Exact,
    Bigger,
    Smaller,
    Between,
    Unknown,
    Increased,
    Decreased,
    Changed,
    Unchanged,
}

impl ScanType {
    pub const PRIMARY: [ScanType; 5] = [
        ScanType::Exact,
        ScanType::Bigger,
        ScanType::Smaller,
        ScanType::Between,
        ScanType::Unknown,
    ];
    pub const RELATIVE: [ScanType; 4] = [
        ScanType::Increased,
        ScanType::Decreased,
        ScanType::Changed,
        ScanType::Unchanged,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ScanType::Exact => "精确值",
            ScanType::Bigger => "大于",
            ScanType::Smaller => "小于",
            ScanType::Between => "数值范围",
            ScanType::Unknown => "未知初值",
            ScanType::Increased => "数值增加",
            ScanType::Decreased => "数值减少",
            ScanType::Changed => "数值变化",
            ScanType::Unchanged => "数值未变",
        }
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, ScanType::Unknown | ScanType::Changed | ScanType::Unchanged)
    }

    pub fn needs_two(self) -> bool {
        matches!(self, ScanType::Between)
    }

    pub fn is_relative(self) -> bool {
        matches!(
            self,
            ScanType::Increased | ScanType::Decreased | ScanType::Changed | ScanType::Unchanged
        )
    }

    pub fn first_scan_ok(self) -> bool {
        !self.is_relative()
    }
}

#[derive(Clone, Debug)]
pub struct ScanHit {
    pub address: u64,
    pub vtype: VType,
    pub value: Value,
    pub previous: Option<Value>,
}

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: u64,
    pub size: u64,
    pub writable: bool,
    pub executable: bool,
}

impl Region {
    pub fn end(&self) -> u64 {
        self.base.saturating_add(self.size)
    }
    pub fn mb(&self) -> f64 {
        self.size as f64 / (1024.0 * 1024.0)
    }
}

pub fn parse_address(text: &str) -> Option<u64> {
    let t = text.trim().replace([' ', '_'], "");
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("0x") {
        u64::from_str_radix(&lower[2..], 16).ok()
    } else if lower.chars().any(|c| c.is_ascii_alphabetic()) {
        u64::from_str_radix(&lower, 16).ok()
    } else {
        lower.parse::<u64>().ok()
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub struct ValueTypeParseError;

pub fn parse_smart_value(text: &str) -> Option<(String, f64)> {
    let mut start = None;
    let mut end = 0usize;
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_digit() || (*c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            start = Some(i);
            end = i + 1;
            break;
        }
    }
    let s = start?;
    while end < chars.len()
        && (chars[end].is_ascii_digit() || matches!(chars[end], '.' | ',' | '_'))
    {
        end += 1;
    }
    let raw: String = chars[s..end]
        .iter()
        .filter(|c| **c != ',' && **c != '_')
        .collect();
    raw.parse::<f64>().ok().map(|v| (raw, v))
}
