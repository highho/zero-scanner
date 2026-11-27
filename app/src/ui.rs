use crate::app::{EditState, ScanPhase, Tab, ZeroApp};
use crate::model::{format_bytes, parse_address, ScanType, VType};
use crate::disasm;
use crate::hotkey;
use crate::triage;
use crate::smart::{self, Preset};
use crate::syms;
use crate::theme::{self, IconKind, ToastKind};
use crate::winmem;
use eframe::egui;
use egui::{Align, Align2, Color32, FontId, Frame, Layout, Margin, Response, Rounding, Sense, Stroke, Vec2};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).size(26.0).strong().color(theme::TEXT));
            ui.label(egui::RichText::new(subtitle).size(12.5).color(theme::MUTED));
        });
    });
}

fn icon_text_button(ui: &mut egui::Ui, icon: IconKind, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::proportional(13.0),
        theme::MUTED,
    );
    let size = Vec2::new(galley.size().x + 42.0, 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        theme::CARD_HI
    } else {
        Color32::from_rgb(15, 21, 34)
    };
    ui.painter().rect_filled(rect, Rounding::same(9.0), fill);
    ui.painter().rect_stroke(
        rect,
        Rounding::same(9.0),
        Stroke::new(1.0, theme::BORDER_SOFT),
    );
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            rect.left_center() + Vec2::new(18.0, 0.0),
            Vec2::splat(15.0),
        ),
        icon,
        theme::MUTED,
    );
    ui.painter().galley(
        rect.left_center() + Vec2::new(31.0, -galley.size().y / 2.0),
        galley,
        theme::MUTED,
    );
    response
}

fn toggle_switch(ui: &mut egui::Ui, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(34.0, 19.0), Sense::click());
    let bg = if on { theme::SUCCESS } else { Color32::from_rgb(45, 56, 75) };
    ui.painter().rect_filled(rect, Rounding::same(10.0), bg);
    let knob_x = if on {
        rect.right() - 9.5
    } else {
        rect.left() + 9.5
    };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, rect.center().y), 7.0, Color32::WHITE);
    response
}

fn progress_bar(ui: &mut egui::Ui, fraction: f32) {
    let fraction = fraction.clamp(0.0, 1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(4.0), Color32::from_rgb(26, 35, 51));
    let fill = egui::Rect::from_min_size(
        rect.left_top(),
        Vec2::new(rect.width() * fraction, rect.height()),
    );
    theme::paint_gradient_rect(
        ui.painter(),
        fill,
        theme::ACCENT,
        theme::ACCENT_2,
        Rounding::same(4.0),
    );
}

fn numbered_step(ui: &mut egui::Ui, n: u32, text: &str, current: bool, done: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
        let color = if done {
            theme::SUCCESS
        } else if current {
            theme::ACCENT
        } else {
            theme::FAINT
        };
        ui.painter()
            .circle_filled(rect.center(), 11.0, color.gamma_multiply(if current { 0.2 } else { 0.1 }));
        ui.painter()
            .circle_stroke(rect.center(), 11.0, Stroke::new(1.0, color.gamma_multiply(0.7)));
        if done {
            theme::paint_icon(ui.painter(), rect.shrink(7.0), IconKind::Check, color);
        } else {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                n.to_string(),
                FontId::proportional(11.0),
                color,
            );
        }
        ui.label(
            egui::RichText::new(text)
                .size(12.5)
                .color(if current { theme::TEXT } else { theme::MUTED }),
        );
    });
}

fn empty_state(ui: &mut egui::Ui, icon: IconKind, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(10.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
        ui.painter().circle_filled(
            rect.center(),
            26.0,
            theme::ACCENT.gamma_multiply(0.10),
        );
        theme::paint_icon(ui.painter(), rect.shrink(16.0), icon, theme::ACCENT);
        ui.add_space(6.0);
        ui.label(egui::RichText::new(title).size(15.0).strong());
        ui.label(
            egui::RichText::new(body)
                .size(12.0)
                .color(theme::MUTED),
        );
        ui.add_space(8.0);
    });
}

fn type_pill(ui: &mut egui::Ui, ty: VType, selected: bool) -> Response {
    theme::pill(ui, ty.label(), selected)
}

impl ZeroApp {
    pub fn ui_topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("zero_top")
            .exact_height(70.0)
            .frame(
                Frame::none()
                    .fill(theme::PANEL)
                    .inner_margin(Margin::symmetric(18.0, 11.0))
                    .stroke(Stroke::new(1.0, theme::BORDER_SOFT)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                    theme::logo(ui.painter(), rect);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("ZERO")
                                .size(18.0)
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.label(
                            egui::RichText::new("SMART MEMORY LAB")
                                .size(9.0)
                                .color(theme::FAINT),
                        );
                    });
                    ui.add_space(22.0);

