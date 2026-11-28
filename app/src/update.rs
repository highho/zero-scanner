//! 自动更新。
//!
//! 版本信息取自 GitHub release（highho/zero-scanner），网络请求统一走
//! https://gh-proxy.com/ 代理——本机对 github.com / api.github.com 的直连
//! 不可靠，代理只做转发，不改变内容。
//!
//! 全部用 WinHTTP（windows-sys 已有依赖），不引入新的第三方 crate。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// 本程序版本，来自 Cargo.toml。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "highho/zero-scanner";
pub const RELEASES_PAGE: &str = "https://github.com/highho/zero-scanner/releases";
/// 下载 / 接口统一走这个镜像。
pub const PROXY: &str = "https://gh-proxy.com/";

const AGENT: &str = "Zero-Updater";
/// 自动检查的间隔：6 小时。手动检查不受限制。
const AUTO_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub notes: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub page: String,
}

pub enum Msg {
    Checked(Result<Option<Release>, String>),
    Progress(f32),
    /// 下载完成：文件路径 + 与发布说明里哈希的核对结果。
    Downloaded(Result<(PathBuf, HashCheck), String>),
}

pub enum State {
    Idle,
    Checking,
    Current,
    Available(Release),
    Downloading(f32),
    /// 已下载并核对过。哈希不一致时不会走到这里（下载阶段就判失败）。
    Ready { path: PathBuf, hash: HashCheck },
    Failed(String),
}

pub struct Updater {
    pub state: State,
    /// 启动后自动检查（可在设置里关掉）。
    pub auto: bool,
    pub last_check: Option<Instant>,
    /// 用户选择跳过的版本。
    pub skipped: Option<String>,
    rx: Option<Receiver<Msg>>,
    /// 手动检查时失败才提示；自动检查失败保持安静。
    pub manual_check: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Self::from_prefs()
    }
}

impl Updater {
    /// 从磁盘偏好恢复「自动检查」开关和「已跳过版本」。
    ///
    /// 这两项以前只活在内存里：跳过某个版本后一重启就忘，于是每次启动都再
    /// 提示一遍；「自动检查已关闭」的分支也永远走不到，因为没有任何入口能
    /// 把它关掉。
    pub fn from_prefs() -> Self {
        let prefs = crate::prefs::load();
        Self {
            state: State::Idle,
            auto: prefs.auto_update,
            last_check: None,
            skipped: if prefs.skipped_version.is_empty() {
                None
            } else {
                Some(prefs.skipped_version)
            },
            rx: None,
            manual_check: false,
        }
    }

    /// 记住跳过的版本并落盘，重启后不再提示同一个版本。
    pub fn skip_version(&mut self, version: &str) {
        self.skipped = Some(version.to_string());
        crate::prefs::set_skipped_version(version);
    }

    /// 切换自动检查并落盘。
    pub fn set_auto(&mut self, enabled: bool) {
        self.auto = enabled;
        crate::prefs::set_auto_update(enabled);
        if enabled {
            // 刚打开就该尽快生效，不必等到下一个 6 小时窗口。
            self.last_check = None;
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Downloading(_))
    }

    /// 到期就自动检查一次。
    pub fn tick(&mut self) {
        if !self.auto || self.busy() {
            return;
        }
        let due = match self.last_check {
            None => true,
            Some(at) => at.elapsed() >= AUTO_INTERVAL,
        };
        if due {
            self.check(false);
        }
    }

