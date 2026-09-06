//! The property diff (`docs/specs/property_diff.md`): a pure function over
//! two [`PropertyProfile`] maps and a working-tree dependency graph.
//!
//! `diff_profiles` performs **no I/O** and reads no ledger, snapshot, or
//! backend (`docs/specs/property_diff.md` §Constraints item 2, "Diff
//! purity"). The edited set and `project_config_changed` arrive on
//! [`DiffGraph`] as inputs the caller computes — a later phase (git
//! materialisation) never enters this module
//! (`docs/outcomes/20260905-property-diff/phases/03-plan.md`, ruling R4).
//! Enforced structurally by `crates/smelt-logical/tests/diff_purity.rs`.
//!
//! **Direction totality** (§Constraints item 3): [`ChangeKind::direction`]
//! and [`ChangeKind::dimension`] are each one `match` over [`ChangeKind`]
//! with no wildcard arm — a new variant is a compile error until both are
//! given an arm. The table is keyed on `ChangeKind`, not `Dimension`,
//! because several direction rows are computed from the variant's own typed
//! `old`/`new` values (the technique ladder, bound widening, grain), not a
//! per-dimension constant.
//!
//! **Field coverage** (closing the hole `docs/specs/property_diff.md` §"The
//! diff" describes): [`diff_property_set`] and [`diff_cell_verdicts`]
//! destructure [`PropertySet`] and [`CellVerdict`] field-by-field with no
//! `..` rest pattern, so a field added to either later is a compile error
//! until it is given a dimension and a direction rule, rather than silently
//! going undiffed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;
use serde_json::Value;
use smelt_core::graph::DependencyGraph;
use smelt_core::refs::SmeltRef;

use crate::analysis::profile::PropertySet;
use crate::analysis::profile::{CellVerdict, ProfileProbe, ProfileRefusal, PropertyProfile};
use crate::analysis::source_bounds::BoundResult;
use crate::analysis::walk::{Comparability as Comp, DerivedFd, Determinism as Det, Grain};
use crate::contract::ContractPointView;
use crate::maintenance::{RowIdentity, RowIdentityVerdict, Technique};

/// The dimension a change is reported under — the JSON `dimension` string
/// (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    Grain,
    RowIdentity,
    SourceBound,
    CellTechnique,
    CellCorner,
    CellRowIdentity,
    CellAdded,
    CellRemoved,
    RefusalAdded,
    RefusalRemoved,
    ContractPoint,
    ProbeAdded,
    ProbeRemoved,
    ColumnAdded,
    ColumnRemoved,
    Determinism,
    Discriminant,
    Comparability,
    FdAdded,
    FdRemoved,
    LiteralColumn,
    SetOpBarrier,
    FanOutJoin,
    MaintenanceLost,
    MaintenanceGained,
    StateDowngrade,
}

/// Whether a change makes the model's maintenance proofs weaker, stronger,
/// or neither (`docs/specs/property_diff.md` §"Direction").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Downgrade,
    Upgrade,
    Neutral,
}

/// The technique cost ladder (`docs/specs/property_diff.md` §"Direction"):
/// `KeyedFold` ≻ `ColumnScopedMerge` ≻ `InPlaceUpdate` ≻ `PerGroupRecompute`
/// ≻ `DeleteInsert`. Higher rank costs less per run.
fn technique_rank(t: Technique) -> u8 {
    match t {
        // Shares `KeyedFold`'s tier: both are ledger-backed, targeted-write
        // techniques with no full-table read. A succession cell never
        // transitions to/from another technique in practice (the grain is
        // derived, not declared, so there is no `contract:`/`grain:` edit
        // that would trigger a property-diff comparison across techniques
        // here) — the exact rank is a placeholder until a real transition
        // exists to pin it against.
        Technique::KeyedFold | Technique::SuccessionPatch => 5,
        Technique::ColumnScopedMerge => 4,
        Technique::InPlaceUpdate => 3,
        Technique::PerGroupRecompute => 2,
        Technique::DeleteInsert => 1,
    }
}

/// `Bounded` ≻ `{Unbounded, NotDerivable}` (`docs/specs/property_diff.md`
/// §"The property profile" item 2 / §"Direction" "source_bound" row):
/// `Unbounded` and `NotDerivable` share a rank because both force a full
/// read.
fn bound_rank(b: &BoundResult) -> u8 {
    match b {
        BoundResult::Bounded { .. } => 1,
        BoundResult::Unbounded | BoundResult::NotDerivable => 0,
    }
}

/// `before + after`, in seconds, for a `Bounded` verdict — the width
/// `source_bound` widening/narrowing compares.
fn bound_width_seconds(b: &BoundResult) -> Option<u64> {
    match b {
        BoundResult::Bounded { before, after, .. } => Some(before.0 + after.0),
        _ => None,
    }
}

mod change_kind;
mod change_types;
mod diff_cell_verdicts;
mod diff_graph;
mod diff_profile;
mod diff_property_set;
#[cfg(test)]
mod tests;

pub use change_kind::ChangeKind;
pub use change_types::{
    BaselineInfo, Cause, CauseKind, Change, DiffReport, DiffSummary, ModelDiff, PropertyDiff,
};
pub use diff_graph::{apply_failure_reasons, DiffGraph};
pub use diff_profile::diff_profiles;

use change_types::{multiset_excess, to_json};
use diff_cell_verdicts::{diff_cell_verdicts, diff_probes, diff_refusals};
#[cfg(test)]
use diff_profile::diff_profile;
use diff_property_set::diff_property_set;
