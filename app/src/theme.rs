//! Theme engine: one runtime palette that recolours the whole window.
//!
//! Zero used to hard-code a single dark palette as constants. Every widget read
//! those constants, so adding a theme would have meant editing hundreds of call
//! sites. The palette now lives behind accessors (bg(), text(), accent()...),
//! which means switching a theme is a single index change and every existing
//! widget follows on the next frame.
//!
//! Three layers of theming, deliberately in this order:
//!
//! 1. Built-in presets below, always available.
//! 2. Theme packs: JSON files in %APPDATA%/Zero/themes. A pack picks a base
//!    preset and overrides any subset of colours, so a pack stays short and
//!    survives preset tweaks.
//! 3. Plugin-provided packs: a plugin manifest may point at a theme file, which
//!    is installed through the same parser as layer 2. Plugins are a reserved
//!    extension surface; theme packs are the first capability that uses it.
//!
//! A broken pack never prevents startup: it is skipped and its reason is kept
//! for the settings page, because a typo in a colour should not cost the user
//! their session.

use egui::{
    epaint::{Mesh, Shadow, Vertex, WHITE_UV},
    Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Response,
    Rounding, Sense, Stroke, TextStyle, Ui, Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once, RwLock};

/// Everything the UI needs to draw in one colour scheme.
#[derive(Clone, Debug)]
pub struct Palette {
    pub id: String,
    pub name: String,
    pub hint: String,
    /// Where this palette came from: a built-in preset or a pack file path.
    pub origin: String,
    pub dark: bool,
    pub bg: Color32,
    pub bg_top: Color32,
    pub panel: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_hi: Color32,
    pub card_soft: Color32,
    pub border: Color32,
    pub border_soft: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_2: Color32,
    pub accent_dark: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub success: Color32,
    /// Ambient glow behind the workspace, and the logo gradient.
    pub glow_a: Color32,
    pub glow_b: Color32,
    pub logo_a: Color32,
    pub logo_b: Color32,
    /// Text colour drawn on top of the accent gradient.
    pub on_accent: Color32,
}

impl Palette {
    fn blank(id: &str, name: &str, hint: &str, dark: bool) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            hint: hint.to_string(),
            origin: "内置".to_string(),
            dark,
            bg: Color32::from_rgb(9, 13, 21),
            bg_top: Color32::from_rgb(14, 20, 33),
            panel: Color32::from_rgb(13, 18, 29),
            sidebar: Color32::from_rgb(10, 15, 25),
            card: Color32::from_rgb(18, 25, 39),
            card_hi: Color32::from_rgb(23, 32, 50),
            card_soft: Color32::from_rgb(15, 21, 34),
            border: Color32::from_rgb(35, 47, 68),
            border_soft: Color32::from_rgb(27, 37, 55),
            text: Color32::from_rgb(232, 238, 249),
            muted: Color32::from_rgb(144, 158, 182),
            faint: Color32::from_rgb(102, 116, 140),
            accent: Color32::from_rgb(92, 140, 255),
            accent_2: Color32::from_rgb(52, 211, 190),
            accent_dark: Color32::from_rgb(43, 88, 200),
            warn: Color32::from_rgb(246, 184, 78),
            danger: Color32::from_rgb(255, 106, 112),
            success: Color32::from_rgb(74, 222, 158),
            glow_a: Color32::from_rgb(73, 117, 235),
            glow_b: Color32::from_rgb(40, 190, 176),
            logo_a: Color32::from_rgb(88, 136, 255),
            logo_b: Color32::from_rgb(43, 205, 183),
            on_accent: Color32::from_rgb(8, 18, 28),
        }
    }
}

fn hex(text: &str) -> Option<Color32> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    match t.len() {
        6 => Some(Color32::from_rgb(
            byte(&t[0..2])?,
            byte(&t[2..4])?,
            byte(&t[4..6])?,
        )),
        8 => Some(Color32::from_rgba_unmultiplied(
            byte(&t[0..2])?,
            byte(&t[2..4])?,
            byte(&t[4..6])?,
            byte(&t[6..8])?,
        )),
        _ => None,
    }
}

/// Set one named colour. Shared by built-in presets and pack files so a pack
/// can override exactly the same fields a preset declares.
fn set_color(palette: &mut Palette, key: &str, value: Color32) -> bool {
    let slot = match key {
        "bg" => &mut palette.bg,
        "bg_top" => &mut palette.bg_top,
        "panel" => &mut palette.panel,
        "sidebar" => &mut palette.sidebar,
        "card" => &mut palette.card,
        "card_hi" => &mut palette.card_hi,
        "card_soft" => &mut palette.card_soft,
        "border" => &mut palette.border,
        "border_soft" => &mut palette.border_soft,
        "text" => &mut palette.text,
        "muted" => &mut palette.muted,
        "faint" => &mut palette.faint,
        "accent" => &mut palette.accent,
        "accent_2" => &mut palette.accent_2,
        "accent_dark" => &mut palette.accent_dark,
        "warn" => &mut palette.warn,
        "danger" => &mut palette.danger,
        "success" => &mut palette.success,
        "glow_a" => &mut palette.glow_a,
        "glow_b" => &mut palette.glow_b,
        "logo_a" => &mut palette.logo_a,
        "logo_b" => &mut palette.logo_b,
        "on_accent" => &mut palette.on_accent,
        _ => return false,
    };
    *slot = value;
    true
}