    pub fn check(&mut self, manual: bool) {
        if self.busy() {
            return;
        }
        self.manual_check = manual;
        self.last_check = Some(Instant::now());
        self.state = State::Checking;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let result = fetch_release();
            let _ = tx.send(Msg::Checked(result));
        });
    }

    pub fn download(&mut self, release: &Release) {
        if self.busy() {
            return;
        }
        self.state = State::Downloading(0.0);
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let release = release.clone();
        std::thread::spawn(move || {
            // 下载完立刻核对哈希：等用户点「重启并安装」时再查，文件可能已经
            // 被别的东西换掉，那时拒绝安装已经太晚（界面会以为是文件坏了）。
            let got = download_release(&release, tx.clone()).and_then(|path| {
                let check = check_hash(&path, &release.notes)?;
                if let HashCheck::Mismatch { expected, actual } = &check {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!(
                        "下载的文件哈希不对，已删除：官方 {expected}，实际 {actual}"
                    ));
                }
                Ok((path, check))
            });
            let _ = tx.send(Msg::Downloaded(got));
        });
    }

    /// 收集后台线程的消息；返回需要提示给用户的事件。
    pub fn poll(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        loop {
            let msg = match self.rx.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(msg)) => msg,
                Some(Err(std::sync::mpsc::TryRecvError::Empty)) => break,
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    if matches!(self.state, State::Checking) {
                        self.state = State::Failed("检查更新中断".to_string());
                    }
                    self.rx = None;
                    break;
                }
                None => break,
            };
            match msg {
                Msg::Checked(Ok(Some(release))) => {
                    let skipped = self.skipped.as_deref() == Some(release.version.as_str());
                    let version = release.version.clone();
                    self.state = State::Available(release);
                    if !skipped {
                        notes.push(format!("发现新版本 v{version}"));
                    }
                }
                Msg::Checked(Ok(None)) => {
                    self.state = State::Current;
                    if self.manual_check {
                        notes.push("已是最新版本".to_string());
                    }
                    self.manual_check = false;
                    self.rx = None;
                }
                Msg::Checked(Err(e)) => {
                    let manual = self.manual_check;
                    self.state = State::Failed(e.clone());
                    if manual {
                        notes.push(format!("检查更新失败：{e}"));
                    }
                    self.manual_check = false;
                    self.rx = None;
                }
                Msg::Progress(p) => {
                    self.state = State::Downloading(p);
                }
                Msg::Downloaded(Ok((path, hash))) => {
                    let verified = matches!(hash, HashCheck::Match(_));
                    self.state = State::Ready { path, hash };
                    notes.push(if verified {
                        "更新已下载，SHA256 与官方公布的一致，重启后生效".to_string()
                    } else {
                        "更新已下载，但发布说明没公布 SHA256，无法核对".to_string()
                    });
                    self.rx = None;
                }
                Msg::Downloaded(Err(e)) => {
                    self.state = State::Failed(e.clone());
                    notes.push(format!("下载失败：{e}"));
                    self.rx = None;
                }
            }
        }
        notes
    }
}

// ---------------------------------------------------------------- 版本与发布信息

/// 把 tag（如 `v1.2.0`）规整成纯版本号 `1.2.0`。
pub fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    // 自己剥掉 v 前缀：调用方直接传 tag（"v1.2.0"）时也要能解析。
    // 不剥的话 "v1".parse() 失败 -> 返回 None -> is_newer 恒 false，
    // 表现是「永远显示已是最新」，属于最不该有的静默失败。
    let text = text.trim().trim_start_matches(['v', 'V']);
    let mut parts = text.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts
        .next()
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .and_then(|p| if p.is_empty() { Some(0) } else { p.parse::<u64>().ok() })
        .unwrap_or(0);
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

pub fn fetch_release() -> Result<Option<Release>, String> {
    let Some(release) = fetch_release_any()? else {
        return Ok(None);
    };
    if !is_newer(&release.version, VERSION) {
        return Ok(None);
    }
    Ok(Some(release))
}

/// 取仓库最新的发布，不判断它是否比当前版本新。
///
/// 正常更新流程用 fetch_release（会过滤掉不比当前新的发布）。这个函数是给
/// 验证用的：远端版本可能落后于本地开发版，此时下载链路否则永远跑不到，
/// 而「下载 → 校验字节数 → 改名落地」正是最容易出错、最需要真跑一遍的一段。
pub fn fetch_release_any() -> Result<Option<Release>, String> {
    let url = format!("{PROXY}https://api.github.com/repos/{REPO}/releases/latest");
    let body = http_get_vec(&url)?;
    let text = String::from_utf8_lossy(&body);
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("发布信息不是有效 JSON：{e}"))?;

    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "发布信息里没有 tag_name".to_string())?
        .to_string();
    let version = normalize_tag(&tag);

    let notes = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let page = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or(RELEASES_PAGE)
        .to_string();

    let assets = json
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "发布信息里没有可下载文件".to_string())?;

    let mut best: Option<(u8, Release)> = None;
    for asset in assets {
        let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }
        // 优先裸 exe（就地替换，不跑安装器）；没有就退回安装包。
        let rank = if name.eq_ignore_ascii_case("zero-scanner.exe") {
            2
        } else if name.to_ascii_lowercase().ends_with("-setup-x64.exe") {
            1
        } else {
            0
        };
        if rank == 0 {
            continue;
        }
        if best.as_ref().map(|(r, _)| *r >= rank).unwrap_or(false) {
            continue;
        }
        let size = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        best = Some((
            rank,
            Release {
                version: version.clone(),
                tag: tag.clone(),
                notes: notes.clone(),
                asset_name: name.to_string(),
                asset_url: format!("{PROXY}{url}"),
                asset_size: size,
                page: page.clone(),
            },
        ));
    }

    match best {
        Some((_, release)) => Ok(Some(release)),
        None => Err("这个版本没有可用的下载文件".to_string()),
    }
}

