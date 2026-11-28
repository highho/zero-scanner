#![windows_subsystem = "windows"]

mod aob;
mod addr;
mod assembler;
mod ctable;
mod debugger;
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
mod plugins;
mod practice;
mod prefs;
mod scan;
mod selftest;
mod session;
mod smart;
mod speed;
mod structs;
mod syms;
mod theme;
mod triage;
mod ui;
mod update;
mod winmem;

use app::{Tab, ZeroApp};
use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--practice-target") {
        return practice::run();
    }
    if args.iter().any(|a| a == "--dbg-target") {
        return debugger::target_main();
    }
    if args.iter().any(|a| a == "--dbg-test") {
        // 专用调试器闭环验证：spawn 靶场 -> 附加 -> 设写断点 -> 等命中 -> detach。
        let code = debugger::cli_test();
        std::process::exit(if code { 0 } else { 1 });
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
    if args.iter().any(|a| a == "--update-check") {
        // Headless: query the latest release through the mirror and report.
        match update::cli_check() {
            Ok(line) => {
                println!("{line}");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    let download_any = args.iter().any(|a| a == "--update-download-any");
    let download_pos = args
        .iter()
        .position(|a| a == "--update-download" || a == "--update-download-any");
    if let Some(pos) = download_pos {
        let dir = args
            .get(pos + 1)
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        match update::cli_download(&dir, download_any) {
            Ok(line) => {
                println!("{line}");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if let Some(pos) = args.iter().position(|a| a == "--update-apply-from") {
        // 验证专用：用指定文件走一遍「等本进程退出 -> 替换 exe -> 重启」的批处理，
        // 目标可以是任意路径，便于在临时目录里真跑一遍而不动到当前程序。
        let source = match args.get(pos + 1) {
            Some(value) => PathBuf::from(value),
            None => {
                eprintln!("--update-apply-from 需要一个文件路径");
                std::process::exit(2);
            }
        };
        match update::install_and_restart(&source) {
            Ok(()) => {
                println!("update script queued for {}", source.display());
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--update-apply") {
        // Headless: download the newest build and swap this exe out of the way.
        // Used to verify the replace-and-restart path end to end.
        let (tx, _rx) = std::sync::mpsc::channel();
        let result = update::fetch_release()
            .and_then(|maybe| maybe.ok_or_else(|| "已是最新，没有可下载的版本".to_string()))
            .and_then(|release| update::download_for_cli(&release, tx))
            .and_then(|path| update::install_and_restart(&path));
        match result {
            Ok(()) => {
                println!("update queued, exiting");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("{error}");
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
    let settings_scroll = args
        .iter()
        .position(|a| a == "--settings-scroll")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0);
    let initial_tab = parse_tab(&args);
    let boot_theme = parse_theme(&args);
    eframe::run_native(
        "Zero",
        options,
        Box::new(move |cc| {
            let mut app = ZeroApp::new(cc, path.clone(), initial_tab, demo);
            app.settings_scroll = settings_scroll;
            app.show_hotkeys = show_hotkeys;
            app.show_settings = show_settings;
            if let Some(theme_id) = &boot_theme {
                crate::theme::set_theme(theme_id);
                crate::theme::apply_style(&cc.egui_ctx);
            }
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
                "struct" | "dissect" => Some(Tab::Struct),
                "debugger" | "breakpoint" => Some(Tab::Debugger),
                _ => Some(Tab::Scan),
            };
        }
        i += 1;
    }
    None
}

fn parse_theme(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--theme" {
            if let Some(value) = args.get(i + 1) {
                return Some(value.clone());
            }
        }
        if let Some(value) = args[i].strip_prefix("--theme=") {
            return Some(value.to_string());
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
    const PNG: &[u8] = include_bytes!("../assets/icon.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let img = img.to_rgba8();
            let (width, height) = img.dimensions();
            egui::IconData {
                rgba: img.into_raw(),
                width,
                height,
            }
        }
        Err(_) => egui::IconData {
            rgba: vec![0, 0, 0, 0],
            width: 1,
            height: 1,
        },
    }
}
