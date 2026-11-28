use crate::model::Region;
use crate::winmem::{ModuleInfo, ProcessHandle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const CHUNK: u64 = 128 * 1024;
const ALIGNMENT: u64 = 4;

/// Where a pointer chain starts: a module plus a fixed offset (survives ASLR and
/// restarts) or, as a fallback, the module base itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StaticBase {
    pub module: String,
    pub module_base: u64,
    pub offset: u64,
}

impl StaticBase {
    pub fn address(&self) -> u64 {
        self.module_base.saturating_add(self.offset)
    }
    pub fn display(&self) -> String {
        if self.offset == 0 {
            format!("\"{}\"", self.module)
        } else {
            format!("\"{}\"+{:X}", self.module, self.offset)
        }
    }
}

/// One pointer chain: base -> [offset1, offset2, ...] -> final field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PointerPath {
    pub base: StaticBase,
    pub offsets: Vec<u64>,
    pub final_address: u64,
    pub is_32bit: bool,
}

impl PointerPath {
    pub fn display(&self) -> String {
        let mut text = self.base.display();
        for offset in &self.offsets {
            if *offset == 0 {
                text.push_str(" -> +0");
            } else {
                text.push_str(&format!(" -> +{:X}", offset));
            }
        }
        text
    }

    pub fn depth(&self) -> usize {
        self.offsets.len()
    }
}

pub struct PtrHandle {
    pub rx: mpsc::Receiver<PtrDone>,
    pub progress: Arc<AtomicU64>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
}

pub struct PtrDone {
    pub paths: Vec<PointerPath>,
    pub elapsed: Duration,
    pub cancelled: bool,
    pub truncated: bool,
    pub scanned_pointers: u64,
}

/// Result of the reverse walk: for every level, which addresses point into the
/// level below, and what value they store.
struct LevelLinks(HashMap<u64, Vec<(u64, u64)>>);

/// Find pointer chains that reach `target` within `max_depth` levels.
///
/// Pass 1 indexes every aligned pointer-sized value inside readable regions into
/// `value -> [locations]`. Pass 2 walks backwards from the target: for each level,
/// an address becomes a parent when some stored value plus an offset lands on a
/// level-below address. Chains whose head sits inside a loaded module are reported,
/// because `module + offset` is what survives a restart.
pub fn start_scan(
    handle: Arc<ProcessHandle>,
    regions: Vec<Region>,
    modules: Vec<ModuleInfo>,
    target: u64,
    max_depth: usize,
    max_offset: u64,
    max_results: usize,
) -> Result<PtrHandle, String> {
    if max_depth == 0 {
        return Err("层级至少为 1。".to_string());
    }
    if regions.is_empty() {
        return Err("这个进程没有可扫描的内存区域（试试取消「只扫描可写内存」）。".to_string());
    }
    if max_offset == 0 {
        return Err("最大偏移至少要为 1。".to_string());
    }
    let pointer_size: u64 = if handle.is_64bit { 8 } else { 4 };
    let total: u64 = regions.iter().map(|r| r.size).sum();
    let progress = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();

    let progress_thread = progress.clone();
    let cancel_thread = cancel.clone();
    let is_64bit = handle.is_64bit;
    let max_offset = max_offset.min(0x1_0000);

    std::thread::Builder::new()
        .name("zero-pointer".to_string())
        .spawn(move || {
            // Pass 1: pointer value -> locations that store it.
            let mut value_map: HashMap<u64, Vec<u64>> = HashMap::new();
            let mut scanned: u64 = 0;
            let space = AddressSpace::new(&regions);
            'pass1: for region in &regions {
                if cancel_thread.load(Ordering::Relaxed) {
                    break;
                }
                let mut pos = 0u64;
                while pos < region.size {
                    if cancel_thread.load(Ordering::Relaxed) {
                        break 'pass1;
                    }
                    let read_len = CHUNK.min(region.size - pos);
                    let mut buf = vec![0u8; read_len as usize];
                    if handle.read(region.base + pos, &mut buf) {
                        collect_pointers(
                            &buf,
                            region.base + pos,
                            pointer_size,
                            &space,
                            &mut value_map,
                            &mut scanned,
                        );
                    }
                    progress_thread.fetch_add(read_len, Ordering::Relaxed);
                    pos += read_len;
                }
            }

            // Pass 2: reverse BFS from the target.
            let mut rev: Vec<Vec<u64>> = Vec::with_capacity(max_depth + 1);
            rev.push(vec![target]);
            let mut links: Vec<LevelLinks> = Vec::new();

            for level in 1..=max_depth {
                if cancel_thread.load(Ordering::Relaxed) {
                    break;
                }
                let previous = &rev[level - 1];
                let mut parents: Vec<u64> = Vec::new();
                let mut layer: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
                for &pointed in previous {
                    for off in 0..=max_offset {
                        let value = pointed.saturating_sub(off);
                        if let Some(locs) = value_map.get(&value) {
                            for &loc in locs {
                                parents.push(loc);
                                layer
                                    .entry(loc)
                                    .or_default()
                                    .push((pointed, value));
                            }
                        }
                    }
                }
                parents.sort_unstable();
                parents.dedup();
                if parents.is_empty() {
                    break;
                }
                links.push(LevelLinks(layer));
                rev.push(parents);
            }

            // Turn reachable addresses into module-anchored chains.
            let mut paths: Vec<PointerPath> = Vec::new();
            'level: for level in 1..rev.len() {
                let depth = level;
                let layer = &links[level - 1].0;
                for &candidate in &rev[level] {
                    let Some(module) = module_containing(&modules, candidate) else {
                        continue;
                    };
                    if let Some(offsets) = reconstruct(candidate, depth, &links) {
                        paths.push(PointerPath {
                            base: StaticBase {
                                module: module.name.clone(),
                                module_base: module.base,
                                offset: module.offset_of(candidate),
                            },
                            offsets,
                            final_address: target,
                            is_32bit: !is_64bit,
                        });
                    }
                    if paths.len() >= max_results {
                        break 'level;
                    }
                }
            }

            paths.sort_by(|a, b| {
                a.depth()
                    .cmp(&b.depth())
                    .then(a.base.module.cmp(&b.base.module))
                    .then(a.base.offset.cmp(&b.base.offset))
            });
            paths.dedup_by(|a, b| {
                a.base.module == b.base.module
                    && a.base.offset == b.base.offset
                    && a.offsets == b.offsets
            });
            let truncated = paths.len() >= max_results;
            paths.truncate(max_results);

            let _ = tx.send(PtrDone {
                paths,
                elapsed: started.elapsed(),
                cancelled: cancel_thread.load(Ordering::Relaxed),
                truncated,
                scanned_pointers: scanned,
            });
            progress_thread.fetch_add(total, Ordering::Relaxed);
        })
        .map_err(|e| format!("无法启动指针扫描线程：{e}"))?;

    Ok(PtrHandle {
        rx,
        progress,
        total: total.max(1),
        cancel,
        started,
    })
}

