//! Cheat Table (.ct) support.
//!
//! A .ct file is XML. The parts that matter for Zero are the cheat entries:
//! each one carries a description, a variable type, an address expression, and
//! optionally a frozen value. This module reads and writes those, keeping the
//! original document so that unrecognised sections are not destroyed on save.

use crate::model::VType;

/// One cheat entry from a table.
#[derive(Clone, Debug)]
pub struct CtEntry {
    pub description: String,
    pub vtype: VType,
    /// Address as written in the file: a hex offset, a module expression, or
    /// a pointer chain such as [[game.exe+1A2B]+18]+30.
    pub address: String,
    /// The frozen value, when the entry has one.
    pub value: Option<String>,
    /// True when the entry was active in the file (the checkbox was ticked).
    pub active: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CtTable {
    pub entries: Vec<CtEntry>,
    /// The raw document, so saving can preserve unknown sections.
    pub raw: String,
}

impl CtTable {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// A deliberate small XML reader. It only needs to walk well-formed CE tables,
// so it tracks elements and attributes and ignores everything else; it never
// tries to resolve namespaces or entities beyond the basic five.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children
            .iter()
            .filter(move |c| c.name.eq_ignore_ascii_case(name))
    }

    /// Trimmed text content of this element.
    pub fn text_trimmed(&self) -> String {
        self.text.trim().to_string()
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(semi) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => None,
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    if let Ok(code) = u32::from_str_radix(hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                } else if let Some(dec) = entity.strip_prefix('#') {
                    if let Ok(code) = dec.parse::<u32>() {
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                } else {
                    out.push_str(&tail[..=semi]);
                }
            }
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Parse an XML document into a tree. Comments, the declaration, and doctypes
/// are skipped; malformed markup returns an error naming the offset.
pub fn parse_xml(source: &str) -> Result<Element, String> {
    // Work on bytes throughout: the document is UTF-8 and markup is ASCII, so
    // every structural character sits on a byte boundary. Slicing by char index
    // here would cut multi-byte text (CJK descriptions) in half.
    let bytes = source.as_bytes();
    let mut pos = 0usize;
    let mut stack: Vec<Element> = vec![Element {
        name: "#document".to_string(),
        attributes: Vec::new(),
        children: Vec::new(),
        text: String::new(),
    }];
    while pos < bytes.len() {
        let ch = bytes[pos];
        if ch == b'<' {
            // Comments, declarations, processing instructions, doctypes.
            if source[pos..].starts_with("<!--") {
                match source[pos + 4..].find("-->") {
                    Some(end) => pos = pos + 4 + end + 3,
                    None => return Err("注释没有闭合。".to_string()),
                }
                continue;
            }
            if source[pos..].starts_with("<?") {
                match source[pos + 2..].find("?>") {
                    Some(end) => pos = pos + 2 + end + 2,
                    None => return Err("处理指令没有闭合。".to_string()),
                }
                continue;
            }
            if source[pos..].starts_with("<!") {
                match source[pos..].find('>') {
                    Some(end) => pos = pos + end + 1,
                    None => return Err("DOCTYPE 没有闭合。".to_string()),
                }
                continue;
            }
            if source[pos..].starts_with("</") {
                let Some(end) = source[pos..].find('>') else {
                    return Err("闭合标签没有结束。".to_string());
                };
                let name = source[pos + 2..pos + end].trim().to_string();
                if stack.len() < 2 {
                    return Err(format!("多余的闭合标签 </{name}>。"));
                }
                let element = stack.pop().unwrap();
                if !element.name.eq_ignore_ascii_case(&name) {
                    return Err(format!("标签不匹配：<{}> 与 </{name}>。", element.name));
                }
                stack.last_mut().unwrap().children.push(element);
                pos = pos + end + 1;
                continue;
            }
            // An opening tag.
            let Some(end) = source[pos..].find('>') else {
                return Err("开始标签没有结束。".to_string());
            };
            let inner = &source[pos + 1..pos + end];
            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/').trim();
            let mut parts = inner.splitn(2, char::is_whitespace);
            let name = parts.next().unwrap_or("").to_string();
            let mut attributes = Vec::new();
            if let Some(rest) = parts.next() {
                attributes = parse_attributes(rest);
            }
            let element = Element {
                name,
                attributes,
                children: Vec::new(),
                text: String::new(),
            };
            if self_closing {
                stack.last_mut().unwrap().children.push(element);
            } else {
                stack.push(element);
            }
            pos = pos + end + 1;
            continue;
        }
        // Character data.
        let start = pos;
        while pos < bytes.len() && bytes[pos] != b'<' {
            pos += 1;
        }
        let text = String::from_utf8_lossy(&bytes[start..pos]).to_string();
        if let Some(current) = stack.last_mut() {
            current.text.push_str(&unescape(&text));
        }
    }
    if stack.len() != 1 {
        let open = stack.last().map(|e| e.name.clone()).unwrap_or_default();
        return Err(format!("标签没有闭合：<{open}>。"));
    }
    Ok(stack.pop().unwrap())
}


/// Map a CE VariableType name onto our value types.
fn vtype_from_ct(name: &str) -> VType {
    match name.trim().to_ascii_lowercase().as_str() {
        "4 bytes" | "dword" => VType::I32,
        "8 bytes" | "qword" => VType::I64,
        "2 bytes" | "word" => VType::I16,
        "byte" => VType::I8,
        "float" => VType::F32,
        "double" => VType::F64,
        "string" => VType::Str,
        _ => VType::I32,
    }
}

/// The name CE writes for each of our value types.
pub fn vtype_to_ct(vtype: VType) -> &'static str {
    match vtype {
        VType::I8 => "Byte",
        VType::I16 => "2 Bytes",
        VType::I32 | VType::Auto => "4 Bytes",
        VType::I64 => "8 Bytes",
        VType::F32 => "Float",
        VType::F64 => "Double",
        VType::Str => "String",
    }
}

/// Parse a .ct document.
pub fn parse(source: &str) -> Result<CtTable, String> {
    let root = parse_xml(source)?;
    let cheat_table = root
        .child("CheatTable")
        .ok_or_else(|| "这不是 Cheat Table：缺少 CheatTable 根元素。".to_string())?;
    let mut entries = Vec::new();
    collect_entries(cheat_table, &mut entries);
    Ok(CtTable {
        entries,
        raw: source.to_string(),
    })
}

/// Cheat entries may nest inside groups, so walk the whole tree.
fn collect_entries(element: &Element, out: &mut Vec<CtEntry>) {
    for child in &element.children {
        if child.name.eq_ignore_ascii_case("CheatEntry") {
            if let Some(entry) = entry_from_element(child) {
                out.push(entry);
            }
        }
        collect_entries(child, out);
    }
}

fn entry_from_element(element: &Element) -> Option<CtEntry> {
    // CE wraps descriptions in double quotes; strip exactly one pair so a
    // write/read round-trip does not accumulate them.
    let description = element
        .child("Description")
        .map(|e| {
            let text = e.text_trimmed();
            let inner = text
                .strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(&text);
            inner.to_string()
        })
        .unwrap_or_default();
    let vtype_name = element
        .child("VariableType")
        .map(|e| e.text_trimmed())
        .unwrap_or_default();
    let vtype = vtype_from_ct(&vtype_name);
    let address = element
        .child("Address")
        .map(|e| e.text_trimmed())
        .unwrap_or_default();
    let value = element
        .child("Value")
        .map(|e| e.text_trimmed())
        .filter(|v| !v.is_empty());
    // An entry is "active" when it has a frozen value.
    let active = value.is_some();
    if description.is_empty() && address.is_empty() {
        return None;
    }
    Some(CtEntry {
        description,
        vtype,
        address,
        value,
        active,
    })
}

/// Resolve a CE address expression against a module list.
///
/// Supported forms:
///   deadbeef                 absolute hex address
///   game.exe+1A2B            module base + offset
///   [game.exe+1A2B]+18       one dereference
///   [[game.exe+1A2B]+18]+30  nested pointer chain
///
/// `read` performs one pointer read and returns None when it fails, so an
/// unreadable chain reports a clear error instead of a bogus address.
pub fn resolve_address(
    expression: &str,
    modules: &[(String, u64)],
    is_64bit: bool,
    read_pointer: &dyn Fn(u64) -> Option<u64>,
) -> Result<u64, String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("地址为空。".to_string());
    }
    let mut depth = 0usize;
    if expression.starts_with('[') {
        // Find the bracket that closes the outermost one.
        let mut close = None;
        for (index, ch) in expression.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            return Err(format!("方括号没有闭合：{expression}"));
        };
        // Inside may itself be a chain, so resolve it recursively first.
        let inner = &expression[1..close];
        let tail = &expression[close + 1..];
        let base = resolve_address(inner, modules, is_64bit, read_pointer)?;
        let Some(pointer) = read_pointer(base) else {
            return Err(format!("无法读取 {base:X} 处的指针。"));
        };
        if pointer == 0 {
            return Err(format!(
                "{base:X} 处的指针是空的（进程可能还没到那一步）。"
            ));
        }
        if !is_64bit && pointer > 0xFFFF_FFFF {
            return Err(format!("32 位进程里读到了越界的指针 {pointer:X}。"));
        }
        return apply_offsets(pointer, tail);
    }
    resolve_base(expression, modules)
}