fn preset(id: &str, name: &str, hint: &str, dark: bool, colors: &[(&str, &str)]) -> Palette {
    let mut palette = Palette::blank(id, name, hint, dark);
    for (key, value) in colors {
        match hex(value) {
            Some(color) => {
                set_color(&mut palette, key, color);
            }
            None => eprintln!("内置主题 {id} 的颜色 {key}={value} 无法解析"),
        }
    }
    palette
}

/// The built-in presets. Order is the order shown in settings.
pub fn presets() -> Vec<Palette> {
    vec![
        preset("midnight", "深空蓝", "默认工作台配色", true, &[
            ("bg", "#090d15"), ("bg_top", "#0e1421"), ("panel", "#0d121d"),
            ("sidebar", "#0a0f19"), ("card", "#121927"), ("card_hi", "#172032"),
            ("card_soft", "#0f1522"), ("border", "#232f44"), ("border_soft", "#1b2537"),
            ("text", "#e8eef9"), ("muted", "#909eb6"), ("faint", "#66748c"),
            ("accent", "#5c8cff"), ("accent_2", "#34d3be"), ("accent_dark", "#2b58c8"),
            ("warn", "#f6b84e"), ("danger", "#ff6a70"), ("success", "#4ade9e"),
            ("glow_a", "#4975eb"), ("glow_b", "#28beb0"),
            ("logo_a", "#5888ff"), ("logo_b", "#2bcdb7"), ("on_accent", "#08121c"),
        ]),
        preset("neon", "霓虹赛博", "洋红与青色，夜色霓虹", true, &[
            ("bg", "#070810"), ("bg_top", "#0b0d1a"), ("panel", "#0b0d18"),
            ("sidebar", "#080910"), ("card", "#10121f"), ("card_hi", "#171a2e"),
            ("card_soft", "#0c0e19"), ("border", "#2a2450"), ("border_soft", "#1c1838"),
            ("text", "#f1ecff"), ("muted", "#a89cc8"), ("faint", "#6f6590"),
            ("accent", "#ff4fd8"), ("accent_2", "#45f0ff"), ("accent_dark", "#b81fa6"),
            ("warn", "#ffd166"), ("danger", "#ff5370"), ("success", "#3ddc97"),
            ("glow_a", "#ff4fd8"), ("glow_b", "#45f0ff"),
            ("logo_a", "#ff4fd8"), ("logo_b", "#45f0ff"), ("on_accent", "#12001a"),
        ]),
        preset("esports", "电竞紫", "紫罗兰与荧光绿", true, &[
            ("bg", "#0a0714"), ("bg_top", "#100b1f"), ("panel", "#0f0a1c"),
            ("sidebar", "#0a0714"), ("card", "#161028"), ("card_hi", "#1e1636"),
            ("card_soft", "#100a1e"), ("border", "#33245c"), ("border_soft", "#241a42"),
            ("text", "#f2edff"), ("muted", "#b0a3d0"), ("faint", "#7c7099"),
            ("accent", "#a06bff"), ("accent_2", "#b6ff4d"), ("accent_dark", "#7a3fe0"),
            ("warn", "#ffc857"), ("danger", "#ff5c8a"), ("success", "#6ee7a8"),
            ("glow_a", "#a06bff"), ("glow_b", "#b6ff4d"),
            ("logo_a", "#a06bff"), ("logo_b", "#b6ff4d"), ("on_accent", "#10051f"),
        ]),
        preset("tactical", "战术绿", "军用终端，绿与沙金", true, &[
            ("bg", "#080d0a"), ("bg_top", "#0c1410"), ("panel", "#0b120e"),
            ("sidebar", "#070c09"), ("card", "#111b15"), ("card_hi", "#17251d"),
            ("card_soft", "#0c130f"), ("border", "#23392c"), ("border_soft", "#1a2a20"),
            ("text", "#e8f6ec"), ("muted", "#9fb8a8"), ("faint", "#6c8474"),
            ("accent", "#6ee7a8"), ("accent_2", "#e3c65a"), ("accent_dark", "#2fae72"),
            ("warn", "#e3c65a"), ("danger", "#ff6b6b"), ("success", "#6ee7a8"),
            ("glow_a", "#6ee7a8"), ("glow_b", "#e3c65a"),
            ("logo_a", "#6ee7a8"), ("logo_b", "#e3c65a"), ("on_accent", "#05130c"),
        ]),
        preset("ember", "暗夜琥珀", "暖橙与玫粉，壁炉光", true, &[
            ("bg", "#100c09"), ("bg_top", "#17110c"), ("panel", "#140f0b"),
            ("sidebar", "#0f0b08"), ("card", "#1d1510"), ("card_hi", "#271c15"),
            ("card_soft", "#16100c"), ("border", "#3d2a1e"), ("border_soft", "#2b1e15"),
            ("text", "#fdf1e3"), ("muted", "#c4a98f"), ("faint", "#8d745c"),
            ("accent", "#ff9f45"), ("accent_2", "#ff7ab6"), ("accent_dark", "#d97706"),
            ("warn", "#ffd166"), ("danger", "#ff5c5c"), ("success", "#7ddc9b"),
            ("glow_a", "#ff9f45"), ("glow_b", "#ff7ab6"),
            ("logo_a", "#ff9f45"), ("logo_b", "#ff7ab6"), ("on_accent", "#1b0d02"),
        ]),
        preset("ice", "极光冰蓝", "冰蓝与紫罗兰，极地夜空", true, &[
            ("bg", "#06101a"), ("bg_top", "#0a1826"), ("panel", "#08141f"),
            ("sidebar", "#060f17"), ("card", "#0e1f2e"), ("card_hi", "#142a3d"),
            ("card_soft", "#091723"), ("border", "#1e3b52"), ("border_soft", "#14293a"),
            ("text", "#e8f6ff"), ("muted", "#93b4c8"), ("faint", "#63849a"),
            ("accent", "#5ec8ff"), ("accent_2", "#a78bfa"), ("accent_dark", "#2b8fd6"),
            ("warn", "#ffc857"), ("danger", "#ff6b8a"), ("success", "#4ade80"),
            ("glow_a", "#5ec8ff"), ("glow_b", "#a78bfa"),
            ("logo_a", "#5ec8ff"), ("logo_b", "#a78bfa"), ("on_accent", "#031018"),
        ]),
        preset("graphite", "石墨灰", "近乎无彩，专注数据", true, &[
            ("bg", "#0b0c0e"), ("bg_top", "#111316"), ("panel", "#0e1013"),
            ("sidebar", "#0a0b0d"), ("card", "#16191d"), ("card_hi", "#1e2227"),
            ("card_soft", "#101316"), ("border", "#2c3238"), ("border_soft", "#1f2429"),
            ("text", "#f2f4f7"), ("muted", "#a7b0ba"), ("faint", "#6f7880"),
            ("accent", "#d9dee6"), ("accent_2", "#8fa3b8"), ("accent_dark", "#aeb7c2"),
            ("warn", "#e8b464"), ("danger", "#f0616d"), ("success", "#7fd1a0"),
            ("glow_a", "#d9dee6"), ("glow_b", "#8fa3b8"),
            ("logo_a", "#c9d1da"), ("logo_b", "#8fa3b8"), ("on_accent", "#101317"),
        ]),
        preset("daylight", "晨曦", "亮色工作台，适合白天与投屏", false, &[
            ("bg", "#f2f5fa"), ("bg_top", "#ffffff"), ("panel", "#ffffff"),
            ("sidebar", "#e9eef7"), ("card", "#ffffff"), ("card_hi", "#eef3fb"),
            ("card_soft", "#f6f8fd"), ("border", "#d5deec"), ("border_soft", "#e4eaf5"),
            ("text", "#101828"), ("muted", "#56657d"), ("faint", "#8494a8"),
            ("accent", "#2563eb"), ("accent_2", "#0d9488"), ("accent_dark", "#1d4ed8"),
            ("warn", "#b45309"), ("danger", "#dc2626"), ("success", "#0f9d58"),
            ("glow_a", "#2563eb"), ("glow_b", "#0d9488"),
            ("logo_a", "#2563eb"), ("logo_b", "#0d9488"), ("on_accent", "#ffffff"),
        ]),
    ]
}

