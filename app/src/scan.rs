use crate::model::{Region, ScanHit, ScanType, VType, Value};
use crate::winmem::ProcessHandle;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ScanConfig {
    pub vtype: VType,
    pub scan_type: ScanType,
    pub value1: String,
    pub value2: String,
    pub fast: bool,
    pub max_results: usize,
}

pub struct ScanHandle {
    pub rx: mpsc::Receiver<ScanDone>,
    pub progress: Arc<AtomicU64>,
    pub hit_count: Arc<AtomicUsize>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
}

pub struct ScanDone {
    pub hits: Vec<ScanHit>,
    pub elapsed: Duration,
    pub truncated: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    /// How many worker threads actually ran, and how many bytes they walked.
    /// The pair turns "the scan feels slow" into a number the user can act on.
    pub threads: usize,
    pub scanned_bytes: u64,
}

struct Target {
    ty: VType,
    exact: Option<Vec<u8>>,
    v1: Option<Value>,
    v2: Option<Value>,
}

const CHUNK: u64 = 64 * 1024;

fn align_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        value
    } else {
        value.div_ceil(align) * align
    }
}

fn thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
}

/// One-line "N 线程 · X MB/s" tail for a finished scan. Bytes are 0 for a
/// repeat scan (it only revisits addresses it already found), so that half is
/// left out instead of printed as a fake zero.
pub fn perf_text(threads: usize, bytes: u64, elapsed: Duration) -> String {
    let mut text = format!("{threads} 线程");
    let secs = elapsed.as_secs_f64();
    if bytes > 0 && secs > 0.000_5 {
        let mbps = bytes as f64 / 1_048_576.0 / secs;
        text.push_str(&format!(" · {mbps:.0} MB/s"));
    }
    text
}

/// How many worker threads a scan of this many bytes will actually use. A
/// small region set does not need eight threads: one thread per region is
/// enough, and the caller shows this so a slow scan can be explained.
pub fn planned_threads(regions: &[Region]) -> usize {
    thread_count().min(regions.len()).max(1)
}

