//! Hit triage: decide which of hundreds of scan hits is worth trusting.
//!
//! A first scan on a real game returns hundreds or thousands of addresses.
//! Picking the right one by hand is the slowest part of using a memory scanner.
//! This module ranks them from evidence the scanner already has, so the user
//! starts at the most plausible candidate instead of the whole list.
//!
//! Every signal here is objective and explainable: nothing is guessed, and each
//! hit carries the reasons that produced its score.

use crate::model::{ScanHit, VType};
use serde::{Deserialize, Serialize};

/// A region of the target's memory, as far as triage cares.
#[derive(Clone, Copy, Debug)]
pub struct RegionHint {
    pub base: u64,
    pub size: u64,
    pub writable: bool,
    pub executable: bool,
}

/// One reason a hit was scored the way it was.
#[derive(Clone, Debug)]
pub struct Signal {
    pub label: String,
    /// Positive favours the hit, negative argues against it.
    pub weight: i32,
}

/// A ranked candidate.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub hit: ScanHit,
    pub score: i32,
    pub signals: Vec<Signal>,
}

impl Candidate {
    /// A one-word verdict, so the UI can colour the row.
    pub fn verdict(&self) -> &'static str {
        if self.score >= 60 {
            "很可能"
        } else if self.score >= 35 {
            "可能"
        } else if self.score >= 15 {
            "存疑"
        } else {
            "不太可能"
        }
    }

    /// The single strongest reason, for a compact row.
    pub fn headline(&self) -> String {
        self.signals
            .iter()
            .filter(|s| s.weight > 0)
            .max_by_key(|s| s.weight)
            .map(|s| s.label.clone())
            .unwrap_or_else(|| "没有明显特征".to_string())
    }
}

fn find_region(regions: &[RegionHint], address: u64) -> Option<&RegionHint> {
    regions
        .iter()
        .find(|r| address >= r.base && address < r.base.saturating_add(r.size))
}

/// Score one hit from the evidence available.
fn score_hit(hit: &ScanHit, regions: &[RegionHint], module_ranges: &[(String, u64, u64)]) -> Candidate {
    let mut signals = Vec::new();
    let mut score = 0i32;

    // Where the value lives matters a lot: game data is in writable heap or
    // data pages, while code pages hold constants that never change.
    if let Some(region) = find_region(regions, hit.address) {
        if region.executable {
            signals.push(Signal {
                label: "位于可执行代码段，通常是常量而不是状态".to_string(),
                weight: -40,
            });
            score -= 40;
        } else if region.writable {
            signals.push(Signal {
                label: "位于可写数据区，符合游戏状态的特征".to_string(),
                weight: 25,
            });
            score += 25;
        } else {
            signals.push(Signal {
                label: "位于只读数据区，可能是常量表".to_string(),
                weight: -15,
            });
            score -= 15;
        }
    }

    // A hit inside a loaded module is more likely a static table or a default
    // value than per-instance state; a hit in private memory is more likely the
    // real variable. Neither is decisive, so the weights stay modest.
    let in_module = module_ranges
        .iter()
        .any(|(_, base, size)| hit.address >= *base && hit.address < base.saturating_add(*size));
    if in_module {
        signals.push(Signal {
            label: "落在模块映像内，可能是静态数据".to_string(),
            weight: -8,
        });
        score -= 8;
    } else {
        signals.push(Signal {
            label: "落在堆或私有内存，更像运行时变量".to_string(),
            weight: 10,
        });
        score += 10;
    }

    // Alignment: most integers are naturally aligned; a value straddling an
    // alignment boundary is usually a coincidence in some other data.
    let size = hit.vtype.size().max(1) as u64;
    if size > 1 && hit.address % size == 0 {
        signals.push(Signal {
            label: format!("地址按 {size} 字节对齐"),
            weight: 12,
        });
        score += 12;
    } else if size > 1 {
        signals.push(Signal {
            label: "地址未对齐，可能是巧合".to_string(),
            weight: -12,
        });
        score -= 12;
    }

    // A value that has not changed across scans is suspicious: real game state
    // usually moves. Only trust this when a previous value actually exists.
    if let Some(previous) = &hit.previous {
        if previous.approx_eq(&hit.value) {
            signals.push(Signal {
                label: "多次扫描值没有变化，可能是无关的常量".to_string(),
                weight: -18,
            });
            score -= 18;
        } else {
            signals.push(Signal {
                label: "两次扫描之间值发生了变化，符合活数据".to_string(),
                weight: 22,
            });
            score += 22;
        }
    }

    // Extreme or suspicious values are usually counters or flags, not the
    // player's health bar.
    if let Some(number) = hit.value.number() {
        if number == 0.0 {
            signals.push(Signal {
                label: "值为 0，可能是未初始化数据".to_string(),
                weight: -10,
            });
            score -= 10;
        } else if number.abs() >= 1.0e9 {
            signals.push(Signal {
                label: "数值极大，可能是地址或时间戳".to_string(),
                weight: -6,
            });
            score -= 6;
        } else if (number.fract()).abs() < f64::EPSILON && number.abs() < 100_000.0 {
            signals.push(Signal {
                label: "数值规模像正常游戏数值".to_string(),
                weight: 8,
            });
            score += 8;
        }
    }

    // Floating point hits at a 4-byte boundary are a weak positive: coordinates
    // and timers live there, but so does noise.
    if hit.vtype == VType::F32 && hit.address % 4 == 0 {
        signals.push(Signal {
            label: "浮点值按 4 字节对齐，符合坐标或计时器".to_string(),
            weight: 6,
        });
        score += 6;
    }

    signals.sort_by(|a, b| b.weight.cmp(&a.weight));
    Candidate {
        hit: hit.clone(),
        score,
        signals,
    }
}