static THEMES: RwLock<Vec<&'static Palette>> = RwLock::new(Vec::new());
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PLUGIN_PACKS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static LOADED: Once = Once::new();

/// Where user theme packs live. Created on first use so the folder exists by
/// the time the settings page offers to open it.
pub fn themes_dir() -> PathBuf {
    let dir = crate::prefs::data_dir().join("themes");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn install(palettes: Vec<Palette>) -> Vec<&'static Palette> {
    palettes
        .into_iter()
        .map(|palette| &*Box::leak(Box::new(palette)))
        .collect()
}

/// Read one pack file and merge it over its base preset.
///
/// Errors are user-facing Chinese text because they surface in the settings
/// page next to the file name, and a bare JSON parser message does not tell
/// anyone which field is wrong.
pub fn load_pack_file(path: &Path, presets: &[Palette]) -> Result<Palette, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} 不是合法 JSON：{e}", path.display()))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "pack".to_string());
    let id = match json.get("id").and_then(|v| v.as_str()) {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => format!("pack-{stem}"),
    };
    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&stem)
        .to_string();
    let base_id = json.get("base").and_then(|v| v.as_str()).unwrap_or("midnight");
    let base = presets.iter().find(|p| p.id == base_id).ok_or_else(|| {
        let known: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        format!("未知的基础主题 base=\"{base_id}\"（可用：{}）", known.join(" / "))
    })?;
    let mut palette = base.clone();
    palette.id = id;
    palette.name = name;
    palette.origin = path.display().to_string();
    palette.hint = json
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("主题包")
        .to_string();
    if let Some(dark) = json.get("dark").and_then(|v| v.as_bool()) {
        palette.dark = dark;
    }
    let colors = json
        .get("colors")
        .or_else(|| json.get("palette"))
        .and_then(|v| v.as_object())
        .ok_or_else(|| "缺少 colors 对象".to_string())?;
    for (key, value) in colors {
        let text = value
            .as_str()
            .ok_or_else(|| format!("颜色 {key} 必须是 \"#rrggbb\" 字符串"))?;
        let color = hex(text).ok_or_else(|| format!("颜色 {key}=\"{text}\" 无法解析"))?;
        if !set_color(&mut palette, key, color) {
            return Err(format!("未知颜色字段 {key}（可用字段见 docs/themes.md）"));
        }
    }
    Ok(palette)
}

fn collect_packs(presets: &[Palette], out: &mut Vec<Palette>, errors: &mut Vec<String>) {
    let dir = themes_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for file in files {
        match load_pack_file(&file, presets) {
            Ok(palette) => out.push(palette),
            Err(error) => errors.push(error),
        }
    }
    let plugin_packs = PLUGIN_PACKS.lock().map(|p| p.clone()).unwrap_or_default();
    for file in plugin_packs {
        match load_pack_file(&file, presets) {
            Ok(mut palette) => {
                palette.origin = format!("插件 · {}", palette.origin);
                out.push(palette);
            }
            Err(error) => errors.push(error),
        }
    }
}

