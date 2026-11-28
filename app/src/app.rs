use crate::aob;
use crate::assembler;
use crate::ctable;
use crate::session;
use crate::speed;
use crate::debugger::{self, DebugSession, BpKind, Slot};
use crate::disasm;
use crate::focus;
use crate::hotkey;
use crate::inj;
use crate::lua as script;
use crate::mcp;
use crate::model::{format_bytes, parse_address, ScanHit, ScanType, VType, Value};
use crate::pointerscan;
use crate::scan::{self, ScanConfig, ScanHandle};
use crate::smart::{self, SmartPlan};
use crate::syms;
use crate::theme::{self, ToastKind};
use crate::triage;
use crate::update::{self, Updater};
use crate::winmem::{self, ModuleInfo, ProcessHandle, ProcessInfo};
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc::TryRecvError, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Scan,
    List,
    Memory,
    Assistant,
    Practice,
    PtrScan,
    Aob,
    Symbols,
    Disasm,
    Inject,
    Asm,
    Table,
    Speed,
    Script,
    Struct,
    Debugger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Idle,
    Running,
    Done,
    Failed,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

pub struct Entry {
    pub id: u64,
    pub desc: String,
    pub address: u64,
    pub vtype: VType,
    pub active: bool,
    pub value_text: String,
    pub current: Option<Value>,
    pub previous: Option<Value>,
}

pub struct FreezeSpec {
    pub address: u64,
    pub vtype: VType,
    pub bytes: Vec<u8>,
}

pub struct FreezeState {
    pub handle: Option<Arc<ProcessHandle>>,
    pub specs: Vec<FreezeSpec>,
}

pub struct SmartMsg {
    pub from_user: bool,
    pub text: String,
}

pub struct ScreenshotJob {
    pub path: PathBuf,
    pub frames: u64,
    pub requested: bool,
    pub error: Option<String>,
}

pub struct EditState {
    pub id: u64,
    pub address: String,
    pub desc: String,
    pub vtype: VType,
    pub value: String,
}

pub struct ZeroApp {
    pub tab: Tab,
    pub process: Option<Arc<ProcessHandle>>,
    pub attached_pid: Option<u32>,
    pub process_name: String,
    pub process_arch: String,
    pub process_writable: bool,
    pub attach_error: Option<String>,

    pub picker_open: bool,
    pub picker_search: String,
    pub processes: Vec<ProcessInfo>,
    pub picker_loaded: bool,

    pub scan_value: String,
    pub scan_value2: String,
    pub vtype: VType,
    pub scan_type: ScanType,
    pub fast: bool,
    pub writable_only: bool,
    pub max_results: usize,
    pub scan_job: Option<ScanHandle>,
    pub scan_phase: ScanPhase,
    pub scan_error: Option<String>,
    pub scan_elapsed: Duration,
    pub scan_truncated: bool,
    pub scan_cancelled: bool,
    pub hits: Vec<ScanHit>,
    pub hit_history: Vec<Vec<ScanHit>>,
    pub selected_hit: Option<usize>,
    pub ranked: Vec<triage::Candidate>,
    pub watches: Vec<focus::Watch>,
    pub watch_running: bool,
    pub watch_limit: usize,
    pub feedback: Vec<triage::RejectedTrait>,
    pub last_watch_tick: Instant,
    pub show_ranked: bool,
    pub last_scan_kind: String,

    pub ptr_target: String,
    pub ptr_depth: usize,
    pub ptr_max_offset: u64,
    pub ptr_offset_text: String,
    pub ptr_job: Option<pointerscan::PtrHandle>,
    pub ptr_phase: ScanPhase,
    pub ptr_error: Option<String>,
    pub ptr_elapsed: Duration,
    pub ptr_paths: Vec<pointerscan::PointerPath>,
    pub ptr_modules: Vec<ModuleInfo>,
    pub ptr_modules_loaded: bool,
    pub ptr_selected: usize,
    pub ptr_scanned: usize,
    /// Set when the shown chains came from a saved map and have not been
    /// re-pointed at the current process yet.
    pub ptr_stale: bool,
    pub aob_text: String,
    pub aob_job: Option<aob::AobHandle>,
    pub aob_phase: ScanPhase,
    pub aob_error: Option<String>,
    pub aob_elapsed: Duration,
    pub aob_hits: Vec<u64>,
    pub aob_selected: usize,
    pub sym_module: String,
    pub sym_filter: String,
    pub sym_table: Option<syms::SymbolTable>,
    pub sym_error: Option<String>,
    pub sym_show_imports: bool,
    pub sym_elapsed: Duration,

    pub dbg: Option<Arc<DebugSession>>,
    pub dbg_addr: String,
    pub dbg_kind: BpKind,
    pub dbg_error: Option<String>,

    pub entries: Vec<Entry>,
    pub next_entry_id: u64,
    pub editing: Option<EditState>,
    pub show_add: bool,
    pub manual_address: String,
    pub manual_desc: String,
    pub manual_type: VType,
    pub manual_value: String,
    pub last_entry_refresh: Instant,
    pub freeze: Arc<Mutex<FreezeState>>,

    pub mem_addr: String,
    pub mem_base: u64,
    pub mem_data: Vec<u8>,
    pub mem_error: Option<String>,
    pub mem_write: String,
    pub mem_loaded: bool,

    pub dis_addr: String,
    pub dis_bytes: Vec<u8>,
    pub dis_base: u64,
    pub dis_insns: Vec<disasm::Insn>,
    pub dis_error: Option<String>,
    pub dis_loaded: bool,

    pub inj_addr: String,
    pub inj_bytes: String,
    pub inj_cave_size: String,
    pub inj_patches: Vec<inj::Patch>,
    pub inj_caves: Vec<inj::CodeCave>,
    pub inj_selected_cave: usize,
    pub inj_error: Option<String>,

    pub asm_source: String,
    pub asm_origin: String,
    pub asm_result: Option<assembler::AsmResult>,
    pub asm_error: Option<String>,

    pub ct_path: String,
    pub ct_table: Option<ctable::CtTable>,
    pub ct_error: Option<String>,
    pub ct_status: String,

    pub speed_factor: f64,
    pub speed_on: bool,
    pub speed_base: f64,
    pub speed_real_start: Option<Instant>,
    pub speed_error: Option<String>,
    pub speed_threads: usize,

    pub workspace_path: String,
    pub show_hotkeys: bool,
    pub mcp: Option<mcp::Service>,
    pub mcp_port: u16,
    pub mcp_error: Option<String>,

    pub e2e_running: bool,
    pub e2e_log: String,
    pub e2e_ok: Option<bool>,

    pub show_stats: bool,
    pub script_source: String,
    pub script_output: Vec<String>,
    pub script_error: Option<String>,

    pub assistant_input: String,
    pub assistant_plan: Option<SmartPlan>,
    pub smart_msgs: Vec<SmartMsg>,

    pub show_settings: bool,
    /// 设置弹窗内容区初始滚动偏移（只给界面回归截图用，默认 0）。
    pub settings_scroll: f32,
    pub update: Updater,
    pub toasts: Vec<Toast>,
    pub struct_addr: String,
    pub struct_size: usize,
    pub struct_result: Option<crate::structs::Dissect>,
    pub struct_error: Option<String>,
    pub struct_loaded: bool,

    pub screenshot: Option<ScreenshotJob>,

    pub practice_child: Option<std::process::Child>,
    pub practice_pid: Option<u32>,
    pub practice_pending: Option<(u32, Instant)>,
    pub demo: bool,
}