fn download_release(release: &Release, tx: Sender<Msg>) -> Result<PathBuf, String> {
    let name = if release.asset_name.eq_ignore_ascii_case("zero-scanner.exe") {
        "zero-scanner.exe".to_string()
    } else {
        format!("zero-scanner-{}-setup.exe", release.version)
    };
    let dir = std::env::temp_dir().join("zero-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建下载目录：{e}"))?;
    let dest = dir.join(name);
    let tmp = dest.with_extension("part");

    let total = release.asset_size;
    let mut written = 0u64;
    let mut last_push = Instant::now();

    let mut file = std::fs::File::create(&tmp).map_err(|e| format!("无法写入下载文件：{e}"))?;
    let result = http_get_stream(&release.asset_url, &mut |chunk| {
        use std::io::Write;
        file.write_all(chunk)
            .map_err(|e| format!("写入失败：{e}"))?;
        written += chunk.len() as u64;
        if last_push.elapsed() >= Duration::from_millis(120) {
            last_push = Instant::now();
            let pct = if total > 0 {
                (written.min(total) as f32 / total as f32) * 100.0
            } else {
                0.0
            };
            let _ = tx.send(Msg::Progress(pct));
        }
        Ok(())
    });
    drop(file);

    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if total > 0 && written != total {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("下载不完整：{written} / {total} 字节"));
    }
    std::fs::rename(&tmp, &dest).map_err(|e| format!("无法保存下载文件：{e}"))?;
    let _ = tx.send(Msg::Progress(100.0));
    Ok(dest)
}

// ---------------------------------------------------------------- 安装并重启

/// 确认待安装的文件还在、且看起来是个完整的 exe。
///
/// 下载完成后到用户点「重启并安装」之间可能隔了很久，期间文件可能被清理工具
/// 删掉、被下载器改成半截。这一步必须在拉起批处理之前做：批处理一旦跑起来，
/// 失败只会写进 apply.log，用户界面里看不到任何提示。
pub fn validate_exe(path: &Path) -> Result<(), String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("下载的更新文件不见了：{e}（请重新下载）"))?;
    if !meta.is_file() {
        return Err("下载的更新文件不是一个普通文件".to_string());
    }
    if meta.len() < 1024 {
        return Err(format!("下载的更新文件只有 {} 字节，明显不完整", meta.len()));
    }
    let mut head = [0u8; 2];
    {
        use std::io::Read;
        let mut f =
            std::fs::File::open(path).map_err(|e| format!("无法打开下载的更新文件：{e}"))?;
        f.read_exact(&mut head)
            .map_err(|e| format!("无法读取下载的更新文件：{e}"))?;
    }
    if &head != b"MZ" {
        return Err("下载的更新文件不是有效的 exe（缺少 MZ 头）".to_string());
    }
    Ok(())
}

