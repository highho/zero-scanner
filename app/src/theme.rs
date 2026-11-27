use egui::{
    epaint::{Mesh, Shadow, Vertex, WHITE_UV},
    Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Response,
    Rounding, Sense, Stroke, TextStyle, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(9, 13, 21);
pub const BG_TOP: Color32 = Color32::from_rgb(14, 20, 33);
pub const PANEL: Color32 = Color32::from_rgb(13, 18, 29);
pub const SIDEBAR: Color32 = Color32::from_rgb(10, 15, 25);
pub const CARD: Color32 = Color32::from_rgb(18, 25, 39);
pub const CARD_HI: Color32 = Color32::from_rgb(23, 32, 50);
pub const CARD_SOFT: Color32 = Color32::from_rgb(15, 21, 34);
pub const BORDER: Color32 = Color32::from_rgb(35, 47, 68);
pub const BORDER_SOFT: Color32 = Color32::from_rgb(27, 37, 55);
pub const TEXT: Color32 = Color32::from_rgb(232, 238, 249);
pub const MUTED: Color32 = Color32::from_rgb(144, 158, 182);
pub const FAINT: Color32 = Color32::from_rgb(102, 116, 140);
pub const ACCENT: Color32 = Color32::from_rgb(92, 140, 255);
pub const ACCENT_2: Color32 = Color32::from_rgb(52, 211, 190);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(43, 88, 200);
pub const WARN: Color32 = Color32::from_rgb(246, 184, 78);
pub const DANGER: Color32 = Color32::from_rgb(255, 106, 112);
pub const SUCCESS: Color32 = Color32::from_rgb(74, 222, 158);

pub fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        ("C:\\Windows\\Fonts\\Deng.ttf", 0u32),
        ("C:\\Windows\\Fonts\\msyh.ttc", 0),
        ("C:\\Windows\\Fonts\\simhei.ttf", 0),
    ];
    for (path, index) in candidates {
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
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = Color32::from_rgb(11, 16, 26);
    style.visuals.faint_bg_color = CARD_SOFT;
    style.visuals.code_bg_color = CARD_SOFT;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.38);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.window_rounding = Rounding::same(16.0);
    style.visuals.menu_rounding = Rounding::same(10.0);
    style.visuals.window_shadow = Shadow {
        offset: Vec2::new(0.0, 18.0),
        blur: 48.0,
        spread: 0.0,
        color: Color32::from_black_alpha(130),
    };
    style.visuals.popup_shadow = Shadow {
        offset: Vec2::new(0.0, 8.0),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(110),
    };
    style.visuals.widgets.noninteractive.bg_fill = CARD_SOFT;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER_SOFT);
    style.visuals.widgets.noninteractive.rounding = Rounding::same(10.0);
    style.visuals.widgets.inactive.bg_fill = CARD;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.rounding = Rounding::same(9.0);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.hovered.bg_fill = CARD_HI;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    style.visuals.widgets.hovered.rounding = Rounding::same(9.0);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.active.bg_fill = CARD_HI;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.active.rounding = Rounding::same(9.0);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.open.bg_fill = CARD_HI;
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.open.rounding = Rounding::same(9.0);
    style.visuals.window_stroke = Stroke::new(1.0, BORDER);
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

pub fn card() -> Frame {
    Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(Rounding::same(16.0))
        .inner_margin(Margin::same(16.0))
        .shadow(Shadow {
            offset: Vec2::new(0.0, 8.0),
            blur: 28.0,
            spread: 0.0,
            color: Color32::from_black_alpha(60),
        })
}

pub fn soft_card() -> Frame {
    Frame::none()
        .fill(CARD_SOFT)
        .stroke(Stroke::new(1.0, BORDER_SOFT))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
}

pub fn divider(ui: &mut Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, Rounding::ZERO, BORDER_SOFT);
}

pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(11.5)
            .color(FAINT)
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
    // A clipped fill at the rounded corners keeps the gradient inside the chip.
    painter.rect_stroke(rect, rounding, Stroke::new(0.5, Color32::from_white_alpha(20)));
}