/// Rank every hit, best first. The order is stable for equal scores so the list
/// does not reshuffle between renders.
pub fn rank(
    hits: &[ScanHit],
    regions: &[RegionHint],
    module_ranges: &[(String, u64, u64)],
) -> Vec<Candidate> {
    let mut ranked: Vec<Candidate> = hits
        .iter()
        .map(|hit| score_hit(hit, regions, module_ranges))
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.hit.address.cmp(&b.hit.address))
    });
    ranked
}


// ---------------------------------------------------------------------------
// User feedback.
//
// Triage can be wrong, and when a user says "that is not it", the answer should
// make the next ranking better instead of being forgotten. Feedback is stored
// as a small set of remembered addresses plus the trait that made them wrong,
// so the lesson generalises to new scans rather than only hiding one row.
// ---------------------------------------------------------------------------

/// A trait a rejected address exhibited, remembered so similar hits rank lower.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectedTrait {
    /// The address never changed across scans.
    NeverChanged,
    /// The address sat in an executable page.
    InCode,
    /// The address sat inside a module image.
    InModule,
    /// The address was not naturally aligned.
    Unaligned,
}

impl RejectedTrait {
    pub fn label(&self) -> &'static str {
        match self {
            RejectedTrait::NeverChanged => "你否定过一个值从不变化的地址",
            RejectedTrait::InCode => "你否定过一个代码段里的地址",
            RejectedTrait::InModule => "你否定过一个模块内的静态地址",
            RejectedTrait::Unaligned => "你否定过一个未对齐的地址",
        }
    }

    /// How much to subtract from a hit that shows the same trait.
    pub fn penalty(&self) -> i32 {
        match self {
            RejectedTrait::NeverChanged => 20,
            RejectedTrait::InCode => 35,
            RejectedTrait::InModule => 12,
            RejectedTrait::Unaligned => 15,
        }
    }
}

/// What a hit would have to show for the user to reject it.
///
/// This is an independent check from the scoring signals: it re-derives the
/// traits that a ranker used, so rejecting a hit can be turned into a rule.
pub fn traits_of(
    hit: &ScanHit,
    regions: &[RegionHint],
    module_ranges: &[(String, u64, u64)],
) -> Vec<RejectedTrait> {
    let mut traits = Vec::new();
    if let Some(previous) = &hit.previous {
        if previous.approx_eq(&hit.value) {
            traits.push(RejectedTrait::NeverChanged);
        }
    }
    if let Some(region) = find_region(regions, hit.address) {
        if region.executable {
            traits.push(RejectedTrait::InCode);
        }
    }
    if module_ranges
        .iter()
        .any(|(_, base, size)| hit.address >= *base && hit.address < base.saturating_add(*size))
    {
        traits.push(RejectedTrait::InModule);
    }
    let size = hit.vtype.size().max(1) as u64;
    if size > 1 && hit.address % size != 0 {
        traits.push(RejectedTrait::Unaligned);
    }
    traits
}

/// Learn from a rejected hit and return the traits learned.
pub fn learn_rejection(
    feedback: &mut Vec<RejectedTrait>,
    hit: &ScanHit,
    regions: &[RegionHint],
    module_ranges: &[(String, u64, u64)],
) -> Vec<RejectedTrait> {
    let traits = traits_of(hit, regions, module_ranges);
    let mut learned = Vec::new();
    for trait_kind in traits {
        // One entry per trait: repeated rejections of the same kind must not
        // stack into an unbounded penalty.
        if !feedback.contains(&trait_kind) {
            feedback.push(trait_kind.clone());
            learned.push(trait_kind);
        }
    }
    learned
}

/// Rank with what the user has taught this session.
pub fn rank_with_feedback(
    hits: &[ScanHit],
    regions: &[RegionHint],
    module_ranges: &[(String, u64, u64)],
    feedback: &[RejectedTrait],
) -> Vec<Candidate> {
    let mut ranked = rank(hits, regions, module_ranges);
    if feedback.is_empty() {
        return ranked;
    }
    for candidate in ranked.iter_mut() {
        let traits = traits_of(&candidate.hit, regions, module_ranges);
        for learned in feedback {
            if traits.contains(learned) {
                candidate.score -= learned.penalty();
                candidate.signals.push(Signal {
                    label: format!("已按你的反馈降低（{}）", learned.label()),
                    weight: -learned.penalty(),
                });
            }
        }
        candidate.signals.sort_by(|a, b| b.weight.cmp(&a.weight));
    }
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.hit.address.cmp(&b.hit.address))
    });
    ranked
}
/// How many candidates look worth a person's attention.
pub fn strong_count(candidates: &[Candidate]) -> usize {
    candidates.iter().filter(|c| c.score >= 35).count()
}