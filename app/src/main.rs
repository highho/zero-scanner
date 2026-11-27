#![windows_subsystem = "windows"]

mod aob;
mod assembler;
mod ctable;
mod app;
mod disasm;
mod e2e;
mod focus;
mod hotkey;
mod inj;
mod lua;
mod mcp;
mod model;
mod pointerscan;
mod practice;
mod scan;
mod selftest;
mod session;
mod smart;
mod speed;
mod syms;
mod theme;
mod triage;
mod ui;
mod winmem;

use app::{Tab, ZeroApp};
use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--practice-target") {
        return practice::run();
    }
    if let Some(pos) = args.iter().position(|a| a == "--e2e") {
        // --e2e <pid> <info-file>
        let pid = args.get(pos + 1).and_then(|s| s.parse::<u32>().ok());
        let info = args.get(pos + 2).cloned().unwrap_or_else(|| "e2e_target.info".to_string());
        let ok = match pid {
            Some(pid) => e2e::run(pid, &info),
            None => {
                println!("usage: zero-scanner --e2e <pid> [info-file]");
                false
            }
        };
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some(pos) = args.iter().position(|a| a == "--verify") {
        // --verify <pid>: the same one-click check the UI runs.
        let ok = match args.get(pos + 1).and_then(|s| s.parse::<u32>().ok()) {
            Some(pid) => match winmem::open_process(pid, "verify") {
                Ok(handle) => {
                    let (log, ok) = app::ZeroApp::verify_process(&handle, &handle.name);
                    std::fs::write("verify.log", &log).ok();
                    println!("{log}");
                    ok
                }
                Err(e) => {
                    eprintln!("无法打开进程 {pid}：{e}");
                    false
                }
            },
            None => {
                eprintln!("usage: zero-scanner --verify <pid>");
                false
            }
        };
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some(pos) = args.iter().position(|a| a == "--mcp") {
        // Headless MCP service: useful for testing and for running Zero as a
        // pure backend without the window.
        let port = args
            .get(pos + 1)
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(3777);
        match mcp::start(port) {
            Ok(service) => {
                println!("MCP listening on {}", service.url());
                println!("Press Ctrl+C to stop.");
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--selftest") {
        let ok = selftest::run();
        std::process::exit(if ok { 0 } else { 1 });
    }

    let screenshot = parse_screenshot(&args);
    if let Some(path) = &screenshot {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 960.0])
        .with_min_inner_size([1120.0, 720.0])
        .with_title("Zero · 智能内存工作台")
        .with_active(screenshot.is_none())
        .with_icon(app_icon());

    #[cfg(target_os = "windows")]
    if screenshot.is_some() {
        viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
    }

    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        vsync: true,
        ..Default::default()
    };

    let path = screenshot.clone();
    let demo = args.iter().any(|a| a == "--demo");
    let show_hotkeys = args.iter().any(|a| a == "--hotkeys");
    let show_settings = args.iter().any(|a| a == "--settings");
    let initial_tab = parse_tab(&args);
    eframe::run_native(
        "Zero",
        options,
        Box::new(move |cc| {
            let mut app = ZeroApp::new(cc, path.clone(), initial_tab, demo);
            app.show_hotkeys = show_hotkeys;
            app.show_settings = show_settings;
            Box::new(app)
        }),
    )
}

fn parse_tab(args: &[String]) -> Option<Tab> {
    let mut i = 0;
    while i < args.len() {
        let value = if args[i] == "--tab" {
            args.get(i + 1).map(|s| s.as_str())
        } else {
            args[i].strip_prefix("--tab=")
        };
        if let Some(value) = value {
            return match value {
                "scan" => Some(Tab::Scan),
                "ptr" | "pointer" => Some(Tab::PtrScan),
                "aob" => Some(Tab::Aob),
                "list" => Some(Tab::List),
                "memory" => Some(Tab::Memory),
                "assistant" | "smart" => Some(Tab::Assistant),
                "practice" => Some(Tab::Practice),
                "symbols" | "syms" => Some(Tab::Symbols),
                "disasm" | "code" => Some(Tab::Disasm),
                "inject" | "patch" => Some(Tab::Inject),
                "asm" | "assemble" => Some(Tab::Asm),
                "table" | "ct" => Some(Tab::Table),
                "speed" => Some(Tab::Speed),
                "script" | "lua" => Some(Tab::Script),
                _ => Some(Tab::Scan),
            };
        }
        i += 1;
    }
    None
}

fn parse_screenshot(args: &[String]) -> Option<PathBuf> {
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--screenshot" {
            if let Some(path) = args.get(i + 1) {
                return Some(PathBuf::from(path));
            }
            return Some(PathBuf::from("screenshot.png"));
        }
        if let Some(value) = arg.strip_prefix("--screenshot=") {
            return Some(PathBuf::from(value));
        }
        i += 1;
    }
    None
}

fn app_icon() -> egui::IconData {
    let size = 64usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let t = (x + y) as f32 / (2.0 * size as f32);
            let r = (92.0 + (52.0 - 92.0) * t) as u8;
            let g = (140.0 + (211.0 - 140.0) * t) as u8;
            let b = (255.0 + (190.0 - 255.0) * t) as u8;
            let edge = {
                let cx = x as f32 - 31.5;
                let cy = y as f32 - 31.5;
                let d = (cx * cx + cy * cy).sqrt();
                if d > 30.0 {
                    0
                } else if d > 28.5 {
                    180
                } else {
                    255
                }
            };
            let idx = (y * size + x) * 4;
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = edge;
            let in_z = (x as i32 - y as i32).abs() < 5
                && x > 17
                && x < 47
                && y > 17
                && y < 47;
            if in_z && edge > 0 {
                rgba[idx] = 9;
                rgba[idx + 1] = 18;
                rgba[idx + 2] = 29;
                rgba[idx + 3] = 255;
            }
        }
    }
    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    }
}