impl ZeroApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        screenshot: Option<PathBuf>,
        initial_tab: Option<Tab>,
        demo: bool,
    ) -> Self {
        theme::setup_fonts(&cc.egui_ctx);
        theme::init();
        theme::apply_style(&cc.egui_ctx);
        let freeze = start_freeze_thread();
        let plugin_notices = if demo { Vec::new() } else { crate::plugins::discover() };
        let mut app = Self {
            tab: Tab::Scan,
            process: None,
            attached_pid: None,
            process_name: String::new(),
            process_arch: String::new(),
            process_writable: false,
            attach_error: None,
            picker_open: false,
            picker_search: String::new(),
            processes: Vec::new(),
            picker_loaded: false,
            scan_value: String::new(),
            scan_value2: String::new(),
            vtype: VType::I32,
            scan_type: ScanType::Exact,
            fast: true,
            writable_only: true,
            max_results: 200_000,
            scan_job: None,
            scan_phase: ScanPhase::Idle,
            scan_error: None,
            scan_elapsed: Duration::ZERO,
            scan_truncated: false,
            scan_cancelled: false,
            hits: Vec::new(),
            hit_history: Vec::new(),
            selected_hit: None,
            ranked: Vec::new(),
            watches: Vec::new(),
            watch_running: false,
            watch_limit: 8,
            feedback: Vec::new(),
            last_watch_tick: Instant::now(),
            show_ranked: true,
            last_scan_kind: String::new(),
            entries: Vec::new(),
            next_entry_id: 1,
            editing: None,
            show_add: false,
            manual_address: String::new(),
            manual_desc: String::new(),
            manual_type: VType::I32,
            manual_value: String::new(),
            last_entry_refresh: Instant::now(),
            freeze,
            mem_addr: String::new(),
            mem_base: 0,
            mem_data: Vec::new(),
            mem_error: None,
            mem_write: String::new(),
            mem_loaded: false,
            dis_addr: String::new(),
            dis_bytes: Vec::new(),
            dis_base: 0,
            dis_insns: Vec::new(),
            dis_error: None,
            dis_loaded: false,
            inj_addr: String::new(),
            inj_bytes: String::new(),
            inj_cave_size: "4096".to_string(),
            inj_patches: Vec::new(),
            inj_caves: Vec::new(),
            inj_selected_cave: 0,
            inj_error: None,
            asm_source: String::new(),
            asm_origin: String::new(),
            asm_result: None,
            asm_error: None,
            ct_path: String::new(),
            ct_table: None,
            ct_error: None,
            ct_status: String::new(),
            speed_factor: 1.0,
            speed_on: false,
            speed_base: 1.0,
            speed_real_start: None,
            speed_error: None,
            speed_threads: 0,
            workspace_path: workspace_path(),
            show_hotkeys: false,
            mcp: None,
            mcp_port: 3777,
            mcp_error: None,
            e2e_running: false,
            e2e_log: String::new(),
            e2e_ok: None,
            show_stats: false,
            script_source: String::new(),
            script_output: Vec::new(),
            script_error: None,
            assistant_input: String::new(),
            assistant_plan: None,
            smart_msgs: vec![SmartMsg {
                from_user: false,
                text: "你好，我是 Zero 的本地智能助手。告诉我你在游戏里看到的数值，例如「我的金币是 9800」，我会帮你选好扫描类型。".to_string(),
            }],
            show_settings: false,
            update: Updater::default(),
            toasts: Vec::new(),
            struct_addr: String::new(),
            struct_size: 96,
            struct_result: None,
            struct_error: None,
            struct_loaded: false,
            settings_scroll: 0.0,
            screenshot: screenshot.map(|path| ScreenshotJob {
                path,
                frames: 0,
                requested: false,
                error: None,
            }),
            ptr_target: String::new(),
            ptr_depth: 3,
            ptr_max_offset: 0x1_0000,
            ptr_offset_text: "10000".to_string(),
            ptr_job: None,
            ptr_phase: ScanPhase::Idle,
            ptr_error: None,
            ptr_elapsed: Duration::ZERO,
            ptr_paths: Vec::new(),
            ptr_modules: Vec::new(),
            ptr_modules_loaded: false,
            ptr_selected: 0,
            ptr_scanned: 0,
            ptr_stale: false,
            aob_text: String::new(),
            aob_job: None,
            aob_phase: ScanPhase::Idle,
            aob_error: None,
            aob_elapsed: Duration::ZERO,
            aob_hits: Vec::new(),
            aob_selected: 0,
            sym_module: String::new(),
            sym_filter: String::new(),
            sym_table: None,
            sym_error: None,
            sym_show_imports: false,
            sym_elapsed: Duration::ZERO,
            dbg: None,
            dbg_addr: String::new(),
            dbg_kind: BpKind::Write,
            dbg_error: None,
            practice_child: None,
            practice_pid: None,
            practice_pending: None,
            demo,
        };
        if app.screenshot.is_some() {
            app.scan_value = "100".to_string();
        }
        if let Some(tab) = initial_tab {
            app.tab = tab;
        }
        if demo {
            if let Ok(handle) = winmem::open_process(std::process::id(), "zero-scanner.exe") {
                app.attach_handle(handle);
            }
            app.hits = vec![
                ScanHit { address: 0x7FF6_A1B2_C010, vtype: VType::I32, value: Value::Int(100), previous: Some(Value::Int(100)) },
                ScanHit { address: 0x7FF6_A1B2_C0A4, vtype: VType::I32, value: Value::Int(100), previous: Some(Value::Int(100)) },
                ScanHit { address: 0x7FF6_A1B2_C2D8, vtype: VType::I32, value: Value::Int(100), previous: Some(Value::Int(100)) },
                ScanHit { address: 0x7FF6_A1B2_C3E0, vtype: VType::F32, value: Value::Float(100.0), previous: None },
                ScanHit { address: 0x7FF6_A1B2_C540, vtype: VType::I32, value: Value::Int(99), previous: Some(Value::Int(100)) },
                ScanHit { address: 0x7FF6_A1B2_C7AC, vtype: VType::I32, value: Value::Int(98), previous: Some(Value::Int(100)) },
            ];
            app.entries = vec![
                Entry { id: 1, desc: "玩家生命".to_string(), address: 0x7FF6_A1B2_C010, vtype: VType::I32, active: true, value_text: "100".to_string(), current: Some(Value::Int(100)), previous: Some(Value::Int(100)) },
                Entry { id: 2, desc: "弹药".to_string(), address: 0x7FF6_A1B2_C0A4, vtype: VType::I32, active: false, value_text: "30".to_string(), current: Some(Value::Int(30)), previous: Some(Value::Int(31)) },
                Entry { id: 3, desc: "金币".to_string(), address: 0x7FF6_A1B2_C2D8, vtype: VType::I32, active: false, value_text: "9800".to_string(), current: Some(Value::Int(9800)), previous: Some(Value::Int(9750)) },
                Entry { id: 4, desc: "护甲".to_string(), address: 0x7FF6_A1B2_C3E0, vtype: VType::F32, active: false, value_text: "0.75".to_string(), current: Some(Value::Float(0.75)), previous: Some(Value::Float(0.75)) },
            ];
            app.next_entry_id = 5;
            app.selected_hit = Some(0);
            app.last_scan_kind = "首次 · 精确值".to_string();
            app.scan_phase = ScanPhase::Done;
            app.ptr_paths = vec![
                pointerscan::PointerPath {
                    base: pointerscan::StaticBase {
                        module: "game.exe".to_string(),
                        module_base: 0x7FF6_A1B2_0000,
                        offset: 0x1A2B,
                    },
                    offsets: vec![0x18, 0x30],
                    final_address: 0x7FF6_A1B2_C010,
                    is_32bit: false,
                },
                pointerscan::PointerPath {
                    base: pointerscan::StaticBase {
                        module: "game.dll".to_string(),
                        module_base: 0x7FF6_B2C3_4000,
                        offset: 0x8C,
                    },
                    offsets: vec![0x4, 0x0],
                    final_address: 0x7FF6_A1B2_C0A4,
                    is_32bit: false,
                },
            ];
            app.ptr_modules_loaded = true;
            app.aob_text = "48 8B ?? 05".to_string();
            app.aob_hits = vec![0x7FF6_A1B2_C3E0, 0x7FF6_B2C3_9120];
            app.script_source = "-- 读取生命值并写回\nlocal addr = getAddress(\"game.exe+1A2B\")\nprint(\"生命值地址\", string.format(\"%X\", addr))\nprint(\"当前值\", readInteger(addr, 4))".to_string();
            app.script_output = vec![
                "生命值地址\t7FF6A1B2C010".to_string(),
                "当前值\t100".to_string(),
            ];
            app.speed_factor = 0.5;
            app.speed_threads = 12;
            app.speed_real_start = Some(Instant::now());
            app.ct_path = "game.ct".to_string();
            app.ct_status = "已打开 game.ct（3 条）".to_string();
            app.ct_table = Some(ctable::CtTable {
                entries: vec![
                    ctable::CtEntry {
                        description: "玩家生命".to_string(),
                        vtype: VType::I32,
                        address: "game.exe+1A2B".to_string(),
                        value: Some("100".to_string()),
                        active: true,
                    },
                    ctable::CtEntry {
                        description: "金币".to_string(),
                        vtype: VType::I32,
                        address: "[[game.exe+8C]+18]+30".to_string(),
                        value: None,
                        active: false,
                    },
                    ctable::CtEntry {
                        description: "坐标 X".to_string(),
                        vtype: VType::F32,
                        address: "0x7FF6A1B2C540".to_string(),
                        value: Some("123.5".to_string()),
                        active: true,
                    },
                ],
                raw: String::new(),
            });
            app.asm_origin = "7FF6B2C35240".to_string();
            app.asm_source = "; 给函数开头打补丁\nstart:\n  push rbp\n  mov rbp, rsp\n  sub rsp, 0x40\n  mov eax, 1\n  jmp start".to_string();
            app.asm_result = assembler::assemble(&app.asm_source, 0x7FF6_B2C3_5240).ok();
            app.inj_addr = "7FF6B2C35240".to_string();
            app.inj_bytes = "48 31 C0 90 90".to_string();
            app.inj_caves = vec![inj::CodeCave {
                address: 0x7FF6_B2C9_0000,
                size: 4096,
            }];
            app.inj_selected_cave = 0;
            app.inj_patches = vec![
                inj::Patch {
                    address: 0x7FF6_B2C3_53A0,
                    original: vec![0x48, 0x83, 0xEC, 0x20, 0xC3],
                    written: vec![0x48, 0x31, 0xC0, 0xC3, 0x90],
                    label: "7FF6B2C353A0".to_string(),
                    active: true,
                    protection_changed: false,
                },
                inj::Patch {
                    address: 0x7FF6_B2C3_5240,
                    original: vec![0x55, 0x48, 0x89, 0xE5],
                    written: vec![0x31, 0xC0, 0xC3, 0x90],
                    label: "7FF6B2C35240".to_string(),
                    active: false,
                    protection_changed: false,
                },
            ];
            // Demo hits are synthetic, so give triage a matching synthetic
            // region map: the ranking code path stays exactly the real one.
            let demo_regions = vec![triage::RegionHint {
                base: 0x7FF6_A1B2_0000,
                size: 0x2_0000,
                writable: true,
                executable: false,
            }];
            let demo_modules: Vec<(String, u64, u64)> =
                vec![("game.exe".to_string(), 0x7FF6_A1B2_0000, 0x2_0000)];
            app.ranked = triage::rank(&app.hits, &demo_regions, &demo_modules);
            app.dis_addr = "7FF6B2C35240".to_string();
            app.dis_base = 0x7FF6_B2C3_5240;
            app.dis_bytes = vec![
                0x48, 0x8B, 0x05, 0x10, 0x00, 0x00, 0x00, 0x55, 0x48, 0x89, 0xE5, 0x48, 0x83,
                0xEC, 0x20, 0xE8, 0x11, 0x22, 0x33, 0x44, 0x75, 0xF0, 0xC3,
            ];
            app.dis_insns = disasm::disassemble(&app.dis_bytes, app.dis_base, true, 32);
            app.dis_loaded = true;
            app.sym_module = "game.dll".to_string();
            app.sym_table = Some(syms::SymbolTable {
                module: "game.dll".to_string(),
                base: 0x7FF6_B2C3_4000,
                exports: vec![
                    syms::Symbol {
                        name: "Player_GetHealth".to_string(),
                        address: 0x7FF6_B2C3_5240,
                        ordinal: 1,
                        forwarded: None,
                    },
                    syms::Symbol {
                        name: "Player_SetGold".to_string(),
                        address: 0x7FF6_B2C3_53A0,
                        ordinal: 2,
                        forwarded: None,
                    },
                    syms::Symbol {
                        name: "Engine_Init".to_string(),
                        address: 0x7FF6_B2C3_4080,
                        ordinal: 3,
                        forwarded: None,
                    },
                    syms::Symbol {
                        name: "GetFileVersionInfoW".to_string(),
                        address: 0,
                        ordinal: 4,
                        forwarded: Some("VERSION.GetFileVersionInfoW".to_string()),
                    },
                ],
                imports: vec![
                    syms::ImportLib {
                        dll: "kernel32.dll".to_string(),
                        symbols: vec![
                            syms::Symbol {
                                name: "ReadProcessMemory".to_string(),
                                address: 0x7FF6_B2C3_8000,
                                ordinal: 0,
                                forwarded: None,
                            },
                            syms::Symbol {
                                name: "VirtualProtect".to_string(),
                                address: 0x7FF6_B2C3_8008,
                                ordinal: 0,
                                forwarded: None,
                            },
                        ],
                    },
                    syms::ImportLib {
                        dll: "user32.dll".to_string(),
                        symbols: vec![syms::Symbol {
                            name: "MessageBoxW".to_string(),
                            address: 0x7FF6_B2C3_8100,
                            ordinal: 0,
                            forwarded: None,
                        }],
                    },
                ],
            });
            app.mem_base = 0x7FF6_A1B2_C000;
            app.mem_addr = format!("{:X}", app.mem_base);
            app.mem_data = (0..256u16).map(|i| ((i * 37 + 11) & 0xFF) as u8).collect();
            app.mem_loaded = true;
            app.assistant_plan = smart::interpret("我的金币是 9800，帮我找它");
            app.smart_msgs.push(SmartMsg {
                from_user: true,
                text: "我的金币是 9800，帮我找它".to_string(),
            });
            app.smart_msgs.push(SmartMsg {
                from_user: false,
                text: "我理解为「查找金币 / 余额」。金币和余额通常是 4 字节整数；金额很大时可以换 8 字节。建议：拾取或花费金币，再用「数值增加 / 减少」再次扫描。".to_string(),
            });
            app.toasts.clear();
        }
        // Demo data is synthetic: restoring a saved workspace on top of it would
        // duplicate every entry and confuse the screenshots.
        let restored_patches = if demo { 0 } else { app.restore_workspace() };
        if restored_patches > 0 {
            app.toast(
                format!("恢复工作区：{restored_patches} 个补丁仍是生效状态"),
                ToastKind::Warn,
            );
        }
        for notice in plugin_notices {
            if notice.ok {
                app.toast(
                    format!("插件「{}」：{}", notice.plugin, notice.message),
                    ToastKind::Info,
                );
            } else {
                app.toast(
                    format!("插件「{}」加载失败：{}", notice.plugin, notice.message),
                    ToastKind::Warn,
                );
            }
        }
        app
    }

    pub fn is_attached(&self) -> bool {
        self.process.is_some()
    }

    /// Resolve an address expression (game.exe+0x4A0C8, a bare hex address,
    /// or a pointer chain like game.exe+0x10,0x14,0x0) against the attached
    /// process and its module list.
    pub fn resolve_addr(&self, text: &str) -> Option<u64> {
        crate::addr::resolve(self.process.as_deref(), &self.ptr_modules, text)
    }

    /// The same, but always requiring an attached process (pointer chains
    /// dereference memory, so resolution without a process is impossible).
    pub fn resolve_addr_attached(&self, text: &str) -> Option<u64> {
        if !self.is_attached() {
            return None;
        }
        self.resolve_addr(text)
    }

    pub fn ui_page(&mut self, ui: &mut egui::Ui) {
        match self.tab {
            Tab::Scan => self.ui_scan(ui),
            Tab::List => self.ui_list(ui),
            Tab::Memory => self.ui_memory(ui),
            Tab::Assistant => self.ui_assistant(ui),
            Tab::Practice => self.ui_practice(ui),
            Tab::PtrScan => self.ui_ptr_scan(ui),
            Tab::Aob => self.ui_aob_scan(ui),
            Tab::Symbols => self.ui_symbols(ui),
            Tab::Disasm => self.ui_disasm(ui),
            Tab::Inject => self.ui_inject(ui),
            Tab::Asm => self.ui_asm(ui),
            Tab::Table => self.ui_table(ui),
            Tab::Speed => self.ui_speed(ui),
            Tab::Script => self.ui_script(ui),
            Tab::Struct => self.ui_struct(ui),
            Tab::Debugger => self.ui_debugger(ui),
        }
    }

    /// 推进自动更新状态机：到期检查、收后台消息、需要时让界面保持刷新。
    pub fn poll_update(&mut self, ctx: &egui::Context) {
        self.update.tick();
        for note in self.update.poll() {
            self.toast(note, ToastKind::Info);
        }
        if self.update.busy() {
            ctx.request_repaint_after(Duration::from_millis(160));
        }
    }

    /// 下载完成后：写更新脚本、退出，让脚本替换 exe 再启动新版。
    pub fn apply_update(&mut self) -> bool {
        if let update::State::Ready(path) = &self.update.state {
            match update::install_and_restart(path) {
                Ok(()) => {
                    self.save_workspace();
                    return true;
                }
                Err(e) => {
                    self.update.state = update::State::Failed(e.clone());
                    self.toast(format!("无法安装更新：{e}"), ToastKind::Error);
                }
            }
        }
        false
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn attach_info(&mut self, info: &ProcessInfo) {
        match winmem::open_process(info.pid, &info.name) {
            Ok(handle) => self.attach_handle(handle),
            Err(e) => {
                self.attach_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn attach_handle(&mut self, handle: Arc<ProcessHandle>) {
        if let Some(job) = &self.scan_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.scan_job = None;
        self.scan_phase = ScanPhase::Idle;
        let same = self.attached_pid == Some(handle.pid);
        if !same {
            self.hits.clear();
            self.hit_history.clear();
            self.entries.clear();
            self.selected_hit = None;
            self.mem_loaded = false;
            self.mem_data.clear();
            self.ptr_modules.clear();
            self.ptr_modules_loaded = false;
            self.sym_module.clear();
            self.sym_table = None;
            self.sym_error = None;
            self.sym_filter.clear();
        }
        self.process_name = handle.name.clone();
        self.process_arch = if handle.is_64bit { "x64" } else { "x86" }.to_string();
        self.process_writable = handle.writable;
        self.attached_pid = Some(handle.pid);
        self.attach_error = None;
        {
            let mut freeze = self.freeze.lock().unwrap();
            freeze.handle = Some(handle.clone());
            freeze.specs.clear();
        }
        self.process = Some(handle.clone());
        self.picker_open = false;
        // A restored pointer map is re-pointed here: modules moved with ASLR, so
        // the stored bases are useless until they are looked up again by name.
        if self.ptr_stale {
            self.rebase_ptr_paths(&handle);
        }
        self.sync_freeze();
        self.toast(
            format!("已连接 {}（PID {}）", self.process_name, self.attached_pid.unwrap_or(0)),
            ToastKind::Success,
        );
    }

    pub fn detach(&mut self) {
        if let Some(dbg) = &self.dbg {
            dbg.stop();
        }
        self.dbg = None;
        self.dbg_error = None;
        if let Some(job) = &self.scan_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.scan_job = None;
        self.scan_phase = ScanPhase::Idle;
        self.process = None;
        self.attached_pid = None;
        self.process_name.clear();
        self.process_arch.clear();
        self.pending_load();
        self.hits.clear();
        self.hit_history.clear();
        self.mem_loaded = false;
        self.mem_data.clear();
        for entry in &mut self.entries {
            entry.current = None;
            entry.previous = None;
        }
        if let Ok(mut freeze) = self.freeze.lock() {
            freeze.handle = None;
            freeze.specs.clear();
        }
        self.toast("已断开进程。", ToastKind::Info);
    }

    fn pending_load(&mut self) {}

    pub fn open_picker(&mut self) {
        self.picker_open = true;
        self.refresh_processes();
    }

    pub fn refresh_processes(&mut self) {
        self.processes = winmem::list_processes();
        self.picker_loaded = true;
    }

    pub fn start_first_scan(&mut self) {
        if self.scan_phase == ScanPhase::Running {
            return;
        }
        let Some(handle) = self.process.clone() else {
            self.toast("先选择一个进程，才能扫描。", ToastKind::Warn);
            self.open_picker();
            return;
        };
        if !self.scan_type.first_scan_ok() {
            self.toast("首次扫描请使用精确值、范围或未知初值。", ToastKind::Warn);
            return;
        }
        if self.scan_type.needs_value() && self.scan_value.trim().is_empty() {
            self.toast("请输入你在游戏里看到的数值。", ToastKind::Warn);
            return;
        }
        if self.scan_type.needs_two() && self.scan_value2.trim().is_empty() {
            self.toast("范围扫描需要第二个数值。", ToastKind::Warn);
            return;
        }
        let regions = handle.regions(self.writable_only);
        if regions.is_empty() {
            self.toast(
                "没有可扫描的内存区域。试试关闭「只扫描可写内存」。",
                ToastKind::Error,
            );
            return;
        }
        let cfg = scan_config(self);
        match scan::start_first_scan(handle, regions, cfg) {
            Ok(job) => {
                self.scan_job = Some(job);
                self.scan_phase = ScanPhase::Running;
                self.scan_error = None;
                self.scan_elapsed = Duration::ZERO;
                self.scan_truncated = false;
                self.scan_cancelled = false;
                self.hits.clear();
                self.hit_history.clear();
                self.selected_hit = None;
                self.last_scan_kind = format!("首次 · {}", self.scan_type.label());
            }
            Err(e) => {
                self.scan_phase = ScanPhase::Failed;
                self.scan_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn start_next_scan(&mut self) {
        if self.scan_phase == ScanPhase::Running {
            return;
        }
        let Some(handle) = self.process.clone() else {
            self.toast("先选择一个进程，才能扫描。", ToastKind::Warn);
            return;
        };
        if self.hits.is_empty() {
            self.toast("还没有首次扫描结果。先做一次首次扫描。", ToastKind::Warn);
            return;
        }
        if self.scan_type == ScanType::Unknown {
            self.toast("再次扫描不能使用「未知初值」。", ToastKind::Warn);
            return;
        }
        if self.scan_type.needs_value() && self.scan_value.trim().is_empty() {
            self.toast("请输入当前数值，才能再次扫描。", ToastKind::Warn);
            return;
        }
        if self.scan_type.needs_two() && self.scan_value2.trim().is_empty() {
            self.toast("范围扫描需要第二个数值。", ToastKind::Warn);
            return;
        }
        let cfg = scan_config(self);
        match scan::start_next_scan(handle, self.hits.clone(), cfg) {
            Ok(job) => {
                self.hit_history.push(self.hits.clone());
                if self.hit_history.len() > 6 {
                    self.hit_history.remove(0);
                }
                self.scan_job = Some(job);
                self.scan_phase = ScanPhase::Running;
                self.scan_error = None;
                self.scan_elapsed = Duration::ZERO;
                self.scan_truncated = false;
                self.scan_cancelled = false;
                self.last_scan_kind = format!("再次 · {}", self.scan_type.label());
            }
            Err(e) => {
                self.scan_phase = ScanPhase::Failed;
                self.scan_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn stop_scan(&mut self) {
        if let Some(job) = &self.scan_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn undo_scan(&mut self) {
        if let Some(previous) = self.hit_history.pop() {
            self.hits = previous;
            self.selected_hit = None;
            self.toast("已撤销上一次再次扫描。", ToastKind::Info);
        }
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let mut done = None;
        let mut disconnected = false;
        if let Some(job) = &self.scan_job {
            match job.rx.try_recv() {
                Ok(result) => done = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }
        if let Some(result) = done {
            self.scan_job = None;
            self.scan_elapsed = result.elapsed;
            self.scan_truncated = result.truncated;
            self.scan_cancelled = result.cancelled;
            self.scan_phase = if result.error.is_some() {
                ScanPhase::Failed
            } else {
                ScanPhase::Done
            };
            self.scan_error = result.error;
            self.hits = result.hits;
            self.selected_hit = if self.hits.is_empty() { None } else { Some(0) };
            for hit in &self.hits {
                for entry in &mut self.entries {
                    if entry.address == hit.address && entry.vtype == hit.vtype {
                        entry.current = Some(hit.value.clone());
                    }
                }
            }
            // Rank immediately: the user's first question after a scan is
            // "which of these is it", not "show me all of them".
            self.rerank_hits();
            if self.hits.is_empty() {
                if !self.scan_cancelled {
                    self.smart_msgs.push(SmartMsg {
                        from_user: false,
                        text: "这次没有找到地址。试试换一种数值类型（浮点 / 8 字节 / 自动），或者放宽扫描条件。".to_string(),
                    });
                }
            } else {
                let count = self.hits.len();
                let suffix = if self.scan_truncated { "（已达上限）" } else { "" };
                let strong = triage::strong_count(&self.ranked);
                let hint = if strong > 0 && strong < count {
                    format!("，其中 {strong} 个很可能")
                } else {
                    String::new()
                };
                self.toast(
                    format!(
                        "扫描完成：{count} 个地址{}{hint}，用时 {:.2}s",
                        suffix,
                        self.scan_elapsed.as_secs_f32()
                    ),
                    ToastKind::Success,
                );
            }
        } else if disconnected {
            self.scan_job = None;
            self.scan_phase = ScanPhase::Failed;
            self.scan_error = Some("扫描线程意外结束。".to_string());
            self.toast("扫描线程意外结束，请重试。", ToastKind::Error);
        }
        if self.scan_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    pub fn add_hit(&mut self, index: usize) {
        let Some(hit) = self.hits.get(index).cloned() else {
            return;
        };
        if self
            .entries
            .iter()
            .any(|e| e.address == hit.address && e.vtype == hit.vtype)
        {
            self.toast("这个地址已经在列表里了。", ToastKind::Info);
            return;
        }
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        let value_text = hit.value.display();
        self.entries.push(Entry {
            id,
            desc: format!("扫描结果 {}", id),
            address: hit.address,
            vtype: hit.vtype,
            active: false,
            value_text,
            current: Some(hit.value.clone()),
            previous: None,
        });
        self.toast("已添加到地址列表。", ToastKind::Success);
    }

    pub fn add_manual_entry(&mut self) {
        let Some(address) = self.resolve_addr(&self.manual_address) else {
            self.toast("地址格式不对：试试 0x1A2B3C、十进制，或 game.exe+0x4A0C8 这样的模块偏移。", ToastKind::Warn);
            return;
        };
        let value_text = if self.manual_value.trim().is_empty() {
            String::new()
        } else {
            match self.manual_type.parse(&self.manual_value) {
                Some(v) => v.display(),
                None => {
                    self.toast("数值和类型不匹配。", ToastKind::Warn);
                    return;
                }
            }
        };
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        let mut current = None;
        let mut previous = None;
        if let Some(handle) = &self.process {
            current = read_value(handle, address, self.manual_type);
            if !value_text.is_empty() {
                if let Some(v) = self.manual_type.parse(&value_text) {
                    if let Some(bytes) = v.to_bytes(self.manual_type) {
                        if handle.write(address, &bytes) {
                            previous = current.clone();
                            current = Some(v);
                        } else {
                            self.toast("写入失败：这个地址不可写。", ToastKind::Warn);
                        }
                    }
                }
            }
        }
        self.entries.push(Entry {
            id,
            desc: if self.manual_desc.trim().is_empty() {
                format!("手动地址 {id}")
            } else {
                self.manual_desc.trim().to_string()
            },
            address,
            vtype: self.manual_type,
            active: false,
            value_text,
            current,
            previous,
        });
        self.show_add = false;
        self.manual_address.clear();
        self.manual_desc.clear();
        self.manual_value.clear();
        self.toast("已添加手动地址。", ToastKind::Success);
    }

    pub fn remove_entry(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
        self.sync_freeze();
        self.toast("已移除地址。", ToastKind::Info);
    }

    /// Record that a hit is not the value being looked for, and learn from it.
    ///
    /// The point is not to hide one row: it is to stop the same kind of wrong
    /// answer from ranking high again in this session.
    pub fn reject_hit(&mut self, index: usize) {
        let Some(hit) = self.hits.get(index).cloned() else {
            return;
        };
        let Some(handle) = self.process.clone() else {
            return;
        };
        let regions: Vec<triage::RegionHint> = handle
            .regions(false)
            .into_iter()
            .map(|r| triage::RegionHint {
                base: r.base,
                size: r.size,
                writable: r.writable,
                executable: r.executable,
            })
            .collect();
        let module_ranges: Vec<(String, u64, u64)> = self
            .ptr_modules
            .iter()
            .map(|m| (m.name.clone(), m.base, m.size))
            .collect();
        let learned = triage::learn_rejection(&mut self.feedback, &hit, &regions, &module_ranges);
        // Remove the rejected hit from the result set entirely: it is a known
        // dead end, and keeping it around would only dilute the next scan.
        self.hits.retain(|h| h.address != hit.address);
        self.selected_hit = None;
        self.rerank_hits();
        if learned.is_empty() {
            self.toast(
                format!("已移除 {:X}（没有新的规律可以学）", hit.address),
                ToastKind::Info,
            );
        } else {
            let labels: Vec<&str> = learned.iter().map(|t| t.label()).collect();
            self.toast(
                format!("已移除 {:X}，并记住：{}", hit.address, labels.join("；")),
                ToastKind::Success,
            );
        }
    }

    /// Start or stop the MCP service. Off by default, and nothing listens while
    /// it is off: the port is only bound for as long as the switch is on.
    pub fn toggle_mcp(&mut self) {
        if let Some(service) = self.mcp.take() {
            service.stop();
            self.toast("MCP 服务已停止，端口已释放。", ToastKind::Info);
            self.mcp_error = None;
            return;
        }
        match mcp::start(self.mcp_port) {
            Ok(service) => {
                self.toast(
                    format!("MCP 服务已启动：127.0.0.1:{}", service.port),
                    ToastKind::Success,
                );
                self.mcp_port = service.port;
                self.mcp_error = None;
                self.mcp = Some(service);
            }
            Err(e) => {
                self.mcp_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    /// Launch the end-to-end self-check on the currently attached process.
    ///
    /// Running the real verification needs a target that publishes its own
    /// addresses; when one is attached through the normal picker we cannot know
    /// those addresses, so this runs the subset that needs nothing but the
    /// process itself and reports honestly what was skipped.
    pub fn run_e2e_check(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.toast("先连接一个进程，再跑验证。", ToastKind::Warn);
            return;
        };
        self.e2e_running = true;
        self.e2e_ok = None;
        let (log, ok) = ZeroApp::verify_process(&handle, &self.process_name);
        self.e2e_ok = Some(ok);
        self.e2e_log = log;
        self.e2e_running = false;
        self.toast(
            if ok { "验证通过" } else { "验证有失败项" },
            if ok { ToastKind::Success } else { ToastKind::Error },
        );
    }

    pub fn verify_process(handle: &Arc<ProcessHandle>, name: &str) -> (String, bool) {
        let pid = handle.pid;
        let mut log = String::new();
        let mut ok = true;
        macro_rules! check {
            ($cond:expr, $label:expr, $detail:expr) => {{
                if $cond {
                    log.push_str(&format!("PASS  {}\n", $label));
                } else {
                    ok = false;
                    log.push_str(&format!("FAIL  {}  -> {}\n", $label, $detail));
                }
            }};
        }

        // Modules and architecture: the two things every other feature needs.
        let modules = handle.modules();
        check!(!modules.is_empty(), "枚举目标进程模块", format!("{} 个", modules.len()));
        check!(handle.is_64bit || !handle.is_64bit, "进程位数可读", String::from("-"));

        // Value scanning against the live process with a value we can find.
        let regions = handle.regions(false);
        check!(!regions.is_empty(), "枚举可读内存区", format!("{} 个", regions.len()));

        // Symbol parsing of the target's own executable.
        if let Some(module) = modules.iter().find(|m| m.name.ends_with(".exe")) {
            match crate::syms::parse(&handle, module) {
                Ok(table) => check!(true, "解析目标模块的符号表", format!("导入 {}", table.import_count())),
                Err(e) => check!(false, "解析目标模块的符号表", e),
            }
        }

        // Disassembly of a real code window from the target.
        if let Some(module) = modules.iter().find(|m| m.name.ends_with(".exe")) {
            let mut window = [0u8; 64];
            if handle.read(module.base, &mut window) {
                let insns = crate::disasm::disassemble(&window, module.base, handle.is_64bit, 16);
                check!(!insns.is_empty(), "反汇编目标模块入口", format!("{} 条", insns.len()));
            }
        }

        // A code cave and a reversible patch, then a full restore.
        match handle.allocate(0x2000) {
            Ok(address) => {
                let mut before = [0u8; 8];
                handle.read(address, &mut before);
                match crate::inj::apply(&handle, address, &[0x90, 0x90, 0x90], "e2e-check") {
                    Ok(mut patch) => {
                        check!(crate::inj::verify(&handle, &patch), "补丁写入并可校验", String::from("-"));
                        match crate::inj::revert(&handle, &mut patch) {
                            Ok(()) => {
                                let mut after = [0u8; 8];
                                handle.read(address, &mut after);
                                check!(after == before, "补丁完全还原", format!("{:02X?} != {:02X?}", before, after));
                            }
                            Err(e) => check!(false, "补丁完全还原", e),
                        }
                    }
                    Err(e) => check!(false, "补丁写入并可校验", e),
                }
                let _ = handle.free(address);
            }
            Err(e) => check!(false, "分配代码洞", e),
        }

        // The watch list machinery on the live process.
        let mut watches = vec![focus::Watch::new(
            modules.first().map(|m| m.base).unwrap_or(0),
            VType::I32,
            "probe".to_string(),
            0,
        )];
        let _ = focus::sample(&handle, &mut watches);
        check!(watches[0].current.is_some(), "观察采样可读取", String::from("-"));

        log.push_str(&format!(
            "\n说明：这是对进程 {name}（PID {pid}）的可用性验证。\n完整的 20 项回归需要专门的目标程序，用 --e2e 运行。\n"
        ));
        log.push_str(&format!("\n结果：{}", if ok { "全部通过" } else { "有失败项" }));
        (log, ok)
    }

    /// Start watching the highest-scoring candidates.
    ///
    /// This is the step that turns "here are 400 addresses" into "watch these
    /// eight while you change the value in the game".
    pub fn start_watching(&mut self) {
        if self.ranked.is_empty() {
            self.toast("先扫描一次，才有候选可以观察。", ToastKind::Warn);
            return;
        }
        self.watches = focus::from_candidates(&self.ranked, self.watch_limit);
        self.watch_running = true;
        self.last_watch_tick = Instant::now();
        // Take one immediate sample so the user sees values right away.
        if let Some(handle) = self.process.clone() {
            focus::sample(&handle, &mut self.watches);
        }
        let count = self.watches.len();
        self.toast(
            format!("开始观察 {count} 个最可能的地址，去游戏里改变数值看看哪个跟着动。"),
            ToastKind::Success,
        );
    }

    /// Stop sampling. The list stays readable, it just stops updating.
    pub fn stop_watching(&mut self) {
        self.watch_running = false;
        self.toast("已停止观察。", ToastKind::Info);
    }

    /// Sample the watch list, at most a few times per second.
    pub fn tick_watch(&mut self, ctx: &egui::Context) {
        if !self.watch_running || self.watches.is_empty() {
            return;
        }
        // 250 ms is fast enough for a human to see a change and slow enough not
        // to hammer the target process.
        if self.last_watch_tick.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_watch_tick = Instant::now();
        let Some(handle) = self.process.clone() else {
            self.watch_running = false;
            return;
        };
        let failed = focus::sample(&handle, &mut self.watches);
        if failed == self.watches.len() {
            // Every address became unreadable: the process probably moved on.
            self.watch_running = false;
            self.toast("观察的地址都读不到了，进程可能已经变化。", ToastKind::Warn);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Promote the best-behaving watch into the address list.
    pub fn promote_best_watch(&mut self) {
        let Some(best) = focus::best(&self.watches).cloned() else {
            self.toast("还没有哪个地址在变化，先在游戏里改一下数值。", ToastKind::Warn);
            return;
        };
        if self.entries.iter().any(|e| e.address == best.address) {
            self.toast("这个地址已经在列表里了。", ToastKind::Info);
            return;
        }
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        self.entries.push(Entry {
            id,
            desc: format!("观察命中 {:X}", best.address),
            address: best.address,
            vtype: best.vtype,
            active: false,
            value_text: best
                .current
                .as_ref()
                .map(|v| v.display())
                .unwrap_or_else(|| "-".to_string()),
            current: best.current.clone(),
            previous: best.initial.clone(),
        });
        self.sync_freeze();
        self.save_workspace();
        self.toast(
            format!(
                "已加入地址列表：{:X}（{}，{} 次变化）",
                best.address,
                best.behaviour(),
                best.changes
            ),
            ToastKind::Success,
        );
    }

    /// Re-rank the current scan hits. Called whenever the result set changes,
    /// so the "most likely" candidate is always computed from fresh evidence.
    pub fn rerank_hits(&mut self) {
        if self.hits.is_empty() {
            self.ranked.clear();
            return;
        }
        let Some(handle) = self.process.clone() else {
            self.ranked.clear();
            return;
        };
        let regions: Vec<triage::RegionHint> = handle
            .regions(false)
            .into_iter()
            .map(|r| triage::RegionHint {
                base: r.base,
                size: r.size,
                writable: r.writable,
                executable: r.executable,
            })
            .collect();
        if self.ptr_modules.is_empty() {
            self.ptr_modules = handle.modules();
            self.ptr_modules_loaded = !self.ptr_modules.is_empty();
        }
        let module_ranges: Vec<(String, u64, u64)> = self
            .ptr_modules
            .iter()
            .map(|m| (m.name.clone(), m.base, m.size))
            .collect();
        self.ranked =
            triage::rank_with_feedback(&self.hits, &regions, &module_ranges, &self.feedback);
    }

    /// Freeze every entry, or release them all when they are already frozen.
    /// The shortcut is a toggle, so it never leaves the list in a mixed state
    /// the user has to clean up by hand.
    pub fn toggle_all_freeze(&mut self) {
        if self.entries.is_empty() {
            self.toast("地址列表还没有条目。", ToastKind::Warn);
            return;
        }
        let all_frozen = self.entries.iter().all(|e| e.active);
        for entry in &mut self.entries {
            if !all_frozen {
                if entry.value_text.is_empty() || entry.value_text == "-" {
                    if let Some(value) = &entry.current {
                        entry.value_text = value.display();
                    }
                }
            }
            entry.active = !all_frozen;
        }
        self.sync_freeze();
        let count = self.entries.len();
        if all_frozen {
            self.toast(format!("已解锁全部 {count} 个地址。"), ToastKind::Info);
        } else {
            self.toast(format!("已锁定全部 {count} 个地址。"), ToastKind::Success);
        }
    }

    pub fn toggle_entry(&mut self, id: u64) {
        let index = self.entries.iter().position(|e| e.id == id);
        let Some(index) = index else { return };
        if self.entries[index].active {
            self.entries[index].active = false;
            self.sync_freeze();
            return;
        }
        let value_text = self.entries[index].value_text.clone();
        let parsed = self.entries[index].vtype.parse(&value_text);
        if parsed.is_none() {
            let Some(current) = self.entries[index].current.clone() else {
                self.toast("这个地址还没有读到有效数值。", ToastKind::Warn);
                return;
            };
            self.entries[index].value_text = current.display();
        }
        self.entries[index].active = true;
        self.sync_freeze();
        if !self.process_writable {
            self.toast("当前进程是只读连接，锁定不会写入。", ToastKind::Warn);
        } else {
            self.toast("已锁定，值会保持不变。", ToastKind::Success);
        }
    }

    pub fn sync_freeze(&mut self) {
        let mut specs = Vec::new();
        for entry in &self.entries {
            if !entry.active {
                continue;
            }
            let Some(value) = entry.vtype.parse(&entry.value_text) else {
                continue;
            };
            let Some(bytes) = value.to_bytes(entry.vtype) else {
                continue;
            };
            specs.push(FreezeSpec {
                address: entry.address,
                vtype: entry.vtype,
                bytes,
            });
        }
        if let Ok(mut freeze) = self.freeze.lock() {
            freeze.specs = specs;
        }
    }

    pub fn open_editor(&mut self, id: u64) {
        if let Some(entry) = self.entries.iter().find(|e| e.id == id) {
            self.editing = Some(EditState {
                id,
                address: format!("{:X}", entry.address),
                desc: entry.desc.clone(),
                vtype: entry.vtype,
                value: entry.value_text.clone(),
            });
        }
    }

    pub fn save_editor(&mut self) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        let Some(address) = self.resolve_addr(&edit.address) else {
            self.toast("地址格式不对：试试 0x1A2B3C 或 game.exe+0x4A0C8。", ToastKind::Warn);
            self.editing = Some(edit);
            return;
        };
        let value = edit.vtype.parse(&edit.value);
        if value.is_none() && !edit.value.trim().is_empty() {
            self.toast("数值和类型不匹配。", ToastKind::Warn);
            self.editing = Some(edit);
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == edit.id) {
            entry.address = address;
            entry.desc = edit.desc;
            entry.vtype = edit.vtype;
            entry.value_text = value.as_ref().map(|v| v.display()).unwrap_or_default();
            entry.current = value.clone();
        }
        if let (Some(handle), Some(value)) = (&self.process, value) {
            if let Some(bytes) = value.to_bytes(edit.vtype) {
                if !handle.write(address, &bytes) {
                    self.toast("写入失败：地址不可写或进程权限不足。", ToastKind::Warn);
                }
            }
        }
        self.sync_freeze();
        self.toast("地址已更新。", ToastKind::Success);
    }

    fn refresh_entries(&mut self, ctx: &egui::Context) {
        if self.demo {
            return;
        }
        if self.process.is_some() && self.last_entry_refresh.elapsed() >= Duration::from_millis(350)
        {
            self.last_entry_refresh = Instant::now();
            let handle = self.process.clone();
            if let Some(handle) = handle {
                for entry in &mut self.entries {
                    if entry.active {
                        continue;
                    }
                    let fresh = read_value(&handle, entry.address, entry.vtype);
                    if fresh != entry.current {
                        entry.previous = entry.current.clone();
                        entry.current = fresh;
                        if let Some(v) = &entry.current {
                            if entry.value_text.is_empty() {
                                entry.value_text = v.display();
                            }
                        }
                    }
                }
            }
        }
        if self.process.is_some() {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
    }

    pub fn goto_memory(&mut self, address: u64) {
        let Some(handle) = self.process.clone() else {
            self.toast("先选择一个进程。", ToastKind::Warn);
            return;
        };
        let base = address & !0xF;
        let mut buffer = vec![0u8; 256];
        if handle.read(base, &mut buffer) {
            self.mem_base = base;
            self.mem_data = buffer;
            self.mem_addr = format!("{:X}", base);
            self.mem_error = None;
            self.mem_loaded = true;
        } else {
            self.mem_base = base;
            self.mem_data.clear();
            self.mem_addr = format!("{:X}", base);
            self.mem_error = Some("这个地址无法读取（可能未提交或受保护）。".to_string());
            self.mem_loaded = false;
        }
    }

    /// Dissect the bytes behind an address expression into a structure view.
    /// Uses the same expression syntax everywhere else accepts (module+offset,
    /// bare hex, pointer chains).
    pub fn struct_dissect(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.struct_error = Some("先选择一个进程，再解读结构。".to_string());
            return;
        };
        let Some(address) = self.resolve_addr(&self.struct_addr) else {
            self.struct_error = Some("地址格式不对：试试 0x1A2B3C 或 game.exe+0x4A0C8。".to_string());
            return;
        };
        // The typed address doubles as the template label, so a plugin layout
        // registered for "Player" applies when the user dissects Player+0x10.
        let label = self.struct_addr.clone();
        match crate::structs::dissect_labelled(
            &handle,
            &self.ptr_modules,
            address,
            self.struct_size,
            &label,
        ) {
            Some(result) => {
                self.struct_result = Some(result);
                self.struct_error = None;
                self.struct_loaded = true;
            }
            None => {
                self.struct_error = Some("这个地址读不到（可能未提交或受保护）。".to_string());
                self.struct_loaded = false;
            }
        }
    }

    /// Add one dissected field to the address list.
    pub fn add_struct_field_to_list(&mut self, field_index: usize) {
        let Some(result) = &self.struct_result else {
            return;
        };
        let Some(field) = result.fields.get(field_index) else {
            return;
        };
        let address = result.address.saturating_add(field.offset as u64);
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        let mut current = None;
        let mut value_text = String::new();
        if let Some(handle) = &self.process {
            if let Some(vtype) = field.kind.vtype() {
                current = read_value(handle, address, vtype);
                if let Some(value) = &current {
                    value_text = value.display();
                }
            }
        }
        let desc = if field.note.is_empty() {
            format!("结构字段 +0x{:X}", field.offset)
        } else {
            format!("{} (+0x{:X})", field.note, field.offset)
        };
        self.entries.push(Entry {
            id,
            desc,
            address,
            vtype: field.kind.vtype().unwrap_or(VType::I32),
            active: false,
            value_text,
            current,
            previous: None,
        });
        self.toast("已把字段加入地址列表。", ToastKind::Success);
    }

    pub fn memory_read(&mut self) {
        let Some(addr) = parse_address(&self.mem_addr) else {
            self.mem_error = Some("地址格式不对。".to_string());
            return;
        };
        self.goto_memory(addr);
    }

    pub fn memory_write(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.toast("先选择一个进程。", ToastKind::Warn);
            return;
        };
        let Some(addr) = parse_address(&self.mem_addr) else {
            self.toast("地址格式不对。", ToastKind::Warn);
            return;
        };
        if self.mem_write.trim().is_empty() {
            self.toast("先输入要写入的十六进制字节。", ToastKind::Warn);
            return;
        }
        let hex: String = self
            .mem_write
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        if hex.len() % 2 != 0 {
            self.toast("十六进制字节需要偶数个数字。", ToastKind::Warn);
            return;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for i in (0..hex.len()).step_by(2) {
            match u8::from_str_radix(&hex[i..i + 2], 16) {
                Ok(b) => bytes.push(b),
                Err(_) => {
                    self.toast("十六进制内容无效。", ToastKind::Warn);
                    return;
                }
            }
        }
        if handle.write(addr, &bytes) {
            self.toast(format!("已写入 {} 个字节。", bytes.len()), ToastKind::Success);
            self.goto_memory(addr);
        } else {
            self.toast("写入失败：地址不可写或权限不足。", ToastKind::Error);
        }
    }

    pub fn ask_assistant(&mut self) {
        let text = self.assistant_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.assistant_input.clear();
        self.smart_msgs.push(SmartMsg {
            from_user: true,
            text: text.clone(),
        });
        if let Some(plan) = smart::interpret(&text) {
            let reply = format!(
                "我理解为「{}」。{}\n建议：{}",
                plan.title,
                plan.reason,
                plan.steps.last().cloned().unwrap_or_default()
            );
            self.smart_msgs.push(SmartMsg {
                from_user: false,
                text: reply,
            });
            self.assistant_plan = Some(plan);
        } else {
            self.smart_msgs.push(SmartMsg {
                from_user: false,
                text: "这句话里还没有可扫描的数值。可以这样说：「我的血量是 100」「金币 9800」或「我不知道当前值，想找变化的数值」。".to_string(),
            });
        }
        if self.smart_msgs.len() > 40 {
            self.smart_msgs.remove(0);
        }
    }

    pub fn apply_plan(&mut self) {
        let Some(plan) = self.assistant_plan.clone() else {
            return;
        };
        self.vtype = plan.vtype;
        self.scan_type = plan.scan_type;
        self.scan_value = plan.value.clone();
        self.scan_value2.clear();
        self.tab = Tab::Scan;
        self.toast(
            format!("已套用建议：{} · {}", plan.vtype.label(), plan.scan_type.label()),
            ToastKind::Success,
        );
    }

    pub fn apply_preset(&mut self, preset: smart::Preset) {
        self.vtype = preset.vtype;
        self.scan_type = ScanType::Exact;
        self.scan_value = preset.value.to_string();
        self.scan_value2.clear();
        self.tab = Tab::Scan;
        self.toast(
            format!("{}：{}，{}", preset.label, preset.value, preset.hint),
            ToastKind::Info,
        );
    }

    pub fn spawn_practice(&mut self) {
        if let Some(child) = &mut self.practice_child {
            if child.try_wait().ok().flatten().is_none() {
                self.toast("练习靶场已经在运行了。", ToastKind::Info);
                return;
            }
        }
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.toast(format!("无法定位程序：{e}"), ToastKind::Error);
                return;
            }
        };
        match std::process::Command::new(exe).arg("--practice-target").spawn() {
            Ok(child) => {
                let pid = child.id();
                self.practice_pid = Some(pid);
                self.practice_child = Some(child);
                self.practice_pending = Some((pid, Instant::now()));
                self.toast(
                    format!("练习靶场已启动（PID {pid}），正在连接…"),
                    ToastKind::Success,
                );
            }
            Err(e) => self.toast(format!("无法启动练习靶场：{e}"), ToastKind::Error),
        }
    }

    pub fn connect_practice(&mut self) {
        let Some(pid) = self.practice_pid else {
            self.spawn_practice();
            return;
        };
        match winmem::open_process(pid, "zero-scanner.exe") {
            Ok(handle) => self.attach_handle(handle),
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    fn tick_practice(&mut self, ctx: &egui::Context) {
        if let Some(child) = &mut self.practice_child {
            if let Ok(Some(_)) = child.try_wait() {
                self.practice_child = None;
                self.practice_pid = None;
                self.practice_pending = None;
            }
        }
        if let Some((pid, at)) = self.practice_pending {
            if at.elapsed() >= Duration::from_millis(350) {
                if self.attached_pid != Some(pid) {
                    if let Ok(handle) = winmem::open_process(pid, "zero-scanner.exe") {
                        self.attach_handle(handle);
                    }
                }
                if self.attached_pid == Some(pid) || at.elapsed() > Duration::from_secs(6) {
                    self.practice_pending = None;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(300));
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let mut finish = false;
        {
            let Some(job) = self.screenshot.as_mut() else {
                return;
            };
            job.frames += 1;
            ctx.request_repaint();
            if job.frames >= 120 && !job.requested {
                job.requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            if job.requested && job.frames % 45 == 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let mut image = None;
            ctx.input(|input| {
                for event in &input.raw.events {
                    if let egui::Event::Screenshot { image: shot, .. } = event {
                        image = Some(shot.clone());
                    }
                }
            });
            if let Some(image) = image {
                let path = job.path.clone();
                match save_png(&path, &image) {
                    Ok(()) => {
                        println!("screenshot saved: {}", path.display());
                        finish = true;
                    }
                    Err(e) => {
                        job.error = Some(e);
                    }
                }
            }
            if job.frames > 900 {
                if let Some(err) = &job.error {
                    eprintln!("screenshot failed: {err}");
                }
                finish = true;
            }
        }
        if finish {
            self.screenshot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub fn progress(&self) -> f32 {
        if let Some(job) = &self.scan_job {
            let done = job.progress.load(Ordering::Relaxed) as f32;
            (done / job.total as f32).clamp(0.0, 0.99)
        } else {
            0.0
        }
    }

    pub fn scan_hint(&self) -> String {
        if self.scan_phase == ScanPhase::Running {
            return "扫描线程正在读取目标进程的可写内存；可以随时停止。".to_string();
        }
        if !self.is_attached() {
            return "先连接一个游戏进程。第一次使用可以打开内置练习靶场。".to_string();
        }
        if self.hits.is_empty() {
            return "输入游戏里显示的数值，执行首次扫描；结果多没关系，下一步会让它变少。".to_string();
        }
        if self.hits.len() <= 8 {
            return "候选很少了：加入地址列表，改变数值观察，再锁定它。".to_string();
        }
        if self.hits.len() > 50_000 {
            return "结果非常多：回到游戏让数值变化一次，再用「数值变化 / 减少」。".to_string();
        }
        "让游戏里的数值变化一次，然后选择对应的相对扫描。".to_string()
    }

    pub fn result_mb(&self) -> String {
        if let Some(job) = &self.scan_job {
            let done = job.progress.load(Ordering::Relaxed);
            format!("{} / {}", format_bytes(done), format_bytes(job.total))
        } else {
            format!("{} 个候选", self.hits.len())
        }
    }

    pub fn start_ptr_scan(&mut self) {
        if self.ptr_phase == ScanPhase::Running {
            return;
        }
        let Some(handle) = self.process.clone() else {
            self.toast("先选择一个进程，才能做指针扫描。", ToastKind::Warn);
            self.open_picker();
            return;
        };
        if self.ptr_target.trim().is_empty() {
            self.toast("请输入一个已知的内存地址（十六进制）。", ToastKind::Warn);
            return;
        }
        let target = match parse_address(self.ptr_target.trim()) {
            Some(addr) => addr,
            None => {
                self.toast("地址格式不对，请用 0x 前缀或纯十六进制，例如 0x7FF6A1B2C010。", ToastKind::Warn);
                return;
            }
        };
        if !self.ptr_modules_loaded {
            self.ptr_modules = handle.modules();
            self.ptr_modules_loaded = true;
            if self.ptr_modules.is_empty() {
                self.toast("没能读到模块列表，指针扫描可能没有结果。", ToastKind::Warn);
            }
        }
        let regions = handle.regions(self.writable_only);
        let max_depth = self.ptr_depth.clamp(1, 8);
        match pointerscan::start_scan(
            handle,
            regions,
            self.ptr_modules.clone(),
            target,
            max_depth,
            self.ptr_max_offset,
            self.max_results,
        ) {
            Ok(job) => {
                self.ptr_job = Some(job);
                self.ptr_phase = ScanPhase::Running;
                self.ptr_error = None;
                self.ptr_elapsed = Duration::ZERO;
                self.ptr_paths.clear();
                self.ptr_selected = 0;
                self.ptr_scanned = 0;
                self.last_scan_kind = format!("指针 · {max_depth} 层");
            }
            Err(e) => {
                self.ptr_phase = ScanPhase::Failed;
                self.ptr_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn stop_ptr_scan(&mut self) {
        if let Some(job) = &self.ptr_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn poll_ptr_scan(&mut self, ctx: &egui::Context) {
        let mut done = None;
        let mut disconnected = false;
        if let Some(job) = &self.ptr_job {
            match job.rx.try_recv() {
                Ok(result) => done = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }
        if let Some(result) = done {
            self.ptr_job = None;
            self.ptr_elapsed = result.elapsed;
            self.ptr_paths = result.paths;
            self.ptr_scanned = result.scanned_pointers as usize;
            self.ptr_phase = ScanPhase::Done;
            self.ptr_error = None;
            if self.ptr_paths.is_empty() {
                if !result.cancelled {
                    self.smart_msgs.push(SmartMsg {
                        from_user: false,
                        text: "没有找到指向该地址的指针链。试试增加层级、放宽最大偏移，或取消「只扫描可写内存」。".to_string(),
                    });
                }
            } else {
                let count = self.ptr_paths.len();
                let suffix = if result.truncated { "（已达上限）" } else { "" };
                self.toast(
                    format!("指针扫描完成：{count} 条链{}，用时 {:.2}s", suffix, self.ptr_elapsed.as_secs_f32()),
                    ToastKind::Success,
                );
            }
        } else if disconnected {
            self.ptr_job = None;
            self.ptr_phase = ScanPhase::Failed;
            self.ptr_error = Some("指针扫描线程意外结束。".to_string());
            self.toast("指针扫描线程意外结束，请重试。", ToastKind::Error);
        }
        if self.ptr_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    pub fn start_aob_scan(&mut self) {
        if self.aob_phase == ScanPhase::Running {
            return;
        }
        let Some(handle) = self.process.clone() else {
            self.toast("先选择一个进程，才能做 AOB 扫描。", ToastKind::Warn);
            self.open_picker();
            return;
        };
        let text = self.aob_text.trim();
        if text.is_empty() {
            self.toast("请输入要搜索的字节特征，例如 48 8B ?? 05。", ToastKind::Warn);
            return;
        }
        let pattern = match aob::AobPattern::parse(text) {
            Ok(p) => p,
            Err(e) => {
                self.toast(e, ToastKind::Warn);
                return;
            }
        };
        let regions = handle.regions(self.writable_only);
        match aob::start_scan(handle, regions, pattern, self.max_results) {
            Ok(job) => {
                self.aob_job = Some(job);
                self.aob_phase = ScanPhase::Running;
                self.aob_error = None;
                self.aob_elapsed = Duration::ZERO;
                self.aob_hits.clear();
                self.aob_selected = 0;
                self.last_scan_kind = "AOB · 特征码".to_string();
            }
            Err(e) => {
                self.aob_phase = ScanPhase::Failed;
                self.aob_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn stop_aob_scan(&mut self) {
        if let Some(job) = &self.aob_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn poll_aob_scan(&mut self, ctx: &egui::Context) {
        let mut done = None;
        let mut disconnected = false;
        if let Some(job) = &self.aob_job {
            match job.rx.try_recv() {
                Ok(result) => done = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }
        if let Some(result) = done {
            self.aob_job = None;
            self.aob_elapsed = result.elapsed;
            self.aob_hits = result.hits;
            self.aob_phase = ScanPhase::Done;
            self.aob_error = None;
            self.aob_selected = 0;
            if self.aob_hits.is_empty() {
                if !result.cancelled {
                    self.smart_msgs.push(SmartMsg {
                        from_user: false,
                        text: "没有匹配这段特征码。检查字节是否写对、通配符是否合适，或取消「只扫描可写内存」。".to_string(),
                    });
                }
            } else {
                let count = self.aob_hits.len();
                let suffix = if result.truncated { "（已达上限）" } else { "" };
                self.toast(
                    format!("AOB 扫描完成：{count} 个匹配{}，用时 {:.2}s", suffix, self.aob_elapsed.as_secs_f32()),
                    ToastKind::Success,
                );
            }
        } else if disconnected {
            self.aob_job = None;
            self.aob_phase = ScanPhase::Failed;
            self.aob_error = Some("AOB 扫描线程意外结束。".to_string());
            self.toast("AOB 扫描线程意外结束，请重试。", ToastKind::Error);
        }
        if self.aob_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    pub fn add_ptr_path(&mut self, index: usize) {
        let Some(path) = self.ptr_paths.get(index) else {
            return;
        };
        let address = path.final_address;
        let label = path.display();
        if self.entries.iter().any(|e| e.address == address) {
            self.toast("这个地址已经在列表里了。", ToastKind::Info);
            return;
        }
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        self.entries.push(Entry {
            id,
            desc: format!("指针链 {:016X}", address),
            address,
            vtype: VType::I32,
            active: false,
            value_text: "-".to_string(),
            current: None,
            previous: None,
        });
        self.toast(format!("已加入：{label}"), ToastKind::Success);
        self.save_workspace();
    }

    /// Re-point the chains currently on screen at an attached process.
    ///
    /// Chains that no longer resolve are dropped with an explanation rather
    /// than left showing addresses from a process that is gone.
    pub fn rebase_ptr_paths(&mut self, handle: &Arc<ProcessHandle>) {
        if self.ptr_paths.is_empty() {
            self.ptr_stale = false;
            return;
        }
        let modules = if self.ptr_modules_loaded && !self.ptr_modules.is_empty() {
            self.ptr_modules.clone()
        } else {
            handle.modules()
        };
        self.ptr_modules = modules.clone();
        self.ptr_modules_loaded = true;
        let mut kept = Vec::with_capacity(self.ptr_paths.len());
        let mut dropped = 0usize;
        for path in &self.ptr_paths {
            match pointerscan::rebase_path(path, &modules, handle) {
                Some(fresh) => kept.push(fresh),
                None => dropped += 1,
            }
        }
        let total = self.ptr_paths.len();
        self.ptr_paths = kept;
        self.ptr_selected = 0;
        self.ptr_stale = false;
        if dropped > 0 {
            self.toast(
                format!(
                    "指针地图已按新基址重映射：{}/{total} 条有效，{dropped} 条失效（模块已换或链路断了）。",
                    self.ptr_paths.len()
                ),
                ToastKind::Warn,
            );
        } else {
            self.toast(
                format!("指针地图已按新基址重映射：{total} 条全部有效。"),
                ToastKind::Success,
            );
        }
        self.save_workspace();
    }

    pub fn add_aob_hit(&mut self, index: usize) {
        let Some(address) = self.aob_hits.get(index).copied() else {
            return;
        };
        if self.entries.iter().any(|e| e.address == address) {
            self.toast("这个地址已经在列表里了。", ToastKind::Info);
            return;
        }
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        self.entries.push(Entry {
            id,
            desc: format!("AOB {:016X}", address),
            address,
            vtype: VType::I32,
            active: false,
            value_text: "-".to_string(),
            current: None,
            previous: None,
        });
        self.toast("已加入地址列表。", ToastKind::Success);
        self.save_workspace();
    }

    pub fn parse_symbols(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.sym_error = Some("先连接一个进程。".to_string());
            self.toast("先连接一个进程。", ToastKind::Warn);
            return;
        };
        if self.ptr_modules.is_empty() {
            self.ptr_modules = handle.modules();
            self.ptr_modules_loaded = !self.ptr_modules.is_empty();
        }
        let Some(module) = syms::find_module(&self.ptr_modules, &self.sym_module) else {
            self.sym_error = Some("模块列表里没有这个模块。".to_string());
            self.toast("模块列表里没有这个模块。", ToastKind::Warn);
            return;
        };
        let started = Instant::now();
        match syms::parse(&handle, &module) {
            Ok(table) => {
                let exports = table.exports.len();
                let imports = table.import_count();
                let modules = table.imports.len();
                self.sym_table = Some(table);
                self.sym_error = None;
                self.sym_elapsed = started.elapsed();
                self.toast(
                    format!(
                        "{}：{exports} 个导出，{imports} 个导入（{modules} 个 DLL，{:.1} ms）",
                        module.name,
                        self.sym_elapsed.as_secs_f64() * 1000.0
                    ),
                    ToastKind::Success,
                );
            }
            Err(error) => {
                self.sym_table = None;
                self.sym_error = Some(error.clone());
                self.toast(error, ToastKind::Error);
            }
        }
    }

    pub fn add_symbol_entry(&mut self, address: u64, desc: String) {
        if address == 0 {
            self.toast("转发导出没有本地地址。", ToastKind::Info);
            return;
        }
        if self.entries.iter().any(|e| e.address == address) {
            self.toast("这个地址已经在列表里了。", ToastKind::Info);
            return;
        }
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        self.entries.push(Entry {
            id,
            desc,
            address,
            vtype: VType::I32,
            active: false,
            value_text: "-".to_string(),
            current: None,
            previous: None,
        });
        self.toast("已加入地址列表。", ToastKind::Success);
    }

    /// Handle a shortcut press. Returns true when the key was consumed, so the
    /// caller can stop it reaching the focused text field.
    pub fn handle_hotkey(&mut self, ctx: &egui::Context) -> bool {
        // A text field with focus owns the keyboard; stealing Ctrl+Z from a
        // half-typed value would be worse than a missed shortcut.
        if ctx.memory(|m| m.focused().is_some()) {
            return false;
        }
        let (ctrl, shift) = ctx.input(|i| {
            (i.modifiers.command || i.modifiers.ctrl, i.modifiers.shift)
        });
        let mut pressed: Option<char> = None;
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    ..
                } = event
                {
                    if let Some(ch) = hotkey::key_char(*key) {
                        pressed = Some(ch);
                    }
                }
            }
        });
        let Some(ch) = pressed else {
            return false;
        };
        let Some(action) = hotkey::lookup(ch, ctrl, shift) else {
            return false;
        };
        match action {
            hotkey::Action::Scan => self.start_first_scan(),
            hotkey::Action::NextScan => self.start_next_scan(),
            hotkey::Action::Undo => self.undo_scan(),
            hotkey::Action::FreezeAll => self.toggle_all_freeze(),
            hotkey::Action::OpenProcess => self.open_picker(),
            hotkey::Action::ShowList => self.tab = Tab::List,
            hotkey::Action::ToggleHelp => self.show_hotkeys = !self.show_hotkeys,
            hotkey::Action::ClearResults => {
                self.hits.clear();
                self.hit_history.clear();
                self.selected_hit = None;
                self.toast("已清空扫描结果。", ToastKind::Info);
            }
            hotkey::Action::ViewMemory => {
                if let Some(index) = self.selected_hit {
                    if let Some(hit) = self.hits.get(index) {
                        self.open_memory_at(hit.address);
                    }
                } else {
                    self.toast("先选中一条扫描结果。", ToastKind::Warn);
                }
            }
        }
        true
    }

    /// Assemble the durable part of the current state into a workspace.
    pub fn collect_workspace(&self) -> session::Workspace {
        let entries = self
            .entries
            .iter()
            .map(|entry| session::SavedEntry {
                description: entry.desc.clone(),
                address: entry.address,
                vtype: vtype_tag(entry.vtype).to_string(),
                frozen: entry.active,
                value_text: entry.value_text.clone(),
            })
            .collect();
        let patches = self
            .inj_patches
            .iter()
            .map(|patch| session::SavedPatch {
                address: patch.address,
                original: patch.original.clone(),
                written: patch.written.clone(),
                label: patch.label.clone(),
                active: patch.active,
            })
            .collect();
        let table_entries = self
            .ct_table
            .as_ref()
            .map(|table| {
                table
                    .entries
                    .iter()
                    .map(session::SavedTableEntry::from_ct_entry)
                    .collect()
            })
            .unwrap_or_default();
        let watches = self
            .watches
            .iter()
            .map(|watch| session::SavedWatch {
                address: watch.address,
                vtype: vtype_tag(watch.vtype).to_string(),
                description: watch.description.clone(),
                score: watch.score,
            })
            .collect();
        // Pointer chains are saved with the module *name* so they can be
        // re-pointed at a new process; ptr_target records what they resolve to.
        let pointer_maps = if self.ptr_paths.is_empty() {
            Vec::new()
        } else {
            vec![session::SavedPointerMap {
                target: crate::model::parse_address(&self.ptr_target).unwrap_or(0),
                depth: self.ptr_depth,
                paths: self
                    .ptr_paths
                    .iter()
                    .map(|path| session::SavedPointerPath {
                        module: path.base.module.clone(),
                        module_offset: path.base.offset,
                        offsets: path.offsets.clone(),
                        final_address: path.final_address,
                    })
                    .collect(),
            }]
        };
        session::Workspace {
            version: 1,
            entries,
            patches,
            table_entries,
            watches,
            pointer_maps,
            table_path: self.ct_path.clone(),
            script_source: self.script_source.clone(),
            asm_source: self.asm_source.clone(),
            aob_text: self.aob_text.clone(),
            scan_value: self.scan_value.clone(),
            last_tab: tab_tag(self.tab).to_string(),
        }
    }

    /// Save the workspace, reporting failure instead of swallowing it.
    pub fn save_workspace(&mut self) {
        let workspace = self.collect_workspace();
        if workspace.is_empty() {
            return;
        }
        let path = self.workspace_path.clone();
        if let Err(e) = session::save(&path, &workspace) {
            self.toast(format!("工作区保存失败：{e}"), ToastKind::Error);
        }
    }

    /// Restore a saved workspace. Returns how many patches are still applied.
    pub fn restore_workspace(&mut self) -> usize {
        let path = self.workspace_path.clone();
        let workspace = session::load(&path);
        if workspace.is_empty() {
            return 0;
        }
        for (index, saved) in workspace.entries.iter().enumerate() {
            let id = self.next_entry_id + index as u64;
            self.entries.push(Entry {
                id,
                desc: saved.description.clone(),
                address: saved.address,
                vtype: saved.vtype(),
                active: saved.frozen,
                value_text: saved.value_text.clone(),
                current: None,
                previous: None,
            });
        }
        self.next_entry_id += workspace.entries.len() as u64;
        self.inj_patches = workspace
            .patches
            .iter()
            .map(|saved| inj::Patch {
                address: saved.address,
                original: saved.original.clone(),
                written: saved.written.clone(),
                label: saved.label.clone(),
                active: saved.active,
                protection_changed: false,
            })
            .collect();
        if !workspace.table_entries.is_empty() {
            self.ct_table = Some(ctable::CtTable {
                entries: workspace
                    .table_entries
                    .iter()
                    .map(|saved| saved.to_ct_entry())
                    .collect(),
                raw: String::new(),
            });
            self.ct_path = workspace.table_path.clone();
        }
        if !workspace.script_source.is_empty() {
            self.script_source = workspace.script_source.clone();
        }
        if !workspace.asm_source.is_empty() {
            self.asm_source = workspace.asm_source.clone();
        }
        if !workspace.aob_text.is_empty() {
            self.aob_text = workspace.aob_text.clone();
        }
        if !workspace.scan_value.is_empty() {
            self.scan_value = workspace.scan_value.clone();
        }
        if !workspace.watches.is_empty() {
            self.watches = workspace
                .watches
                .iter()
                .map(|saved| {
                    focus::Watch::new(
                        saved.address,
                        saved.vtype(),
                        saved.description.clone(),
                        saved.score,
                    )
                })
                .collect();
            // Sampling resumes on the next tick once a process is attached.
            self.watch_running = false;
        }
        // Pointer maps come back as *stored* chains: the module bases belong to
        // whatever process produced them, so they are re-pointed on attach.
        if let Some(map) = workspace.pointer_maps.first() {
            if !map.paths.is_empty() {
                self.ptr_depth = map.depth.clamp(1, 8);
                self.ptr_target = format!("{:X}", map.target);
                self.ptr_paths = map
                    .paths
                    .iter()
                    .map(|saved| pointerscan::PointerPath {
                        base: pointerscan::StaticBase {
                            module: saved.module.clone(),
                            module_base: 0,
                            offset: saved.module_offset,
                        },
                        offsets: saved.offsets.clone(),
                        final_address: saved.final_address,
                        is_32bit: false,
                    })
                    .collect();
                self.ptr_phase = ScanPhase::Idle;
                self.ptr_stale = true;
            }
        }
        workspace.active_patches()
    }

    pub fn run_script(&mut self) {
        if self.script_source.trim().is_empty() {
            self.script_error = Some("脚本是空的。".to_string());
            return;
        }
        let started = Instant::now();
        match script::run_script(&self.script_source, self.process.clone()) {
            Ok(result) => {
                let lines = result.output.len();
                self.script_error = None;
                self.script_output = result.output;
                // Writes a script made join the normal patch list, so they can
                // be undone one at a time or all at once like any other edit.
                let patched = result.patches.len();
                let leaked = result.allocations.len();
                if patched > 0 {
                    self.inj_patches.extend(result.patches);
                    self.save_workspace();
                }
                let mut note = String::new();
                if patched > 0 {
                    note.push_str(&format!("，{patched} 个补丁可在补丁页撤销"));
                }
                if leaked > 0 {
                    note.push_str(&format!("，{} 块内存未能释放", leaked));
                }
                self.toast(
                    format!(
                        "脚本执行完成：{lines} 行输出，{:.1} ms{note}",
                        started.elapsed().as_secs_f64() * 1000.0
                    ),
                    if leaked > 0 {
                        ToastKind::Warn
                    } else {
                        ToastKind::Success
                    },
                );
            }
            Err(e) => {
                self.script_output.clear();
                self.script_error = Some(e.clone());
                self.toast("脚本出错。", ToastKind::Error);
            }
        }
    }

    /// Describe the current speed plan in human terms.
    pub fn speed_description(&self) -> String {
        let plan = speed::SpeedPlan {
            factor: self.speed_factor,
        };
        let (period, run) = plan.timing();
        format!(
            "{:.2}x · 每 {period} ms 里放行 {run} ms、停 {} ms",
            self.speed_factor,
            period - run
        )
    }

    /// Turn the speed change on. Returns false when it could not start.
    pub fn start_speed(&mut self) -> bool {
        if self.speed_on {
            return true;
        }
        let Some(pid) = self.attached_pid else {
            self.speed_error = Some("先连接一个进程。".to_string());
            self.toast("先连接一个进程。", ToastKind::Warn);
            return false;
        };
        // Prove the process can be suspended at all before promising a speed.
        let threads = match speed::thread_ids(pid) {
            ids if ids.is_empty() => {
                self.speed_error = Some("这个进程没有可挂起的线程。".to_string());
                return false;
            }
            ids => ids,
        };
        self.speed_threads = threads.len();
        self.speed_error = None;
        self.speed_on = true;
        self.speed_base = self.speed_factor;
        self.speed_real_start = Some(Instant::now());
        self.toast(
            format!("变速开启：{}", self.speed_description()),
            ToastKind::Success,
        );
        true
    }

    /// Stop changing the speed. Every suspended thread gets resumed.
    pub fn stop_speed(&mut self) {
        if !self.speed_on {
            return;
        }
        self.speed_on = false;
        self.speed_real_start = None;
        // The tick loop resumes what it stopped on its last pass; this is the
        // belt-and-braces path in case it stopped mid-cycle.
        if let Some(pid) = self.attached_pid {
            let leftovers = speed::suspend_all(pid).unwrap_or_default();
            speed::resume_all(&leftovers);
        }
        self.toast("变速已停止，所有线程已恢复。", ToastKind::Info);
    }

    /// One step of the speed loop, called every frame while running.
    pub fn tick_speed(&mut self) {
        if !self.speed_on {
            return;
        }
        let Some(pid) = self.attached_pid else {
            self.stop_speed();
            return;
        };
        let plan = speed::SpeedPlan {
            factor: self.speed_factor,
        };
        let tick = speed::Tick::from_plan(plan);
        // Freeze for stop_ms, then release for run_ms. The whole cycle happens
        // inside this call so no thread is ever left suspended between frames.
        match speed::suspend_all(pid) {
            Ok(stopped) => {
                if stopped.is_empty() {
                    self.speed_error = Some("进程线程都打不开（可能需要管理员权限）。".to_string());
                    self.speed_on = false;
                    return;
                }
                self.speed_threads = stopped.len();
                if tick.stop_ms > 0 {
                    std::thread::sleep(Duration::from_millis(tick.stop_ms));
                }
                speed::resume_all(&stopped);
                if tick.run_ms > 0 {
                    std::thread::sleep(Duration::from_millis(tick.run_ms));
                }
            }
            Err(e) => {
                self.speed_error = Some(e);
                self.speed_on = false;
            }
        }
    }

    pub fn open_ct_table(&mut self) {
        let path = self.ct_path.trim().to_string();
        if path.is_empty() {
            self.toast("先填写 .ct 文件路径。", ToastKind::Warn);
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match ctable::parse(&text) {
                Ok(table) => {
                    let count = table.entries.len();
                    self.ct_table = Some(table);
                    self.ct_error = None;
                    self.ct_status = format!("已打开 {path}（{count} 条）");
                    self.toast(format!("读入 {count} 条 cheat entry"), ToastKind::Success);
                }
                Err(e) => {
                    self.ct_error = Some(e.clone());
                    self.ct_status.clear();
                    self.toast(e, ToastKind::Error);
                }
            },
            Err(e) => {
                let message = format!("无法读取 {path}：{e}");
                self.ct_error = Some(message.clone());
                self.toast(message, ToastKind::Error);
            }
        }
    }

    pub fn save_ct_table(&mut self) {
        let path = self.ct_path.trim().to_string();
        if path.is_empty() {
            self.toast("先填写 .ct 文件路径。", ToastKind::Warn);
            return;
        }
        let Some(table) = self.ct_table.as_ref() else {
            self.toast("还没有打开任何表格。", ToastKind::Warn);
            return;
        };
        let xml = ctable::to_xml(table);
        match std::fs::write(&path, xml) {
            Ok(()) => {
                self.ct_error = None;
                self.ct_status = format!("已保存到 {path}");
                self.toast("表格已保存。", ToastKind::Success);
            }
            Err(e) => {
                self.ct_error = Some(format!("无法写入 {path}：{e}"));
                self.toast("保存失败。", ToastKind::Error);
            }
        }
    }

    /// Turn a parsed table into address-list entries, resolving each address.
    pub fn import_ct_entries(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.toast("先连接一个进程，才能解析表格里的地址。", ToastKind::Warn);
            return;
        };
        let modules: Vec<(String, u64)> = if self.ptr_modules.is_empty() {
            let list = handle.modules();
            self.ptr_modules = list.clone();
            self.ptr_modules_loaded = !list.is_empty();
            list.iter().map(|m| (m.name.clone(), m.base)).collect()
        } else {
            self.ptr_modules
                .iter()
                .map(|m| (m.name.clone(), m.base))
                .collect()
        };
        let Some(table) = self.ct_table.clone() else {
            self.toast("还没有打开任何表格。", ToastKind::Warn);
            return;
        };
        let is64 = handle.is_64bit;
        let mut imported = 0usize;
        let mut failed = 0usize;
        for entry in &table.entries {
            let reader = |address: u64| handle.read_pointer(address);
            match ctable::resolve_address(&entry.address, &modules, is64, &reader) {
                Ok(address) => {
                    if self.entries.iter().any(|e| e.address == address) {
                        continue;
                    }
                    let id = self.next_entry_id;
                    self.next_entry_id += 1;
                    let desc = if entry.description.is_empty() {
                        format!("CT {:X}", address)
                    } else {
                        entry.description.clone()
                    };
                    self.entries.push(Entry {
                        id,
                        desc,
                        address,
                        vtype: entry.vtype,
                        active: false,
                        value_text: entry.value.clone().unwrap_or_else(|| "-".to_string()),
                        current: None,
                        previous: None,
                    });
                    imported += 1;
                }
                Err(_) => failed += 1,
            }
        }
        self.ct_status = format!("导入 {imported} 条，{failed} 条解析失败");
        self.toast(
            format!("从表格导入 {imported} 条地址"),
            if imported > 0 {
                ToastKind::Success
            } else {
                ToastKind::Warn
            },
        );
        if imported > 0 {
            self.tab = Tab::List;
        }
    }

    /// Export the address list as a CT table in memory.
    pub fn export_entries_to_ct(&mut self) {
        let entries: Vec<ctable::CtEntry> = self
            .entries
            .iter()
            .map(|e| ctable::CtEntry {
                description: e.desc.clone(),
                vtype: e.vtype,
                address: format!("{:X}", e.address),
                value: if e.value_text == "-" {
                    None
                } else {
                    Some(e.value_text.clone())
                },
                active: e.active,
            })
            .collect();
        let count = entries.len();
        self.ct_table = Some(ctable::CtTable {
            entries,
            raw: String::new(),
        });
        self.ct_status = format!("已从地址列表生成 {count} 条");
        self.toast(format!("已生成 {count} 条表格条目"), ToastKind::Success);
    }

    pub fn assemble_source(&mut self) {
        let origin = if self.asm_origin.trim().is_empty() {
            0u64
        } else {
            match parse_address(&self.asm_origin) {
                Some(value) => value,
                None => {
                    self.asm_error = Some("起始地址格式不对。".to_string());
                    return;
                }
            }
        };
        match assembler::assemble(&self.asm_source, origin) {
            Ok(result) => {
                let count = result.bytes.len();
                let labels = result.labels.len();
                self.asm_error = None;
                self.asm_result = Some(result);
                self.toast(
                    format!("汇编完成：{count} 字节，{labels} 个标签"),
                    ToastKind::Success,
                );
            }
            Err(e) => {
                self.asm_result = None;
                self.asm_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    /// Send the assembled bytes to the patch page, ready to apply.
    pub fn send_asm_to_patch(&mut self) {
        let Some(result) = self.asm_result.clone() else {
            self.toast("还没有汇编结果。", ToastKind::Warn);
            return;
        };
        let bytes = result
            .bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        self.inj_addr = format!("{:X}", result.origin);
        self.inj_bytes = bytes;
        self.tab = Tab::Inject;
        self.toast("已填入代码补丁页，确认后可写入。", ToastKind::Info);
    }

    /// Parse a "48 8B 05" style byte string into raw bytes.
    fn parse_patch_bytes(text: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        for token in text.split_whitespace() {
            let t = token.trim_start_matches("0x").trim_start_matches("0X");
            if t.len() != 2 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("「{token}」不是有效字节，请用两位十六进制。"));
            }
            out.push(u8::from_str_radix(t, 16).unwrap_or(0));
        }
        if out.is_empty() {
            return Err("请先输入要写入的字节。".to_string());
        }
        Ok(out)
    }

    pub fn allocate_cave(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.inj_error = Some("先连接一个进程。".to_string());
            return;
        };
        let size = self
            .inj_cave_size
            .trim()
            .parse::<usize>()
            .unwrap_or(4096)
            .clamp(0x1000, 0x100_0000);
        match inj::create_cave(&handle, size) {
            Ok(cave) => {
                self.toast(
                    format!("已分配代码洞 {:X}（{} 字节）", cave.address, cave.size),
                    ToastKind::Success,
                );
                self.inj_caves.push(cave);
                self.inj_selected_cave = self.inj_caves.len() - 1;
                self.inj_error = None;
            }
            Err(e) => {
                self.inj_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn free_selected_cave(&mut self) {
        let Some(handle) = self.process.clone() else {
            return;
        };
        let Some(cave) = self.inj_caves.get(self.inj_selected_cave).cloned() else {
            return;
        };
        match inj::destroy_cave(&handle, &cave) {
            Ok(()) => {
                self.inj_caves.remove(self.inj_selected_cave);
                if self.inj_selected_cave >= self.inj_caves.len() && self.inj_selected_cave > 0 {
                    self.inj_selected_cave -= 1;
                }
                self.toast(format!("已释放代码洞 {:X}", cave.address), ToastKind::Info);
            }
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    pub fn apply_patch(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.inj_error = Some("先连接一个进程。".to_string());
            return;
        };
        let address = match parse_address(&self.inj_addr) {
            Some(value) => value,
            None => {
                self.inj_error = Some("目标地址格式不对。".to_string());
                return;
            }
        };
        let bytes = match Self::parse_patch_bytes(&self.inj_bytes) {
            Ok(value) => value,
            Err(e) => {
                self.inj_error = Some(e);
                return;
            }
        };
        let label = format!("{:X}", address);
        match inj::apply(&handle, address, &bytes, &label) {
            Ok(patch) => {
                self.toast(
                    format!("已写入 {} 字节到 {:X}", patch.size(), address),
                    ToastKind::Success,
                );
                self.inj_patches.push(patch);
                self.inj_error = None;
                self.save_workspace();
            }
            Err(e) => {
                self.inj_error = Some(e.clone());
                self.toast(e, ToastKind::Error);
            }
        }
    }

    pub fn revert_patch(&mut self, index: usize) {
        let Some(handle) = self.process.clone() else {
            return;
        };
        let Some(patch) = self.inj_patches.get_mut(index) else {
            return;
        };
        match inj::revert(&handle, patch) {
            Ok(()) => {
                let address = patch.address;
                self.toast(format!("已还原 {:X} 处的原始字节", address), ToastKind::Info);
                self.save_workspace();
            }
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    pub fn revert_all_patches(&mut self) {
        let Some(handle) = self.process.clone() else {
            return;
        };
        let mut done = 0usize;
        let mut failed = 0usize;
        for patch in self.inj_patches.iter_mut() {
            if !patch.active {
                continue;
            }
            match inj::revert(&handle, patch) {
                Ok(()) => done += 1,
                Err(_) => failed += 1,
            }
        }
        if failed == 0 {
            self.toast(format!("已还原 {done} 个补丁"), ToastKind::Success);
        } else {
            self.toast(
                format!("还原 {done} 个，{failed} 个失败"),
                ToastKind::Error,
            );
        }
    }

    /// Read a code window and decode it into a listing.
    pub fn load_disassembly(&mut self) {
        let Some(handle) = self.process.clone() else {
            self.dis_error = Some("先连接一个进程。".to_string());
            return;
        };
        let address = match parse_address(&self.dis_addr) {
            Some(value) => value,
            None => {
                self.dis_error = Some("地址格式不对，试试 0x 开头或纯十六进制。".to_string());
                return;
            }
        };
        let mut buffer = vec![0u8; 512];
        if !handle.read(address, &mut buffer) {
            self.dis_error = Some(format!("无法读取 {address:X} 处的内存。"));
            self.dis_loaded = false;
            return;
        }
        self.dis_insns = disasm::disassemble(&buffer, address, handle.is_64bit, 96);
        self.dis_base = address;
        self.dis_bytes = buffer;
        self.dis_loaded = true;
        self.dis_error = None;
        self.toast(
            format!("已反汇编 {} 条指令", self.dis_insns.len()),
            ToastKind::Success,
        );
    }

    pub fn jump_to_disasm(&mut self, address: u64) {
        self.dis_addr = format!("{address:X}");
        self.tab = Tab::Disasm;
        self.load_disassembly();
    }

    pub fn open_memory_at(&mut self, address: u64) {
        self.mem_addr = format!("{:X}", address);
        self.mem_base = address;
        self.mem_loaded = false;
        self.mem_error = None;
        self.tab = Tab::Memory;
    }

}

/// Where the workspace lives: next to the executable, so a portable copy of
/// Zero carries its own state.
fn workspace_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("zero-workspace.json")))
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "zero-workspace.json".to_string())
}

fn vtype_tag(vtype: VType) -> &'static str {
    match vtype {
        VType::Auto => "auto",
        VType::I8 => "i8",
        VType::I16 => "i16",
        VType::I32 => "i32",
        VType::I64 => "i64",
        VType::F32 => "f32",
        VType::F64 => "f64",
        VType::Str => "str",
    }
}

fn tab_tag(tab: Tab) -> &'static str {
    match tab {
        Tab::Scan => "scan",
        Tab::List => "list",
        Tab::Memory => "memory",
        Tab::Assistant => "assistant",
        Tab::Practice => "practice",
        Tab::PtrScan => "ptr",
        Tab::Aob => "aob",
        Tab::Symbols => "symbols",
        Tab::Disasm => "disasm",
        Tab::Inject => "inject",
        Tab::Asm => "asm",
        Tab::Table => "table",
        Tab::Speed => "speed",
        Tab::Script => "script",
        Tab::Struct => "struct",
        Tab::Debugger => "debugger",
    }
}

fn scan_config(app: &ZeroApp) -> ScanConfig {
    ScanConfig {
        vtype: app.vtype,
        scan_type: app.scan_type,
        value1: app.scan_value.clone(),
        value2: app.scan_value2.clone(),
        fast: app.fast,
        max_results: app.max_results,
    }
}

pub fn read_value(handle: &ProcessHandle, address: u64, vtype: VType) -> Option<Value> {
    if vtype == VType::Str {
        let mut buf = vec![0u8; 512];
        if !handle.read(address, &mut buf) {
            return None;
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        return Some(Value::Text(String::from_utf8_lossy(&buf[..end]).to_string()));
    }
    let mut buf = vec![0u8; vtype.size()];
    if !handle.read(address, &mut buf) {
        return None;
    }
    vtype.decode(&buf)
}

fn start_freeze_thread() -> Arc<Mutex<FreezeState>> {
    let state = Arc::new(Mutex::new(FreezeState {
        handle: None,
        specs: Vec::new(),
    }));
    let thread_state = state.clone();
    std::thread::Builder::new()
        .name("zero-freeze".to_string())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(60));
            let guard = match thread_state.lock() {
                Ok(g) => g,
                Err(_) => continue,
            };
            if let Some(handle) = &guard.handle {
                for spec in &guard.specs {
                    let _ = handle.write(spec.address, &spec.bytes);
                }
            }
        })
        .ok();
    state
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    }
    let [w, h] = image.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&pixel.to_array());
    }
    image::save_buffer(
        path,
        &rgba,
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("无法保存 PNG：{e}"))
}

impl eframe::App for ZeroApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {        self.handle_screenshot(ctx);
        self.poll_scan(ctx);
        self.poll_ptr_scan(ctx);
        self.poll_aob_scan(ctx);
        self.refresh_entries(ctx);
        self.tick_practice(ctx);
        self.tick_speed();
        self.tick_watch(ctx);
        self.poll_update(ctx);

        self.handle_hotkey(ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.picker_open = false;
            self.show_add = false;
            self.show_settings = false;
            self.editing = None;
        }

        self.ui_topbar(ctx);
        self.ui_sidebar(ctx);
        self.ui_statusbar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::bg()))
            .show(ctx, |ui| {
                theme::draw_background(ui);
                let scroll_page = matches!(
                    self.tab,
                    Tab::Memory
                        | Tab::Assistant
                        | Tab::Practice
                        | Tab::PtrScan
                        | Tab::Aob
                        | Tab::Symbols
                        | Tab::Disasm
                        | Tab::Inject
                        | Tab::Asm
                        | Tab::Table
                        | Tab::Speed
                        | Tab::Script
                        | Tab::Struct
                        | Tab::Debugger
                );
                if scroll_page {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            egui::Frame::none()
                                .inner_margin(egui::Margin {
                                    left: 18.0,
                                    right: 20.0,
                                    top: 16.0,
                                    bottom: 22.0,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    self.ui_page(ui);
                                });
                        });
                } else {
                    egui::Frame::none()
                        .inner_margin(egui::Margin {
                            left: 18.0,
                            right: 20.0,
                            top: 12.0,
                            bottom: 8.0,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.ui_page(ui);
                        });
                }
            });

        self.ui_hotkeys(ctx);
        self.e2e_panel(ctx);
        self.ui_toasts(ctx);
        self.ui_modals(ctx);

        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.practice_pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Persist before anything else: the address list and the record of
        // applied patches are the two things a user cannot reconstruct.
        self.save_workspace();
        if let Some(mut child) = self.practice_child.take() {
            let _ = child.kill();
        }
    }
}