pub fn start_first_scan(
    handle: Arc<ProcessHandle>,
    regions: Vec<Region>,
    cfg: ScanConfig,
) -> Result<ScanHandle, String> {
    if !cfg.scan_type.first_scan_ok() {
        return Err("首次扫描请选择精确值、大于/小于、范围或未知初值。".to_string());
    }
    if regions.is_empty() {
        return Err("这个进程没有可扫描的内存区域（试试取消‘只扫描可写内存’）。".to_string());
    }
    let mut targets: Vec<Target> = Vec::new();
    for ty in cfg.vtype.concrete() {
        if cfg.scan_type == ScanType::Unknown {
            targets.push(Target {
                ty: *ty,
                exact: None,
                v1: None,
                v2: None,
            });
            continue;
        }
        if *ty == VType::Str && !matches!(cfg.scan_type, ScanType::Exact) {
            continue;
        }
        let v1 = match ty.parse(&cfg.value1) {
            Some(v) => v,
            None => continue,
        };
        let v2 = if cfg.scan_type.needs_two() {
            match ty.parse(&cfg.value2) {
                Some(v) => Some(v),
                None => {
                    if cfg.vtype == VType::Auto {
                        continue;
                    }
                    return Err("范围扫描的第二个数值无效。".to_string());
                }
            }
        } else {
            None
        };
        let exact = if cfg.scan_type == ScanType::Exact {
            v1.to_bytes(*ty)
        } else {
            None
        };
        targets.push(Target {
            ty: *ty,
            exact,
            v1: Some(v1),
            v2,
        });
    }
    if targets.is_empty() {
        return Err("无法解析这个数值，请检查类型和内容。".to_string());
    }
    let total: u64 = regions
        .iter()
        .map(|r| r.size.saturating_mul(targets.len() as u64))
        .sum();
    let progress = Arc::new(AtomicU64::new(0));
    let hit_count = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();

    let progress_thread = progress.clone();
    let hits_thread = hit_count.clone();
    let cancel_thread = cancel.clone();
    let max_results = cfg.max_results;
    let fast = cfg.fast;
    let scan_type = cfg.scan_type;
    let targets = Arc::new(targets);

    std::thread::Builder::new()
        .name("zero-scan".to_string())
        .spawn(move || {
            let n = thread_count();
            let total_bytes: u64 = regions.iter().map(|r| r.size).sum();
            let mut buckets: Vec<Vec<Region>> = (0..n).map(|_| Vec::new()).collect();
            let mut load = vec![0u64; n];
            for region in regions {
                let idx = (0..n).min_by_key(|i| load[*i]).unwrap_or(0);
                load[idx] = load[idx].saturating_add(region.size);
                buckets[idx].push(region);
            }
            let mut joins = Vec::new();
            let mut threads = 0usize;
            for bucket in buckets {
                if bucket.is_empty() {
                    continue;
                }
                threads += 1;
                let handle = handle.clone();
                let targets = targets.clone();
                let progress = progress_thread.clone();
                let hits_count = hits_thread.clone();
                let cancel = cancel_thread.clone();
                joins.push(std::thread::spawn(move || {
                    let mut local = Vec::new();
                    'regions: for region in bucket {
                        if cancel.load(Ordering::Relaxed)
                            || hits_count.load(Ordering::Relaxed) >= max_results
                        {
                            break;
                        }
                        scan_region(
                            &handle,
                            region,
                            &targets,
                            scan_type,
                            fast,
                            max_results,
                            &progress,
                            &hits_count,
                            &cancel,
                            &mut local,
                        );
                        if local.len() >= max_results {
                            break 'regions;
                        }
                    }
                    local
                }));
            }
            let mut all = Vec::new();
            for join in joins {
                if let Ok(mut part) = join.join() {
                    all.append(&mut part);
                }
            }
            all.sort_by_key(|h| h.address);
            let truncated = all.len() > max_results
                || hits_thread.load(Ordering::Relaxed) > max_results
                || all.len() >= max_results;
            all.truncate(max_results);
            all.dedup_by(|a, b| a.address == b.address && a.vtype == b.vtype);
            let _ = tx.send(ScanDone {
                hits: all,
                elapsed: started.elapsed(),
                truncated,
                cancelled: cancel_thread.load(Ordering::Relaxed),
                error: None,
                threads,
                scanned_bytes: total_bytes,
            });
        })
        .map_err(|e| format!("无法启动扫描线程：{e}"))?;

    Ok(ScanHandle {
        rx,
        progress,
        hit_count,
        total: total.max(1),
        cancel,
        started,
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_region(
    handle: &ProcessHandle,
    region: Region,
    targets: &[Target],
    scan_type: ScanType,
    fast: bool,
    max_results: usize,
    progress: &AtomicU64,
    hits_count: &AtomicUsize,
    cancel: &AtomicBool,
    out: &mut Vec<ScanHit>,
) {
    let mut pos = 0u64;
    while pos < region.size {
        if cancel.load(Ordering::Relaxed) || hits_count.load(Ordering::Relaxed) >= max_results {
            break;
        }
        let read_len = CHUNK.min(region.size - pos);
        let mut buf = vec![0u8; (read_len + 8) as usize];
        let readable = handle.read(region.base + pos, &mut buf);
        if readable {
            for target in targets {
                scan_buffer(
                    region.base + pos,
                    &buf,
                    read_len as usize,
                    target,
                    scan_type,
                    fast,
                    max_results,
                    hits_count,
                    out,
                );
                if hits_count.load(Ordering::Relaxed) >= max_results {
                    break;
                }
            }
        }
        progress.fetch_add(read_len.saturating_mul(targets.len() as u64), Ordering::Relaxed);
        pos += read_len;
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_buffer(
    base: u64,
    buf: &[u8],
    scan_limit: usize,
    target: &Target,
    scan_type: ScanType,
    fast: bool,
    max_results: usize,
    hits_count: &AtomicUsize,
    out: &mut Vec<ScanHit>,
) {
    let ty = target.ty;
    if ty == VType::Str {
        let Some(needle) = target.exact.as_ref() else {
            return;
        };
        if needle.is_empty() || needle.len() > scan_limit {
            return;
        }
        let mut offset = 0usize;
        while offset + needle.len() <= scan_limit {
            if &buf[offset..offset + needle.len()] == needle.as_slice() {
                out.push(ScanHit {
                    address: base + offset as u64,
                    vtype: ty,
                    value: Value::Text(String::from_utf8_lossy(needle).to_string()),
                    previous: None,
                });
                let count = hits_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= max_results {
                    return;
                }
            }
            offset += 1;
        }
        return;
    }

    let size = ty.size();
    let align = if fast { size.min(4).max(1) as u64 } else { 1 };
    let first = align_up(base, align);
    let mut offset = (first - base) as usize;
    let mut checked = 0usize;
    while offset + size <= scan_limit {
        if checked % 8192 == 0 && hits_count.load(Ordering::Relaxed) >= max_results {
            return;
        }
        let slice = &buf[offset..offset + size];
        let keep = match scan_type {
            ScanType::Exact => target
                .exact
                .as_ref()
                .map(|needle| slice == needle.as_slice())
                .unwrap_or(false),
            ScanType::Unknown => true,
            ScanType::Bigger | ScanType::Smaller | ScanType::Between => {
                let Some(value) = ty.decode(slice) else {
                    offset += size;
                    continue;
                };
                compare_candidate(&value, target, scan_type)
            }
            _ => false,
        };
        if keep {
            let value = ty
                .decode(slice)
                .or_else(|| target.v1.clone())
                .unwrap_or(Value::Int(0));
            out.push(ScanHit {
                address: base + offset as u64,
                vtype: ty,
                value: value.clone(),
                previous: if scan_type == ScanType::Unknown {
                    Some(value)
                } else {
                    None
                },
            });
            let count = hits_count.fetch_add(1, Ordering::Relaxed) + 1;
            if count >= max_results {
                return;
            }
        }
        checked += 1;
        offset += size;
    }
}

fn compare_candidate(value: &Value, target: &Target, scan_type: ScanType) -> bool {
    let Some(v1) = target.v1.as_ref() else {
        return false;
    };
    match scan_type {
        ScanType::Bigger => value.number().map(|n| n > v1.number().unwrap_or(0.0)).unwrap_or(false),
        ScanType::Smaller => value.number().map(|n| n < v1.number().unwrap_or(0.0)).unwrap_or(false),
        ScanType::Between => {
            let Some(v2) = target.v2.as_ref() else {
                return false;
            };
            match (
                value.number(),
                v1.number(),
                v2.number(),
            ) {
                (Some(n), Some(a), Some(b)) => {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    n >= lo && n <= hi
                }
                _ => false,
            }
        }
        _ => false,
    }
}

pub fn start_next_scan(
    handle: Arc<ProcessHandle>,
    hits: Vec<ScanHit>,
    cfg: ScanConfig,
) -> Result<ScanHandle, String> {
    if hits.is_empty() {
        return Err("还没有首次扫描结果。".to_string());
    }
    let mut targets: HashMap<VType, (Option<Value>, Option<Value>)> = HashMap::new();
    for hit in &hits {
        if targets.contains_key(&hit.vtype) {
            continue;
        }
        let v1 = hit.vtype.parse(&cfg.value1);
        let v2 = if cfg.scan_type.needs_two() {
            hit.vtype.parse(&cfg.value2)
        } else {
            None
        };
        if cfg.scan_type.needs_value() && v1.is_none() {
            if cfg.vtype != VType::Auto {
                return Err("再次扫描的数值无效。".to_string());
            }
            continue;
        }
        targets.insert(hit.vtype, (v1, v2));
    }
    let total = hits.len() as u64;
    let progress = Arc::new(AtomicU64::new(0));
    let hit_count = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();
    let cancel_thread = cancel.clone();
    let progress_thread = progress.clone();
    let hits_thread = hit_count.clone();
    let scan_type = cfg.scan_type;

    std::thread::Builder::new()
        .name("zero-next-scan".to_string())
        .spawn(move || {
            let n = if hits.len() < 4000 {
                1
            } else {
                thread_count()
            };
            let per = hits.len().div_ceil(n).max(1);
            let hits = Arc::new(hits);
            let targets = Arc::new(targets);
            let mut joins = Vec::new();
            let mut threads = 0usize;
            for i in 0..n {
                let start = i * per;
                if start >= hits.len() {
                    break;
                }
                threads += 1;
                let end = ((i + 1) * per).min(hits.len());
                let handle = handle.clone();
                let hits = hits.clone();
                let targets = targets.clone();
                let progress = progress_thread.clone();
                let cancel = cancel_thread.clone();
                joins.push(std::thread::spawn(move || {
                    let mut kept = Vec::new();
                    for hit in &hits[start..end] {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let current = read_value_at(&handle, hit);
                        progress.fetch_add(1, Ordering::Relaxed);
                        let Some(current) = current else {
                            continue;
                        };
                        let (target1, target2) = targets
                            .get(&hit.vtype)
                            .cloned()
                            .unwrap_or((None, None));
                        if next_matches(
                            scan_type,
                            &current,
                            &hit.value,
                            target1.as_ref(),
                            target2.as_ref(),
                            hit.vtype,
                        ) {
                            kept.push(ScanHit {
                                address: hit.address,
                                vtype: hit.vtype,
                                value: current,
                                previous: Some(hit.value.clone()),
                            });
                        }
                    }
                    kept
                }));
            }
            let mut all = Vec::new();
            for join in joins {
                if let Ok(mut part) = join.join() {
                    all.append(&mut part);
                }
            }
            all.sort_by_key(|h| h.address);
            let _ = tx.send(ScanDone {
                hits: all,
                elapsed: started.elapsed(),
                truncated: false,
                cancelled: cancel_thread.load(Ordering::Relaxed),
                error: None,
                threads,
                scanned_bytes: 0,
            });
        })
        .map_err(|e| format!("无法启动扫描线程：{e}"))?;

    Ok(ScanHandle {
        rx,
        progress,
        hit_count,
        total: total.max(1),
        cancel,
        started,
    })
}

fn read_value_at(handle: &ProcessHandle, hit: &ScanHit) -> Option<Value> {
    if hit.vtype == VType::Str {
        let mut buf = vec![0u8; 512];
        if !handle.read(hit.address, &mut buf) {
            return None;
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        return Some(Value::Text(String::from_utf8_lossy(&buf[..end]).to_string()));
    }
    let mut buf = vec![0u8; hit.vtype.size()];
    if !handle.read(hit.address, &mut buf) {
        return None;
    }
    hit.vtype.decode(&buf)
}

fn next_matches(
    scan_type: ScanType,
    current: &Value,
    previous: &Value,
    target1: Option<&Value>,
    target2: Option<&Value>,
    _ty: VType,
) -> bool {
    match scan_type {
        ScanType::Exact => match target1 {
            Some(target) => values_equal(current, target),
            None => values_equal(current, previous),
        },
        ScanType::Bigger => current
            .number()
            .map(|n| n > target1.and_then(|v| v.number()).unwrap_or(0.0))
            .unwrap_or(false),
        ScanType::Smaller => current
            .number()
            .map(|n| n < target1.and_then(|v| v.number()).unwrap_or(0.0))
            .unwrap_or(false),
        ScanType::Between => match (current.number(), target1.and_then(|v| v.number()), target2.and_then(|v| v.number())) {
            (Some(n), Some(a), Some(b)) => {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                n >= lo && n <= hi
            }
            _ => false,
        },
        ScanType::Unknown => true,
        ScanType::Increased => match (current.number(), previous.number()) {
            (Some(a), Some(b)) => a > b,
            _ => false,
        },
        ScanType::Decreased => match (current.number(), previous.number()) {
            (Some(a), Some(b)) => a < b,
            _ => false,
        },
        ScanType::Changed => !values_equal(current, previous),
        ScanType::Unchanged => values_equal(current, previous),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => x == y,
        _ => a.approx_eq(b),
    }
}