                    let pill_size = Vec2::new(342.0, 44.0);
                    let (rect, response) = ui.allocate_exact_size(pill_size, Sense::click());
                    let hovered = response.hovered();
                    ui.painter().rect_filled(
                        rect,
                        Rounding::same(12.0),
                        if hovered { theme::CARD_HI } else { theme::CARD_SOFT },
                    );
                    ui.painter().rect_stroke(
                        rect,
                        Rounding::same(12.0),
                        Stroke::new(
                            1.0,
                            if hovered { theme::ACCENT.gamma_multiply(0.6) } else { theme::BORDER },
                        ),
                    );
                    let icon_rect = egui::Rect::from_center_size(
                        rect.left_center() + Vec2::new(24.0, 0.0),
                        Vec2::splat(28.0),
                    );
                    ui.painter()
                        .rect_filled(icon_rect, Rounding::same(8.0), theme::ACCENT.gamma_multiply(0.14));
                    theme::paint_icon(ui.painter(), icon_rect.shrink(7.0), IconKind::Practice, theme::ACCENT);
                    if self.is_attached() {
                        let name = if self.process_name.chars().count() > 24 {
                            let s: String = self.process_name.chars().take(22).collect();
                            format!("{s}…")
                        } else {
                            self.process_name.clone()
                        };
                        ui.painter().text(
                            rect.left_center() + Vec2::new(46.0, -9.0),
                            Align2::LEFT_CENTER,
                            name,
                            FontId::proportional(13.5),
                            theme::TEXT,
                        );
                        let detail = format!(
                            "PID {}  ·  {}  ·  {}",
                            self.attached_pid.unwrap_or(0),
                            self.process_arch,
                            if self.process_writable { "可写" } else { "只读" }
                        );
                        ui.painter().text(
                            rect.left_center() + Vec2::new(46.0, 10.0),
                            Align2::LEFT_CENTER,
                            detail,
                            FontId::proportional(10.5),
                            theme::FAINT,
                        );
                    } else {
                        ui.painter().text(
                            rect.left_center() + Vec2::new(46.0, -8.0),
                            Align2::LEFT_CENTER,
                            "选择进程",
                            FontId::proportional(13.5),
                            theme::TEXT,
                        );
                        ui.painter().text(
                            rect.left_center() + Vec2::new(46.0, 10.0),
                            Align2::LEFT_CENTER,
                            "连接一个正在运行的游戏",
                            FontId::proportional(10.5),
                            theme::FAINT,
                        );
                    }
                    theme::paint_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(
                            rect.right_center() - Vec2::new(20.0, 0.0),
                            Vec2::splat(14.0),
                        ),
                        IconKind::ChevronRight,
                        theme::FAINT,
                    );
                    if response.clicked() {
                        self.open_picker();
                    }

                    ui.add_space(10.0);
                    if self.scan_phase == ScanPhase::Running {
                        theme::status_chip(ui, "扫描中", theme::WARN);
                    } else if self.is_attached() {
                        theme::status_chip(ui, "已连接", theme::SUCCESS);
                    } else {
                        theme::status_chip(ui, "未连接", theme::FAINT);
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(ui, IconKind::Settings).on_hover_text("扫描设置").clicked() {
                            self.show_settings = true;
                        }
                        if theme::icon_button(ui, IconKind::Memory).on_hover_text("内存浏览").clicked() {
                            self.tab = Tab::Memory;
                            if self.is_attached() && !self.mem_loaded {
                                if let Some(hit) = self.hits.first() {
                                    self.goto_memory(hit.address);
                                }
                            }
                        }
                        if theme::icon_button(ui, IconKind::Assistant).on_hover_text("智能助手").clicked() {
                            self.tab = Tab::Assistant;
                        }
                        if self.is_attached() && theme::outline_button(ui, "断开", true).clicked() {
                            self.detach();
                        }
                    });
                });
            });
    }

    pub fn ui_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("zero_nav")
            .exact_width(218.0)
            .resizable(false)
            .frame(
                Frame::none()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::symmetric(14.0, 16.0))
                    .stroke(Stroke::new(1.0, theme::BORDER_SOFT)),
            )
            .show(ctx, |ui| {
                ui.add_space(4.0);
                theme::section_label(ui, "工作区");
                ui.add_space(2.0);
                self.nav_item(ui, Tab::Scan, IconKind::Scan, "扫描", Some(self.hits.len().to_string()));
                self.nav_item(
                    ui,
                    Tab::List,
                    IconKind::List,
                    "地址列表",
                    Some(self.entries.len().to_string()),
                );
                self.nav_item(ui, Tab::Memory, IconKind::Memory, "内存浏览", None);
                self.nav_item(ui, Tab::Assistant, IconKind::Assistant, "智能助手", None);
                self.nav_item(ui, Tab::Practice, IconKind::Practice, "练习靶场", None);
                self.nav_item(ui, Tab::PtrScan, IconKind::Pointer, "指针扫描", None);
                self.nav_item(ui, Tab::Aob, IconKind::Aob, "AOB 扫描", None);
                self.nav_item(ui, Tab::Symbols, IconKind::Symbol, "符号解析", None);
                self.nav_item(ui, Tab::Disasm, IconKind::Bolt, "反汇编", None);
                self.nav_item(ui, Tab::Inject, IconKind::Pencil, "代码补丁", None);
                self.nav_item(ui, Tab::Asm, IconKind::ChevronRight, "自动汇编", None);
                self.nav_item(ui, Tab::Table, IconKind::List, "CT 表格", None);
                self.nav_item(ui, Tab::Speed, IconKind::Play, "变速齿轮", None);
                self.nav_item(ui, Tab::Script, IconKind::Bolt, "Lua 脚本", None);
                ui.add_space(14.0);
                theme::divider(ui);
                ui.add_space(10.0);
                theme::section_label(ui, "当前会话");
                ui.add_space(4.0);
                theme::soft_card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                        ui.painter()
                            .circle_filled(rect.center(), 13.0, theme::ACCENT.gamma_multiply(0.12));
                        theme::paint_icon(
                            ui.painter(),
                            rect.shrink(7.0),
                            IconKind::Practice,
                            theme::ACCENT,
                        );
                        ui.vertical(|ui| {
                            if self.is_attached() {
                                ui.label(
                                    egui::RichText::new(&self.process_name)
                                        .size(12.0)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} · PID {}",
                                        self.process_arch,
                                        self.attached_pid.unwrap_or(0)
                                    ))
                                    .size(10.5)
                                    .color(theme::FAINT),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new("还没有连接进程")
                                        .size(12.0)
                                        .color(theme::MUTED),
                                );
                                ui.label(
                                    egui::RichText::new("点击上方进程卡片")
                                        .size(10.5)
                                        .color(theme::FAINT),
                                );
                            }
                        });
                    });
                });

                let remaining = ui.available_height();
                if remaining > 128.0 {
                    ui.add_space(remaining - 128.0);
                }
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                        ui.painter()
                            .circle_filled(rect.center(), 12.0, theme::ACCENT_2.gamma_multiply(0.12));
                        theme::paint_icon(ui.painter(), rect.shrink(7.0), IconKind::Lock, theme::ACCENT_2);
                        ui.label(
                            egui::RichText::new("本地 · 离线")
                                .size(12.0)
                                .strong()
                                .color(theme::TEXT),
                        );
                    });
                    ui.add_space(3.0);
                    ui.label(
                        egui::RichText::new("所有扫描都在本机完成，不上传内容，也不注入目标进程。")
                            .size(10.5)
                            .color(theme::MUTED),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("v0.1 · 独立重制版")
                        .size(10.0)
                        .color(theme::FAINT),
                );
            });
    }

    fn nav_item(
        &mut self,
        ui: &mut egui::Ui,
        tab: Tab,
        icon: IconKind,
        label: &str,
        badge: Option<String>,
    ) {
        let selected = self.tab == tab;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, Rounding::same(10.0), theme::ACCENT.gamma_multiply(0.13));
            painter.rect_filled(
                egui::Rect::from_min_size(
                    rect.left_center() + Vec2::new(0.0, -12.0),
                    Vec2::new(3.0, 24.0),
                ),
                Rounding::same(2.0),
                theme::ACCENT,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, Rounding::same(10.0), Color32::from_white_alpha(5));
        }
        let color = if selected { theme::ACCENT } else { theme::MUTED };
        theme::paint_icon(
            painter,
            egui::Rect::from_center_size(
                rect.left_center() + Vec2::new(24.0, 0.0),
                Vec2::splat(18.0),
            ),
            icon,
            color,
        );
        painter.text(
            rect.left_center() + Vec2::new(46.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.5),
            if selected { theme::TEXT } else { theme::MUTED },
        );
        if let Some(badge) = badge {
            if badge != "0" {
                painter.text(
                    rect.right_center() - Vec2::new(14.0, 0.0),
                    Align2::RIGHT_CENTER,
                    badge,
                    FontId::proportional(10.5),
                    if selected { theme::ACCENT } else { theme::FAINT },
                );
            }
        }
        if response.clicked() {
            self.tab = tab;
            if tab == Tab::Memory && self.is_attached() && !self.mem_loaded {
                if let Some(hit) = self.hits.first() {
                    self.goto_memory(hit.address);
                } else if let Some(entry) = self.entries.first() {
                    self.goto_memory(entry.address);
                }
            }
        }
    }

    pub fn ui_statusbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("zero_status")
            .exact_height(30.0)
            .frame(
                Frame::none()
                    .fill(theme::PANEL)
                    .inner_margin(Margin::symmetric(18.0, 6.0))
                    .stroke(Stroke::new(1.0, theme::BORDER_SOFT)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let running = self.scan_phase == ScanPhase::Running
                        || self.ptr_phase == ScanPhase::Running
                        || self.aob_phase == ScanPhase::Running;
                    let color = if running {
                        theme::WARN
                    } else if self.is_attached() {
                        theme::SUCCESS
                    } else {
                        theme::FAINT
                    };
                    ui.painter().circle_filled(
                        ui.cursor().left_center() + Vec2::new(4.0, 0.0),
                        3.0,
                        color,
                    );
                    ui.add_space(12.0);
                    let status = if self.scan_phase == ScanPhase::Running {
                        format!("扫描中 {:.0}%", self.progress() * 100.0)
                    } else if self.ptr_phase == ScanPhase::Running {
                        format!("指针扫描中 · 已扫 {} 个指针", self.ptr_scanned)
                    } else if self.aob_phase == ScanPhase::Running {
                        "AOB 扫描中 · 逐段匹配特征码".to_string()
                    } else if self.is_attached() {
                        format!("已连接 · {} 个候选地址", self.hits.len())
                    } else {
                        "就绪 · 选择一个进程开始".to_string()
                    };
                    ui.label(egui::RichText::new(status).size(11.0).color(theme::MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("单机 / 离线 · 无注入")
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                        ui.label(
                            egui::RichText::new("扫描引擎 v0.1")
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                    });
                });
            });
    }

    pub fn ui_scan(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("扫描").size(26.0).strong());
                ui.label(
                    egui::RichText::new("从一个你看到的数值开始，Zero 会帮你一步步缩小范围。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::outline_button(ui, "智能助手", true).clicked() {
                    self.tab = Tab::Assistant;
                }
                theme::status_chip(
                    ui,
                    &format!("{} 个候选", self.hits.len()),
                    if self.hits.is_empty() {
                        theme::FAINT
                    } else {
                        theme::ACCENT_2
                    },
                );
            });
        });
        ui.add_space(4.0);

        let full = ui.available_width();
        let right_w = 322.0;
        let left_w = (full - right_w - 14.0).max(520.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                self.scan_panel(ui);
            });
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.set_width(right_w);
                self.assistant_side(ui);
            });
        });

        ui.add_space(14.0);
        self.results_panel(ui);
    }

    fn scan_panel(&mut self, ui: &mut egui::Ui) {
        let mut do_first = false;
        let mut do_next = false;
        let mut do_undo = false;
        let mut do_stop = false;
        let mut new_scan = false;
        let mut preset: Option<Preset> = None;
        let running = self.scan_phase == ScanPhase::Running;

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                theme::section_label(ui, "目标数值");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("在游戏里看到的当前值")
                        .size(10.5)
                        .color(theme::FAINT),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::tag(ui, "LOCAL", theme::ACCENT_2);
                });
            });
            ui.add_space(2.0);
            Frame::none()
                .fill(Color32::from_rgb(10, 15, 25))
                .stroke(Stroke::new(
                    1.2,
                    if running { theme::WARN } else { theme::ACCENT.gamma_multiply(0.65) },
                ))
                .rounding(Rounding::same(13.0))
                .inner_margin(Margin::symmetric(14.0, 8.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (icon_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        theme::paint_icon(
                            ui.painter(),
                            icon_rect.shrink(2.0),
                            IconKind::Scan,
                            theme::ACCENT,
                        );
                        let response = ui.add_sized(
                            [ui.available_width() - 4.0, 34.0],
                            egui::TextEdit::singleline(&mut self.scan_value)
                                .font(FontId::proportional(28.0))
                                .frame(false)
                                .hint_text("输入数值，例如 100"),
                        );
                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !running
                        {
                            do_first = true;
                        }
                    });
                });

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("试试").size(11.0).color(theme::FAINT));
                for p in smart::PRESETS {
                    if theme::pill(ui, p.label, false).clicked() {
                        preset = Some(p);
                    }
                }
                if theme::pill(ui, "自动识别类型", self.vtype == VType::Auto).clicked() {
                    self.vtype = VType::Auto;
                }
            });

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("数值类型")
                        .size(11.0)
                        .color(theme::FAINT),
                );
                for ty in VType::SELECTABLE {
                    if type_pill(ui, ty, self.vtype == ty).clicked() {
                        self.vtype = ty;
                    }
                }
            });

            ui.add_space(4.0);
            if self.hits.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("首次扫描")
                            .size(11.0)
                            .color(theme::FAINT),
                    );
                    for st in ScanType::PRIMARY {
                        if theme::pill(ui, st.label(), self.scan_type == st).clicked() {
                            self.scan_type = st;
                        }
                    }
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("数值如何变化")
                            .size(11.0)
                            .color(theme::FAINT),
                    );
                    for st in ScanType::RELATIVE {
                        if theme::pill(ui, st.label(), self.scan_type == st).clicked() {
                            self.scan_type = st;
                        }
                    }
                    for st in [ScanType::Exact, ScanType::Bigger, ScanType::Smaller] {
                        if theme::pill(ui, st.label(), self.scan_type == st).clicked() {
                            self.scan_type = st;
                        }
                    }
                    if theme::pill(ui, "清空重来", false).clicked() {
                        new_scan = true;
                    }
                });
            }
            if self.scan_type.needs_two() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("范围下限 / 上限").size(11.0).color(theme::FAINT));
                    ui.add_sized(
                        [130.0, 28.0],
                        egui::TextEdit::singleline(&mut self.scan_value)
                            .hint_text("下限")
                            .font(FontId::proportional(13.0)),
                    );
                    ui.add_sized(
                        [130.0, 28.0],
                        egui::TextEdit::singleline(&mut self.scan_value2)
                            .hint_text("上限")
                            .font(FontId::proportional(13.0)),
                    );
                });
            }

            ui.add_space(11.0);
            if running {
                progress_bar(ui, self.progress());
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.0}%  ·  {}  ·  {}",
                            self.progress() * 100.0,
                            self.result_mb(),
                            format_elapsed(
                                self.scan_job
                                    .as_ref()
                                    .map(|j| j.started.elapsed())
                                    .unwrap_or_default()
                            )
                        ))
                        .size(11.0)
                        .color(theme::MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::outline_button(ui, "停止扫描", true).clicked() {
                            do_stop = true;
                        }
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    let value_ok = if self.scan_type.needs_value() {
                        !self.scan_value.trim().is_empty()
                    } else {
                        true
                    };
                    let second_ok = if self.scan_type.needs_two() {
                        !self.scan_value2.trim().is_empty()
                    } else {
                        true
                    };
                    let can_first = self.is_attached()
                        && self.scan_type.first_scan_ok()
                        && value_ok
                        && second_ok;
                    if theme::gradient_button(ui, "首次扫描", 148.0, can_first).clicked() && can_first {
                        do_first = true;
                    }
                    let can_next = self.is_attached()
                        && !self.hits.is_empty()
                        && self.scan_type != ScanType::Unknown
                        && value_ok
                        && second_ok;
                    if theme::outline_button(ui, "再次扫描", can_next).clicked() && can_next {
                        do_next = true;
                    }
                    if theme::outline_button(ui, "撤销", !self.hit_history.is_empty()).clicked()
                        && !self.hit_history.is_empty()
                    {
                        do_undo = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(if self.is_attached() {
                                "扫描只读取内存"
                            } else {
                                "等待连接"
                            })
                            .size(10.5)
                            .color(theme::FAINT),
                        );
                    });
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                theme::paint_icon(ui.painter(), rect.shrink(2.0), IconKind::Assistant, theme::ACCENT_2);
                ui.label(
                    egui::RichText::new(self.scan_hint())
                        .size(11.0)
                        .color(theme::MUTED),
                );
            });
            if let Some(error) = &self.scan_error {
                ui.add_space(3.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
        });

        if let Some(p) = preset {
            self.apply_preset(p);
        }
        if do_stop {
            self.stop_scan();
        }
        if do_first {
            self.start_first_scan();
        }
        if do_next {
            self.start_next_scan();
        }
        if do_undo {
            self.undo_scan();
        }
        if new_scan {
            self.hits.clear();
            self.hit_history.clear();
            self.selected_hit = None;
            self.scan_type = ScanType::Exact;
            self.toast("已清空结果，可以重新开始首次扫描。", ToastKind::Info);
        }
    }

    fn assistant_side(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                theme::paint_icon(ui.painter(), rect.shrink(2.0), IconKind::Assistant, theme::ACCENT_2);
                ui.label(egui::RichText::new("智能建议").size(14.5).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::tag(ui, "本机推理", theme::ACCENT_2);
                });
            });
            ui.add_space(6.0);

            let (step, steps) = smart::state_steps(self.is_attached(), self.hits.len(), self.entries.len());
            for (i, text) in steps.iter().enumerate() {
                numbered_step(ui, (i + 1) as u32, text, i == step, i < step);
                ui.add_space(1.0);
            }

            ui.add_space(5.0);
            theme::divider(ui);
            ui.add_space(5.0);
            theme::section_label(ui, "诊断");
            let live = if self.scan_phase == ScanPhase::Running {
                format!(
                    "正在扫描，已完成 {:.0}%。{}",
                    self.progress() * 100.0,
                    if self.scan_truncated { "已到结果上限。" } else { "" }
                )
            } else if !self.is_attached() {
                "还没有连接进程。先选择游戏，或者打开练习靶场。".to_string()
            } else if self.hits.is_empty() {
                "输入你看到的数值。如果不知道当前值，选择「未知初值」先拍快照。".to_string()
            } else if self.hits.len() <= 8 {
                format!("只有 {} 个候选。把它们加入地址列表，再观察哪一个会跟着游戏变化。", self.hits.len())
            } else if self.hits.len() > 20_000 {
                "候选很多。回到游戏让数值变化，使用「数值减少 / 增加」再次扫描。".to_string()
            } else {
                format!("还有 {} 个候选。再改变一次数值并再次扫描。", self.hits.len())
            };
            ui.label(egui::RichText::new(live).size(11.5).color(theme::MUTED));

            // Triage summary: the numbers that answer "should I keep reading
            // this list, or is it already narrowed enough".
            if !self.ranked.is_empty() {
                ui.add_space(8.0);
                theme::divider(ui);
                ui.add_space(6.0);
                theme::section_label(ui, "分诊");
                let strong = triage::strong_count(&self.ranked);
                let top = self.ranked.first().map(|c| c.score).unwrap_or(0);
                let weak = self
                    .ranked
                    .iter()
                    .filter(|c| c.score < 15)
                    .count();
                ui.horizontal(|ui| {
                    theme::tag(ui, &format!("{strong} 个很可能"), theme::ACCENT_2);
                    theme::tag(ui, &format!("最高 {top} 分"), theme::ACCENT);
                    if weak > 0 {
                        theme::tag(ui, &format!("{weak} 个可忽略"), theme::FAINT);
                    }
                });
                if !self.feedback.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "已从你的反馈里学到 {} 条规律",
                            self.feedback.len()
                        ))
                        .size(10.5)
                        .color(theme::SUCCESS),
                    );
                }
            }

            ui.add_space(10.0);
            if theme::gradient_button(ui, "打开智能助手", ui.available_width(), true).clicked() {
                self.tab = Tab::Assistant;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.fast, "快速扫描");
                ui.checkbox(&mut self.writable_only, "仅可写内存");
            });
            // One-click health check against whatever is attached: proves the
            // engine works on this specific process without a special target.
            ui.add_space(6.0);
            if theme::outline_button(ui, "验证这台进程", self.is_attached() && !self.e2e_running)
                .clicked()
                && self.is_attached()
                && !self.e2e_running
            {
                self.run_e2e_check();
            }
            if let Some(ok) = self.e2e_ok {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(if ok { "上次验证：全部通过" } else { "上次验证：有失败项" })
                        .size(10.5)
                        .color(if ok { theme::SUCCESS } else { theme::DANGER }),
                );
            }
        });
    }

    fn results_panel(&mut self, ui: &mut egui::Ui) {
        let mut add_index = None;
        let mut add_selected = false;
        let mut clear = false;
        let mut browse = None;
        let mut reject_index: Option<usize> = None;
        let hits_len = self.hits.len();

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("扫描结果").size(15.0).strong());
                theme::tag(ui, &format!("{hits_len}"), if hits_len > 0 { theme::ACCENT_2 } else { theme::FAINT });
                if !self.last_scan_kind.is_empty() {
                    ui.label(
                        egui::RichText::new(&self.last_scan_kind)
                            .size(10.5)
                            .color(theme::FAINT),
                    );
                }
                if self.scan_truncated {
                    theme::tag(ui, "已达上限", theme::WARN);
                }
                if !self.ranked.is_empty() {
                    let strong = triage::strong_count(&self.ranked);
                    let top = self.ranked.first().map(|c| c.score).unwrap_or(0);
                    if strong > 0 {
                        theme::tag(ui, &format!("{strong} 个很可能"), theme::ACCENT_2);
                        ui.label(
                            egui::RichText::new(format!("最高 {top} 分 · 按证据排序"))
                                .size(10.0)
                                .color(theme::FAINT),
                        );
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::outline_button(ui, "清空", hits_len > 0).clicked() && hits_len > 0 {
                        clear = true;
                    }
                    if theme::outline_button(ui, "添加所选", self.selected_hit.is_some()).clicked()
                        && self.selected_hit.is_some()
                    {
                        add_selected = true;
                    }
                });
            });
            ui.add_space(4.0);
            if hits_len == 0 {
                empty_state(
                    ui,
                    IconKind::Scan,
                    "还没有扫描结果",
                    "连接进程，输入数值，然后点击「首次扫描」。结果会出现在这里。",
                );
                return;
            }
            // The watch list lives directly above the result table: after a
            // scan the next thing the user wants is to stop reading 400 rows.
            ui.add_space(6.0);
            self.watch_panel(ui);
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.add_space(8.0);
                cell_label(ui, 158.0, "地址", true);
                cell_label(ui, 56.0, "类型", true);
                cell_label(ui, 118.0, "当前值", true);
                cell_label(ui, 108.0, "上次值", true);
                cell_label(ui, 190.0, "操作", true);
            });
            theme::divider(ui);
            let row_height = 31.0;
            let mut new_selected = self.selected_hit;
            egui::ScrollArea::vertical()
                .max_height(252.0)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, hits_len, |ui, range| {
                    // Reserve room for the scrollbar: rows draw their own
                    // background, so a full-width row would slide under it and
                    // clip the right-most button.
                    let row_width = (ui.available_width() - 12.0).max(320.0);
                    for i in range {
                        let hit = &self.hits[i];
                        let selected = self.selected_hit == Some(i);
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::click());
                        if selected {
                            ui.painter()
                                .rect_filled(rect, Rounding::same(6.0), theme::ACCENT.gamma_multiply(0.10));
                        } else if response.hovered() {
                            ui.painter()
                                .rect_filled(rect, Rounding::same(6.0), Color32::from_white_alpha(4));
                        }
                        let mut row_ui =
                            ui.child_ui(rect, Layout::left_to_right(Align::Center));
                        row_ui.add_space(8.0);
                        // The verdict is the whole point of triage: it answers
                        // "which of these is it" before the user reads addresses.
                        let candidate = self.ranked.iter().find(|c| c.hit.address == hit.address);
                        let (marker, colour) = match candidate.map(|c| c.score) {
                            Some(score) if score >= 60 => ("●", theme::ACCENT_2),
                            Some(score) if score >= 35 => ("◐", theme::SUCCESS),
                            Some(score) if score >= 15 => ("○", theme::WARN),
                            Some(_) => ("·", theme::FAINT),
                            None => (" ", theme::FAINT),
                        };
                        let mark = row_ui.label(
                            egui::RichText::new(marker).size(13.0).color(colour),
                        );
                        if let Some(candidate) = candidate {
                            let reasons = candidate
                                .signals
                                .iter()
                                .take(4)
                                .map(|s| {
                                    let sign = if s.weight >= 0 { "+" } else { "" };
                                    format!("{sign}{}  {}", s.weight, s.label)
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            mark.on_hover_text(format!(
                                "{}（{} 分）\n\n{reasons}",
                                candidate.verdict(),
                                candidate.score
                            ));
                        }
                        cell_mono(&mut row_ui, 158.0, &format!("0x{:X}", hit.address), theme::ACCENT);
                        cell_tag(&mut row_ui, 56.0, hit.vtype.short(), type_color(hit.vtype));
                        cell_label(&mut row_ui, 118.0, &hit.value.display(), false);
                        cell_label(
                            &mut row_ui,
                            108.0,
                            &hit.previous.as_ref().map(|v| v.display()).unwrap_or_else(|| "—".to_string()),
                            true,
                        );
                        if cell_button(&mut row_ui, 58.0, "+ 添加", theme::ACCENT_2).clicked() {
                            add_index = Some(i);
                        }
                        if cell_button(&mut row_ui, 45.0, "内存", theme::MUTED).clicked() {
                            browse = Some(hit.address);
                        }
                        // Negative feedback: the one signal a scanner cannot
                        // derive on its own is "no, that is not it".
                        if cell_button(&mut row_ui, 52.0, "不是它", theme::DANGER).clicked() {
                            reject_index = Some(i);
                        }
                        if response.clicked() {
                            new_selected = Some(i);
                        }
                    }
                });
            self.selected_hit = new_selected;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("提示：点击行选中，双击「+ 添加」可把候选放入地址列表。")
                        .size(10.0)
                        .color(theme::FAINT),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.scan_truncated {
                        ui.label(
                            egui::RichText::new("结果达到上限，建议让数值变化后再次扫描。")
                                .size(10.0)
                                .color(theme::WARN),
                        );
                    }
                });
            });
        });

        if let Some(index) = reject_index {
            self.reject_hit(index);
        }
        if let Some(addr) = browse {
            self.tab = Tab::Memory;
            self.goto_memory(addr);
        }
        if let Some(i) = add_index {
            self.add_hit(i);
        } else if add_selected {
            if let Some(i) = self.selected_hit {
                self.add_hit(i);
            }
        }
        if clear {
            self.hits.clear();
            self.hit_history.clear();
            self.selected_hit = None;
            self.toast("已清空扫描结果。", ToastKind::Info);
        }
    }

    pub fn ui_list(&mut self, ui: &mut egui::Ui) {
        let mut toggle = None;
        let mut remove = None;
        let mut edit = None;
        let mut code: Option<u64> = None;
        let mut lock_all = false;
        let mut unlock_all = false;
        let mut clear = false;
        let count = self.entries.len();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("地址列表").size(26.0).strong());
                ui.label(
                    egui::RichText::new("把值得保留的地址钉在这里，随时读取、修改或锁定。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::gradient_button(ui, "手动添加地址", 150.0, true).clicked() {
                    self.show_add = true;
                }
                theme::status_chip(
                    ui,
                    &format!("{count} 个地址"),
                    if count > 0 { theme::ACCENT_2 } else { theme::FAINT },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("已保存").size(14.5).strong());
                ui.label(
                    egui::RichText::new("锁定后每 60ms 写回一次")
                        .size(10.5)
                        .color(theme::FAINT),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::outline_button(ui, "全部解锁", count > 0).clicked() && count > 0 {
                        unlock_all = true;
                    }
                    if theme::outline_button(ui, "全部锁定", count > 0).clicked() && count > 0 {
                        lock_all = true;
                    }
                    if theme::outline_button(ui, "清空", count > 0).clicked() && count > 0 {
                        clear = true;
                    }
                });
            });
            ui.add_space(4.0);
            if self.entries.is_empty() {
                empty_state(
                    ui,
                    IconKind::List,
                    "列表还是空的",
                    "扫描出候选后点「+ 添加」，或者直接手动输入一个地址。",
                );
                return;
            }
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                cell_label(ui, 34.0, "锁定", true);
                cell_label(ui, 150.0, "描述", true);
                cell_label(ui, 158.0, "地址", true);
                cell_label(ui, 72.0, "类型", true);
                cell_label(ui, 130.0, "当前值", true);
                cell_label(ui, 130.0, "上次值", true);
                cell_label(ui, 92.0, "操作", true);
            });
            theme::divider(ui);
            let row_height = 38.0;
            let scroll_out = egui::ScrollArea::vertical()
                .max_height(420.0)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, count, |ui, range| {
                    for i in range {
                        let entry = &self.entries[i];
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            let mut active = entry.active;
                            if toggle_switch(ui, active).clicked() {
                                active = !active;
                                toggle = Some((entry.id, active));
                            }
                            cell_label(ui, 150.0, &entry.desc, false);
                            cell_mono(ui, 158.0, &format!("0x{:X}", entry.address), theme::ACCENT);
                            cell_tag(ui, 72.0, entry.vtype.short(), type_color(entry.vtype));
                            let current = entry
                                .current
                                .as_ref()
                                .map(|v| v.display())
                                .unwrap_or_else(|| "—".to_string());
                            if cell_button(
                                ui,
                                130.0,
                                &current,
                                if entry.active { theme::SUCCESS } else { theme::TEXT },
                            )
                            .clicked()
                            {
                                edit = Some(entry.id);
                            }
                            let prev = entry
                                .previous
                                .as_ref()
                                .map(|v| v.display())
                                .unwrap_or_else(|| "—".to_string());
                            cell_label(ui, 130.0, &prev, true);
                            if cell_button(ui, 36.0, "代码", theme::ACCENT_2).clicked() {
                                code = Some(entry.address);
                            }
                            if cell_button(ui, 36.0, "编辑", theme::MUTED).clicked() {
                                edit = Some(entry.id);
                            }
                            if cell_button(ui, 36.0, "移除", theme::DANGER).clicked() {
                                remove = Some(entry.id);
                            }
                        });
                        theme::divider(ui);
                    }
                });
        });

        if let Some((id, active)) = toggle {
            let was = self.entries.iter().find(|e| e.id == id).map(|e| e.active);
            if was == Some(!active) {
                self.toggle_entry(id);
            }
        }
        if let Some(id) = remove {
            self.remove_entry(id);
        }
        if let Some(id) = edit {
            self.open_editor(id);
        }
        if let Some(address) = code {
            self.jump_to_disasm(address);
        }
        if clear {
            self.entries.clear();
            self.sync_freeze();
            self.toast("已清空地址列表。", ToastKind::Info);
        }
        if lock_all {
            for entry in &mut self.entries {
                if !entry.active {
                    if entry.value_text.is_empty() {
                        if let Some(v) = &entry.current {
                            entry.value_text = v.display();
                        }
                    }
                    entry.active = true;
                }
            }
            self.sync_freeze();
            self.toast("已锁定全部地址。", ToastKind::Success);
        }
        if unlock_all {
            for entry in &mut self.entries {
                entry.active = false;
            }
            self.sync_freeze();
            self.toast("已解锁全部地址。", ToastKind::Info);
        }
    }

    pub fn ui_memory(&mut self, ui: &mut egui::Ui) {
        let mut read = false;
        let mut prev = false;
        let mut next = false;
        let mut write = false;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("内存浏览").size(26.0).strong());
                ui.label(
                    egui::RichText::new("以十六进制查看目标进程的内存，谨慎写入。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.is_attached() {
                    theme::status_chip(ui, "已连接", theme::SUCCESS);
                } else {
                    theme::status_chip(ui, "未连接", theme::FAINT);
                }
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("地址").size(11.0).color(theme::FAINT));
                ui.add_sized(
                    [260.0, 32.0],
                    egui::TextEdit::singleline(&mut self.mem_addr)
                        .font(FontId::monospace(13.0))
                        .hint_text("0x7FF6..."),
                );
                if theme::gradient_button(ui, "读取", 82.0, self.is_attached()).clicked()
                    && self.is_attached()
                {
                    read = true;
                }
                if theme::outline_button(ui, "上一页", self.is_attached() && self.mem_loaded).clicked()
                    && self.is_attached()
                {
                    prev = true;
                }
                if theme::outline_button(ui, "下一页", self.is_attached() && self.mem_loaded).clicked()
                    && self.is_attached()
                {
                    next = true;
                }
                if theme::outline_button(ui, "看代码", self.is_attached()).clicked()
                    && self.is_attached()
                {
                    self.jump_to_disasm(self.mem_base);
                }
            });
            if let Some(error) = &self.mem_error {
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
        });

        ui.add_space(12.0);
        if !self.is_attached() {
            theme::card().show(ui, |ui| {
                empty_state(
                    ui,
                    IconKind::Memory,
                    "还没有连接进程",
                    "连接进程后，输入地址或用扫描结果里的「内存」按钮跳转。",
                );
            });
            return;
        }
        if self.mem_data.is_empty() {
            theme::card().show(ui, |ui| {
                empty_state(
                    ui,
                    IconKind::Memory,
                    "输入一个地址开始",
                    "可以输入 0x 开头的十六进制地址；也可以在扫描结果里点「内存」。",
                );
            });
            return;
        }

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("内存区域").size(12.0).color(theme::FAINT));
                if let Some(handle) = &self.process {
                    if let Some(region) = handle.region_at(self.mem_base) {
                        theme::tag(ui, &format!("{:X}", region.base), theme::ACCENT);
                        theme::tag(ui, &format!("{:.1} KB", region.size as f64 / 1024.0), theme::MUTED);
                        theme::tag(
                            ui,
                            if region.writable { "可写" } else { "只读" },
                            if region.writable { theme::SUCCESS } else { theme::WARN },
                        );
                        ui.label(
                            egui::RichText::new(handle.protection_at(self.mem_base))
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("基址 0x{:X}", self.mem_base))
                            .size(10.5)
                            .color(theme::FAINT),
                    );
                });
            });
            ui.add_space(8.0);
            theme::divider(ui);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("地址")
                        .size(10.5)
                        .color(theme::FAINT)
                        .monospace(),
                );
                ui.add_space(72.0);
                ui.label(
                    egui::RichText::new("00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F")
                        .size(10.5)
                        .color(theme::FAINT)
                        .monospace(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("ASCII")
                            .size(10.5)
                            .color(theme::FAINT)
                            .monospace(),
                    );
                });
            });
            for row in 0..16 {
                let start = row * 16;
                if start >= self.mem_data.len() {
                    break;
                }
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:016X}", self.mem_base + start as u64))
                            .size(12.0)
                            .color(theme::ACCENT)
                            .monospace(),
                    );
                    ui.add_space(14.0);
                    let mut hex = String::new();
                    for i in 0..16 {
                        if let Some(byte) = self.mem_data.get(start + i) {
                            hex.push_str(&format!("{:02X}", byte));
                        } else {
                            hex.push_str("  ");
                        }
                        hex.push(if i == 7 { ' ' } else { ' ' });
                    }
                    ui.label(
                        egui::RichText::new(hex)
                            .size(12.0)
                            .color(theme::TEXT)
                            .monospace(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ascii: String = self.mem_data[start..(start + 16).min(self.mem_data.len())]
                            .iter()
                            .map(|b| {
                                if b.is_ascii_graphic() || *b == b' ' {
                                    *b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        ui.label(
                            egui::RichText::new(ascii)
                                .size(12.0)
                                .color(theme::MUTED)
                                .monospace(),
                        );
                    });
                });
            }
        });

        ui.add_space(12.0);
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("写入字节").size(13.0).strong());
                    ui.label(
                        egui::RichText::new("十六进制，例如 90 90 90；写入会立即生效。")
                            .size(10.5)
                            .color(theme::FAINT),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::gradient_button(ui, "写入", 84.0, self.process_writable).clicked()
                        && self.process_writable
                    {
                        write = true;
                    }
                    ui.add_sized(
                        [240.0, 30.0],
                        egui::TextEdit::singleline(&mut self.mem_write)
                            .font(FontId::monospace(13.0))
                            .hint_text("90 90 90"),
                    );
                });
            });
        });

        if read {
            self.memory_read();
        }
        if prev {
            let addr = self.mem_base.saturating_sub(256);
            self.goto_memory(addr);
        }
        if next {
            let addr = self.mem_base.saturating_add(256);
            self.goto_memory(addr);
        }
        if write {
            self.memory_write();
        }
    }

    pub fn ui_assistant(&mut self, ui: &mut egui::Ui) {
        let mut send = false;
        let mut apply = false;
        let mut preset: Option<Preset> = None;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("智能助手").size(26.0).strong());
                ui.label(
                    egui::RichText::new("用一句话描述你要找的数值。所有推断都在本机完成。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::tag(ui, "LOCAL AI", theme::ACCENT_2);
            });
        });
        ui.add_space(4.0);

        // Lay the plan panel out FIRST as a right-aligned block, then give the
        // conversation whatever width remains. Building left-to-right let the
        // chat card claim its full preferred width and push the panel off the
        // right edge, which no amount of set_width on the left could prevent.
        let full = ui.available_width();

        // Two columns are deliberately NOT used here. Inside a horizontal
        // layout egui's set_width is only a hint: a card containing a scroll
        // area and a text field kept claiming more than its column and pushing
        // the plan panel off the right edge. Stacking is narrower but always
        // correct, and it reads better in a small window anyway.
        ui.vertical(|ui| {
            let left_w = full;
            ui.vertical(|ui| {
                ui.set_width(left_w);
                ui.set_max_width(left_w);
                theme::card().show(ui, |ui| {
                    ui.set_max_width(left_w - 24.0);
                    ui.set_height(410.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        theme::paint_icon(ui.painter(), rect.shrink(2.0), IconKind::Assistant, theme::ACCENT_2);
                        ui.label(egui::RichText::new("对话").size(14.0).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            theme::tag(ui, "不会联网", theme::SUCCESS);
                        });
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            let width = (ui.available_width() * 0.80).max(240.0);
                            for msg in &self.smart_msgs {
                                if msg.from_user {
                                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                        Frame::none()
                                            .fill(theme::ACCENT.gamma_multiply(0.24))
                                            .rounding(Rounding::same(12.0))
                                            .inner_margin(Margin::symmetric(12.0, 8.0))
                                            .show(ui, |ui| {
                                                ui.set_max_width(width);
                                                ui.label(
                                                    egui::RichText::new(&msg.text)
                                                        .size(12.5)
                                                        .color(theme::TEXT),
                                                );
                                            });
                                    });
                                } else {
                                    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                                        Frame::none()
                                            .fill(theme::CARD_SOFT)
                                            .stroke(Stroke::new(1.0, theme::BORDER_SOFT))
                                            .rounding(Rounding::same(12.0))
                                            .inner_margin(Margin::symmetric(12.0, 8.0))
                                            .show(ui, |ui| {
                                                ui.set_max_width(width);
                                                ui.label(
                                                    egui::RichText::new(&msg.text)
                                                        .size(12.5)
                                                        .color(theme::MUTED),
                                                );
                                            });
                                    });
                                }
                                ui.add_space(8.0);
                            }
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [ui.available_width() - 92.0, 36.0],
                            egui::TextEdit::singleline(&mut self.assistant_input)
                                .hint_text("例如：我的金币是 9800，帮我找它")
                                .font(FontId::proportional(13.0)),
                        );
                        if theme::gradient_button(ui, "解析", 84.0, true).clicked() {
                            send = true;
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("快捷").size(10.5).color(theme::FAINT));
                        for p in smart::PRESETS {
                            if theme::pill(ui, p.label, false).clicked() {
                                preset = Some(p);
                            }
                        }
                    });
                });
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(full);
                ui.set_max_width(full);
                self.assistant_plan_panel(ui, &mut apply);
            });
        });

        if send {
            self.ask_assistant();
        }
        if apply {
            self.apply_plan();
        }
        if let Some(p) = preset {
            self.apply_preset(p);
        }
    }

    fn assistant_plan_panel(&mut self, ui: &mut egui::Ui, apply: &mut bool) {
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("识别结果").size(14.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(plan) = &self.assistant_plan {
                        theme::tag(ui, &format!("{}%", plan.confidence), theme::ACCENT_2);
                    }
                });
            });
            ui.add_space(4.0);
            if let Some(plan) = self.assistant_plan.clone() {
                ui.label(
                    egui::RichText::new(&plan.title)
                        .size(18.0)
                        .strong()
                        .color(theme::TEXT),
                );
                ui.label(egui::RichText::new(&plan.reason).size(11.5).color(theme::MUTED));
                ui.add_space(8.0);
                theme::soft_card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("类型").size(10.5).color(theme::FAINT));
                        theme::tag(ui, plan.vtype.label(), theme::ACCENT);
                        ui.label(egui::RichText::new("方式").size(10.5).color(theme::FAINT));
                        theme::tag(ui, plan.scan_type.label(), theme::ACCENT_2);
                    });
                    if !plan.value.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("数值  {}", plan.value))
                                .size(20.0)
                                .monospace()
                                .color(theme::TEXT),
                        );
                    }
                });
                ui.add_space(6.0);
                for (i, step) in plan.steps.iter().enumerate() {
                    numbered_step(ui, (i + 1) as u32, step, false, false);
                    ui.add_space(2.0);
                }
                ui.add_space(8.0);
                if theme::gradient_button(ui, "套用并去扫描", ui.available_width(), true).clicked() {
                    *apply = true;
                }
            } else {
                empty_state(
                    ui,
                    IconKind::Assistant,
                    "等待你的描述",
                    "我会从一句话里识别数值、类型和建议的扫描方式。",
                );
            }
        });

        ui.add_space(12.0);
        theme::card().show(ui, |ui| {
            ui.label(egui::RichText::new("常用目标").size(13.0).strong());
            ui.add_space(4.0);
            for p in smart::PRESETS {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    theme::paint_icon(ui.painter(), rect.shrink(2.0), IconKind::Practice, theme::ACCENT_2);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(p.label).size(12.0).strong());
                        ui.label(egui::RichText::new(p.hint).size(10.0).color(theme::FAINT));
                    });
                });
            }
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("这里的“智能”是本地规则推理：不调用云端模型，也不读取聊天以外的内容。")
                    .size(10.0)
                    .color(theme::FAINT),
            );
        });
    }

    pub fn ui_practice(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("练习靶场").size(26.0).strong());
                ui.label(
                    egui::RichText::new("没有游戏可试？用一个安全的小程序把扫描流程走一遍。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(210.0, 210.0), Sense::hover());
                theme::paint_gradient_rect(
                    ui.painter(),
                    rect,
                    Color32::from_rgb(20, 30, 48),
                    Color32::from_rgb(12, 19, 31),
                    Rounding::same(16.0),
                );
                for (i, alpha) in [(70.0, 30u8), (46.0, 42), (24.0, 80)] {
                    ui.painter()
                        .circle_stroke(rect.center(), i, Stroke::new(1.0, theme::ACCENT_2.gamma_multiply(alpha as f32 / 255.0)));
                }
                ui.painter()
                    .circle_filled(rect.center(), 5.0, theme::ACCENT_2);
                ui.painter().line_segment(
                    [rect.center() + Vec2::new(-84.0, 0.0), rect.center() + Vec2::new(-16.0, 0.0)],
                    Stroke::new(1.0, theme::ACCENT_2.gamma_multiply(0.6)),
                );
                ui.painter().line_segment(
                    [rect.center() + Vec2::new(16.0, 0.0), rect.center() + Vec2::new(84.0, 0.0)],
                    Stroke::new(1.0, theme::ACCENT_2.gamma_multiply(0.6)),
                );
                ui.painter().line_segment(
                    [rect.center() + Vec2::new(0.0, -84.0), rect.center() + Vec2::new(0.0, -16.0)],
                    Stroke::new(1.0, theme::ACCENT_2.gamma_multiply(0.6)),
                );
                ui.painter().line_segment(
                    [rect.center() + Vec2::new(0.0, 16.0), rect.center() + Vec2::new(0.0, 84.0)],
                    Stroke::new(1.0, theme::ACCENT_2.gamma_multiply(0.6)),
                );

                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("一个会自己变数值的靶场").size(18.0).strong());
                    ui.label(
                        egui::RichText::new(
                            "靶场是一个独立的小窗口，里面有生命、弹药、金币和浮点护甲。点按钮改变它们，再回到 Zero 扫描。",
                        )
                        .size(12.0)
                        .color(theme::MUTED),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if self.practice_child.is_some() {
                            if theme::gradient_button(ui, "连接靶场进程", 158.0, true).clicked() {
                                self.connect_practice();
                            }
                        } else if theme::gradient_button(ui, "启动练习靶场", 158.0, true).clicked() {
                            self.spawn_practice();
                        }
                        if let Some(pid) = self.practice_pid {
                            theme::status_chip(
                                ui,
                                &format!("PID {pid}"),
                                if self.attached_pid == Some(pid) {
                                    theme::SUCCESS
                                } else {
                                    theme::WARN
                                },
                            );
                        }
                    });
                    ui.add_space(8.0);
                    if let Some(pid) = self.practice_pid {
                        ui.label(
                            egui::RichText::new(if self.attached_pid == Some(pid) {
                                "已自动连接靶场。切到靶场窗口改变数值，然后回来扫描。"
                            } else {
                                "靶场正在启动或等待连接。"
                            })
                            .size(10.5)
                            .color(if self.attached_pid == Some(pid) { theme::SUCCESS } else { theme::WARN }),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("靶场会以独立进程运行，不会影响其他程序。")
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                    }
                });
            });
        });

        ui.add_space(14.0);
        let steps = [
            ("启动靶场", "点上面的按钮，会打开一个独立的靶场窗口。"),
            ("记住一个数值", "例如生命 100，或者金币 250。"),
            ("回到 Zero 扫描", "选择靶场进程，输入数值，执行首次扫描；靶场里按「受伤 -10」，再用「数值减少」再次扫描。"),
            ("锁定或修改", "把剩下的地址加入列表，勾选锁定，观察靶场数值不再变化。"),
        ];
        ui.horizontal_top(|ui| {
            for (i, (title, body)) in steps.iter().enumerate() {
                let width = (ui.available_width() - 36.0) / 4.0;
                ui.vertical(|ui| {
                    ui.set_width(width);
                    theme::soft_card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                            ui.painter()
                                .circle_filled(rect.center(), 11.0, theme::ACCENT.gamma_multiply(0.16));
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                (i + 1).to_string(),
                                FontId::proportional(11.0),
                                theme::ACCENT,
                            );
                            ui.label(egui::RichText::new(*title).size(12.5).strong());
                        });
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(*body).size(10.5).color(theme::MUTED));
                    });
                });
                if i < steps.len() - 1 {
                    ui.add_space(10.0);
                }
            }
        });
    }

    pub fn ui_ptr_scan(&mut self, ui: &mut egui::Ui) {
        let running = self.ptr_phase == ScanPhase::Running;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("指针扫描").size(26.0).strong());
                ui.label(
                    egui::RichText::new("给一个内存地址，反推它的稳定指针链：模块基址 + 若干偏移。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{} 条链", self.ptr_paths.len()),
                    if self.ptr_paths.is_empty() { theme::FAINT } else { theme::ACCENT_2 },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "目标地址");
                ui.add_space(6.0);
                let response = ui.add_sized(
                    [280.0, 34.0],
                    egui::TextEdit::singleline(&mut self.ptr_target).font(FontId::monospace(13.0)),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.start_ptr_scan();
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                theme::section_label(ui, "最大层级");
                ui.add_space(6.0);
                ui.add(egui::Slider::new(&mut self.ptr_depth, 1..=6));
                ui.add_space(14.0);
                theme::section_label(ui, "单级偏移上限");
                ui.add_space(6.0);
                let off = ui.add_sized(
                    [130.0, 34.0],
                    egui::TextEdit::singleline(&mut self.ptr_offset_text).font(FontId::monospace(13.0)),
                );
                if off.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(value) = parse_address(&self.ptr_offset_text) {
                        self.ptr_max_offset = value.min(0x1_0000);
                    }
                }
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if theme::gradient_button(ui, "开始指针扫描", 176.0, true).clicked() {
                    self.start_ptr_scan();
                }
                if running && theme::outline_button(ui, "停止", true).clicked() {
                    self.stop_ptr_scan();
                }
                ui.add_space(10.0);
                if running {
                    if let Some(job) = &self.ptr_job {
                        let done = job.progress.load(Ordering::Relaxed);
                        theme::status_chip(
                            ui,
                            &format!("已扫 {} / {}", format_bytes(done), format_bytes(job.total)),
                            theme::WARN,
                        );
                    }
                }
            });
        });
        ui.add_space(14.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "指针链");
                ui.add_space(4.0);
                if self.process.is_some() && !self.ptr_modules_loaded {
                    if theme::outline_button(ui, "载入模块", true).clicked() {
                        if let Some(handle) = self.process.clone() {
                            self.ptr_modules = handle.modules();
                            self.ptr_modules_loaded = true;
                        }
                    }
                }
            });
            ui.add_space(6.0);
            if self.ptr_paths.is_empty() {
                ui.label(
                    egui::RichText::new("还没有结果。先连接进程、输入一个地址，然后开始扫描。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            }
            let mut to_add = None;
            let mut to_open = None;
            let mut to_disasm: Option<u64> = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (index, path) in self.ptr_paths.iter().enumerate() {
                    theme::soft_card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(path.display())
                                        .size(13.0)
                                        .strong()
                                        .font(FontId::monospace(13.0)),
                                );
                                ui.add_space(3.0);
                                ui.horizontal(|ui| {
                                    theme::tag(ui, &format!("{} 层", path.depth()), theme::ACCENT);
                                    theme::tag(ui, &format!("{:016X}", path.final_address), theme::MUTED);
                                });
                            });
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                if theme::outline_button(ui, "反汇编", true).clicked() {
                                    to_disasm = Some(path.final_address);
                                }
                                if theme::outline_button(ui, "内存中查看", true).clicked() {
                                    to_open = Some(index);
                                }
                                if theme::outline_button(ui, "加入列表", true).clicked() {
                                    to_add = Some(index);
                                }
                            });
                        });
                    });
                    ui.add_space(6.0);
                }
            });
            if let Some(index) = to_add {
                self.add_ptr_path(index);
            }
            if let Some(index) = to_open {
                if let Some(path) = self.ptr_paths.get(index).cloned() {
                    self.open_memory_at(path.final_address);
                }
            }
            if let Some(address) = to_disasm {
                self.jump_to_disasm(address);
            }
        });
    }

    pub fn ui_aob_scan(&mut self, ui: &mut egui::Ui) {
        let running = self.aob_phase == ScanPhase::Running;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("AOB 扫描").size(26.0).strong());
                ui.label(
                    egui::RichText::new("按一段字节特征码搜索内存，用通配符匹配不确定的字节。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{} 个匹配", self.aob_hits.len()),
                    if self.aob_hits.is_empty() { theme::FAINT } else { theme::ACCENT_2 },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "字节特征");
                ui.add_space(6.0);
                let response = ui.add_sized(
                    [380.0, 34.0],
                    egui::TextEdit::singleline(&mut self.aob_text).font(FontId::monospace(13.0)),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.start_aob_scan();
                }
            });
            ui.add_space(6.0);
            if theme::outline_button(ui, "填入示例 48 8B ?? 05", true).clicked() {
                self.aob_text = "48 8B ?? 05".to_string();
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if theme::gradient_button(ui, "开始 AOB 扫描", 176.0, true).clicked() {
                    self.start_aob_scan();
                }
                if running && theme::outline_button(ui, "停止", true).clicked() {
                    self.stop_aob_scan();
                }
                ui.add_space(10.0);
                if running {
                    if let Some(job) = &self.aob_job {
                        let done = job.progress.load(Ordering::Relaxed);
                        theme::status_chip(
                            ui,
                            &format!("已扫 {} / {}", format_bytes(done), format_bytes(job.total)),
                            theme::WARN,
                        );
                    }
                }
            });
        });
        ui.add_space(14.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "匹配地址");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("通配符：?? 匹配一个字节，* / ** / x / xx 同理；至少要有一个确定的字节。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(8.0);
            if self.aob_hits.is_empty() {
                ui.label(
                    egui::RichText::new("还没有匹配。输入一段从反汇编里抄来的特征码，然后开始扫描。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            }
            let mut to_add = None;
            let mut to_open = None;
            let mut to_disasm: Option<u64> = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (index, address) in self.aob_hits.iter().enumerate() {
                    theme::soft_card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:016X}", address))
                                        .size(13.0)
                                        .strong()
                                        .font(FontId::monospace(13.0)),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                if theme::outline_button(ui, "反汇编", true).clicked() {
                                    to_disasm = Some(*address);
                                }
                                if theme::outline_button(ui, "内存中查看", true).clicked() {
                                    to_open = Some(index);
                                }
                                if theme::outline_button(ui, "加入列表", true).clicked() {
                                    to_add = Some(index);
                                }
                            });
                        });
                    });
                    ui.add_space(6.0);
                }
            });
            if let Some(index) = to_add {
                self.add_aob_hit(index);
            }
            if let Some(index) = to_open {
                if let Some(address) = self.aob_hits.get(index).copied() {
                    self.open_memory_at(address);
                }
            }
            if let Some(address) = to_disasm {
                self.jump_to_disasm(address);
            }
        });
    }

    pub fn ui_symbols(&mut self, ui: &mut egui::Ui) {
        let has_process = self.process.is_some();
        let (export_count, import_count, dll_count) = match &self.sym_table {
            Some(table) => (
                table.exports.len(),
                table.import_count(),
                table.imports.len(),
            ),
            None => (0, 0, 0),
        };

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("符号解析").size(26.0).strong());
                ui.label(
                    egui::RichText::new("读取模块的 PE 导出表与导入表，把地址还原成函数名。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{export_count} 导出 · {import_count} 导入"),
                    if self.sym_table.is_some() {
                        theme::ACCENT_2
                    } else {
                        theme::FAINT
                    },
                );
            });
        });
        ui.add_space(4.0);

        let module_names: Vec<(String, u64)> = self
            .ptr_modules
            .iter()
            .map(|m| (m.name.clone(), m.base))
            .collect();

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "模块");
                ui.add_space(8.0);
                if module_names.is_empty() {
                    if has_process {
                        if theme::outline_button(ui, "载入模块列表", true).clicked() {
                            if let Some(handle) = self.process.clone() {
                                self.ptr_modules = handle.modules();
                                self.ptr_modules_loaded = !self.ptr_modules.is_empty();
                                if self.sym_module.is_empty() {
                                    if let Some(first) = self.ptr_modules.first() {
                                        self.sym_module = first.name.clone();
                                    }
                                }
                            }
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("先连接一个进程。")
                                .size(12.0)
                                .color(theme::FAINT),
                        );
                    }
                } else {
                    let mut selected = self.sym_module.clone();
                    egui::ComboBox::from_id_source("zero_symbol_module")
                        .selected_text(if selected.is_empty() {
                            "选择模块".to_string()
                        } else {
                            selected.clone()
                        })
                        .width(320.0)
                        .show_ui(ui, |ui| {
                            for (name, base) in &module_names {
                                ui.selectable_value(
                                    &mut selected,
                                    name.clone(),
                                    format!("{name}  ·  {base:X}"),
                                );
                            }
                        });
                    self.sym_module = selected;
                    ui.add_space(10.0);
                    if !self.sym_module.is_empty()
                        && theme::gradient_button(ui, "解析符号", 118.0, true).clicked()
                    {
                        self.parse_symbols();
                    }
                }
            });
            if let Some(error) = &self.sym_error {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(error).size(12.0).color(theme::DANGER));
            }
            if let Some(table) = &self.sym_table {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ·  基址 {:X}  ·  {} 个导入 DLL  ·  用时 {:.1} ms",
                        table.module,
                        table.base,
                        dll_count,
                        self.sym_elapsed.as_secs_f64() * 1000.0
                    ))
                    .size(11.0)
                    .color(theme::FAINT),
                );
            }
        });
        ui.add_space(14.0);

        let mut to_add: Option<(u64, String)> = None;
        let mut to_open: Option<u64> = None;
        let mut to_disasm: Option<u64> = None;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "符号");
                ui.add_space(6.0);
                ui.add_sized(
                    [240.0, 30.0],
                    egui::TextEdit::singleline(&mut self.sym_filter)
                        .font(FontId::monospace(12.0))
                        .hint_text("按名字过滤"),
                );
                ui.add_space(10.0);
                ui.selectable_value(&mut self.sym_show_imports, false, "导出");
                ui.selectable_value(&mut self.sym_show_imports, true, "导入");
            });
            ui.add_space(8.0);
            let Some(table) = self.sym_table.as_ref() else {
                ui.label(
                    egui::RichText::new("选择模块后点「解析符号」，这里会列出它的导出表和导入表。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            };
            let needle = self.sym_filter.trim().to_ascii_lowercase();
            if self.sym_show_imports {
                let mut shown = 0usize;
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for lib in &table.imports {
                            let lib_matches = needle.is_empty()
                                || lib.dll.to_ascii_lowercase().contains(&needle);
                            let symbols: Vec<&syms::Symbol> = lib
                                .symbols
                                .iter()
                                .filter(|s| {
                                    needle.is_empty()
                                        || s.name.to_ascii_lowercase().contains(&needle)
                                })
                                .collect();
                            if symbols.is_empty() && !lib_matches {
                                continue;
                            }
                            shown += symbols.len();
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}（{} 项）",
                                    lib.dll,
                                    lib.symbols.len()
                                ))
                                .size(12.0)
                                .strong()
                                .color(theme::ACCENT_2)
                                .font(FontId::monospace(12.0)),
                            );
                            for symbol in symbols {
                                theme::soft_card().show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{:016X}",
                                                symbol.address
                                            ))
                                            .size(11.5)
                                            .font(FontId::monospace(11.5))
                                            .color(theme::MUTED),
                                        );
                                        ui.label(
                                            egui::RichText::new(&symbol.name)
                                                .size(12.0)
                                                .font(FontId::monospace(12.0)),
                                        );
                                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                            if theme::outline_button(ui, "查看槽位", true).clicked() {
                                                to_open = Some(symbol.address);
                                            }
                                            if theme::outline_button(ui, "加入列表", true).clicked() {
                                                to_add = Some((
                                                    symbol.address,
                                                    format!("{}!{}", lib.dll, symbol.name),
                                                ));
                                            }
                                        });
                                    });
                                });
                                ui.add_space(2.0);
                            }
                        }
                    });
                if shown == 0 {
                    ui.label(
                        egui::RichText::new("没有匹配的导入符号。")
                            .size(12.0)
                            .color(theme::FAINT),
                    );
                }
            } else {
                let mut shown = 0usize;
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for symbol in &table.exports {
                            if !needle.is_empty()
                                && !symbol.name.to_ascii_lowercase().contains(&needle)
                            {
                                continue;
                            }
                            shown += 1;
                            theme::soft_card().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(symbol.display_name())
                                                .size(12.5)
                                                .strong()
                                                .font(FontId::monospace(12.5)),
                                        );
                                        ui.label(
                                            egui::RichText::new(if symbol.address == 0 {
                                                format!("#{} · 转发", symbol.ordinal)
                                            } else {
                                                format!(
                                                    "{:016X} · 序号 {}",
                                                    symbol.address, symbol.ordinal
                                                )
                                            })
                                            .size(11.0)
                                            .color(theme::FAINT)
                                            .font(FontId::monospace(11.0)),
                                        );
                                    });
                                    if symbol.address != 0 {
                                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                            if theme::outline_button(ui, "反汇编", true)
                                                .clicked()
                                            {
                                                to_disasm = Some(symbol.address);
                                            }
                                            if theme::outline_button(ui, "内存中查看", true)
                                                .clicked()
                                            {
                                                to_open = Some(symbol.address);
                                            }
                                            if theme::outline_button(ui, "加入列表", true)
                                                .clicked()
                                            {
                                                to_add = Some((
                                                    symbol.address,
                                                    format!("{}!{}", table.module, symbol.name),
                                                ));
                                            }
                                        });
                                    }
                                });
                            });
                            ui.add_space(3.0);
                        }
                    });
                if shown == 0 {
                    ui.label(
                        egui::RichText::new("没有匹配的导出符号。")
                            .size(12.0)
                            .color(theme::FAINT),
                    );
                }
            }
        });
        if let Some((address, desc)) = to_add {
            self.add_symbol_entry(address, desc);
        }
        if let Some(address) = to_open {
            self.open_memory_at(address);
        }
        if let Some(address) = to_disasm {
            self.jump_to_disasm(address);
        }
    }

    pub fn ui_disasm(&mut self, ui: &mut egui::Ui) {
        let has_process = self.process.is_some();
        let mut load = false;
        let mut follow: Option<u64> = None;
        let mut open_mem: Option<u64> = None;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("反汇编").size(26.0).strong());
                ui.label(
                    egui::RichText::new("把机器码读成指令，看清一段代码在做什么。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{} 条指令", self.dis_insns.len()),
                    if self.dis_loaded {
                        theme::ACCENT_2
                    } else {
                        theme::FAINT
                    },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("地址").size(11.0).color(theme::FAINT));
                ui.add_sized(
                    [260.0, 32.0],
                    egui::TextEdit::singleline(&mut self.dis_addr)
                        .font(FontId::monospace(13.0))
                        .hint_text("0x7FF6..."),
                );
                if theme::gradient_button(ui, "反汇编", 96.0, has_process).clicked() && has_process {
                    load = true;
                }
            });
            if let Some(error) = &self.dis_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
            if self.dis_loaded {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "基址 0x{:X} · 读取 {} 字节",
                        self.dis_base,
                        self.dis_bytes.len()
                    ))
                    .size(10.5)
                    .color(theme::FAINT),
                );
            }
        });
        ui.add_space(14.0);

        if !self.is_attached() {
            theme::card().show(ui, |ui| {
                empty_state(
                    ui,
                    IconKind::Bolt,
                    "还没有连接进程",
                    "连接进程后输入一个代码地址；也可以从地址列表里点「反汇编」跳过来。",
                );
            });
            return;
        }
        if self.dis_insns.is_empty() {
            theme::card().show(ui, |ui| {
                empty_state(
                    ui,
                    IconKind::Bolt,
                    "输入一个代码地址",
                    "函数入口、AOB 命中的位置、指针链落点，都可以在这里看机器码。",
                );
            });
            return;
        }

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "指令");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("字节与助记符来自目标进程的实际内存。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .max_height(460.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for insn in &self.dis_insns {
                        let (flow_color, marker) = match &insn.flow {
                            disasm::Flow::Call(_) => (theme::ACCENT_2, "↳"),
                            disasm::Flow::Jump(_) => (theme::WARN, "→"),
                            disasm::Flow::CondJump(_) => (theme::WARN, "?"),
                            disasm::Flow::Return => (theme::DANGER, "■"),
                            disasm::Flow::None => (theme::FAINT, " "),
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{marker} {:016X}", insn.address))
                                    .size(11.5)
                                    .color(flow_color)
                                    .font(FontId::monospace(11.5)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("{:<24}", insn.hex()))
                                    .size(11.5)
                                    .color(theme::FAINT)
                                    .font(FontId::monospace(11.5)),
                            );
                            ui.label(
                                egui::RichText::new(&insn.text)
                                    .size(12.5)
                                    .color(theme::TEXT)
                                    .font(FontId::monospace(12.5)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if matches!(
                                    insn.flow,
                                    disasm::Flow::Call(_) | disasm::Flow::Jump(_)
                                ) && theme::outline_button(ui, "跟过去", true).clicked()
                                {
                                    follow = match insn.flow {
                                        disasm::Flow::Call(target) => Some(target),
                                        disasm::Flow::Jump(target) => Some(target),
                                        _ => None,
                                    };
                                }
                                if theme::outline_button(ui, "内存", true).clicked() {
                                    open_mem = Some(insn.address);
                                }
                            });
                        });
                        ui.add_space(1.0);
                    }
                });
        });

        if load {
            self.load_disassembly();
        }
        if let Some(target) = follow {
            self.dis_addr = format!("{target:X}");
            self.load_disassembly();
        }
        if let Some(address) = open_mem {
            self.open_memory_at(address);
        }
    }

    pub fn ui_inject(&mut self, ui: &mut egui::Ui) {
        let has_process = self.process.is_some();
        let active_count = self.inj_patches.iter().filter(|p| p.active).count();
        let mut allocate = false;
        let mut free_cave = false;
        let mut apply = false;
        let mut revert_all = false;
        let mut revert_one: Option<usize> = None;
        let mut jump_to_disasm: Option<u64> = None;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("代码补丁").size(26.0).strong());
                ui.label(
                    egui::RichText::new("把字节写进目标进程，随时能原样还原。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{} 个生效", active_count),
                    if active_count > 0 {
                        theme::WARN
                    } else {
                        theme::FAINT
                    },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("目标地址")
                        .size(11.0)
                        .color(theme::FAINT),
                );
                ui.add_sized(
                    [220.0, 32.0],
                    egui::TextEdit::singleline(&mut self.inj_addr)
                        .font(FontId::monospace(13.0))
                        .hint_text("0x7FF6..."),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new("字节").size(11.0).color(theme::FAINT));
                ui.add_sized(
                    [280.0, 32.0],
                    egui::TextEdit::singleline(&mut self.inj_bytes)
                        .font(FontId::monospace(13.0))
                        .hint_text("90 90 C3"),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::gradient_button(ui, "写入并记录", 138.0, has_process).clicked()
                    && has_process
                {
                    apply = true;
                }
                ui.add_space(8.0);
                if theme::outline_button(ui, "全部还原", !self.inj_patches.is_empty()).clicked()
                    && !self.inj_patches.is_empty()
                {
                    revert_all = true;
                }
            });
            if let Some(error) = &self.inj_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "写入前会先读出原始字节，成功后才记录；页面保护写前抬高、写后恢复。",
                )
                .size(10.5)
                .color(theme::FAINT),
            );
        });
        ui.add_space(14.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "代码洞");
                ui.add_space(8.0);
                ui.add_sized(
                    [90.0, 28.0],
                    egui::TextEdit::singleline(&mut self.inj_cave_size)
                        .font(FontId::monospace(12.0)),
                );
                if theme::outline_button(ui, "分配", has_process).clicked() && has_process {
                    allocate = true;
                }
                if !self.inj_caves.is_empty()
                    && theme::outline_button(ui, "释放所选", has_process).clicked()
                    && has_process
                {
                    free_cave = true;
                }
            });
            ui.add_space(6.0);
            if self.inj_caves.is_empty() {
                ui.label(
                    egui::RichText::new("还没有代码洞。分配一块可执行内存，用来放跳板或自己的代码。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
            } else {
                for (index, cave) in self.inj_caves.iter().enumerate() {
                    theme::soft_card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let selected = index == self.inj_selected_cave;
                            if ui
                                .selectable_label(
                                    selected,
                                    egui::RichText::new(format!("{:016X}", cave.address))
                                        .size(12.5)
                                        .font(FontId::monospace(12.5)),
                                )
                                .clicked()
                            {
                                self.inj_selected_cave = index;
                            }
                            theme::tag(
                                ui,
                                &format!("{} 字节", cave.size),
                                if selected { theme::ACCENT } else { theme::MUTED },
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if theme::outline_button(ui, "看代码", true).clicked() {
                                    jump_to_disasm = Some(cave.address);
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            }
        });
        ui.add_space(14.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "补丁记录");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("每条都带着原始字节，可逐条还原。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(8.0);
            if self.inj_patches.is_empty() {
                ui.label(
                    egui::RichText::new("还没有补丁。填入地址和字节后点「写入并记录」。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, patch) in self.inj_patches.iter().enumerate() {
                        theme::soft_card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{:016X}", patch.address))
                                            .size(12.5)
                                            .strong()
                                            .font(FontId::monospace(12.5)),
                                    );
                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "原 {} → 现 {}（{} 字节）",
                                            patch.original_hex(),
                                            patch.written_hex(),
                                            patch.size()
                                        ))
                                        .size(10.5)
                                        .color(theme::FAINT)
                                        .font(FontId::monospace(10.5)),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    theme::tag(
                                        ui,
                                        if patch.active { "生效中" } else { "已还原" },
                                        if patch.active {
                                            theme::WARN
                                        } else {
                                            theme::SUCCESS
                                        },
                                    );
                                    if patch.active
                                        && theme::outline_button(ui, "还原", true).clicked()
                                    {
                                        revert_one = Some(index);
                                    }
                                    if theme::outline_button(ui, "看代码", true).clicked() {
                                        jump_to_disasm = Some(patch.address);
                                    }
                                });
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
        });

        if allocate {
            self.allocate_cave();
        }
        if free_cave {
            self.free_selected_cave();
        }
        if apply {
            self.apply_patch();
        }
        if revert_all {
            self.revert_all_patches();
        }
        if let Some(index) = revert_one {
            self.revert_patch(index);
        }
        if let Some(address) = jump_to_disasm {
            self.jump_to_disasm(address);
        }
    }

    pub fn ui_asm(&mut self, ui: &mut egui::Ui) {
        let mut assemble = false;
        let mut send = false;
        let (bytes_len, label_count) = match &self.asm_result {
            Some(r) => (r.bytes.len(), r.labels.len()),
            None => (0, 0),
        };

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("自动汇编").size(26.0).strong());
                ui.label(
                    egui::RichText::new("把汇编文本编译成机器码，带标签，可直接送去打补丁。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{bytes_len} 字节 · {label_count} 标签"),
                    if self.asm_result.is_some() {
                        theme::ACCENT_2
                    } else {
                        theme::FAINT
                    },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("起始地址")
                        .size(11.0)
                        .color(theme::FAINT),
                );
                ui.add_sized(
                    [200.0, 30.0],
                    egui::TextEdit::singleline(&mut self.asm_origin)
                        .font(FontId::monospace(12.5))
                        .hint_text("0x7FF6..."),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("标签地址从起始地址算起")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::gradient_button(ui, "汇编", 96.0, true).clicked() {
                    assemble = true;
                }
                if theme::outline_button(ui, "送去打补丁", self.asm_result.is_some()).clicked()
                    && self.asm_result.is_some()
                {
                    send = true;
                }
                if theme::outline_button(ui, "填入示例", true).clicked() {
                    self.asm_source =
                        "; 给一个函数开头打补丁\nstart:\n  push rbp\n  mov rbp, rsp\n  sub rsp, 0x40\n  mov eax, 1\n  jmp start"
                            .to_string();
                }
            });
            if let Some(error) = &self.asm_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
        });
        ui.add_space(12.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "汇编源");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("支持标签、注释（; 或 //）、0x 十六进制、[基址+索引*缩放+偏移]。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_source("zero_asm_source")
                .max_height(240.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 220.0],
                        egui::TextEdit::multiline(&mut self.asm_source)
                            .font(FontId::monospace(13.0))
                            .desired_width(f32::INFINITY)
                            .hint_text("mov rax, rbx"),
                    );
                });
        });
        ui.add_space(12.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "机器码");
                ui.add_space(4.0);
            });
            ui.add_space(8.0);
            let Some(result) = self.asm_result.as_ref() else {
                ui.label(
                    egui::RichText::new("还没有结果。写好汇编后点「汇编」。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            };
            if !result.labels.is_empty() {
                ui.horizontal(|ui| {
                    for (name, address) in &result.labels {
                        theme::tag(ui, &format!("{name} = {:X}", address), theme::ACCENT);
                    }
                });
                ui.add_space(6.0);
            }
            ui.label(
                egui::RichText::new(result.hex())
                    .size(12.5)
                    .color(theme::TEXT)
                    .font(FontId::monospace(12.5)),
            );
            ui.add_space(8.0);
            theme::divider(ui);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("回读校验（反汇编这些字节）")
                    .size(10.5)
                    .color(theme::FAINT),
            );
            ui.add_space(4.0);
            let back = disasm::disassemble(&result.bytes, result.origin, true, 64);
            egui::ScrollArea::vertical()
                .id_source("zero_asm_readback")
                .max_height(200.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for insn in &back {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{:016X}", insn.address))
                                    .size(11.0)
                                    .color(theme::ACCENT)
                                    .font(FontId::monospace(11.0)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("{:<20}", insn.hex()))
                                    .size(11.0)
                                    .color(theme::FAINT)
                                    .font(FontId::monospace(11.0)),
                            );
                            ui.label(
                                egui::RichText::new(&insn.text)
                                    .size(12.0)
                                    .color(theme::TEXT)
                                    .font(FontId::monospace(12.0)),
                            );
                        });
                    }
                });
        });

        if assemble {
            self.assemble_source();
        }
        if send {
            self.send_asm_to_patch();
        }
    }

    pub fn ui_table(&mut self, ui: &mut egui::Ui) {
        let count = self.ct_table.as_ref().map(|t| t.entries.len()).unwrap_or(0);
        let mut open = false;
        let mut save = false;
        let mut import = false;
        let mut export = false;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("CT 表格").size(26.0).strong());
                ui.label(
                    egui::RichText::new("读写 Cheat Engine 的 .ct 文件，和原版表格互通。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::status_chip(
                    ui,
                    &format!("{count} 条"),
                    if count > 0 { theme::ACCENT_2 } else { theme::FAINT },
                );
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("文件").size(11.0).color(theme::FAINT));
                ui.add_sized(
                    [380.0, 32.0],
                    egui::TextEdit::singleline(&mut self.ct_path)
                        .font(FontId::monospace(12.5))
                        .hint_text("C:\\tables\\game.ct"),
                );
                if theme::outline_button(ui, "打开", true).clicked() {
                    open = true;
                }
                if theme::outline_button(ui, "保存", count > 0).clicked() && count > 0 {
                    save = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::gradient_button(ui, "导入到地址列表", 168.0, count > 0).clicked()
                    && count > 0
                {
                    import = true;
                }
                ui.add_space(8.0);
                if theme::outline_button(ui, "从地址列表生成", !self.entries.is_empty()).clicked()
                    && !self.entries.is_empty()
                {
                    export = true;
                }
            });
            if let Some(error) = &self.ct_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
            if !self.ct_status.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(&self.ct_status)
                        .size(10.5)
                        .color(theme::SUCCESS),
                );
            }
        });
        ui.add_space(12.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "条目");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("地址支持模块+偏移、[指针]+偏移 与绝对地址。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(8.0);
            let Some(table) = self.ct_table.as_ref() else {
                ui.label(
                    egui::RichText::new("还没有打开表格。填入 .ct 路径后点「打开」。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            };
            if table.entries.is_empty() {
                ui.label(
                    egui::RichText::new("这个表格里没有 cheat entry。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_source("zero_ct_entries")
                .max_height(400.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in &table.entries {
                        theme::soft_card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(if entry.description.is_empty() {
                                            "(无描述)".to_string()
                                        } else {
                                            entry.description.clone()
                                        })
                                        .size(12.5)
                                        .strong(),
                                    );
                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{}  ·  {}",
                                            entry.address, entry.vtype.short()
                                        ))
                                        .size(10.5)
                                        .color(theme::FAINT)
                                        .font(FontId::monospace(10.5)),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    theme::tag(
                                        ui,
                                        entry.vtype.label(),
                                        theme::MUTED,
                                    );
                                    if let Some(value) = &entry.value {
                                        theme::tag(ui, value, theme::WARN);
                                    }
                                });
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
        });

        if open {
            self.open_ct_table();
        }
        if save {
            self.save_ct_table();
        }
        if import {
            self.import_ct_entries();
        }
        if export {
            self.export_entries_to_ct();
        }
    }

    pub fn ui_speed(&mut self, ui: &mut egui::Ui) {
        let running = self.speed_on;
        let attached = self.is_attached();
        let mut start = false;
        let mut stop = false;
        let mut preset: Option<f64> = None;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("变速齿轮").size(26.0).strong());
                ui.label(
                    egui::RichText::new("周期性暂停目标进程的线程，让游戏变慢或变快。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if running {
                    theme::status_chip(ui, "变速中", theme::WARN);
                } else {
                    theme::status_chip(ui, "未启用", theme::FAINT);
                }
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "倍率");
                ui.add_space(8.0);
                ui.add(
                    egui::Slider::new(&mut self.speed_factor, 0.1..=10.0)
                        .step_by(0.05)
                        .fixed_decimals(2)
                        .suffix("x"),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (label, value) in [
                    ("0.25x 极慢", 0.25),
                    ("0.5x 慢", 0.5),
                    ("1x 正常", 1.0),
                    ("2x 快", 2.0),
                    ("4x 极快", 4.0),
                ] {
                    if theme::outline_button(ui, label, true).clicked() {
                        preset = Some(value);
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if !running {
                    if theme::gradient_button(ui, "开启变速", 128.0, attached).clicked() && attached {
                        start = true;
                    }
                } else if theme::outline_button(ui, "停止并恢复", true).clicked() {
                    stop = true;
                }
                ui.add_space(10.0);
                if running {
                    theme::status_chip(
                        ui,
                        &format!("{} 个线程", self.speed_threads),
                        theme::ACCENT_2,
                    );
                }
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(self.speed_description())
                    .size(10.5)
                    .color(theme::FAINT),
            );
            if let Some(error) = &self.speed_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
            }
        });
        ui.add_space(12.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section_label(ui, "说明");
            ui.add_space(6.0);
            for line in [
                "原理：每 40 ms 为一个周期，先把目标进程的所有线程挂起，再放行其中一小段时间。",
                "放行时间与周期的比例就是倍率：0.5x 只放行四分之一，2x 放行三分之二。",
                "每个周期结束都会把挂起的线程全部恢复，不会留下卡死的进程。",
                "不能对自己挂起（Zero 自己就是被挂起的进程时会直接拒绝）。",
            ] {
                ui.label(
                    egui::RichText::new(format!("· {line}"))
                        .size(11.0)
                        .color(theme::MUTED),
                );
                ui.add_space(2.0);
            }
        });

        if let Some(value) = preset {
            self.speed_factor = value;
        }
        if start {
            self.start_speed();
        }
        if stop {
            self.stop_speed();
        }
    }

    pub fn ui_script(&mut self, ui: &mut egui::Ui) {
        let mut run = false;
        let attached = self.is_attached();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Lua 脚本").size(26.0).strong());
                ui.label(
                    egui::RichText::new("执行 Cheat Engine 风格的 Lua 脚本，直接读写目标进程。")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if attached {
                    theme::status_chip(ui, "已连接", theme::SUCCESS);
                } else {
                    theme::status_chip(ui, "未连接", theme::FAINT);
                }
            });
        });
        ui.add_space(4.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if theme::gradient_button(ui, "运行", 96.0, true).clicked() {
                    run = true;
                }
                if theme::outline_button(ui, "填入示例", true).clicked() {
                    self.script_source = "\n-- 读写目标进程\nlocal addr = getAddress(\"game.exe+1A2B\")\nprint(\"生命值地址\", string.format(\"%X\", addr))\nprint(\"当前值\", readInteger(addr, 4))\nwriteInteger(addr, 100, 4)\nprint(\"已写入 100\")"
                        .to_string();
                }
                ui.add_space(10.0);
                if !attached {
                    ui.label(
                        egui::RichText::new("未连接进程时只能跑纯计算脚本。")
                            .size(10.5)
                            .color(theme::FAINT),
                    );
                }
            });
        });
        ui.add_space(12.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_label(ui, "脚本");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("可用：print / readInteger / writeInteger / readBytes / writeBytes / getAddress")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_source("zero_script_source")
                .max_height(260.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 240.0],
                        egui::TextEdit::multiline(&mut self.script_source)
                            .font(FontId::monospace(13.0))
                            .desired_width(f32::INFINITY)
                            .hint_text("print(\"hello\")"),
                    );
                });
        });
        ui.add_space(12.0);

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section_label(ui, "输出");
            ui.add_space(6.0);
            if let Some(error) = &self.script_error {
                ui.label(
                    egui::RichText::new(error)
                        .size(11.5)
                        .color(theme::DANGER)
                        .font(FontId::monospace(11.5)),
                );
                return;
            }
            if self.script_output.is_empty() {
                ui.label(
                    egui::RichText::new("还没有输出。")
                        .size(12.0)
                        .color(theme::FAINT),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_source("zero_script_output")
                .max_height(260.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &self.script_output {
                        ui.label(
                            egui::RichText::new(line)
                                .size(12.0)
                                .color(theme::TEXT)
                                .font(FontId::monospace(12.0)),
                        );
                    }
                });
        });

        if run {
            self.run_script();
        }
    }

    /// The watch list: the few addresses most worth staring at.
    fn watch_panel(&mut self, ui: &mut egui::Ui) {
        let mut start = false;
        let mut stop = false;
        let mut promote = false;
        let running = self.watch_running;
        let count = self.watches.len();

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("观察队列").size(14.0).strong());
                if count > 0 {
                    theme::tag(ui, &format!("{count} 个"), theme::ACCENT);
                }
                if running {
                    theme::tag(ui, "采样中", theme::SUCCESS);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if count > 0 && theme::outline_button(ui, "选取最佳", running).clicked() && running
                    {
                        promote = true;
                    }
                    if running {
                        if theme::outline_button(ui, "停止", true).clicked() {
                            stop = true;
                        }
                    } else if !self.ranked.is_empty()
                        && theme::gradient_button(ui, "观察最可能的", 132.0, true).clicked()
                    {
                        start = true;
                    }
                });
            });
            ui.add_space(6.0);

            if count == 0 {
                ui.label(
                    egui::RichText::new(
                        "扫描完成后，可以把最可能的几个地址放进观察队列：改变游戏里的数值，看哪个跟着动。",
                    )
                    .size(11.5)
                    .color(theme::FAINT),
                );
                return;
            }

            for watch in &self.watches {
                ui.horizontal(|ui| {
                    theme::tag(
                        ui,
                        match watch.behaviour_rank() {
                            3 => "持续变化",
                            2 => "偶尔变化",
                            1 => "一直没变",
                            _ => "未采样",
                        },
                        match watch.behaviour_rank() {
                            3 => theme::ACCENT_2,
                            2 => theme::SUCCESS,
                            1 => theme::FAINT,
                            _ => theme::FAINT,
                        },
                    );
                    ui.label(
                        egui::RichText::new(format!("{:016X}", watch.address))
                            .size(11.5)
                            .font(FontId::monospace(11.5))
                            .color(theme::ACCENT),
                    );
                    theme::tag(ui, watch.vtype.short(), theme::MUTED);
                    let current = watch
                        .current
                        .as_ref()
                        .map(|v| v.display())
                        .unwrap_or_else(|| "?".to_string());
                    ui.label(
                        egui::RichText::new(current)
                            .size(12.0)
                            .font(FontId::monospace(12.0))
                            .color(theme::TEXT),
                    );
                    if let Some(delta) = watch.delta() {
                        if delta.abs() > f64::EPSILON {
                            ui.label(
                                egui::RichText::new(format!("{delta:+.0}"))
                                    .size(11.0)
                                    .font(FontId::monospace(11.0))
                                    .color(if delta > 0.0 {
                                        theme::WARN
                                    } else {
                                        theme::DANGER
                                    }),
                            );
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{} 分", watch.score))
                                .size(10.0)
                                .color(theme::FAINT),
                        );
                    });
                });
                ui.add_space(3.0);
            }
        });

        if start {
            self.start_watching();
        }
        if stop {
            self.stop_watching();
        }
        if promote {
            self.promote_best_watch();
        }
    }

    /// The MCP switch: on/off, the URL to paste into an agent, and a copy of
    /// the token. Deliberately visible and one click away, like a mobile tool's
    /// service toggle.
    pub fn mcp_panel(&mut self, ui: &mut egui::Ui) {
        let running = self.mcp.is_some();
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                theme::paint_icon(ui.painter(), rect.shrink(2.0), IconKind::Bolt, theme::ACCENT_2);
                ui.label(egui::RichText::new("MCP 服务").size(14.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if running {
                        theme::tag(ui, "运行中", theme::SUCCESS);
                    } else {
                        theme::tag(ui, "已关闭", theme::FAINT);
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "让外部 AI 助手通过 MCP 驱动 Zero 扫描与读写内存。关闭时端口不监听。",
                )
                .size(11.0)
                .color(theme::MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::gradient_button(
                    ui,
                    if running { "停止服务" } else { "启动服务" },
                    112.0,
                    true,
                )
                .clicked()
                {
                    self.toggle_mcp();
                }
                if !running {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("端口").size(10.5).color(theme::FAINT));
                    ui.add(
                        egui::DragValue::new(&mut self.mcp_port)
                            .clamp_range(1024..=65_000)
                            .speed(1.0),
                    );
                }
            });
            if let Some(service) = &self.mcp {
                ui.add_space(8.0);
                let url = service.url();
                ui.label(egui::RichText::new("连接地址").size(10.5).color(theme::FAINT));
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&url)
                            .size(10.5)
                            .font(FontId::monospace(10.5))
                            .color(theme::TEXT),
                    );
                    if theme::outline_button(ui, "复制", true).clicked() {
                        ui.output_mut(|o| o.copied_text = url.clone());
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("POST 到该地址，JSON-RPC 2.0。GET 可做健康检查。")
                        .size(10.0)
                        .color(theme::FAINT),
                );
            }
            if let Some(error) = &self.mcp_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(10.5).color(theme::DANGER));
            }
        });
    }

    /// Shows what the one-click verification found. Honest about what it did
    /// not cover: the full 20-item regression needs its own target program.
    pub fn e2e_panel(&mut self, ctx: &egui::Context) {
        if self.e2e_log.is_empty() {
            return;
        }
        let mut open = true;
        egui::Window::new("验证结果")
            .id(egui::Id::new("zero_e2e"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                Frame::none()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(Margin::same(20.0)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("验证结果").size(18.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        match self.e2e_ok {
                            Some(true) => theme::tag(ui, "全部通过", theme::SUCCESS),
                            Some(false) => theme::tag(ui, "有失败项", theme::DANGER),
                            None => theme::tag(ui, "未运行", theme::FAINT),
                        };
                    });
                });
                ui.add_space(8.0);
                theme::divider(ui);
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_source("zero_e2e_log")
                    .max_height(360.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in self.e2e_log.lines() {
                            let colour = if line.starts_with("PASS") {
                                theme::SUCCESS
                            } else if line.starts_with("FAIL") {
                                theme::DANGER
                            } else {
                                theme::MUTED
                            };
                            ui.label(
                                egui::RichText::new(line)
                                    .size(11.5)
                                    .color(colour)
                                    .font(FontId::monospace(11.5)),
                            );
                        }
                    });
            });
        if !open {
            self.e2e_log.clear();
        }
    }

    /// The shortcut cheat-sheet, shown over everything else on demand.
    pub fn ui_hotkeys(&mut self, ctx: &egui::Context) {
        if !self.show_hotkeys {
            return;
        }
        let mut open = true;
        egui::Window::new("快捷键")
            .id(egui::Id::new("zero_hotkeys"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                Frame::none()
                    .fill(theme::CARD)
                    .stroke(Stroke::new(1.0, theme::BORDER_SOFT))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(Margin::symmetric(22.0, 18.0)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(
                    egui::RichText::new("照着 Cheat Engine 的习惯排的")
                        .size(11.0)
                        .color(theme::FAINT),
                );
                ui.add_space(10.0);
                for shortcut in hotkey::SHORTCUTS {
                    ui.horizontal(|ui| {
                        theme::tag(ui, shortcut.label, theme::ACCENT);
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(shortcut.description)
                                .size(12.0)
                                .color(theme::TEXT),
                        );
                    });
                    ui.add_space(5.0);
                }
                ui.add_space(6.0);
                theme::divider(ui);
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("输入框有焦点时不会抢按键；按 F1 或 Esc 关闭。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
        if !open {
            self.show_hotkeys = false;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_hotkeys = false;
        }
    }

    pub fn ui_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("zero_toasts"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-20.0, 82.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    let (color, label) = theme::toast_style(toast.kind);
                    Frame::none()
                        .fill(theme::CARD)
                        .stroke(Stroke::new(1.0, color.gamma_multiply(0.55)))
                        .rounding(Rounding::same(11.0))
                        .inner_margin(Margin::symmetric(14.0, 10.0))
                        .shadow(egui::epaint::Shadow {
                            offset: Vec2::new(0.0, 8.0),
                            blur: 22.0,
                            spread: 0.0,
                            color: Color32::from_black_alpha(110),
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(320.0);
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                                ui.painter().circle_filled(rect.center(), 9.0, color.gamma_multiply(0.18));
                                ui.painter().text(
                                    rect.center(),
                                    Align2::CENTER_CENTER,
                                    "!",
                                    FontId::proportional(11.0),
                                    color,
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(label)
                                            .size(10.0)
                                            .color(color)
                                            .strong(),
                                    );
                                    ui.label(
                                        egui::RichText::new(&toast.text)
                                            .size(11.5)
                                            .color(theme::TEXT),
                                    );
                                });
                            });
                        });
                    ui.add_space(8.0);
                }
            });
    }

    pub fn ui_modals(&mut self, ctx: &egui::Context) {
        if self.picker_open {
            self.process_picker(ctx);
        }
        if self.show_settings {
            self.settings_modal(ctx);
        }
        if self.show_add {
            self.add_modal(ctx);
        }
        if self.editing.is_some() {
            self.edit_modal(ctx);
        }
    }

    fn process_picker(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut refresh = false;
        let mut attach = None;
        egui::Window::new("process_picker")
            .id(egui::Id::new("zero_process_picker"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(760.0, 610.0))
            .frame(
                Frame::none()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .rounding(Rounding::same(16.0))
                    .inner_margin(Margin::same(18.0))
                    .shadow(egui::epaint::Shadow {
                        offset: Vec2::new(0.0, 20.0),
                        blur: 60.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(150),
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("选择进程").size(20.0).strong());
                        ui.label(
                            egui::RichText::new("选择一个正在运行的游戏或练习靶场")
                                .size(11.5)
                                .color(theme::MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(ui, IconKind::Close).clicked() {
                            close = true;
                        }
                        if theme::icon_button(ui, IconKind::Refresh)
                            .on_hover_text("刷新进程列表")
                            .clicked()
                        {
                            refresh = true;
                        }
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [ui.available_width() - 90.0, 34.0],
                        egui::TextEdit::singleline(&mut self.picker_search)
                            .hint_text("搜索进程名或 PID")
                            .font(FontId::proportional(13.0)),
                    );
                    ui.label(
                        egui::RichText::new(format!("{} 个进程", self.processes.len()))
                            .size(10.5)
                            .color(theme::FAINT),
                    );
                });
                ui.add_space(6.0);
                theme::divider(ui);
                if !self.picker_loaded {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("正在读取进程…").size(12.0).color(theme::MUTED));
                    });
                    return;
                }
                let needle = self.picker_search.to_ascii_lowercase();
                let filtered: Vec<usize> = self
                    .processes
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        needle.is_empty()
                            || p.name.to_ascii_lowercase().contains(&needle)
                            || p.pid.to_string().contains(&needle)
                    })
                    .map(|(i, _)| i)
                    .collect();
                if filtered.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("没有匹配的进程")
                                .size(13.0)
                                .color(theme::MUTED),
                        );
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .auto_shrink([false, false])
                        .show_rows(ui, 50.0, filtered.len(), |ui, range| {
                            for row in range {
                                let info = &self.processes[filtered[row]];
                                let practice = self.practice_pid == Some(info.pid);
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 46.0),
                                    Sense::click(),
                                );
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        Rounding::same(9.0),
                                        Color32::from_white_alpha(5),
                                    );
                                }
                                let icon = egui::Rect::from_center_size(
                                    rect.left_center() + Vec2::new(24.0, 0.0),
                                    Vec2::splat(28.0),
                                );
                                ui.painter().rect_filled(
                                    icon,
                                    Rounding::same(8.0),
                                    if practice {
                                        theme::ACCENT_2.gamma_multiply(0.16)
                                    } else {
                                        theme::ACCENT.gamma_multiply(0.10)
                                    },
                                );
                                theme::paint_icon(
                                    ui.painter(),
                                    icon.shrink(7.0),
                                    IconKind::Practice,
                                    if practice { theme::ACCENT_2 } else { theme::ACCENT },
                                );
                                ui.painter().text(
                                    rect.left_center() + Vec2::new(48.0, -8.0),
                                    Align2::LEFT_CENTER,
                                    if info.name.is_empty() { "系统进程" } else { &info.name },
                                    FontId::proportional(12.5),
                                    theme::TEXT,
                                );
                                let detail = format!(
                                    "PID {}  ·  {}{}",
                                    info.pid,
                                    info.arch,
                                    if practice { "  ·  练习靶场" } else { "" }
                                );
                                ui.painter().text(
                                    rect.left_center() + Vec2::new(48.0, 9.0),
                                    Align2::LEFT_CENTER,
                                    detail,
                                    FontId::proportional(10.0),
                                    if practice { theme::ACCENT_2 } else { theme::FAINT },
                                );
                                if ui
                                    .put(
                                        egui::Rect::from_center_size(
                                            rect.right_center() - Vec2::new(46.0, 0.0),
                                            Vec2::new(72.0, 28.0),
                                        ),
                                        egui::Button::new("连接").min_size(Vec2::new(72.0, 28.0)),
                                    )
                                    .clicked()
                                {
                                    attach = Some(info.pid);
                                } else if response.double_clicked() {
                                    attach = Some(info.pid);
                                }
                            }
                        });
                }
                ui.add_space(4.0);
                theme::divider(ui);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(if self.processes.is_empty() {
                            "提示：有些系统进程需要管理员权限才能读取。"
                        } else {
                            "提示：游戏以管理员运行时，Zero 也需要管理员权限。"
                        })
                        .size(10.5)
                        .color(theme::FAINT),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::outline_button(ui, "以管理员身份重启", true).clicked() {
                            winmem::relaunch_as_admin();
                        }
                    });
                });
                if let Some(error) = &self.attach_error {
                    ui.label(egui::RichText::new(error).size(11.0).color(theme::DANGER));
                }
            });
        if close {
            self.picker_open = false;
        }
        if refresh {
            self.refresh_processes();
        }
        if let Some(pid) = attach {
            if let Some(info) = self.processes.iter().find(|p| p.pid == pid).cloned() {
                self.attach_info(&info);
            }
        }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut admin = false;
        egui::Window::new("settings")
            .id(egui::Id::new("zero_settings"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(540.0, 480.0))
            .frame(
                Frame::none()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .rounding(Rounding::same(16.0))
                    .inner_margin(Margin::same(20.0))
                    .shadow(egui::epaint::Shadow {
                        offset: Vec2::new(0.0, 20.0),
                        blur: 60.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(150),
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("扫描设置").size(20.0).strong());
                        ui.label(
                            egui::RichText::new("这些选项会立即影响下一次扫描")
                                .size(11.5)
                                .color(theme::MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(ui, IconKind::Close).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(10.0);
                theme::soft_card().show(ui, |ui| {
                    ui.checkbox(&mut self.fast, "快速扫描（按 4 字节对齐，速度更快）");
                    ui.checkbox(&mut self.writable_only, "只扫描可写内存（减少无关结果）");
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("最多保留结果").size(12.0));
                        ui.add(
                            egui::DragValue::new(&mut self.max_results)
                                .clamp_range(10_000..=2_000_000)
                                .speed(1_000),
                        );
                        ui.label(egui::RichText::new("个").size(11.0).color(theme::FAINT));
                    });
                    ui.label(
                        egui::RichText::new("默认值：快速扫描开启，仅可写内存，最多 200,000 个结果。")
                            .size(10.0)
                            .color(theme::FAINT),
                    );
                });
                ui.add_space(10.0);
                self.mcp_panel(ui);
                ui.add_space(10.0);
                theme::soft_card().show(ui, |ui| {
                    ui.label(egui::RichText::new("关于 Zero").size(12.5).strong());
                    ui.label(
                        egui::RichText::new(
                            "Zero 是一个独立重制的内存扫描工作台，面向单人 / 离线游戏的学习与调试。它不注入进程、不绕过反作弊，也不上传任何数据。",
                        )
                        .size(10.5)
                        .color(theme::MUTED),
                    );
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if theme::outline_button(ui, "以管理员身份重启", true).clicked() {
                        admin = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::gradient_button(ui, "完成", 92.0, true).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.show_settings = false;
        }
        if admin {
            winmem::relaunch_as_admin();
        }
    }

    fn add_modal(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut add = false;
        egui::Window::new("add_address")
            .id(egui::Id::new("zero_add_address"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(520.0, 470.0))
            .frame(
                Frame::none()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .rounding(Rounding::same(16.0))
                    .inner_margin(Margin::same(20.0))
                    .shadow(egui::epaint::Shadow {
                        offset: Vec2::new(0.0, 20.0),
                        blur: 60.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(150),
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("手动添加地址").size(20.0).strong());
                        ui.label(
                            egui::RichText::new("支持 0x 开头的十六进制地址")
                                .size(11.5)
                                .color(theme::MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(ui, IconKind::Close).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(10.0);
                field_label(ui, "地址");
                ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut self.manual_address)
                        .font(FontId::monospace(13.0))
                        .hint_text("0x7FF6A1B2C3D4"),
                );
                field_label(ui, "描述（可选）");
                ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut self.manual_desc)
                        .hint_text("例如：生命值"),
                );
                field_label(ui, "类型");
                ui.horizontal_wrapped(|ui| {
                    for ty in [VType::I32, VType::I64, VType::F32, VType::F64, VType::I16, VType::I8, VType::Str] {
                        if theme::pill(ui, ty.label(), self.manual_type == ty).clicked() {
                            self.manual_type = ty;
                        }
                    }
                });
                field_label(ui, "当前值（可选，填写后会立即写入）");
                ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut self.manual_value)
                        .font(FontId::monospace(13.0))
                        .hint_text("例如 100"),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if theme::gradient_button(ui, "添加地址", 130.0, true).clicked() {
                        add = true;
                    }
                    ui.label(
                        egui::RichText::new(if self.is_attached() {
                            "写入会立即生效"
                        } else {
                            "未连接进程，只会保存地址"
                        })
                        .size(10.0)
                        .color(theme::FAINT),
                    );
                });
            });
        if close {
            self.show_add = false;
        }
        if add {
            self.add_manual_entry();
        }
    }

    fn edit_modal(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut save = false;
        let mut take: Option<EditState> = None;
        if let Some(edit) = self.editing.as_mut() {
            egui::Window::new("edit_address")
                .id(egui::Id::new("zero_edit_address"))
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .fixed_size(Vec2::new(520.0, 440.0))
                .frame(
                    Frame::none()
                        .fill(theme::PANEL)
                        .stroke(Stroke::new(1.0, theme::BORDER))
                        .rounding(Rounding::same(16.0))
                        .inner_margin(Margin::same(20.0))
                        .shadow(egui::epaint::Shadow {
                            offset: Vec2::new(0.0, 20.0),
                            blur: 60.0,
                            spread: 0.0,
                            color: Color32::from_black_alpha(150),
                        }),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("编辑地址").size(20.0).strong());
                            ui.label(
                                egui::RichText::new("修改会立即写入目标进程")
                                    .size(11.5)
                                    .color(theme::WARN),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if theme::icon_button(ui, IconKind::Close).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(10.0);
                    field_label(ui, "地址");
                    ui.add_sized(
                        [ui.available_width(), 32.0],
                        egui::TextEdit::singleline(&mut edit.address).font(FontId::monospace(13.0)),
                    );
                    field_label(ui, "描述");
                    ui.add_sized(
                        [ui.available_width(), 32.0],
                        egui::TextEdit::singleline(&mut edit.desc),
                    );
                    field_label(ui, "类型");
                    ui.horizontal_wrapped(|ui| {
                        for ty in [VType::I32, VType::I64, VType::F32, VType::F64, VType::I16, VType::I8, VType::Str] {
                            if theme::pill(ui, ty.label(), edit.vtype == ty).clicked() {
                                edit.vtype = ty;
                            }
                        }
                    });
                    field_label(ui, "值");
                    ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::TextEdit::singleline(&mut edit.value)
                            .font(FontId::monospace(16.0)),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if theme::gradient_button(ui, "保存并写入", 138.0, true).clicked() {
                            save = true;
                        }
                        if theme::outline_button(ui, "取消", true).clicked() {
                            close = true;
                        }
                    });
                });
        } else {
            close = true;
        }
        if close {
            self.editing = None;
        }
        if save {
            take = self.editing.take();
            self.editing = take;
            self.save_editor();
        }
    }
}

