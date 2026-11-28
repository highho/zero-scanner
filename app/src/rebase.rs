//! Re-pointing saved addresses at a freshly started process.
//!
//! A saved address is only meaningful in the process that produced it: ASLR
//! moves the module bases on every launch, so the same "gold" lives at a
//! different absolute address next time. What survives is the *relationship*:
//! which module the address belongs to and how far into it the address sits.
//!
//! This module does the arithmetic and nothing else, so the rule can be tested
//! without a live game. The caller supplies the old and the new module table and
//! gets back either a fresh address or a reason the address cannot be trusted.

use crate::winmem::ModuleInfo;

/// What happened to one saved address when the process was re-attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// The module is back and the address maps into it at the same offset.
    Ok,
    /// The module is back, but the address lands outside the module this time.
    /// A module that shrank means the build changed; the old offset is a guess.
    OutOfModule,
    /// The module is not loaded at all any more.
    ModuleGone,
}

impl RebaseOutcome {
    pub fn ok(self) -> bool {
        matches!(self, RebaseOutcome::Ok)
    }

    /// A short label for the address list, empty when nothing is wrong.
    pub fn short(self) -> &'static str {
        match self {
            RebaseOutcome::Ok => "",
            RebaseOutcome::OutOfModule => "越界",
            RebaseOutcome::ModuleGone => "模块没了",
        }
    }

    /// The full sentence shown when hovering the row.
    pub fn detail(self, module: &str) -> String {
        match self {
            RebaseOutcome::Ok => String::new(),
            RebaseOutcome::OutOfModule => format!(
                "这个地址落在「{module}」外面了：模块这次比上次小，说明程序换过版本，旧偏移不可信，重新扫描一次。"
            ),
            RebaseOutcome::ModuleGone => format!(
                "「{module}」这次没有加载：可能是另一个程序，或者模块被换了名字。"
            ),
        }
    }
}

/// The result of re-pointing one address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rebasing {
    pub address: u64,
    pub outcome: RebaseOutcome,
}

/// Where a saved address sits relative to a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchored {
    pub module: String,
    pub offset: u64,
}

/// Find the module a saved address belonged to.
///
/// Modules are matched by name, case-insensitively, because the saved name came
/// from a previous process listing and Windows does not promise a stable case.
pub fn anchor(address: u64, modules: &[ModuleInfo]) -> Option<Anchored> {
    modules
        .iter()
        .find(|module| module.contains(address))
        .map(|module| Anchored {
            module: module.name.clone(),
            offset: module.offset_of(address),
        })
}

/// Move an already-anchored address onto the module table of the new process.
///
/// This is the form used after a restart: the anchor was recorded when the
/// workspace was saved, so there is no old module table to consult.
pub fn rebase_anchored(anchor: &Anchored, new_modules: &[ModuleInfo]) -> Rebasing {
    let Some(module) = new_modules
        .iter()
        .find(|module| module.name.eq_ignore_ascii_case(&anchor.module))
    else {
        return Rebasing {
            address: anchor.offset,
            outcome: RebaseOutcome::ModuleGone,
        };
    };
    let moved = module.base.saturating_add(anchor.offset);
    let outcome = if module.contains(moved) {
        RebaseOutcome::Ok
    } else {
        RebaseOutcome::OutOfModule
    };
    Rebasing {
        address: moved,
        outcome,
    }
}