fn build() {
    let presets = presets();
    let mut all = presets.clone();
    let mut errors = Vec::new();
    collect_packs(&presets, &mut all, &mut errors);
    // Stable, unique ids: a pack that reuses a preset id gets a suffix instead
    // of silently shadowing it.
    let mut seen: Vec<String> = Vec::new();
    for palette in all.iter_mut() {
        let base_id = palette.id.clone();
        let mut candidate = base_id.clone();
        let mut n = 2;
        while seen.contains(&candidate) {
            candidate = format!("{base_id}-{n}");
            n += 1;
        }
        palette.id = candidate.clone();
        seen.push(candidate);
    }
    let installed = install(all);
    if let Ok(mut list) = THEMES.write() {
        *list = installed;
    }
    if let Ok(mut slot) = ERRORS.lock() {
        *slot = errors;
    }
}

fn ensure() {
    LOADED.call_once(build);
}

fn apply_saved_choice(id: &str) {
    if let Ok(list) = THEMES.read() {
        if let Some(index) = list.iter().position(|p| p.id == id) {
            ACTIVE.store(index, Ordering::Relaxed);
        }
    }
}

fn rebuild_keep_active() {
    let active_id = current().id.clone();
    build();
    apply_saved_choice(&active_id);
}

/// Install a theme pack a plugin points at. Kept separate from the themes
/// folder so a plugin can ship colours without asking the user to copy files.
pub fn register_plugin_pack(path: PathBuf) -> Result<(), String> {
    ensure();
    if let Ok(mut list) = PLUGIN_PACKS.lock() {
        if !list.contains(&path) {
            list.push(path.clone());
        }
    }
    let presets = presets();
    load_pack_file(&path, &presets)?;
    rebuild_keep_active();
    Ok(())
}

/// Re-read the themes folder and every plugin pack. The active theme is kept
/// when it still exists, otherwise the default preset is used.
pub fn reload() -> usize {
    ensure();
    rebuild_keep_active();
    themes().len()
}

pub fn themes() -> Vec<&'static Palette> {
    ensure();
    THEMES.read().map(|list| list.clone()).unwrap_or_default()
}

/// Why a pack was skipped. Shown in settings; cleared on a successful reload.
pub fn pack_errors() -> Vec<String> {
    ensure();
    ERRORS.lock().map(|e| e.clone()).unwrap_or_default()
}

static FALLBACK: Palette = Palette {
    id: String::new(),
    name: String::new(),
    hint: String::new(),
    origin: String::new(),
    dark: true,
    bg: Color32::from_rgb(9, 13, 21),
    bg_top: Color32::from_rgb(14, 20, 33),
    panel: Color32::from_rgb(13, 18, 29),
    sidebar: Color32::from_rgb(10, 15, 25),
    card: Color32::from_rgb(18, 25, 39),
    card_hi: Color32::from_rgb(23, 32, 50),
    card_soft: Color32::from_rgb(15, 21, 34),
    border: Color32::from_rgb(35, 47, 68),
    border_soft: Color32::from_rgb(27, 37, 55),
    text: Color32::from_rgb(232, 238, 249),
    muted: Color32::from_rgb(144, 158, 182),
    faint: Color32::from_rgb(102, 116, 140),
    accent: Color32::from_rgb(92, 140, 255),
    accent_2: Color32::from_rgb(52, 211, 190),
    accent_dark: Color32::from_rgb(43, 88, 200),
    warn: Color32::from_rgb(246, 184, 78),
    danger: Color32::from_rgb(255, 106, 112),
    success: Color32::from_rgb(74, 222, 158),
    glow_a: Color32::from_rgb(73, 117, 235),
    glow_b: Color32::from_rgb(40, 190, 176),
    logo_a: Color32::from_rgb(88, 136, 255),
    logo_b: Color32::from_rgb(43, 205, 183),
    on_accent: Color32::from_rgb(8, 18, 28),
};

pub fn current() -> &'static Palette {
    ensure();
    let index = ACTIVE.load(Ordering::Relaxed);
    if let Ok(list) = THEMES.read() {
        if let Some(palette) = list.get(index) {
            return palette;
        }
        if let Some(palette) = list.first() {
            return palette;
        }
    }
    // Unreachable after ensure(), but a process must not panic over a colour.
    &FALLBACK
}

pub fn active_id() -> String {
    current().id.clone()
}

/// Switch the active theme by id and remember it. Returns false when the id is
/// unknown (a pack can disappear between sessions).
pub fn set_theme(id: &str) -> bool {
    ensure();
    let found = THEMES
        .read()
        .map(|list| list.iter().position(|p| p.id == id))
        .unwrap_or(None);
    if let Some(index) = found {
        ACTIVE.store(index, Ordering::Relaxed);
        crate::prefs::set_theme(id);
        true
    } else {
        false
    }
}

pub fn init() {
    ensure();
    let saved = crate::prefs::load().theme;
    if !saved.is_empty() {
        apply_saved_choice(&saved);
    }
}

// ---------------------------------------------------------------------------
// Colour accessors. Every widget reads these instead of constants, which is
// what makes a live theme switch possible.
// ---------------------------------------------------------------------------

