//! Memory heat map: which pages of the target are actually changing?
//!
//! A cheat session usually starts with "scan for a value", and the scan is slow
//! because most of the process is dead weight: code, resources, caches that
//! never change while the game runs. Showing where writes are landing turns
//! that blind search into a much shorter list of regions worth scanning.
//!
//! Sampling is deliberately cheap and read-only: read a window of each region,
//! remember it, read it again later, and count the bytes that moved. Nothing is
//! written to the target, so the map is safe to leave running.

use std::collections::HashMap;

use crate::model::Region;
use crate::winmem::ProcessHandle;

/// One sample of a region: what it looked like and when.
pub struct Sample {
    bytes: Vec<u8>,
}

/// How one region changed between two samples.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegionHeat {
    pub base: u64,
    pub size: u64,
    /// Bytes that differed between the last two samples.
    pub changed: usize,
    /// Bytes actually compared (a region may be larger than the window).
    pub sampled: usize,
    /// 0.0 = nothing moved, 1.0 = the whole window changed.
    pub heat: f32,
}

impl RegionHeat {
    pub fn mb(&self) -> f64 {
        self.size as f64 / (1024.0 * 1024.0)
    }
}

/// Result of one sampling pass.
#[derive(Clone, Debug, Default)]
pub struct Pass {
    pub regions: Vec<RegionHeat>,
    pub sampled_bytes: u64,
    pub elapsed_ms: u64,
}

impl Pass {
    /// Regions sorted hottest first — that is the order the page shows.
    pub fn hottest(&self) -> Vec<&RegionHeat> {
        let mut list: Vec<&RegionHeat> = self.regions.iter().collect();
        list.sort_by(|a, b| {
            b.heat
                .partial_cmp(&a.heat)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.changed.cmp(&a.changed))
        });
        list
    }

    pub fn hot_regions(&self, threshold: f32) -> usize {
        self.regions.iter().filter(|r| r.heat >= threshold).count()
    }
}

/// How many bytes of each region to watch. Large enough to catch a value that
/// lives anywhere in the page, small enough that a pass over hundreds of
/// regions still finishes in a frame.
pub const WINDOW: usize = 4096;

/// Keep the map bounded: a 64-bit game can expose thousands of regions, and
/// holding a copy of each would dwarf the memory we are inspecting.
pub const MAX_REGIONS: usize = 512;

/// The sampler itself. Owns the previous bytes for every region it watches.
pub struct HeatMap {
    /// Region base -> the bytes seen last time.
    previous: HashMap<u64, Vec<u8>>,
    /// The regions being watched, in the order they were adopted.
    regions: Vec<Region>,
    /// Bytes changed per region, kept between passes so a slow tick still has
    /// something to show.
    last: Pass,
    pub passes: u64,
}

impl HeatMap {
    pub fn new() -> Self {
        Self { previous: HashMap::new(), regions: Vec::new(), last: Pass::default(), passes: 0 }
    }

    pub fn is_watching(&self) -> bool {
        !self.regions.is_empty()
    }

    pub fn watched(&self) -> usize {
        self.regions.len()
    }

    pub fn last(&self) -> &Pass {
        &self.last
    }

    /// Forget everything. Called on detach: the next process has different
    /// pages, and keeping the old bytes would report every region as changed.
    pub fn clear(&mut self) {
        self.previous.clear();
        self.regions.clear();
        self.last = Pass::default();
        self.passes = 0;
    }

    /// Adopt the regions of a process. Only writable ones are interesting:
    /// read-only code cannot change, and watching it just wastes a window.
    pub fn adopt(&mut self, regions: &[Region]) -> usize {
        self.previous.clear();
        self.regions.clear();
        self.last = Pass::default();
        self.passes = 0;
        for region in regions {
            if !region.writable {
                continue;
            }
            self.regions.push(*region);
            if self.regions.len() >= MAX_REGIONS {
                break;
            }
        }
        self.regions.len()
    }

    /// One sampling pass. The first pass has nothing to compare against, so it
    /// only fills the baseline and reports zero heat.
    pub fn sample(&mut self, handle: &ProcessHandle) -> &Pass {
        let started = std::time::Instant::now();
        let mut regions = Vec::with_capacity(self.regions.len());
        let mut sampled_bytes = 0u64;
        for region in &self.regions {
            let want = region.size.min(WINDOW as u64) as usize;
            if want == 0 {
                continue;
            }
            let mut buf = vec![0u8; want];
            if !handle.read(region.base, &mut buf) {
                // The region vanished (freed, or the game closed): drop it so
                // the next pass does not keep paying for a dead page.
                self.previous.remove(&region.base);
                continue;
            }
            sampled_bytes += want as u64;
            let changed = match self.previous.get(&region.base) {
                Some(old) if old.len() == buf.len() => {
                    old.iter().zip(buf.iter()).filter(|(a, b)| a != b).count()
                }
                _ => 0,
            };
            self.previous.insert(region.base, buf);
            let heat = changed as f32 / want as f32;
            regions.push(RegionHeat {
                base: region.base,
                size: region.size,
                changed,
                sampled: want,
                heat,
            });
        }
        self.passes += 1;
        self.last = Pass {
            regions,
            sampled_bytes,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
        &self.last
    }
}