/// 用批处理等本进程退出后再替换 exe，然后重新启动。
/// 调用方在成功后应立即退出，把文件句柄交出去。
pub fn install_and_restart(new_exe: &Path) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| format!("取自身路径失败：{e}"))?;
    if current == new_exe {
        return Err("下载文件与当前程序是同一个路径".to_string());
    }
    // 安装前再确认一次文件还在、而且还是个完整的 exe。State::Ready 里的路径
    // 可能已经在内存里放了很久，期间文件被清理工具删掉或下了一半，只有到这
    // 一步才会发现——那时批处理已经跑起来了，报错只会留在日志里，用户看不到。
    validate_exe(new_exe)?;
    let pid = std::process::id();
    // 批处理是被 cmd /C 后台拉起的，它的工作目录不是调用者的目录。
    // 只要有一个相对路径，move 就会找不到文件，而 >nul 会把错误吞掉，
    // 表现成「点了更新，什么都没发生」。这里一律转成绝对路径。
    let new_abs = std::path::absolute(new_exe).map_err(|e| format!("无法解析新文件路径：{e}"))?;
    let dir = std::env::temp_dir().join("zero-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建临时目录：{e}"))?;
    let script = dir.join("apply.cmd");

    let lines = [
        "@echo off".to_string(),
        format!("set LOG={}", dir.join("apply.log").display()),
        "ping -n 2 127.0.0.1 >nul".to_string(),
        ":wait".to_string(),
        format!("tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul"),
        "if not errorlevel 1 (".to_string(),
        "  ping -n 2 127.0.0.1 >nul".to_string(),
        "  goto wait".to_string(),
        ")".to_string(),
        format!("move /Y \"{}\" \"{}\" >> \"%LOG%\" 2>&1", new_abs.display(), current.display()),
        "if errorlevel 1 (".to_string(),
        "  echo FAILED >> \"%LOG%\"".to_string(),
        "  exit /b 1".to_string(),
        ")".to_string(),
        "echo OK >> \"%LOG%\"".to_string(),
        format!("start \"\" \"{}\"", current.display()),
        "echo STARTED >> \"%LOG%\"".to_string(),
        // 先 goto 让 cmd 停止读取脚本，再删自己。直接 del 的话，cmd 删完还要
        // 继续读文件，会报 "The batch file cannot be found."，用户看到会以为更新失败。
        "(goto) 2>nul & del \"%~f0\"".to_string(),
    ];
    std::fs::write(&script, lines.join("\r\n") + "\r\n")
        .map_err(|e| format!("无法写入更新脚本：{e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        std::process::Command::new("cmd.exe")
            .arg("/C")
            .arg(&script)
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()
            .map_err(|e| format!("无法启动更新脚本：{e}"))?;
    }
    Ok(())
}

/// 用系统默认浏览器打开一个网址。
pub fn open_page(url: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let url_w: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let op: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            ShellExecuteW(
                0,
                op.as_ptr(),
                url_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = url;
}

/// 命令行自检：查一次最新发布，把结论打成一行文字。
pub fn cli_check() -> Result<String, String> {
    match fetch_release() {
        Ok(Some(release)) => Ok(format!(
            "current={} latest={} asset={} size={} url={}",
            VERSION, release.version, release.asset_name, release.asset_size, release.asset_url
        )),
        Ok(None) => Ok(format!("current={} latest=已是最新", VERSION)),
        Err(error) => Err(error),
    }
}

/// 命令行自检：下载当前发布里最合适的资产，返回临时文件路径。
pub fn download_for_cli(
    release: &Release,
    tx: Sender<Msg>,
) -> Result<PathBuf, String> {
    download_release(release, tx)
}

/// 命令行自检：真的走一遍下载，把文件放到指定目录，返回落地路径。
///
/// any=true 时忽略版本比较，直接下载远端最新发布——用于验证下载链路本身。
pub fn cli_download(dir: &Path, any: bool) -> Result<String, String> {
    let found = if any { fetch_release_any()? } else { fetch_release()? };
    let release = found.ok_or_else(|| "已是最新，没有可下载的版本".to_string())?;
    let (tx, _rx) = std::sync::mpsc::channel();
    let downloaded = download_release(&release, tx)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建目录：{e}"))?;
    let target = dir.join(&release.asset_name);
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }
    std::fs::copy(&downloaded, &target).map_err(|e| format!("无法复制下载结果：{e}"))?;
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    Ok(format!(
        "downloaded={} size={} expected={}",
        target.display(),
        size,
        release.asset_size
    ))
}

// ---------------------------------------------------------------- WinHTTP

