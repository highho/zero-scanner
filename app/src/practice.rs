use crate::theme;
use eframe::egui;
use std::time::{Duration, Instant};

pub struct PracticeApp {
    stats: Box<Stats>,
    auto: bool,
    last_tick: Instant,
    log: Vec<String>,
}

struct Stats {
    health: i32,
    ammo: i32,
    coins: i32,
    armor: f32,
}

impl PracticeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::setup_fonts(&cc.egui_ctx);
        theme::init();
        theme::apply_style(&cc.egui_ctx);
        Self {
            stats: Box::new(Stats {
                health: 100,
                ammo: 30,
                coins: 250,
                armor: 0.75,
            }),
            auto: false,
            last_tick: Instant::now(),
            log: vec!["靶场已就绪。回到 Zero，扫描这里看到的数值。".to_string()],
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        self.log.push(text.into());
        if self.log.len() > 6 {
            self.log.remove(0);
        }
    }

    fn stat_card(&mut self, ui: &mut egui::Ui, title: &str, value: String, unit: &str, hint: &str, minus: &str, plus: &str, down: i32, up: i32) {
        theme::soft_card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    theme::section_label(ui, title);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(value)
                                .size(34.0)
                                .strong()
                                .color(theme::text()),
                        );
                        ui.label(egui::RichText::new(unit).size(12.0).color(theme::faint()));
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(plus).min_size(egui::vec2(34.0, 30.0))).clicked() {
                            let _ = up;
                        }
                        if ui.add(egui::Button::new(minus).min_size(egui::vec2(34.0, 30.0))).clicked() {
                            let _ = down;
                        }
                    });
                });
            });
            ui.label(egui::RichText::new(hint).size(11.5).color(theme::faint()));
        });
    }
}

impl eframe::App for PracticeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.auto && self.last_tick.elapsed() > Duration::from_millis(1000) {
            self.last_tick = Instant::now();
            self.stats.health = (self.stats.health - 2).max(1);
            self.stats.coins += 3;
            self.note("自动模拟：生命 -2，金币 +3");
        }
        ctx.request_repaint_after(Duration::from_millis(if self.auto { 250 } else { 1000 }));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(theme::bg())
                    .inner_margin(egui::Margin::same(22.0)),
            )
            .show(ctx, |ui| {
                theme::draw_background(ui);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::hover());
                    theme::logo(ui.painter(), rect);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("练习靶场").size(22.0).strong());
                        ui.label(
                            egui::RichText::new("一个可以安全练习扫描的小程序")
                                .size(12.0)
                                .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(6.0);
                theme::divider(ui);
                ui.add_space(6.0);

                let total = self.stats.health as f64 + self.stats.ammo as f64 + self.stats.coins as f64;
                theme::card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            theme::section_label(ui, "当前状态");
                            ui.label(
                                egui::RichText::new(format!("{:.0}", total))
                                    .size(30.0)
                                    .strong()
                                    .color(theme::accent_2()),
                            );
                            ui.label(egui::RichText::new("这是靶场自己的内存，随便改。").size(11.5).color(theme::faint()));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut self.auto, "自动变化");
                        });
                    });
                });
                ui.add_space(8.0);

                theme::soft_card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            theme::section_label(ui, "生命值");
                            ui.label(egui::RichText::new(self.stats.health.to_string()).size(34.0).strong());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new("受伤 -10").min_size(egui::vec2(78.0, 30.0))).clicked() {
                                    self.stats.health = (self.stats.health - 10).max(1);
                                    self.note("生命 -10");
                                }
                                if ui.add(egui::Button::new("治疗 +10").min_size(egui::vec2(78.0, 30.0))).clicked() {
                                    self.stats.health = (self.stats.health + 10).min(200);
                                    self.note("生命 +10");
                                }
                            });
                        });
                    });
                });
                ui.add_space(8.0);

                theme::soft_card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            theme::section_label(ui, "弹药");
                            ui.label(egui::RichText::new(self.stats.ammo.to_string()).size(34.0).strong());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new("射击 -1").min_size(egui::vec2(78.0, 30.0))).clicked() {
                                    self.stats.ammo = (self.stats.ammo - 1).max(0);
                                    self.note("弹药 -1");
                                }
                                if ui.add(egui::Button::new("装填 +30").min_size(egui::vec2(78.0, 30.0))).clicked() {
                                    self.stats.ammo = (self.stats.ammo + 30).min(90);
                                    self.note("弹药 +30");
                                }
                            });
                        });
                    });
                });
                ui.add_space(8.0);

                theme::soft_card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            theme::section_label(ui, "金币");
                            ui.label(egui::RichText::new(self.stats.coins.to_string()).size(34.0).strong());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new("拾取 +50").min_size(egui::vec2(78.0, 30.0))).clicked() {
                                    self.stats.coins += 50;
                                    self.note("金币 +50");
                                }
                                if ui.add(egui::Button::new("花费 -100").min_size(egui::vec2(78.0, 30.0))).clicked() {
                                    self.stats.coins = (self.stats.coins - 100).max(0);
                                    self.note("金币 -100");
                                }
                            });
                        });
                    });
                });
                ui.add_space(8.0);

                theme::soft_card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            theme::section_label(ui, "护甲（浮点练习）");
                            ui.label(
                                egui::RichText::new(format!("{:.2}", self.stats.armor))
                                    .size(30.0)
                                    .strong()
                                    .color(theme::accent()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new("+0.05").min_size(egui::vec2(70.0, 30.0))).clicked() {
                                    self.stats.armor = (self.stats.armor + 0.05).min(1.0);
                                    self.note("护甲 +0.05");
                                }
                                if ui.add(egui::Button::new("-0.05").min_size(egui::vec2(70.0, 30.0))).clicked() {
                                    self.stats.armor = (self.stats.armor - 0.05).max(0.0);
                                    self.note("护甲 -0.05");
                                }
                            });
                        });
                    });
                });

                ui.add_space(10.0);
                theme::card_frame().show(ui, |ui| {
                    theme::section_label(ui, "操作记录");
                    for line in self.log.iter().rev().take(4) {
                        ui.label(egui::RichText::new(format!("· {line}")).size(12.0).color(theme::muted()));
                    }
                });

                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("提示：扫描时请选择本窗口的进程；数值变化后再回 Zero 用「数值减少 / 增加」再次扫描。")
                        .size(11.5)
                        .color(theme::faint()),
                );
            });
    }
}

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([590.0, 760.0])
            .with_min_inner_size([520.0, 640.0])
            .with_title("Zero · 练习靶场"),
        ..Default::default()
    };
    eframe::run_native(
        "Zero 练习靶场",
        options,
        Box::new(|cc| Box::new(PracticeApp::new(cc))),
    )
}