/// A trailing "+18-4" run of signed offsets.
fn apply_offsets(mut address: u64, tail: &str) -> Result<u64, String> {
    let normalised = tail.trim().replace('-', "+-");
    for term in normalised.split('+') {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        let negative = term.starts_with('-');
        let body = term.trim_start_matches('-').trim();
        if body.is_empty() {
            continue;
        }
        let value = parse_hex_or_dec(body).ok_or_else(|| format!("偏移「{body}」无法解析。"))?;
        address = if negative {
            address.wrapping_sub(value)
        } else {
            address.wrapping_add(value)
        };
    }
    Ok(address)
}

/// Resolve "module.exe+OFFSET" or a bare hex address.
fn resolve_base(expression: &str, modules: &[(String, u64)]) -> Result<u64, String> {
    let text = expression.trim();
    // A module name may itself contain dots; match it as a prefix.
    for (name, base) in modules {
        if text.len() < name.len() {
            continue;
        }
        let (head, tail) = text.split_at(name.len());
        if !head.eq_ignore_ascii_case(name) {
            continue;
        }
        if tail.is_empty() {
            return Ok(*base);
        }
        if tail.starts_with('+') || tail.starts_with('-') {
            return apply_offsets(*base, tail);
        }
    }
    if let Some(value) = parse_hex_or_dec(text) {
        return Ok(value);
    }
    // A bare number followed by offsets, e.g. 7FF6A1B2+18.
    if let Some(pos) = text[1..].find(['+', '-']).map(|p| p + 1) {
        let (head, tail) = text.split_at(pos);
        if let Some(value) = parse_hex_or_dec(head) {
            return apply_offsets(value, tail);
        }
    }
    Err(format!("地址「{text}」无法解析。"))
}

