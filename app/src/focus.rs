//! A focused watch list of the most plausible candidates.
//!
//! A first scan usually leaves too many hits to reason about. Triage ranks
//! them, but ranking alone still leaves the user reading a list. This module
//! carries the top candidates forward into a small, continuously sampled watch
//! list: the user changes the value in the game and reads, at a glance, which
//! one moved and by how much.
//!
//! It is deliberately conservative: it only tracks a handful of addresses, and
//! it never writes to the target. Watching is read-only.

use crate::model::{Value, VType};
use crate::winmem::ProcessHandle;

/// One tracked address.
#[derive(Clone, Debug)]
pub struct Watch {
    pub address: u64,
    pub vtype: VType,
    pub description: String,
    /// Score carried over from triage, so the list keeps its reasoning.
    pub score: i32,
    /// The value when the watch started.
    pub initial: Option<Value>,
    /// The most recently sampled value.
    pub current: Option<Value>,
    /// How many samples have differed from the initial value.
    pub changes: u32,
    /// Samples taken so far.
    pub samples: u32,
}

impl Watch {
    pub fn new(address: u64, vtype: VType, description: String, score: i32) -> Self {
        Self {
            address,
            vtype,
            description,
            score,
            initial: None,
            current: None,
            changes: 0,
            samples: 0,
        }
    }

    /// How much the value differs from where it started, as a signed float.
    pub fn delta(&self) -> Option<f64> {
        let before = self.initial.as_ref()?.number()?;
        let now = self.current.as_ref()?.number()?;
        if self.current.as_ref()? == self.initial.as_ref()? {
            return Some(0.0);
        }
        Some(now - before)
    }

    /// A short verdict on whether this address behaves like live state.
    pub fn behaviour(&self) -> &'static str {
        if self.samples == 0 {
            "未采样"
        } else if self.changes == 0 {
            "一直没变"
        } else if self.changes * 2 < self.samples {
            "偶尔变化"
        } else {
            "持续变化"
        }
    }

    /// The colour key the UI uses for this behaviour.
    pub fn behaviour_rank(&self) -> u8 {
        if self.samples == 0 {
            0
        } else if self.changes == 0 {
            1
        } else if self.changes * 2 < self.samples {
            2
        } else {
            3
        }
    }
}

/// Read one value for every watch. Returns how many samples failed.
pub fn sample(handle: &ProcessHandle, watches: &mut [Watch]) -> usize {
    let mut failed = 0;
    for watch in watches.iter_mut() {
        let size = watch.vtype.size().max(1);
        let mut buffer = vec![0u8; size];
        if !handle.read(watch.address, &mut buffer) {
            failed += 1;
            continue;
        }
        let value = watch.vtype.decode(&buffer);
        if watch.initial.is_none() {
            watch.initial = value.clone();
        }
        // Compare against the previous sample, not the initial value: a value
        // that drifts slowly is still changing.
        let changed = match (&watch.current, &value) {
            (Some(previous), Some(now)) => !previous.approx_eq(now),
            (None, Some(_)) => false,
            _ => false,
        };
        if changed {
            watch.changes += 1;
        }
        watch.current = value;
        watch.samples += 1;
    }
    failed
}

/// Build a watch list from the highest-scoring candidates.
pub fn from_candidates(candidates: &[crate::triage::Candidate], limit: usize) -> Vec<Watch> {
    candidates
        .iter()
        .take(limit)
        .map(|candidate| {
            let description = format!("{:X}", candidate.hit.address);
            Watch::new(
                candidate.hit.address,
                candidate.hit.vtype,
                description,
                candidate.score,
            )
        })
        .collect()
}

/// Sort the watch list so the addresses actually behaving like game state come
/// first: a value that keeps moving is what the user is looking for.
pub fn sort_by_behaviour(watches: &mut [Watch]) {
    watches.sort_by(|a, b| {
        b.behaviour_rank()
            .cmp(&a.behaviour_rank())
            .then_with(|| b.changes.cmp(&a.changes))
            .then_with(|| b.score.cmp(&a.score))
    });
}

/// The watch that looks most like the real variable, if any moved at all.
pub fn best(watches: &[Watch]) -> Option<&Watch> {
    watches
        .iter()
        .filter(|w| w.changes > 0)
        .max_by_key(|w| (w.behaviour_rank(), w.changes, w.score))
}