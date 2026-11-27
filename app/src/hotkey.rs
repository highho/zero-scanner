//! Keyboard shortcuts, following the Cheat Engine muscle memory.
//!
//! People who already use CE reach for these without thinking, so matching the
//! layout is worth more than inventing a cleaner one. Every shortcut is declared
//! once here, which means the help overlay and the handler cannot disagree.

/// One action a shortcut can trigger.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Start a scan with the current settings.
    Scan,
    /// Apply the next scan to the current results.
    NextScan,
    /// Undo the last narrowing.
    Undo,
    /// Toggle freeze on every entry.
    FreezeAll,
    /// Bring the process picker up.
    OpenProcess,
    /// Jump back to the address list.
    ShowList,
    /// Toggle the shortcut overlay.
    ToggleHelp,
    /// Clear the result list.
    ClearResults,
    /// Jump to memory view at the selected hit.
    ViewMemory,
}

/// A shortcut: a key plus whether it needs modifiers held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shortcut {
    pub action: Action,
    pub key: char,
    pub ctrl: bool,
    pub shift: bool,
    /// How the shortcut is printed in the overlay.
    pub label: &'static str,
    /// What it does, in the user's words.
    pub description: &'static str,
}

/// The shortcut table. Order is the order shown in the overlay.
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut { action: Action::Scan, key: '\r', ctrl: true, shift: false, label: "Ctrl+Enter", description: "用当前设置开始扫描" },
    Shortcut { action: Action::NextScan, key: '5', ctrl: false, shift: false, label: "F5", description: "再次扫描（缩小结果）" },
    Shortcut { action: Action::Undo, key: 'z', ctrl: true, shift: false, label: "Ctrl+Z", description: "撤销上一次扫描" },
    Shortcut { action: Action::FreezeAll, key: 'f', ctrl: true, shift: false, label: "Ctrl+F", description: "全部锁定 / 全部解锁" },
    Shortcut { action: Action::OpenProcess, key: 'o', ctrl: true, shift: false, label: "Ctrl+O", description: "选择进程" },
    Shortcut { action: Action::ClearResults, key: 'n', ctrl: true, shift: false, label: "Ctrl+N", description: "清空扫描结果" },
    Shortcut { action: Action::ShowList, key: '8', ctrl: false, shift: false, label: "F8", description: "回到地址列表" },
    Shortcut { action: Action::ViewMemory, key: 'm', ctrl: true, shift: false, label: "Ctrl+M", description: "在内存中查看选中的结果" },
    Shortcut { action: Action::ToggleHelp, key: '\'', ctrl: false, shift: false, label: "F1", description: "显示 / 隐藏这份快捷键" },
];

/// Which action, if any, a key press triggers.
///
/// `key` is the character the key produces; function keys arrive as their digit
/// character ('5' for F5), which is why the table stores them that way.
pub fn lookup(key: char, ctrl: bool, shift: bool) -> Option<Action> {
    let key = key.to_ascii_lowercase();
    SHORTCUTS
        .iter()
        .find(|s| s.key == key && s.ctrl == ctrl && s.shift == shift)
        .map(|s| s.action)
}

/// Map an egui key to the character this module uses.
///
/// Function keys are normalised to a digit ('F5' -> '5') so one table covers
/// both letter shortcuts and function keys.
pub fn key_char(key: egui::Key) -> Option<char> {
    use egui::Key;
    Some(match key {
        Key::Enter => '\r',
        Key::F1 => '\'',
        Key::F2 => '2',
        Key::F3 => '3',
        Key::F4 => '4',
        Key::F5 => '5',
        Key::F6 => '6',
        Key::F7 => '7',
        Key::F8 => '8',
        Key::F9 => '9',
        Key::F10 => '0',
        Key::A => 'a', Key::B => 'b', Key::C => 'c', Key::D => 'd', Key::E => 'e',
        Key::F => 'f', Key::G => 'g', Key::H => 'h', Key::I => 'i', Key::J => 'j',
        Key::K => 'k', Key::L => 'l', Key::M => 'm', Key::N => 'n', Key::O => 'o',
        Key::P => 'p', Key::Q => 'q', Key::R => 'r', Key::S => 's', Key::T => 't',
        Key::U => 'u', Key::V => 'v', Key::W => 'w', Key::X => 'x', Key::Y => 'y',
        Key::Z => 'z',
        _ => return None,
    })
}