fn http_get_vec(url: &str) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    http_get_stream(url, &mut |chunk| {
        out.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(out)
}

fn http_get_stream<F>(url: &str, sink: &mut F) -> Result<(), String>
where
    F: FnMut(&[u8]) -> Result<(), String>,
{
    match raw_get(url, sink, false) {
        Ok(()) => Ok(()),
        Err(first) => {
            // 本机吊销列表离线时 schannel 会直接失败，放宽校验再试一次。
            raw_get(url, sink, true).map_err(|second| {
                if first == second {
                    first
                } else {
                    format!("{first}；放宽证书校验后仍失败：{second}")
                }
            })
        }
    }
}

#[cfg(windows)]
fn raw_get<F>(url: &str, sink: &mut F, relaxed: bool) -> Result<(), String>
where
    F: FnMut(&[u8]) -> Result<(), String>,
{
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
        WinHttpSetTimeouts, SECURITY_FLAG_IGNORE_CERT_CN_INVALID,
        SECURITY_FLAG_IGNORE_CERT_DATE_INVALID,
        SECURITY_FLAG_IGNORE_UNKNOWN_CA, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        WINHTTP_FLAG_SECURE, WINHTTP_OPTION_SECURITY_FLAGS, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE,
    };

    /// windows-sys 0.52 没有导出这个常量（WinHTTP 的 0x80）。
    const SECURITY_FLAG_IGNORE_REVOCATION: u32 = 0x0000_0080;

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn split_https(url: &str) -> Result<(String, String), String> {
        let rest = url
            .strip_prefix("https://")
            .ok_or_else(|| format!("只支持 https 地址：{url}"))?;
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        if host.is_empty() {
            return Err(format!("地址不合法：{url}"));
        }
        Ok((host.to_string(), path.to_string()))
    }

    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    let (host, path) = split_https(url)?;
    let agent = wide(AGENT);
    let host_w = wide(&host);
    let path_w = wide(&path);
    let verb = wide("GET");

    unsafe {
        let session = Handle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(format!("无法初始化网络会话（{}）", GetLastError()));
        }
        WinHttpSetTimeouts(session.0, 8000, 8000, 15000, 30000);

        let connect = Handle(WinHttpConnect(session.0, host_w.as_ptr(), 443, 0));
        if connect.0.is_null() {
            return Err(format!("无法连接 {host}（{}）", GetLastError()));
        }

        let request = Handle(WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(format!("无法创建请求（{}）", GetLastError()));
        }

        if relaxed {
            let flags = SECURITY_FLAG_IGNORE_REVOCATION
                | SECURITY_FLAG_IGNORE_UNKNOWN_CA
                | SECURITY_FLAG_IGNORE_CERT_CN_INVALID
                | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID;
            let flag_buf = flags.to_le_bytes();
            WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_SECURITY_FLAGS,
                flag_buf.as_ptr() as *const core::ffi::c_void,
                flag_buf.len() as u32,
            );
        }

        if WinHttpSendRequest(
            request.0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            0,
            0,
        ) == 0
        {
            return Err(format!("请求发送失败（{}）", GetLastError()));
        }
        if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err(format!("没有收到响应（{}）", GetLastError()));
        }

        let mut status: u32 = 0;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status as *mut u32 as *mut core::ffi::c_void,
            &mut status_len,
            std::ptr::null_mut(),
        );
        if !(200..300).contains(&status) {
            return Err(format!("服务器返回 HTTP {status}"));
        }

        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read: u32 = 0;
            if WinHttpReadData(
                request.0,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
                &mut read,
            ) == 0
            {
                return Err(format!("读取响应失败（{}）", GetLastError()));
            }
            if read == 0 {
                break;
            }
            sink(&buf[..read as usize])?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn raw_get<F>(_url: &str, _sink: &mut F, _relaxed: bool) -> Result<(), String>
where
    F: FnMut(&[u8]) -> Result<(), String>,
{
    Err("自动更新仅支持 Windows".to_string())
}

// ---------------------------------------------------------------- SHA256 校验