fn cell_label(ui: &mut egui::Ui, width: f32, text: &str, muted: bool) {
    cell_text(
        ui,
        width,
        text,
        FontId::proportional(11.5),
        if muted { theme::MUTED } else { theme::TEXT },
    );
}

fn cell_mono(ui: &mut egui::Ui, width: f32, text: &str, color: Color32) {
    cell_text(ui, width, text, FontId::monospace(11.5), color);
}

fn cell_text(ui: &mut egui::Ui, width: f32, text: &str, font: FontId, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let clipped = ui.painter().with_clip_rect(rect);
    clipped.galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

fn cell_tag(ui: &mut egui::Ui, width: f32, text: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(10.5), color);
    let tag_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        Vec2::new(galley.size().x + 14.0, 19.0),
    );
    ui.painter()
        .rect_filled(tag_rect, Rounding::same(5.0), color.gamma_multiply(0.14));
    let clipped = ui.painter().with_clip_rect(rect);
    clipped.galley(
        tag_rect.center() - galley.size() / 2.0,
        galley,
        color,
    );
}

fn cell_button(ui: &mut egui::Ui, width: f32, text: &str, color: Color32) -> Response {
    ui.add_sized(
        [width, 24.0],
        egui::Button::new(egui::RichText::new(text).size(11.0).color(color)).frame(false),
    )
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).size(10.5).color(theme::FAINT));
}

fn type_color(ty: VType) -> Color32 {
    match ty {
        VType::F32 | VType::F64 => theme::ACCENT_2,
        VType::Str => theme::WARN,
        VType::Auto => theme::ACCENT,
        _ => theme::ACCENT,
    }
}

fn format_elapsed(duration: Duration) -> String {
    if duration.as_secs_f32() >= 1.0 {
        format!("{:.2}s", duration.as_secs_f32())
    } else {
        format!("{}ms", duration.as_millis())
    }
}