pub fn bg() -> Color32 { current().bg }
pub fn bg_top() -> Color32 { current().bg_top }
pub fn panel() -> Color32 { current().panel }
pub fn sidebar() -> Color32 { current().sidebar }
pub fn card() -> Color32 { current().card }
pub fn card_hi() -> Color32 { current().card_hi }
pub fn card_soft() -> Color32 { current().card_soft }
pub fn border() -> Color32 { current().border }
pub fn border_soft() -> Color32 { current().border_soft }
pub fn text() -> Color32 { current().text }
pub fn muted() -> Color32 { current().muted }
pub fn faint() -> Color32 { current().faint }
pub fn accent() -> Color32 { current().accent }
pub fn accent_2() -> Color32 { current().accent_2 }
pub fn accent_dark() -> Color32 { current().accent_dark }
pub fn warn() -> Color32 { current().warn }
pub fn danger() -> Color32 { current().danger }
pub fn success() -> Color32 { current().success }
pub fn on_accent() -> Color32 { current().on_accent }

/// A translucent tint that reads correctly on both light and dark palettes.
pub fn veil(alpha: u8) -> Color32 {
    if current().dark {
        Color32::from_white_alpha(alpha)
    } else {
        Color32::from_black_alpha(alpha)
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        "C:\\Windows\\Fonts\\Deng.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = FontDefinitions::default();
            let data = FontData::from_owned(bytes);
            fonts.font_data.insert("cjk".to_string(), data);
            if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
                list.push("cjk".to_string());
            }
            if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
                list.push("cjk".to_string());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

pub fn apply_style(ctx: &egui::Context) {
    let p = current();
    let mut style = (*ctx.style()).clone();
    style.visuals = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.panel_fill = p.bg;
    style.visuals.window_fill = p.card;
    style.visuals.extreme_bg_color = if p.dark { p.bg } else { p.card_soft };
    style.visuals.faint_bg_color = p.card_soft;
    style.visuals.code_bg_color = p.card_soft;
    style.visuals.override_text_color = Some(p.text);
    style.visuals.hyperlink_color = p.accent;
    style.visuals.selection.bg_fill = p.accent.gamma_multiply(0.38);
    style.visuals.selection.stroke = Stroke::new(1.0, p.accent);
    style.visuals.window_rounding = Rounding::same(16.0);
    style.visuals.menu_rounding = Rounding::same(10.0);
    style.visuals.window_shadow = Shadow {
        offset: Vec2::new(0.0, 18.0),
        blur: 48.0,
        spread: 0.0,
        color: Color32::from_black_alpha(if p.dark { 130 } else { 40 }),
    };
    style.visuals.popup_shadow = Shadow {
        offset: Vec2::new(0.0, 8.0),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(if p.dark { 110 } else { 30 }),
    };
    style.visuals.widgets.noninteractive.bg_fill = p.card_soft;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border_soft);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    style.visuals.widgets.noninteractive.rounding = Rounding::same(10.0);
    style.visuals.widgets.inactive.bg_fill = p.card;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    style.visuals.widgets.inactive.rounding = Rounding::same(9.0);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    style.visuals.widgets.hovered.bg_fill = p.card_hi;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.7));
    style.visuals.widgets.hovered.rounding = Rounding::same(9.0);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    style.visuals.widgets.active.bg_fill = p.card_hi;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    style.visuals.widgets.active.rounding = Rounding::same(9.0);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    style.visuals.widgets.open.bg_fill = p.card_hi;
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, p.border);
    style.visuals.widgets.open.rounding = Rounding::same(9.0);
    style.visuals.window_stroke = Stroke::new(1.0, p.border);
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.spacing.interact_size = Vec2::new(36.0, 34.0);
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = true;

    style.text_styles.insert(TextStyle::Heading, FontId::proportional(27.0));
    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
    ctx.set_style(style);
}

pub fn card_frame() -> Frame {
    Frame::none()
        .fill(card())
        .stroke(Stroke::new(1.0, border()))
        .rounding(Rounding::same(16.0))
        .inner_margin(Margin::same(16.0))
        .shadow(Shadow {
            offset: Vec2::new(0.0, 8.0),
            blur: 28.0,
            spread: 0.0,
            color: Color32::from_black_alpha(if current().dark { 60 } else { 18 }),
        })
}

pub fn soft_card() -> Frame {
    Frame::none()
        .fill(card_soft())
        .stroke(Stroke::new(1.0, border_soft()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
}

pub fn divider(ui: &mut Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, Rounding::ZERO, border_soft());
}

pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(11.5)
            .color(faint())
            .extra_letter_spacing(1.0),
    );
}

pub fn paint_gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    c1: Color32,
    c2: Color32,
    rounding: Rounding,
) {
    let mut mesh = Mesh::default();
    let idx = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        pos: rect.left_top(),
        uv: WHITE_UV,
        color: c1,
    });
    mesh.vertices.push(Vertex {
        pos: rect.right_top(),
        uv: WHITE_UV,
        color: c2,
    });
    mesh.vertices.push(Vertex {
        pos: rect.right_bottom(),
        uv: WHITE_UV,
        color: c2,
    });
    mesh.vertices.push(Vertex {
        pos: rect.left_bottom(),
        uv: WHITE_UV,
        color: c1,
    });
    mesh.add_triangle(idx, idx + 1, idx + 2);
    mesh.add_triangle(idx, idx + 2, idx + 3);
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(rect, rounding, Stroke::new(0.5, veil(20)));
}

pub fn gradient_button(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> Response {
    let height = 42.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter();
    if !enabled {
        painter.rect_filled(rect, Rounding::same(12.0), card_soft());
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(14.5),
            faint(),
        );
        return response;
    }
    let hovered = response.hovered();
    let base = if hovered {
        rect.translate(Vec2::new(0.0, -1.0))
    } else {
        rect
    };
    painter.rect_filled(
        base.translate(Vec2::new(0.0, 3.0)),
        Rounding::same(12.0),
        accent().gamma_multiply(0.35),
    );
    let c1 = if hovered { accent_2() } else { accent() };
    let c2 = if hovered { accent() } else { accent_2() };
    paint_gradient_rect(&painter, base, c1, c2, Rounding::same(12.0));
    if hovered {
        painter.rect_filled(base, Rounding::same(12.0), veil(10));
    }
    painter.text(
        base.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(15.0),
        on_accent(),
    );
    response
}

