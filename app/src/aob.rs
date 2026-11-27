use crate::model::Region;
use crate::winmem::ProcessHandle;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const CHUNK: u64 = 64 * 1024;

/// A byte signature such as `48 8B ?? 05`. `mask[i] == false` means "any byte".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AobPattern {
    pub bytes: Vec<u8>,
    pub mask: Vec<bool>,
}

impl AobPattern {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut bytes = Vec::new();
        let mut mask = Vec::new();
        for raw in text.split_whitespace() {
            let tok = raw.trim();
            if tok.is_empty() {
                continue;
            }
            if matches!(tok, "?" | "??" | "*" | "**" | "x" | "xx" | "X" | "XX") {
                bytes.push(0);
                mask.push(false);
                continue;
            }
            let t = tok.trim_start_matches("0x").trim_start_matches("0X");
            if t.len() != 2 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!(
                    "「{tok}」不是有效字节，请用两位十六进制（例如 48 8B ?? 05）。"
                ));
            }
            bytes.push(u8::from_str_radix(t, 16).unwrap_or(0));
            mask.push(true);
        }
        if bytes.is_empty() {
            return Err("请先输入要搜索的字节特征。".to_string());
        }
        if !mask.iter().any(|m| *m) {
            return Err("至少要有一个确定的字节，不能全是通配符。".to_string());
        }
        Ok(Self { bytes, mask })
    }

    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            bytes: data.to_vec(),
            mask: vec![true; data.len()],
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// First exact byte: a cheap anchor for the inner search loop.
    pub fn anchor(&self) -> Option<(usize, u8)> {
        self.mask.iter().position(|m| *m).map(|i| (i, self.bytes[i]))
    }

    pub fn display(&self) -> String {
        self.bytes
            .iter()
            .zip(self.mask.iter())
            .map(|(b, m)| {
                if *m {
                    format!("{b:02X}")
                } else {
                    "??".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub struct AobHandle {
    pub rx: mpsc::Receiver<AobDone>,
    pub progress: Arc<AtomicU64>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
}

pub struct AobDone {
    pub hits: Vec<u64>,
    pub elapsed: Duration,
    pub cancelled: bool,
    pub truncated: bool,
}

/// Scan every region for the pattern. Results are sorted and de-duplicated.
pub fn start_scan(
    handle: Arc<ProcessHandle>,
    regions: Vec<Region>,
    pattern: AobPattern,
    max_results: usize,
) -> Result<AobHandle, String> {
    if pattern.is_empty() {
        return Err("字节特征不能为空。".to_string());
    }
    if regions.is_empty() {
        return Err("这个进程没有可扫描的内存区域（试试取消「只扫描可写内存」）。".to_string());
    }
    let max_results = max_results.max(1);
    let total: u64 = regions.iter().map(|r| r.size).sum();
    let progress = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();

    let progress_thread = progress.clone();
    let cancel_thread = cancel.clone();

    std::thread::Builder::new()
        .name("zero-aob".to_string())
        .spawn(move || {
            let mut hits: Vec<u64> = Vec::new();
            let anchor = pattern.anchor();
            let plen = pattern.len();
            'regions: for region in regions {
                if cancel_thread.load(Ordering::Relaxed) {
                    break;
                }
                if region.size < plen as u64 {
                    progress_thread.fetch_add(region.size, Ordering::Relaxed);
                    continue;
                }
                let overlap = (plen as u64).saturating_sub(1);
                let mut pos = 0u64;
                loop {
                    if cancel_thread.load(Ordering::Relaxed) {
                        break 'regions;
                    }
                    let read_len = CHUNK.min(region.size - pos);
                    let mut buf = vec![0u8; read_len as usize];
                    if handle.read(region.base + pos, &mut buf) {
                        let full = scan_chunk(
                            &buf,
                            read_len as usize,
                            region.base + pos,
                            &pattern,
                            anchor,
                            &mut hits,
                            max_results,
                        );
                        if full {
                            break 'regions;
                        }
                    }
                    progress_thread.fetch_add(read_len, Ordering::Relaxed);
                    if pos + read_len >= region.size {
                        break;
                    }
                    pos += read_len.saturating_sub(overlap).max(1);
                }
            }

            hits.sort_unstable();
            hits.dedup();
            let truncated = hits.len() >= max_results;
            hits.truncate(max_results);
            let _ = tx.send(AobDone {
                hits,
                elapsed: started.elapsed(),
                cancelled: cancel_thread.load(Ordering::Relaxed),
                truncated,
            });
        })
        .map_err(|e| format!("无法启动扫描线程：{e}"))?;

    Ok(AobHandle {
        rx,
        progress,
        total: total.max(1),
        cancel,
        started,
    })
}

fn scan_chunk(
    buf: &[u8],
    valid: usize,
    base: u64,
    pattern: &AobPattern,
    anchor: Option<(usize, u8)>,
    out: &mut Vec<u64>,
    max_results: usize,
) -> bool {
    let plen = pattern.len();
    if valid < plen {
        return false;
    }
    let Some((anchor_index, anchor_byte)) = anchor else {
        return false;
    };
    let mut start = 0usize;
    while start + plen <= valid {
        // Jump straight to the next anchor byte instead of testing every offset.
        let mut pos = start + anchor_index;
        let mut found = None;
        while pos < valid {
            if buf[pos] == anchor_byte {
                found = Some(pos);
                break;
            }
            pos += 1;
        }
        let Some(anchor_pos) = found else {
            return false;
        };
        let candidate = anchor_pos - anchor_index;
        let mut matched = true;
        for k in 0..plen {
            if pattern.mask[k] && buf[candidate + k] != pattern.bytes[k] {
                matched = false;
                break;
            }
        }
        if matched {
            out.push(base + candidate as u64);
            if out.len() >= max_results {
                return true;
            }
        }
        start = candidate + 1;
    }
    false
}