fn collect_pointers(
    buf: &[u8],
    base: u64,
    pointer_size: u64,
    space: &AddressSpace,
    map: &mut HashMap<u64, Vec<u64>>,
    scanned: &mut u64,
) {
    let step = pointer_size as usize;
    let mut offset = 0usize;
    while offset + step <= buf.len() {
        let address = base + offset as u64;
        if address % ALIGNMENT != 0 {
            offset += step;
            continue;
        }
        let value = if pointer_size == 8 {
            u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap())
        } else {
            u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as u64
        };
        *scanned += 1;
        if value != 0 && space.is_plausible(value) {
            map.entry(value).or_default().push(address);
        }
        offset += step;
    }
}

/// Minimal address-space bounds used to drop junk pointer values.
struct AddressSpace {
    min: u64,
    max: u64,
}

impl AddressSpace {
    fn new(regions: &[Region]) -> Self {
        let min = regions.iter().map(|r| r.base).min().unwrap_or(0);
        let max = regions.iter().map(|r| r.end()).max().unwrap_or(0);
        Self {
            min: min & !0xFFFF,
            max,
        }
    }
    fn is_plausible(&self, value: u64) -> bool {
        value >= self.min && value <= self.max
    }
}

fn module_containing<'a>(modules: &'a [ModuleInfo], address: u64) -> Option<&'a ModuleInfo> {
    modules.iter().find(|m| m.contains(address))
}

/// Re-point a stored chain at a freshly attached process.
///
/// The module is looked up by name (bases move with ASLR), then every hop is
/// re-read from the live process. Returns None when the module is gone or any
/// hop no longer dereferences: a stale map must fail loudly, never silently
/// produce a plausible-but-wrong address.
pub fn rebase_path(path: &PointerPath, modules: &[ModuleInfo], handle: &ProcessHandle) -> Option<PointerPath> {
    let wanted = path.base.module.to_ascii_lowercase();
    let module = modules
        .iter()
        .find(|m| m.name.to_ascii_lowercase() == wanted)?;
    let mut address = module.base.saturating_add(path.base.offset);
    for offset in &path.offsets {
        address = handle.read_pointer(address)?.saturating_add(*offset);
    }
    Some(PointerPath {
        base: StaticBase {
            module: module.name.clone(),
            module_base: module.base,
            offset: path.base.offset,
        },
        offsets: path.offsets.clone(),
        final_address: address,
        is_32bit: !handle.is_64bit,
    })
}
/// Walk the stored links from a base address down to the target, collecting the
/// per-hop offsets (innermost first, then reversed so the list reads base->target).
fn reconstruct(base: u64, depth: usize, links: &[LevelLinks]) -> Option<Vec<u64>> {
    let mut offsets = Vec::with_capacity(depth);
    let mut current = base;
    for level in (1..=depth).rev() {
        let entry = links[level - 1].0.get(&current)?;
        let (child, value) = *entry.first()?;
        let off = child.saturating_sub(value);
        offsets.push(off);
        current = child;
    }
    offsets.reverse();
    Some(offsets)
}