pub fn outline_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        FontId::proportional(14.0),
        if enabled { text() } else { faint() },
    );
    let width = (galley.size().x + 32.0).max(88.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() && enabled {
        card_hi()
    } else {
        card_soft()
    };
    painter.rect_filled(rect, Rounding::same(10.0), fill);
    painter.rect_stroke(
        rect,
        Rounding::same(10.0),
        Stroke::new(1.0, if response.hovered() && enabled { accent().gamma_multiply(0.65) } else { border() }),
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if enabled { text() } else { faint() },
    );
    response
}

pub fn pill(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        FontId::proportional(13.0),
        if selected { on_accent() } else { muted() },
    );
    let width = galley.size().x + 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, Rounding::same(9.0), accent().gamma_multiply(0.22));
        painter.rect_stroke(
            rect,
            Rounding::same(9.0),
            Stroke::new(1.0, accent().gamma_multiply(0.75)),
        );
    } else {
        painter.rect_filled(rect, Rounding::same(9.0), card_soft());
        if response.hovered() {
            painter.rect_stroke(rect, Rounding::same(9.0), Stroke::new(1.0, border()));
        }
    }
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if selected { text() } else { muted() },
    );
    response
}

pub fn tag(ui: &mut Ui, label: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), FontId::proportional(11.0), color);
    let size = Vec2::new(galley.size().x + 14.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(6.0), color.gamma_multiply(0.14));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
}

pub fn icon_button(ui: &mut Ui, kind: IconKind) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, Rounding::same(9.0), veil(8));
    }
    paint_icon(ui.painter(), rect.shrink(9.0), kind, muted());
    response
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Scan,
    List,
    Memory,
    Assistant,
    Practice,
    Settings,
    Search,
    Plus,
    Refresh,
    Close,
    Check,
    Lock,
    Bolt,
    ChevronRight,
    Trash,
    Pencil,
    Play,
    Stop,
    Pointer,
    Aob,
    Symbol,
    Freeze,
    ArrowLeft,
    ArrowRight,
    Structure,
    Gamepad,
}