pub fn gradient_button(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> Response {
    let height = 42.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter();
    if !enabled {
        painter.rect_filled(rect, Rounding::same(12.0), Color32::from_rgb(30, 39, 55));
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(14.5),
            FAINT,
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
        Color32::from_rgba_unmultiplied(26, 58, 120, 120),
    );
    let c1 = if hovered {
        Color32::from_rgb(108, 154, 255)
    } else {
        Color32::from_rgb(83, 132, 246)
    };
    let c2 = if hovered {
        Color32::from_rgb(58, 224, 201)
    } else {
        Color32::from_rgb(45, 195, 177)
    };
    paint_gradient_rect(&painter, base, c1, c2, Rounding::same(12.0));
    if hovered {
        painter.rect_filled(base, Rounding::same(12.0), Color32::from_white_alpha(10));
    }
    painter.text(
        base.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(15.0),
        Color32::from_rgb(8, 18, 28),
    );
    response
}

pub fn outline_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        FontId::proportional(14.0),
        if enabled { TEXT } else { FAINT },
    );
    let width = (galley.size().x + 32.0).max(88.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() && enabled {
        CARD_HI
    } else {
        Color32::from_rgb(15, 21, 34)
    };
    painter.rect_filled(rect, Rounding::same(10.0), fill);
    painter.rect_stroke(
        rect,
        Rounding::same(10.0),
        Stroke::new(1.0, if response.hovered() && enabled { ACCENT.gamma_multiply(0.65) } else { BORDER }),
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if enabled { TEXT } else { FAINT },
    );
    response
}

pub fn pill(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        FontId::proportional(13.0),
        if selected { Color32::WHITE } else { MUTED },
    );
    let width = galley.size().x + 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, Rounding::same(9.0), ACCENT.gamma_multiply(0.22));
        painter.rect_stroke(
            rect,
            Rounding::same(9.0),
            Stroke::new(1.0, ACCENT.gamma_multiply(0.75)),
        );
    } else {
        painter.rect_filled(rect, Rounding::same(9.0), Color32::from_rgb(16, 22, 35));
        if response.hovered() {
            painter.rect_stroke(rect, Rounding::same(9.0), Stroke::new(1.0, BORDER));
        }
    }
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if selected { Color32::WHITE } else { MUTED },
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
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        color,
    );
}

pub fn icon_button(ui: &mut Ui, kind: IconKind) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(9.0), Color32::from_white_alpha(8));
    }
    paint_icon(ui.painter(), rect.shrink(9.0), kind, MUTED);
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
    ArrowLeft,
    ArrowRight,
    Pointer,
    Aob,
    Symbol,
    Freeze,
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
        IconKind::Pointer => {
            for i in 0..2 {
                let left = c + Vec2::new(-r * 0.9 + i as f32 * r * 0.9, 0.0);
                let right = c + Vec2::new(-r * 0.9 + (i + 1) as f32 * r * 0.9, 0.0);
                painter.line_segment([left, right], Stroke::new(1.6, color));
                painter.circle_filled(left, r * 0.24, color);
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
        IconKind::Practice | IconKind::Freeze => {
            painter.circle_stroke(c, r * 0.9, Stroke::new(1.3, color));
            painter.circle_stroke(c, r * 0.48, Stroke::new(1.3, color));
            painter.circle_filled(c, r * 0.15, color);
        }
        IconKind::Settings => {
            for (i, offset) in [-0.55f32, 0.0, 0.55].iter().enumerate() {
                let y = c.y + r * offset;
                painter.line_segment(
                    [
                        egui::pos2(c.x - r * 0.9, y),
                        egui::pos2(c.x + r * 0.9, y),
                    ],
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
    paint_gradient_rect(
        painter,
        rect,
        Color32::from_rgb(88, 136, 255),
        Color32::from_rgb(43, 205, 183),
        Rounding::same(11.0),
    );
    let c = rect.center();
    let r = rect.width() * 0.27;
    let stroke = Stroke::new(2.2, Color32::from_rgb(8, 17, 28));
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
    paint_gradient_rect(&painter, rect, BG_TOP, BG, Rounding::ZERO);
    let center = rect.right_top() + Vec2::new(-140.0, 80.0);
    for i in (1..=8).rev() {
        painter.circle_filled(
            center,
            70.0 + i as f32 * 38.0,
            Color32::from_rgba_unmultiplied(73, 117, 235, 3),
        );
    }
    let center2 = rect.left_bottom() + Vec2::new(80.0, -60.0);
    for i in (1..=6).rev() {
        painter.circle_filled(
            center2,
            60.0 + i as f32 * 40.0,
            Color32::from_rgba_unmultiplied(40, 190, 176, 3),
        );
    }
}

pub fn toast_style(kind: ToastKind) -> (Color32, &'static str) {
    match kind {
        ToastKind::Success => (SUCCESS, "完成"),
        ToastKind::Info => (ACCENT, "提示"),
        ToastKind::Warn => (WARN, "注意"),
        ToastKind::Error => (DANGER, "出错了"),
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
    ui.painter().circle_filled(
        rect.left_center() + Vec2::new(10.0, 0.0),
        3.0,
        color,
    );
    ui.painter().galley(
        rect.left_center() + Vec2::new(17.0, -galley.size().y / 2.0),
        galley,
        color,
    );
}