/// 从发布说明里找出官方公布的安装包哈希。
///
/// 约定写在 release notes 里的一行：`sha256: <64 位十六进制>`（大小写不敏感，
/// 冒号可以换成空格）。找不到就返回 None——此时不做校验，只在界面上说明
/// 「这次没有哈希可核对」，而不是假装校验过了。
pub fn published_sha256(notes: &str) -> Option<String> {
    for line in notes.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(at) = lower.find("sha256") else { continue };
        let rest = &line[at + "sha256".len()..];
        let rest = rest.trim_start_matches(|c: char| c == ':' || c == '=' || c.is_whitespace());
        let hex: String = rest
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if hex.len() == 64 {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

/// 算一个文件的 SHA256，返回小写十六进制。
///
/// 走系统 BCrypt，不引第三方哈希库：更新校验这种安全相关的事，用操作系统
/// 自带的实现比多一个依赖更可靠。
#[cfg(windows)]
pub fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    use windows_sys::Win32::Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
        BCryptGetProperty, BCryptHashData, BCryptOpenAlgorithmProvider, BCRYPT_ALG_HANDLE,
        BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALGORITHM,
    };

    // 这两个属性名 windows-sys 0.52 没有导出成常量，按 SDK 的字面量写。
    const OBJECT_LENGTH: &[u16] = &[79, 98, 106, 101, 99, 116, 76, 101, 110, 103, 116, 104, 0]; // "ObjectLength"
    const HASH_LENGTH: &[u16] = &[72, 97, 115, 104, 68, 105, 103, 101, 115, 116, 76, 101, 110, 103, 116, 104, 0]; // "HashDigestLength"

    struct Alg(BCRYPT_ALG_HANDLE);
    impl Drop for Alg {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
            }
        }
    }
    struct Hash(BCRYPT_HASH_HANDLE);
    impl Drop for Hash {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { BCryptDestroyHash(self.0) };
            }
        }
    }

    unsafe {
        let mut alg: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_SHA256_ALGORITHM, std::ptr::null(), 0) < 0
        {
            return Err("系统没有可用的 SHA256 实现".to_string());
        }
        let alg = Alg(alg);

        let mut object_len: u32 = 0;
        let mut got: u32 = 0;
        if BCryptGetProperty(
            alg.0,
            OBJECT_LENGTH.as_ptr(),
            &mut object_len as *mut u32 as *mut u8,
            std::mem::size_of::<u32>() as u32,
            &mut got,
            0,
        ) < 0
        {
            return Err("无法取得 SHA256 工作区大小".to_string());
        }
        let mut hash_len: u32 = 0;
        if BCryptGetProperty(
            alg.0,
            HASH_LENGTH.as_ptr(),
            &mut hash_len as *mut u32 as *mut u8,
            std::mem::size_of::<u32>() as u32,
            &mut got,
            0,
        ) < 0
        {
            return Err("无法取得 SHA256 摘要长度".to_string());
        }
        if hash_len != 32 {
            return Err(format!("SHA256 摘要长度异常：{hash_len}"));
        }

        let mut object = vec![0u8; object_len as usize];
        let mut handle: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        if BCryptCreateHash(
            alg.0,
            &mut handle,
            object.as_mut_ptr(),
            object_len,
            std::ptr::null(),
            0,
            0,
        ) < 0
        {
            return Err("无法创建 SHA256 摘要".to_string());
        }
        let hash = Hash(handle);

        let mut file = std::fs::File::open(path)
            .map_err(|e| format!("无法读取待校验文件：{e}"))?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("读取待校验文件失败：{e}"))?;
            if n == 0 {
                break;
            }
            if BCryptHashData(hash.0, buf.as_ptr(), n as u32, 0) < 0 {
                return Err("计算摘要时出错".to_string());
            }
        }
        let mut digest = [0u8; 32];
        if BCryptFinishHash(hash.0, digest.as_mut_ptr(), 32, 0) < 0 {
            return Err("收尾摘要时出错".to_string());
        }
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }
}

#[cfg(not(windows))]
pub fn sha256_file(_path: &Path) -> Result<String, String> {
    Err("SHA256 校验仅支持 Windows".to_string())
}

/// 校验一个已下载文件是否与官方公布的一致。
///
/// 三种结果都要能区分开：一致、不一致（危险，必须拒绝安装）、没有公布哈希
/// （只能给出「无法核对」，不能当成通过）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HashCheck {
    Match(String),
    Mismatch { expected: String, actual: String },
    NotPublished,
}

pub fn check_hash(path: &Path, notes: &str) -> Result<HashCheck, String> {
    let Some(expected) = published_sha256(notes) else {
        return Ok(HashCheck::NotPublished);
    };
    let actual = sha256_file(path)?;
    if actual == expected {
        Ok(HashCheck::Match(actual))
    } else {
        Ok(HashCheck::Mismatch { expected, actual })
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    #[test]
    fn finds_published_hash_in_notes() {
        let notes = "## 1.5.0\n- 修复若干问题\n\nsha256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n";
        assert_eq!(
            published_sha256(notes).as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn accepts_upper_case_and_equals() {
        let notes = "SHA256 = 0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        assert_eq!(
            published_sha256(notes).as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn ignores_short_hex() {
        assert_eq!(published_sha256("sha256: abcdef"), None);
    }

    #[test]
    fn notes_without_hash_report_none() {
        assert_eq!(published_sha256("- 只有文字说明"), None);
    }
}
