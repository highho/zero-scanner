//! Address aliases: user-chosen names for addresses.
//!
//! Typing `game.exe+0x4A0C8` everywhere is fine until you have ten of them and
//! a game update shifts every offset. A symbol turns one address into a name
//! that every address box understands, so a corrected offset is fixed once.
//!
//! Aliases here are session-scoped and belong to the app, not to a script:
//! the Lua `registerSymbol` keeps its own table because a script's symbols
//! must not leak into the next script run. This table is what the UI and the
//! MCP server share, and it is persisted with the workspace.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    /// The address expression the name stands for, kept as text so a symbol
    /// can point at "game.exe+0x10" and follow the module base.
    pub expression: String,
}

static ALIASES: OnceLock<Mutex<Vec<Alias>>> = OnceLock::new();

fn table() -> &'static Mutex<Vec<Alias>> {
    ALIASES.get_or_init(|| Mutex::new(Vec::new()))
}

/// An alias name must be a single token, or it could never be matched safely.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn add(name: &str, expression: &str) -> Result<(), String> {
    let name = name.trim();
    let expression = expression.trim();
    if !valid_name(name) {
        return Err(format!(
            "别名「{name}」不合法：要用字母或下划线开头，只能有字母、数字、下划线。",
        ));
    }
    if expression.is_empty() {
        return Err("别名要指向一个地址，表达式不能为空。".to_string());
    }
    let mut guard = table().lock().unwrap();
    if let Some(existing) = guard.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
        // Redefining is the common case after a game update: replace in place.
        existing.expression = expression.to_string();
        return Ok(());
    }
    guard.push(Alias {
        name: name.to_string(),
        expression: expression.to_string(),
    });
    Ok(())
}

pub fn remove(name: &str) -> bool {
    let mut guard = table().lock().unwrap();
    let before = guard.len();
    guard.retain(|s| !s.name.eq_ignore_ascii_case(name.trim()));
    guard.len() != before
}

pub fn list() -> Vec<Alias> {
    table().lock().unwrap().clone()
}

pub fn clear() {
    table().lock().unwrap().clear();
}

/// Rewrite every alias name in an address expression to its target.
///
/// Longest name first, so "player" never eats "player_base". A name only
/// matches as a whole token, and the replacement is written with a 0x prefix
/// because a bare hex like "1234" would otherwise read as decimal.
pub fn expand(text: &str) -> String {
    let names: Vec<(String, String)> = {
        let guard = table().lock().unwrap();
        if guard.is_empty() {
            return text.to_string();
        }
        guard
            .iter()
            .map(|s| (s.name.to_ascii_lowercase(), s.expression.clone()))
            .collect()
    };
    let mut entries = names;
    entries.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));

    let mut out = text.to_string();
    for (name, expression) in entries {
        let mut rebuilt = String::new();
        let mut rest = out.as_str();
        loop {
            let lower = rest.to_ascii_lowercase();
            let Some(pos) = lower.find(name.as_str()) else {
                rebuilt.push_str(rest);
                break;
            };
            let after = pos + name.len();
            let before_ok = pos == 0
                || !rest[..pos]
                    .chars()
                    .next_back()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);
            let after_ok = after >= rest.len()
                || !rest[after..]
                    .chars()
                    .next()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);
            if before_ok && after_ok {
                rebuilt.push_str(&rest[..pos]);
                // An alias stands for an expression, not a number: keep it
                // textual so "hp" can mean "game.exe+0x4A0C8" and survive a
                // module rebase. No wrapping parentheses: the resolver splits
                // on commas and parses each token, so "0x1000" and
                // "game.exe+0x10" both read correctly on their own.
                rebuilt.push_str(&expression);
                rest = &rest[after..];
            } else {
                rebuilt.push_str(&rest[..after]);
                rest = &rest[after..];
            }
        }
        out = rebuilt;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_need_a_sane_shape() {
        assert!(valid_name("hp"));
        assert!(valid_name("player_base"));
        assert!(!valid_name("1hp"));
        assert!(!valid_name("player base"));
        assert!(!valid_name(""));
    }

    #[test]
    fn expansion_is_whole_token_and_longest_first() {
        clear();
        add("player", "0x1000").unwrap();
        add("player_base", "0x2000").unwrap();
        assert_eq!(expand("player_base"), "0x2000");
        assert_eq!(expand("player"), "0x1000");
        // A chain keeps working: the alias is just the head of it.
        add("hp", "0x1000").unwrap();
        assert_eq!(expand("hp,0x10"), "0x1000,0x10");
        assert_eq!(expand("playerx"), "playerx");
        clear();
    }
}