pub fn paint_icon(painter: &egui::Painter, rect: egui::Rect, kind: IconKind, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    match kind {
        IconKind::Scan | IconKind::Search => {
            painter.circle_stroke(c + Vec2::new(-r * 0.16, -r * 0.16), r * 0.58, stroke);
            painter.line_segment(
                [
                    c + Vec2::new(r * 0.26, r * 0.26),
                    c + Vec2::new(r * 0.9, r * 0.9),
                ],
                stroke,
            );
        }
        IconKind::List => {
            for i in [-0.5f32, 0.0, 0.5] {
                painter.circle_filled(c + Vec2::new(-r * 0.75, r * i), 1.5, color);
                painter.line_segment(
                    [
                        c + Vec2::new(-r * 0.35, r * i),
                        c + Vec2::new(r * 0.85, r * i),
                    ],
                    Stroke::new(1.4, color),
                );
            }
        }
        IconKind::Memory => {
            let body = egui::Rect::from_center_size(c, Vec2::splat(r * 1.15));
            painter.rect_stroke(body, Rounding::same(2.0), Stroke::new(1.3, color));
            for i in 0..3 {
                let y = body.top() + body.height() * (i as f32 + 1.0) / 4.0;
                painter.line_segment(
                    [
                        egui::pos2(body.left() - r * 0.28, y),
                        egui::pos2(body.left(), y),
                    ],
                    Stroke::new(1.2, color),
                );
                painter.line_segment(
                    [
                        egui::pos2(body.right(), y),
                        egui::pos2(body.right() + r * 0.28, y),
                    ],
                    Stroke::new(1.2, color),
                );
            }
            painter.rect_filled(
                egui::Rect::from_center_size(c, Vec2::splat(r * 0.42)),
                Rounding::same(1.0),
                color,
            );
        }
        IconKind::Assistant => {
            let points = vec![
                c + Vec2::new(0.0, -r),
                c + Vec2::new(r * 0.24, -r * 0.24),
                c + Vec2::new(r, 0.0),
                c + Vec2::new(r * 0.24, r * 0.24),
                c + Vec2::new(0.0, r),
                c + Vec2::new(-r * 0.24, r * 0.24),
                c + Vec2::new(-r, 0.0),
                c + Vec2::new(-r * 0.24, -r * 0.24),
            ];
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        }
        IconKind::Bolt => {
            let points = vec![
                c + Vec2::new(-r * 0.2, -r),
                c + Vec2::new(r * 0.55, -r * 0.15),
                c + Vec2::new(r * 0.1, -r * 0.12),
                c + Vec2::new(r * 0.25, r),
                c + Vec2::new(-r * 0.55, r * 0.02),
                c + Vec2::new(-r * 0.08, 0.0),
            ];
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        }
        IconKind::Freeze => {
            painter.circle_stroke(c, r * 0.9, Stroke::new(1.3, color));
            for i in 0..3 {
                let angle = std::f32::consts::PI * i as f32 / 3.0;
                let dir = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment([c - dir * r * 0.75, c + dir * r * 0.75], Stroke::new(1.3, color));
            }
        }
        IconKind::Pointer => {
            for i in 0..2 {
                let x = -r * 0.9 + i as f32 * r * 0.9;
                painter.line_segment(
                    [c + Vec2::new(x, 0.0), c + Vec2::new(x + r * 0.9, 0.0)],
                    Stroke::new(1.6, color),
                );
                painter.circle_filled(c + Vec2::new(x, 0.0), r * 0.24, color);
            }
            painter.circle_filled(c + Vec2::new(r * 0.9, 0.0), r * 0.24, color);
        }
        IconKind::Aob => {
            let cell = r * 0.46;
            let y = c.y - r * 0.06;
            for i in 0..3 {
                let x = c.x - r * 0.92 + i as f32 * r * 0.92;
                painter.rect_stroke(
                    egui::Rect::from_center_size(egui::pos2(x, y), Vec2::splat(cell)),
                    Rounding::same(2.0),
                    Stroke::new(1.2, color.gamma_multiply(0.5)),
                );
            }
            painter.rect_filled(
                egui::Rect::from_center_size(egui::pos2(c.x, y), Vec2::splat(cell)),
                Rounding::same(2.0),
                color,
            );
        }
        IconKind::Symbol => {
            let body = egui::Rect::from_center_size(
                c + Vec2::new(r * 0.12, 0.0),
                Vec2::new(r * 1.45, r * 1.0),
            );
            painter.rect_stroke(body, Rounding::same(r * 0.24), Stroke::new(1.4, color));
            painter.circle_filled(c + Vec2::new(-r * 0.36, 0.0), r * 0.16, color);
            painter.line_segment(
                [
                    c + Vec2::new(r * 0.08, -r * 0.2),
                    c + Vec2::new(r * 0.5, -r * 0.2),
                ],
                Stroke::new(1.2, color),
            );
        }
        IconKind::Structure => {
            for i in 0..3 {
                let y = c.y - r * 0.55 + i as f32 * r * 0.55;
                painter.rect_stroke(
                    egui::Rect::from_min_size(
                        egui::pos2(c.x - r * 0.9, y - r * 0.18),
                        Vec2::new(r * 1.8, r * 0.36),
                    ),
                    Rounding::same(2.0),
                    Stroke::new(1.1, color),
                );
                painter.circle_filled(egui::pos2(c.x - r * 0.6, y), r * 0.08, color);
            }
        }
        IconKind::Gamepad => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, Vec2::new(r * 1.9, r * 1.05)),
                Rounding::same(r * 0.45),
                Stroke::new(1.3, color),
            );
            painter.line_segment(
                [c + Vec2::new(-r * 0.62, -r * 0.18), c + Vec2::new(-r * 0.62, r * 0.18)],
                Stroke::new(1.3, color),
            );
            painter.line_segment(
                [c + Vec2::new(-r * 0.8, 0.0), c + Vec2::new(-r * 0.44, 0.0)],
                Stroke::new(1.3, color),
            );
            painter.circle_filled(c + Vec2::new(r * 0.5, -r * 0.18), r * 0.1, color);
            painter.circle_filled(c + Vec2::new(r * 0.7, r * 0.12), r * 0.1, color);
        }
        IconKind::Practice => {
            painter.circle_stroke(c, r * 0.9, Stroke::new(1.3, color));
            painter.circle_stroke(c, r * 0.48, Stroke::new(1.3, color));
            painter.circle_filled(c, r * 0.15, color);
        }
        IconKind::Settings => {
            for (i, offset) in [-0.55f32, 0.0, 0.55].iter().enumerate() {
                let y = c.y + r * offset;
                painter.line_segment(
                    [egui::pos2(c.x - r * 0.9, y), egui::pos2(c.x + r * 0.9, y)],
                    Stroke::new(1.4, color),
                );
                let knob = match i {
                    0 => -r * 0.25,
                    1 => r * 0.35,
                    _ => -r * 0.05,
                };
                painter.circle_filled(egui::pos2(c.x + knob, y), r * 0.18, color);
            }
        }
        IconKind::Plus => {
            painter.line_segment(
                [c + Vec2::new(-r * 0.7, 0.0), c + Vec2::new(r * 0.7, 0.0)],
                stroke,
            );
            painter.line_segment(
                [c + Vec2::new(0.0, -r * 0.7), c + Vec2::new(0.0, r * 0.7)],
                stroke,
            );
        }
        IconKind::Refresh => {
            painter.add(egui::Shape::line(
                (0..18)
                    .map(|i| {
                        let a = -0.6 + i as f32 * 0.3;
                        c + Vec2::new(a.cos(), a.sin()) * r * 0.7
                    })
                    .collect(),
                Stroke::new(1.5, color),
            ));
            let tip = c + Vec2::new(0.75f32.cos() * r * 0.7, 0.75f32.sin() * r * 0.7);
            painter.line_segment([tip, tip + Vec2::new(-r * 0.3, -r * 0.05)], stroke);
            painter.line_segment([tip, tip + Vec2::new(r * 0.05, r * 0.3)], stroke);
        }
        IconKind::Close => {
            painter.line_segment(
                [
                    c + Vec2::new(-r * 0.55, -r * 0.55),
                    c + Vec2::new(r * 0.55, r * 0.55),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    c + Vec2::new(r * 0.55, -r * 0.55),
                    c + Vec2::new(-r * 0.55, r * 0.55),
                ],
                stroke,
            );
        }
        IconKind::Check => {
            painter.add(egui::Shape::line(
                vec![
                    c + Vec2::new(-r * 0.6, 0.0),
                    c + Vec2::new(-r * 0.15, r * 0.5),
                    c + Vec2::new(r * 0.7, -r * 0.55),
                ],
                Stroke::new(1.9, color),
            ));
        }
        IconKind::Lock => {
            painter.rect_stroke(
                egui::Rect::from_center_size(
                    c + Vec2::new(0.0, r * 0.28),
                    Vec2::new(r * 0.9, r * 0.65),
                ),
                Rounding::same(2.0),
                Stroke::new(1.3, color),
            );
            painter.add(egui::Shape::line(
                (0..10)
                    .map(|i| {
                        let a = std::f32::consts::PI + i as f32 * std::f32::consts::PI / 9.0;
                        c + Vec2::new(a.cos(), a.sin()) * r * 0.45 + Vec2::new(0.0, -r * 0.1)
                    })
                    .collect(),
                Stroke::new(1.3, color),
            ));
        }
        IconKind::ChevronRight => {
            painter.add(egui::Shape::line(
                vec![
                    c + Vec2::new(-r * 0.25, -r * 0.55),
                    c + Vec2::new(r * 0.35, 0.0),
                    c + Vec2::new(-r * 0.25, r * 0.55),
                ],
                Stroke::new(1.5, color),
            ));
        }
        IconKind::ArrowLeft | IconKind::ArrowRight => {
            let sign = if kind == IconKind::ArrowLeft { -1.0 } else { 1.0 };
            painter.line_segment(
                [c - Vec2::new(sign * r * 0.7, 0.0), c + Vec2::new(sign * r * 0.7, 0.0)],
                stroke,
            );
            painter.line_segment(
                [
                    c + Vec2::new(sign * r * 0.7, 0.0),
                    c + Vec2::new(sign * r * 0.25, -r * 0.45),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    c + Vec2::new(sign * r * 0.7, 0.0),
                    c + Vec2::new(sign * r * 0.25, r * 0.45),
                ],
                stroke,
            );
        }
        IconKind::Trash => {
            painter.rect_stroke(
                egui::Rect::from_center_size(
                    c + Vec2::new(0.0, r * 0.2),
                    Vec2::new(r * 0.85, r * 0.95),
                ),
                Rounding::same(2.0),
                Stroke::new(1.3, color),
            );
            painter.line_segment(
                [c + Vec2::new(-r * 0.6, -r * 0.4), c + Vec2::new(r * 0.6, -r * 0.4)],
                Stroke::new(1.5, color),
            );
            painter.line_segment(
                [c + Vec2::new(-r * 0.25, -r * 0.7), c + Vec2::new(r * 0.25, -r * 0.7)],
                Stroke::new(1.3, color),
            );
        }
        IconKind::Pencil => {
            painter.line_segment(
                [c + Vec2::new(-r * 0.6, r * 0.6), c + Vec2::new(r * 0.5, -r * 0.5)],
                Stroke::new(1.6, color),
            );
            painter.line_segment(
                [c + Vec2::new(-r * 0.6, r * 0.6), c + Vec2::new(-r * 0.25, r * 0.5)],
                Stroke::new(1.3, color),
            );
        }
        IconKind::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(-r * 0.4, -r * 0.65),
                    c + Vec2::new(r * 0.65, 0.0),
                    c + Vec2::new(-r * 0.4, r * 0.65),
                ],
                color,
                Stroke::NONE,
            ));
        }
        IconKind::Stop => {
            painter.rect_filled(
                egui::Rect::from_center_size(c, Vec2::splat(r * 0.95)),
                Rounding::same(2.0),
                color,
            );
        }
    }
}

