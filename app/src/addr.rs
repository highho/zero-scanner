//! Address expressions: the way Cheat Engine users actually type addresses.
//!
//! A raw hex address is fine for a one-off, but game addresses are almost
//! always "something plus an offset": the base of a module plus the offset of
//! the variable inside it. That stays true across game updates, which is why
//! every address input in Zero accepts expressions:
//!
//!   game.exe+0x4A0C8        module base plus an offset
//!   game.exe-0x1000         negative offsets are fine
//!   UnityPlayer.dll+0x1F440 a DLL base plus an offset
//!   0x7FF6A1B2C3D4          a plain hex address still works
//!   12345                   and so does a plain decimal
//!   game.exe+0x10,0x14,0x0  pointer chain: read each offset from memory
//!
//! The first token is a module name or an absolute address; the rest of the
//! chain walks pointers. Everything after a comma goes through read_pointer,
//! which needs the process handle, so resolution has two levels: the purely
//! textual module+offset form is always available, and the full pointer-chain
//! form needs a process.

use crate::winmem::{ModuleInfo, ProcessHandle};

/// Parse and resolve an address expression to a concrete address.
/// modules supplies module bases; when a handle is given the comma-chain
/// offsets are walked as pointers.
pub fn resolve(
    handle: Option<&ProcessHandle>,
    modules: &[ModuleInfo],
    text: &str,
) -> Option<u64> {
    // Aliases are user-chosen names; expand them before parsing so a name
    // works in every address box that already accepts expressions.
    let expanded = crate::aliases::expand(text);
    let text = expanded.trim();
    if text.is_empty() {
        return None;
    }
    // Split into the head and the pointer-chain steps.
    let mut parts = text.split(',');
    let head = parts.next()?.trim();
    if head.is_empty() {
        return None;
    }
    let steps: Vec<u64> = parts
        .map(|part| parse_offset(part.trim()))
        .collect::<Option<Vec<u64>>>()?;

    let mut address = resolve_head(handle, modules, head)?;
    if steps.is_empty() {
        return Some(address);
    }
    // The comma-chain first needs a process to dereference.
    let process = handle?;
    for offset in steps {
        address = process.read_pointer(address)?.checked_add(offset)?;
    }
    Some(address)
}

/// Resolve just the head: a module name with an optional +offset, or a bare
/// number. Without a handle the offset form still works (module bases come
/// from the module list alone).
pub fn resolve_head(
    handle: Option<&ProcessHandle>,
    modules: &[ModuleInfo],
    head: &str,
) -> Option<u64> {
    // The head is "name+off", "name-off", or a plain number. Split on the
    // first + or - that follows a module-ish name.
    if let Some(plus) = head.find('+') {
        let name = &head[..plus];
        let offset = parse_offset(&head[plus + 1..])?;
        let base = module_base(handle, modules, name)?;
        return base.checked_add(offset);
    }
    if let Some(minus) = head.find('-') {
        // A bare negative number like -12345 is not useful; only accept a
        // minus when the left side is a module name.
        let name = &head[..minus];
        let may_be_module = name.contains('.') || name.contains('_');
        if may_be_module {
            let offset = parse_offset(&head[minus + 1..])?;
            let base = module_base(handle, modules, name)?;
            return base.checked_sub(offset);
        }
        // Fall through: the whole thing is a negative number.
        return crate::model::parse_address(head);
    }
    // No offset: a plain address or a bare module base.
    if looks_like_module(head) {
        return module_base(handle, modules, head);
    }
    crate::model::parse_address(head)
}

fn module_base(handle: Option<&ProcessHandle>, modules: &[ModuleInfo], name: &str) -> Option<u64> {
    let needle = name.trim_matches(['"', '\'']).to_lowercase();
    let from_list = modules
        .iter()
        .find(|m| m.name.to_lowercase() == needle)
        .map(|m| m.base);
    if from_list.is_some() {
        return from_list;
    }
    let handle = handle?;
    handle
        .modules()
        .iter()
        .find(|m| m.name.to_lowercase() == needle)
        .map(|m| m.base)
}

/// Is this token plausibly a module name (has letters and a dot/underscore)?
pub fn looks_like_module(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains('.') || lower.contains('_')
}

/// Parse an offset token: 0x hex, decimal, or a signed number.
pub fn parse_offset(text: &str) -> Option<u64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(rest) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(rest, 16).ok();
    }
    if let Some(rest) = t.strip_prefix('-') {
        let value = if let Some(hex) = rest.strip_prefix("0x") {
            i64::from_str_radix(hex, 16).ok()?
        } else {
            rest.parse::<i64>().ok()?
        };
        return (0u64).checked_sub(value as u64);
    }
    t.parse::<u64>().ok()
}

/// A short human-readable annotation for an address: game.dll+0x1234 when it
/// is inside a known module, otherwise just hex. Used by the dissect view and
/// the address list to make an opaque number legible again.
pub fn annotate(modules: &[ModuleInfo], address: u64) -> String {
    for module in modules {
        if module.contains(address) {
            let offset = module.offset_of(address);
            return format!("{}+0x{:X}", module.name, offset);
        }
    }
    format!("0x{address:X}")
}

/// Try to turn an existing annotation back into an address, so toggling
/// between raw hex and module+offset is lossless.
pub fn from_annotation(
    handle: Option<&ProcessHandle>,
    modules: &[ModuleInfo],
    text: &str,
) -> Option<u64> {
    resolve(handle, modules, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::winmem::ModuleInfo;

    fn modules() -> Vec<ModuleInfo> {
        vec![ModuleInfo { name: "game.exe".to_string(), base: 0x1000, size: 0x10000 }]
    }

    #[test]
    fn bare_hex() {
        assert_eq!(resolve(None, &modules(), "0x1234"), Some(0x1234));
    }
    #[test]
    fn module_offset() {
        assert_eq!(resolve(None, &modules(), "game.exe+0x4A0C8"), Some(0x4B0C8));
    }
    #[test]
    fn module_negative() {
        assert_eq!(resolve(None, &modules(), "game.exe-0x1000"), Some(0x0));
    }
    #[test]
    fn decimal() {
        assert_eq!(resolve(None, &modules(), "12345"), Some(12345));
    }
    #[test]
    fn offset_parse() {
        assert_eq!(parse_offset("0x10"), Some(16));
        assert_eq!(parse_offset("16"), Some(16));
        assert_eq!(parse_offset("-0x10"), Some(0));
        assert_eq!(parse_offset("-16"), Some(0));
    }
}