/// Parse a number from a CE address expression.
///
/// Everything in a CE address is hexadecimal, including a bare run of digits.
/// "18" in "[game.exe+8C]+18" means 0x18; treating it as decimal silently
/// produced an address six bytes off.
fn parse_hex_or_dec(text: &str) -> Option<u64> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let digits = lower.strip_prefix("0x").unwrap_or(&lower);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

/// Serialise a table back to .ct XML.
pub fn to_xml(table: &CtTable) -> String {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str("<CheatTable>\n");
    xml.push_str("  <CheatEntries>\n");
    for entry in &table.entries {
        xml.push_str("    <CheatEntry>\n");
        xml.push_str(&format!(
            "      <Description>\"{}\"</Description>\n",
            escape(&entry.description)
        ));
        xml.push_str(&format!(
            "      <VariableType>{}</VariableType>\n",
            vtype_to_ct(entry.vtype)
        ));
        xml.push_str(&format!(
            "      <Address>{}</Address>\n",
            escape(&entry.address)
        ));
        if let Some(value) = &entry.value {
            xml.push_str(&format!("      <Value>{}</Value>\n", escape(value)));
        }
        xml.push_str("    </CheatEntry>\n");
    }
    xml.push_str("  </CheatEntries>\n");
    xml.push_str("</CheatTable>\n");
    xml
}
fn parse_attributes(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let name_start = i;
        while i < chars.len() && chars[i] != '=' && !chars[i].is_whitespace() {
            i += 1;
        }
        if name_start == i {
            break;
        }
        let name: String = chars[name_start..i].iter().collect();
        while i < chars.len() && chars[i] != '=' {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let quote = chars[i];
        if quote != '"' && quote != '\'' {
            continue;
        }
        i += 1;
        let value_start = i;
        while i < chars.len() && chars[i] != quote {
            i += 1;
        }
        let value: String = chars[value_start..i].iter().collect();
        i += 1;
        out.push((name, unescape(&value)));
    }
    out
}