/// Move a saved address onto the module table of the new process.
///
/// An address that never belonged to a module (a heap or stack address) is
/// returned unchanged: there is no base to move it against, and pretending
/// otherwise would silently point it at the wrong place. Callers treat that as
/// "still shown, but not verified" rather than as a failure.
pub fn rebase(
    address: u64,
    old_modules: &[ModuleInfo],
    new_modules: &[ModuleInfo],
) -> Option<Rebasing> {
    let anchored = anchor(address, old_modules)?;
    let fresh = new_modules
        .iter()
        .find(|module| module.name.eq_ignore_ascii_case(&anchored.module));
    let Some(module) = fresh else {
        return Some(Rebasing {
            address,
            outcome: RebaseOutcome::ModuleGone,
        });
    };
    let moved = module.base.saturating_add(anchored.offset);
    let outcome = if module.contains(moved) {
        RebaseOutcome::Ok
    } else {
        RebaseOutcome::OutOfModule
    };
    Some(Rebasing {
        address: moved,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(name: &str, base: u64, size: u64) -> ModuleInfo {
        ModuleInfo {
            name: name.to_string(),
            base,
            size,
        }
    }

    #[test]
    fn aslr_moves_the_base_but_keeps_the_offset() {
        let old = vec![module("game.exe", 0x1_4000_0000, 0x1000_0000)];
        let new = vec![module("game.exe", 0x7_FF00_0000, 0x1000_0000)];
        let moved = rebase(0x1_4001_2345, &old, &new).expect("module found");
        assert_eq!(moved.address, 0x7_FF01_2345);
        assert_eq!(moved.outcome, RebaseOutcome::Ok);
    }

    #[test]
    fn a_missing_module_is_reported_not_guessed() {
        let old = vec![module("game.exe", 0x1_4000_0000, 0x1000_0000)];
        let new = vec![module("other.exe", 0x1_4000_0000, 0x1000_0000)];
        let moved = rebase(0x1_4001_2345, &old, &new).expect("module found");
        assert_eq!(moved.outcome, RebaseOutcome::ModuleGone);
        // The old address is kept so the row can still be shown and edited.
        assert_eq!(moved.address, 0x1_4001_2345);
    }

    #[test]
    fn a_shrunken_module_flags_the_offset_as_out_of_range() {
        let old = vec![module("game.exe", 0x1_4000_0000, 0x1000_0000)];
        let new = vec![module("game.exe", 0x7_FF00_0000, 0x1000)];
        let moved = rebase(0x1_4001_2345, &old, &new).expect("module found");
        assert_eq!(moved.outcome, RebaseOutcome::OutOfModule);
    }

    #[test]
    fn a_heap_address_is_left_alone() {
        let old = vec![module("game.exe", 0x1_4000_0000, 0x1000)];
        let new = vec![module("game.exe", 0x7_FF00_0000, 0x1000)];
        assert!(rebase(0x2_0000_0000, &old, &new).is_none());
    }

    #[test]
    fn a_stored_anchor_re_points_without_the_old_process() {
        let anchor = Anchored {
            module: "game.exe".to_string(),
            offset: 0x2345,
        };
        let new = vec![module("game.exe", 0x7_FF00_0000, 0x1000_0000)];
        let moved = rebase_anchored(&anchor, &new);
        assert_eq!(moved.address, 0x7_FF00_2345);
        assert_eq!(moved.outcome, RebaseOutcome::Ok);
    }

    #[test]
    fn a_stored_anchor_for_a_gone_module_reports_it() {
        let anchor = Anchored {
            module: "game.exe".to_string(),
            offset: 0x2345,
        };
        let new = vec![module("other.exe", 0x7_FF00_0000, 0x1000_0000)];
        assert_eq!(
            rebase_anchored(&anchor, &new).outcome,
            RebaseOutcome::ModuleGone
        );
    }

    #[test]
    fn module_names_match_without_caring_about_case() {
        let old = vec![module("Game.EXE", 0x1_4000_0000, 0x1000)];
        let new = vec![module("game.exe", 0x7_FF00_0000, 0x1000)];
        let moved = rebase(0x1_4000_0010, &old, &new).expect("module found");
        assert_eq!(moved.address, 0x7_FF00_0010);
    }

    #[test]
    fn each_outcome_says_what_to_do_next() {
        assert!(RebaseOutcome::Ok.short().is_empty());
        assert!(!RebaseOutcome::OutOfModule.detail("game.exe").is_empty());
        assert!(!RebaseOutcome::ModuleGone.detail("game.exe").is_empty());
    }
}