pub fn logo(painter: &egui::Painter, rect: egui::Rect) {
    let p = current();
    paint_gradient_rect(painter, rect, p.logo_a, p.logo_b, Rounding::same(11.0));
    let c = rect.center();
    let r = rect.width() * 0.27;
    let stroke = Stroke::new(2.2, p.on_accent);
    painter.add(egui::Shape::line(
        vec![
            c + Vec2::new(-r, -r * 0.75),
            c + Vec2::new(r * 0.8, -r * 0.75),
            c + Vec2::new(-r * 0.7, r * 0.7),
            c + Vec2::new(r, r * 0.7),
        ],
        stroke,
    ));
}

pub fn draw_background(ui: &Ui) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    paint_gradient_rect(&painter, rect, bg_top(), bg(), Rounding::ZERO);
    let p = current();
    let glow = |color: Color32, alpha: u8| {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    };
    let center = rect.right_top() + Vec2::new(-140.0, 80.0);
    for i in (1..=8).rev() {
        painter.circle_filled(center, 70.0 + i as f32 * 38.0, glow(p.glow_a, 4));
    }
    let center2 = rect.left_bottom() + Vec2::new(80.0, -60.0);
    for i in (1..=6).rev() {
        painter.circle_filled(center2, 60.0 + i as f32 * 40.0, glow(p.glow_b, 4));
    }
}

pub fn toast_style(kind: ToastKind) -> (Color32, &'static str) {
    match kind {
        ToastKind::Success => (success(), "完成"),
        ToastKind::Info => (accent(), "提示"),
        ToastKind::Warn => (warn(), "注意"),
        ToastKind::Error => (danger(), "出错了"),
    }
}

#[derive(Clone, Copy)]
pub enum ToastKind {
    Success,
    Info,
    Warn,
    Error,
}

pub fn status_chip(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let size = Vec2::new(galley.size().x + 22.0, 24.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(12.0), color.gamma_multiply(0.12));
    ui.painter()
        .circle_filled(rect.left_center() + Vec2::new(10.0, 0.0), 3.0, color);
    ui.painter().galley(
        rect.left_center() + Vec2::new(17.0, -galley.size().y / 2.0),
        galley,
        color,
    );
}
