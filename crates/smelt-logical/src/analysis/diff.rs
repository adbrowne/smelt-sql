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
        Technique::KeyedFold => 5,
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

/// The typed payload of one difference. **This is the one direction
/// table**: [`ChangeKind::direction`] and [`ChangeKind::dimension`] are
/// exhaustive matches over it, with no wildcard arm.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeKind {
    Grain {
        subject: String,
        old: Grain,
        new: Grain,
    },
    RowIdentity {
        old: RowIdentityVerdict,
        new: RowIdentityVerdict,
    },
    SourceBound {
        source: String,
        old: BoundResult,
        new: BoundResult,
    },
    CellTechnique {
        cell: String,
        old: Technique,
        new: Technique,
    },
    CellCorner {
        cell: String,
        old: String,
        new: String,
    },
    CellRowIdentity {
        cell: String,
        old: RowIdentityVerdict,
        new: RowIdentityVerdict,
    },
    CellAdded {
        cell: String,
        new: Box<CellVerdict>,
        still_maintained: bool,
    },
    CellRemoved {
        cell: String,
        old: Box<CellVerdict>,
        still_maintained: bool,
    },
    RefusalAdded(ProfileRefusal),
    RefusalRemoved(ProfileRefusal),
    ContractPoint {
        cell: String,
        old: ContractPointView,
        new: ContractPointView,
    },
    ProbeAdded(ProfileProbe),
    ProbeRemoved(ProfileProbe),
    ColumnAdded(String),
    ColumnRemoved(String),
    Determinism {
        column: String,
        // `Option`: G3 (`docs/outcomes/20260905-property-diff` fix round 1)
        // — the fact can be present on only one side of a matched column
        // (the per-column map desynced from `columns`, an anomaly a
        // consistent derivation should never produce but the field-coverage
        // rule must still surface rather than silently drop). `None` on
        // either side always grades `Neutral` (see `direction`): there is
        // no lattice position to compare a missing fact against.
        old: Option<Det>,
        new: Option<Det>,
    },
    Comparability {
        column: String,
        old: Option<Comp>,
        new: Option<Comp>,
    },
    Discriminant {
        column: String,
        old: Option<crate::analysis::discriminants::Discriminants>,
        new: Option<crate::analysis::discriminants::Discriminants>,
    },
    FdAdded(DerivedFd),
    FdRemoved(DerivedFd),
    LiteralColumn {
        column: String,
        old: Option<String>,
        new: Option<String>,
    },
    SetOpBarrier {
        old: bool,
        new: bool,
    },
    FanOutJoin {
        old: bool,
        new: bool,
    },
    /// A model went from having at least one [`CellVerdict`] to having none
    /// at all — it is no longer incrementally maintained. Emitted ONCE per
    /// model in `diff_profile`, never derived from individual
    /// `cell_removed` changes (`docs/specs/property_diff.md` §"Direction"
    /// "maintenance_lost"/"maintenance_gained"; G1,
    /// `docs/outcomes/20260905-property-diff` fix round 1 — the case a
    /// `refresh: incremental` → `refresh: full` edit hit, which produced
    /// neither a refusal nor a downgrade before this dimension existed).
    MaintenanceLost,
    /// The symmetric partner: a model went from no cells to at least one.
    MaintenanceGained,
    /// A matched cell's [`crate::maintenance::availability::StateDowngrade`]
    /// appeared, disappeared, or changed shape (`docs/specs/property_diff.md`
    /// §"Direction" "state_downgrade" row).
    StateDowngrade {
        cell: String,
        old: Option<crate::maintenance::availability::StateDowngrade>,
        new: Option<crate::maintenance::availability::StateDowngrade>,
    },
}

/// Whether `new`'s `retain_departed` relaxed relative to `old`'s: absent →
/// present is a downgrade, present → absent an upgrade, a shape change with
/// presence unchanged is `None` (neutral) — `docs/specs/property_diff.md`
/// §"Direction" "contract_point" row.
fn retain_departed_direction(old: &Option<String>, new: &Option<String>) -> Option<Direction> {
    match (old, new) {
        (None, Some(_)) => Some(Direction::Downgrade),
        (Some(_), None) => Some(Direction::Upgrade),
        _ => None,
    }
}

/// Whether an `Option<u64>` interval widened (`None` → `Some`, or grew) or
/// narrowed (`Some` → `None`, or shrank) — shared by `frozen_horizon` and
/// `deferral`'s seconds fields.
fn interval_direction(old: Option<u64>, new: Option<u64>) -> Option<Direction> {
    match (old, new) {
        (None, Some(_)) => Some(Direction::Downgrade),
        (Some(_), None) => Some(Direction::Upgrade),
        (Some(o), Some(n)) if n > o => Some(Direction::Downgrade),
        (Some(o), Some(n)) if n < o => Some(Direction::Upgrade),
        _ => None,
    }
}

impl ChangeKind {
    /// The JSON `dimension` string (`docs/specs/property_diff.md` §Surface).
    /// Exhaustive — no wildcard arm.
    pub fn dimension(&self) -> Dimension {
        match self {
            ChangeKind::Grain { .. } => Dimension::Grain,
            ChangeKind::RowIdentity { .. } => Dimension::RowIdentity,
            ChangeKind::SourceBound { .. } => Dimension::SourceBound,
            ChangeKind::CellTechnique { .. } => Dimension::CellTechnique,
            ChangeKind::CellCorner { .. } => Dimension::CellCorner,
            ChangeKind::CellRowIdentity { .. } => Dimension::CellRowIdentity,
            ChangeKind::CellAdded { .. } => Dimension::CellAdded,
            ChangeKind::CellRemoved { .. } => Dimension::CellRemoved,
            ChangeKind::RefusalAdded(_) => Dimension::RefusalAdded,
            ChangeKind::RefusalRemoved(_) => Dimension::RefusalRemoved,
            ChangeKind::ContractPoint { .. } => Dimension::ContractPoint,
            ChangeKind::ProbeAdded(_) => Dimension::ProbeAdded,
            ChangeKind::ProbeRemoved(_) => Dimension::ProbeRemoved,
            ChangeKind::ColumnAdded(_) => Dimension::ColumnAdded,
            ChangeKind::ColumnRemoved(_) => Dimension::ColumnRemoved,
            ChangeKind::Determinism { .. } => Dimension::Determinism,
            ChangeKind::Comparability { .. } => Dimension::Comparability,
            ChangeKind::Discriminant { .. } => Dimension::Discriminant,
            ChangeKind::FdAdded(_) => Dimension::FdAdded,
            ChangeKind::FdRemoved(_) => Dimension::FdRemoved,
            ChangeKind::LiteralColumn { .. } => Dimension::LiteralColumn,
            ChangeKind::SetOpBarrier { .. } => Dimension::SetOpBarrier,
            ChangeKind::FanOutJoin { .. } => Dimension::FanOutJoin,
            ChangeKind::MaintenanceLost => Dimension::MaintenanceLost,
            ChangeKind::MaintenanceGained => Dimension::MaintenanceGained,
            ChangeKind::StateDowngrade { .. } => Dimension::StateDowngrade,
        }
    }

    /// The single direction table (`docs/specs/property_diff.md`
    /// §"Direction"). Exhaustive — no wildcard arm; a value-dependent row
    /// computes its verdict from the variant's own typed `old`/`new`.
    pub fn direction(&self) -> Direction {
        match self {
            ChangeKind::Grain { old, new, .. } => {
                let old_has = !old.keys.is_empty();
                let new_has = !new.keys.is_empty();
                if old_has && !new_has {
                    Direction::Downgrade
                } else if !old_has && new_has {
                    Direction::Upgrade
                } else {
                    // G5 (`docs/outcomes/20260905-property-diff` fix round
                    // 1): compare the UNION of columns each side's keys
                    // cover, not key-set membership — `Key(["id","region"])
                    // -> Key(["id"])` is a composite key column dropped
                    // ("lost a key column", spec §"Direction"), but the two
                    // `KeySet`s are unequal as *values* so the prior
                    // membership check saw both "lost" (old's composite
                    // isn't in new) and "gained" (new's shorter key isn't in
                    // old) and graded it `Neutral`. A dropped column from
                    // the grain's footprint is always worse regardless of
                    // what else changed, so `lost` wins ties.
                    let old_cols: BTreeSet<&String> = old.keys.iter().flatten().collect();
                    let new_cols: BTreeSet<&String> = new.keys.iter().flatten().collect();
                    let lost = !old_cols.is_subset(&new_cols);
                    let gained = !new_cols.is_subset(&old_cols);
                    match (lost, gained) {
                        (false, true) => Direction::Upgrade,
                        (false, false) => Direction::Neutral,
                        _ => Direction::Downgrade,
                    }
                }
            }
            ChangeKind::RowIdentity { old, new } | ChangeKind::CellRowIdentity { old, new, .. } => {
                match (&old.identity, &new.identity) {
                    (RowIdentity::Key(_), RowIdentity::WholeRow) => Direction::Downgrade,
                    (RowIdentity::WholeRow, RowIdentity::Key(_)) => Direction::Upgrade,
                    _ => Direction::Neutral,
                }
            }
            ChangeKind::SourceBound { old, new, .. } => {
                let (old_rank, new_rank) = (bound_rank(old), bound_rank(new));
                if new_rank < old_rank {
                    Direction::Downgrade
                } else if new_rank > old_rank {
                    Direction::Upgrade
                } else {
                    match (bound_width_seconds(old), bound_width_seconds(new)) {
                        (Some(o), Some(n)) if n > o => Direction::Downgrade,
                        (Some(o), Some(n)) if n < o => Direction::Upgrade,
                        _ => Direction::Neutral,
                    }
                }
            }
            ChangeKind::CellTechnique { old, new, .. } => {
                if technique_rank(*new) < technique_rank(*old) {
                    Direction::Downgrade
                } else {
                    Direction::Upgrade
                }
            }
            ChangeKind::CellCorner { .. } => Direction::Neutral,
            ChangeKind::CellAdded { .. } => Direction::Upgrade,
            ChangeKind::CellRemoved {
                still_maintained, ..
            } => {
                // A cell removed while the model is still maintained lost a
                // maintenance route (`docs/specs/property_diff.md`
                // §"Direction"): a downgrade. The spec's row does not name
                // the case where the *whole model* stopped being
                // maintained (`still_maintained == false`) — deviation
                // (Phase 3): treated as `neutral` here rather than
                // downgrade, since the model-wide loss of maintenance is
                // itself visible via every other cell's removal and via
                // the refusal set, so this dimension would otherwise
                // double-count it.
                if *still_maintained {
                    Direction::Downgrade
                } else {
                    Direction::Neutral
                }
            }
            ChangeKind::RefusalAdded(_) => Direction::Downgrade,
            ChangeKind::RefusalRemoved(_) => Direction::Upgrade,
            ChangeKind::ContractPoint { old, new, .. } => {
                let old_relaxed = !old.is_default();
                let new_relaxed = !new.is_default();
                if !old_relaxed && new_relaxed {
                    return Direction::Downgrade;
                }
                if old_relaxed && !new_relaxed {
                    return Direction::Upgrade;
                }
                let fh = interval_direction(old.frozen_horizon_seconds, new.frozen_horizon_seconds);
                let def = interval_direction(old.deferral_seconds, new.deferral_seconds);
                let rd = retain_departed_direction(&old.retain_departed, &new.retain_departed);
                let verdicts = [fh, def, rd];
                if verdicts.contains(&Some(Direction::Downgrade)) {
                    Direction::Downgrade
                } else if verdicts.contains(&Some(Direction::Upgrade)) {
                    Direction::Upgrade
                } else {
                    Direction::Neutral
                }
            }
            ChangeKind::ProbeAdded(_) => Direction::Upgrade,
            ChangeKind::ProbeRemoved(_) => Direction::Downgrade,
            ChangeKind::ColumnAdded(_) | ChangeKind::ColumnRemoved(_) => Direction::Neutral,
            ChangeKind::Determinism { old, new, .. } => match (old, new) {
                (Some(o), Some(n)) if n > o => Direction::Downgrade,
                (Some(o), Some(n)) if n < o => Direction::Upgrade,
                (Some(_), Some(_)) => Direction::Neutral,
                // One side has no fact for this column at all — an
                // asymmetric-presence anomaly (G3); there is no lattice
                // position to rank a missing fact against, so this is
                // deliberately `Neutral` rather than guessed.
                _ => Direction::Neutral,
            },
            ChangeKind::Comparability { old, new, .. } => match (old, new) {
                (Some(o), Some(n)) if n > o => Direction::Downgrade,
                (Some(o), Some(n)) if n < o => Direction::Upgrade,
                (Some(_), Some(_)) => Direction::Neutral,
                _ => Direction::Neutral,
            },
            ChangeKind::Discriminant { .. } => Direction::Neutral,
            ChangeKind::FdAdded(_) | ChangeKind::FdRemoved(_) => Direction::Neutral,
            ChangeKind::LiteralColumn { .. } => Direction::Neutral,
            ChangeKind::SetOpBarrier { old, new } | ChangeKind::FanOutJoin { old, new } => {
                if *new && !*old {
                    Direction::Downgrade
                } else if *old && !*new {
                    Direction::Upgrade
                } else {
                    Direction::Neutral
                }
            }
            ChangeKind::MaintenanceLost => Direction::Downgrade,
            ChangeKind::MaintenanceGained => Direction::Upgrade,
            ChangeKind::StateDowngrade { old, new, .. } => match (old, new) {
                (None, Some(_)) => Direction::Downgrade,
                (Some(_), None) => Direction::Upgrade,
                // Both present with a different shape (a different missing
                // structure or `original` technique) — no interval to widen,
                // same convention as `retain_departed`'s shape-only change.
                _ => Direction::Neutral,
            },
        }
    }

    /// The JSON `subject` string (`docs/specs/property_diff.md` §Surface).
    /// Exhaustive — no wildcard arm.
    pub fn subject(&self) -> String {
        match self {
            ChangeKind::Grain { subject, .. } => subject.clone(),
            ChangeKind::RowIdentity { .. } => String::new(),
            ChangeKind::SourceBound { source, .. } => source.clone(),
            ChangeKind::CellTechnique { cell, .. }
            | ChangeKind::CellCorner { cell, .. }
            | ChangeKind::CellRowIdentity { cell, .. }
            | ChangeKind::CellAdded { cell, .. }
            | ChangeKind::CellRemoved { cell, .. }
            | ChangeKind::ContractPoint { cell, .. }
            | ChangeKind::StateDowngrade { cell, .. } => cell.clone(),
            ChangeKind::RefusalAdded(r) | ChangeKind::RefusalRemoved(r) => r.text.clone(),
            ChangeKind::ProbeAdded(p) | ChangeKind::ProbeRemoved(p) => p.fact.clone(),
            ChangeKind::ColumnAdded(c) | ChangeKind::ColumnRemoved(c) => c.clone(),
            ChangeKind::Determinism { column, .. }
            | ChangeKind::Comparability { column, .. }
            | ChangeKind::Discriminant { column, .. }
            | ChangeKind::LiteralColumn { column, .. } => column.clone(),
            ChangeKind::FdAdded(fd) | ChangeKind::FdRemoved(fd) => {
                format!("{} -> {}", fd.key.join(","), fd.determines)
            }
            ChangeKind::SetOpBarrier { .. } => String::new(),
            ChangeKind::FanOutJoin { .. } => String::new(),
            ChangeKind::MaintenanceLost | ChangeKind::MaintenanceGained => String::new(),
        }
    }

    /// The `old` JSON encoding, `null` for a field with no old value (a
    /// `cell_added`/`probe_added`/etc. change).
    fn old_json(&self) -> Option<Value> {
        match self {
            ChangeKind::Grain { old, .. } => Some(to_json(old)),
            ChangeKind::RowIdentity { old, .. } | ChangeKind::CellRowIdentity { old, .. } => {
                Some(to_json(old))
            }
            ChangeKind::SourceBound { old, .. } => Some(to_json(old)),
            ChangeKind::CellTechnique { old, .. } => Some(to_json(old)),
            ChangeKind::CellCorner { old, .. } => Some(to_json(old)),
            ChangeKind::CellAdded { .. } => None,
            ChangeKind::CellRemoved { old, .. } => Some(to_json(old.as_ref())),
            ChangeKind::RefusalAdded(_) => None,
            ChangeKind::RefusalRemoved(r) => Some(to_json(r)),
            ChangeKind::ContractPoint { old, .. } => Some(to_json(old)),
            ChangeKind::ProbeAdded(_) => None,
            ChangeKind::ProbeRemoved(p) => Some(to_json(p)),
            ChangeKind::ColumnAdded(_) => None,
            ChangeKind::ColumnRemoved(c) => Some(to_json(c)),
            ChangeKind::Determinism { old, .. } => Some(to_json(old)),
            ChangeKind::Comparability { old, .. } => Some(to_json(old)),
            ChangeKind::Discriminant { old, .. } => Some(to_json(old)),
            ChangeKind::FdAdded(_) => None,
            ChangeKind::FdRemoved(fd) => Some(to_json(fd)),
            ChangeKind::LiteralColumn { old, .. } => Some(to_json(old)),
            ChangeKind::SetOpBarrier { old, .. } | ChangeKind::FanOutJoin { old, .. } => {
                Some(to_json(old))
            }
            ChangeKind::MaintenanceLost => Some(to_json(&true)),
            ChangeKind::MaintenanceGained => Some(to_json(&false)),
            ChangeKind::StateDowngrade { old, .. } => Some(to_json(old)),
        }
    }

    /// The `new` JSON encoding, `null` for a field with no new value (a
    /// `cell_removed`/`probe_removed`/etc. change).
    fn new_json(&self) -> Option<Value> {
        match self {
            ChangeKind::Grain { new, .. } => Some(to_json(new)),
            ChangeKind::RowIdentity { new, .. } | ChangeKind::CellRowIdentity { new, .. } => {
                Some(to_json(new))
            }
            ChangeKind::SourceBound { new, .. } => Some(to_json(new)),
            ChangeKind::CellTechnique { new, .. } => Some(to_json(new)),
            ChangeKind::CellCorner { new, .. } => Some(to_json(new)),
            ChangeKind::CellAdded { new, .. } => Some(to_json(new.as_ref())),
            ChangeKind::CellRemoved { .. } => None,
            ChangeKind::RefusalAdded(r) => Some(to_json(r)),
            ChangeKind::RefusalRemoved(_) => None,
            ChangeKind::ContractPoint { new, .. } => Some(to_json(new)),
            ChangeKind::ProbeAdded(p) => Some(to_json(p)),
            ChangeKind::ProbeRemoved(_) => None,
            ChangeKind::ColumnAdded(c) => Some(to_json(c)),
            ChangeKind::ColumnRemoved(_) => None,
            ChangeKind::Determinism { new, .. } => Some(to_json(new)),
            ChangeKind::Comparability { new, .. } => Some(to_json(new)),
            ChangeKind::Discriminant { new, .. } => Some(to_json(new)),
            ChangeKind::FdAdded(fd) => Some(to_json(fd)),
            ChangeKind::FdRemoved(_) => None,
            ChangeKind::LiteralColumn { new, .. } => Some(to_json(new)),
            ChangeKind::SetOpBarrier { new, .. } | ChangeKind::FanOutJoin { new, .. } => {
                Some(to_json(new))
            }
            ChangeKind::MaintenanceLost => Some(to_json(&false)),
            ChangeKind::MaintenanceGained => Some(to_json(&true)),
            ChangeKind::StateDowngrade { new, .. } => Some(to_json(new)),
        }
    }

    /// The one-line reason, quoted verbatim from the property derivation,
    /// never re-derived (`docs/specs/property_diff.md` §Design "Reasons are
    /// quoted, never re-derived").
    fn reason(&self) -> Option<String> {
        match self {
            ChangeKind::RefusalAdded(r) | ChangeKind::RefusalRemoved(r) => Some(r.text.clone()),
            ChangeKind::StateDowngrade { old, new, .. } => {
                new.as_ref().or(old.as_ref()).map(|sd| sd.reason.clone())
            }
            _ => None,
        }
    }
}

fn to_json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The items of `a` that occur more times in `a` than in `b`, one entry per
/// excess occurrence — a multiset difference. `DerivedFd` has no `Ord`/
/// `Hash`, so this is a small O(n²) linear scan rather than a `BTreeMap`
/// one; functional-dependency lists are short (G7,
/// `docs/outcomes/20260905-property-diff` fix round 1 — a plain
/// `Vec::contains` membership check silently drops a duplicate's removal).
fn multiset_excess<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Vec<T> {
    let mut b_remaining: Vec<bool> = vec![true; b.len()];
    let mut excess = Vec::new();
    for item in a {
        if let Some(slot) = b_remaining
            .iter_mut()
            .zip(b.iter())
            .find(|(available, candidate)| **available && *candidate == item)
        {
            *slot.0 = false;
        } else {
            excess.push(item.clone());
        }
    }
    excess
}

/// One reported difference (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub dimension: Dimension,
    pub subject: String,
    pub direction: Direction,
    pub old: Option<Value>,
    pub new: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip)]
    pub kind: ChangeKind,
}

impl Change {
    fn from_kind(kind: ChangeKind) -> Self {
        Change {
            dimension: kind.dimension(),
            subject: kind.subject(),
            direction: kind.direction(),
            old: kind.old_json(),
            new: kind.new_json(),
            reason: kind.reason(),
            kind,
        }
    }
}

/// Which kind of cause a shifted model's entry carries
/// (`docs/specs/property_diff.md` §"Attribution").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseKind {
    Edited,
    Added,
    Removed,
    Downstream,
}

/// A shifted model's attribution (`docs/specs/property_diff.md`
/// §"Attribution").
#[derive(Debug, Clone, Serialize)]
pub struct Cause {
    pub kind: CauseKind,
    pub of: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One shifted model's report (`docs/specs/property_diff.md` §Surface
/// "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct ModelDiff {
    pub model: String,
    pub cause: Cause,
    pub changes: Vec<Change>,
}

/// The diff's summary counts (`docs/specs/property_diff.md` §Surface
/// "JSON").
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffSummary {
    pub downgrades: usize,
    pub upgrades: usize,
    pub neutral: usize,
    pub shifted_models: usize,
}

/// The whole property diff (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct PropertyDiff {
    pub models: Vec<ModelDiff>,
    pub summary: DiffSummary,
}

/// The `baseline` object of the JSON schema
/// (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct BaselineInfo {
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub commit: String,
    pub resolved_as: String,
}

impl From<&smelt_core::baseline::ResolvedBaseline> for BaselineInfo {
    fn from(resolved: &smelt_core::baseline::ResolvedBaseline) -> Self {
        BaselineInfo {
            r#ref: resolved.requested.clone(),
            commit: resolved.commit.clone(),
            resolved_as: match resolved.resolved_as {
                smelt_core::baseline::ResolvedAs::Explicit => "explicit".to_string(),
                smelt_core::baseline::ResolvedAs::MergeBase => "merge_base".to_string(),
            },
        }
    }
}

/// The full `smelt explain --diff` report — top-level key order here IS the
/// §Surface "JSON" schema's top-level key order
/// (`docs/specs/property_diff.md` §Surface "JSON"). Every renderer
/// (`analysis::diff_render`, and later the Markdown/LSP consumers) reads
/// this value; none re-derives or re-sorts `models`
/// (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D5).
#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub baseline: BaselineInfo,
    pub edited_files: Vec<String>,
    pub summary: DiffSummary,
    pub models: Vec<ModelDiff>,
}

impl DiffReport {
    /// Assemble a [`DiffReport`] from a computed [`PropertyDiff`] plus the
    /// baseline/edited-file facts the caller (a later phase, git-aware)
    /// already resolved. `diff_profiles`'s pure `models`/`summary` are
    /// carried unchanged; this is presentation-envelope assembly, not a
    /// second diff.
    pub fn new(
        baseline: BaselineInfo,
        edited_files: Vec<String>,
        diff: PropertyDiff,
    ) -> Self {
        DiffReport {
            baseline,
            edited_files,
            summary: diff.summary,
            models: diff.models,
        }
    }
}

/// The working-tree graph plus the edit provenance the diff attributes with
/// (`docs/specs/property_diff.md` §"Attribution"). Built by the caller
/// (a later phase) — `diff_profiles` never touches git.
///
/// `upstream` carries **model and source** edges: `DependencyGraph::
/// get_upstream` returns model deps only (`build` deliberately drops
/// `smelt.sources.*` refs, `smelt-core/src/graph.rs`), but attribution must
/// walk to "every edited model **or source**"
/// (`docs/specs/property_diff.md` §"Attribution").
#[derive(Debug, Clone, Default)]
pub struct DiffGraph {
    /// name -> direct upstream names (models and sources) it references.
    pub upstream: BTreeMap<String, Vec<String>>,
    pub edited: BTreeSet<String>,
    pub project_config_changed: bool,
}

impl DiffGraph {
    /// Build a [`DiffGraph`] from a loaded [`DependencyGraph`], adding back
    /// the source edges `DependencyGraph::build` drops. A source name is
    /// its bare dot-path with the leading `sources` segment stripped (the
    /// same convention `smelt-cli::explain::find_source_info` and
    /// `PropertySet::source_bounds` use), so an edited source and a
    /// `source_bound` change key against the same name.
    pub fn from_dependency_graph(
        g: &DependencyGraph,
        edited: BTreeSet<String>,
        project_config_changed: bool,
    ) -> Self {
        let mut upstream: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, model) in g.iter_models() {
            let mut deps = g.get_upstream(name);
            for r in &model.refs {
                let SmeltRef::Path(segs) = &r.smelt_ref;
                if let Some((first, rest)) = segs.split_first() {
                    if first == "sources" && !rest.is_empty() {
                        let bare = rest.join(".");
                        if !deps.contains(&bare) {
                            deps.push(bare);
                        }
                    }
                }
            }
            deps.sort();
            deps.dedup();
            upstream.insert(name.to_string(), deps);
        }
        DiffGraph {
            upstream,
            edited,
            project_config_changed,
        }
    }

    /// Attribute `model`'s shift (`docs/specs/property_diff.md`
    /// §"Attribution"): BFS upward over `upstream` from `model`, stopping
    /// at the first edited node on each path (never passing through it).
    /// Own file edited ⇒ `Edited`. No edited ancestor reached and
    /// `project_config_changed` ⇒ `Downstream` with `of: []` and the
    /// model-level reason. No edited ancestor and no config change is not
    /// expected to be called (a model cannot shift with no cause), but
    /// resolves to the same `of: []` shape rather than panicking
    /// (fail-loud discipline never demands a panic where a value suffices).
    pub fn attribute(&self, model: &str) -> Cause {
        if self.edited.contains(model) {
            return Cause {
                kind: CauseKind::Edited,
                of: vec![],
                reason: None,
            };
        }
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        visited.insert(model.to_string());
        queue.push_back(model.to_string());
        let mut ancestors: BTreeSet<String> = BTreeSet::new();
        while let Some(current) = queue.pop_front() {
            let Some(ups) = self.upstream.get(&current) else {
                continue;
            };
            for up in ups {
                if !visited.insert(up.clone()) {
                    continue;
                }
                if self.edited.contains(up) {
                    ancestors.insert(up.clone());
                } else {
                    queue.push_back(up.clone());
                }
            }
        }
        if ancestors.is_empty() {
            Cause {
                kind: CauseKind::Downstream,
                of: vec![],
                reason: if self.project_config_changed {
                    Some("project configuration changed".to_string())
                } else {
                    None
                },
            }
        } else {
            Cause {
                kind: CauseKind::Downstream,
                of: ancestors.into_iter().collect(),
                reason: None,
            }
        }
    }
}

/// Diff two [`PropertySet`]s, field by field, with no `..` rest pattern —
/// a field added later is a compile error here until it is given a
/// dimension and a direction rule (`docs/specs/property_diff.md`
/// §Constraints item 3, and closing the "shifted with an empty changes
/// array" hole in §"The diff").
fn diff_property_set(old: &PropertySet, new: &PropertySet) -> Vec<ChangeKind> {
    let PropertySet {
        columns: old_columns,
        grain: old_grain,
        functional_dependencies: old_fds,
        determinism: old_determinism,
        comparability: old_comparability,
        discriminants: old_discriminants,
        literal_columns: old_literal_columns,
        has_set_op_barrier: old_set_op_barrier,
        has_fan_out_join: old_fan_out_join,
        row_identity: old_row_identity,
        source_bounds: old_source_bounds,
    } = old;
    let PropertySet {
        columns: new_columns,
        grain: new_grain,
        functional_dependencies: new_fds,
        determinism: new_determinism,
        comparability: new_comparability,
        discriminants: new_discriminants,
        literal_columns: new_literal_columns,
        has_set_op_barrier: new_set_op_barrier,
        has_fan_out_join: new_fan_out_join,
        row_identity: new_row_identity,
        source_bounds: new_source_bounds,
    } = new;

    let mut changes = Vec::new();

    if old_grain != new_grain {
        changes.push(ChangeKind::Grain {
            subject: String::new(),
            old: old_grain.clone(),
            new: new_grain.clone(),
        });
    }
    if old_row_identity != new_row_identity {
        changes.push(ChangeKind::RowIdentity {
            old: old_row_identity.clone(),
            new: new_row_identity.clone(),
        });
    }
    if old_set_op_barrier != new_set_op_barrier {
        changes.push(ChangeKind::SetOpBarrier {
            old: *old_set_op_barrier,
            new: *new_set_op_barrier,
        });
    }
    if old_fan_out_join != new_fan_out_join {
        changes.push(ChangeKind::FanOutJoin {
            old: *old_fan_out_join,
            new: *new_fan_out_join,
        });
    }

    // Columns: matched on name (renames are removal + addition, spec
    // "Renames are not detected").
    let old_col_set: BTreeSet<&String> = old_columns.iter().collect();
    let new_col_set: BTreeSet<&String> = new_columns.iter().collect();
    for c in old_col_set.difference(&new_col_set) {
        changes.push(ChangeKind::ColumnRemoved((*c).clone()));
    }
    for c in new_col_set.difference(&old_col_set) {
        changes.push(ChangeKind::ColumnAdded((*c).clone()));
    }

    // Source bounds: matched on source name.
    let old_sources: BTreeSet<&String> = old_source_bounds.keys().collect();
    let new_sources: BTreeSet<&String> = new_source_bounds.keys().collect();
    for s in old_sources.union(&new_sources) {
        let (Some(o), Some(n)) = (old_source_bounds.get(*s), new_source_bounds.get(*s)) else {
            // A source bound present on only one side is a `source_bound`
            // change too (bound derivation appearing/disappearing entirely
            // is itself worth reporting), matched here as widened/
            // narrowed via the same rank comparison against a synthetic
            // absent-side default of `NotDerivable`.
            let default = BoundResult::NotDerivable;
            let old = old_source_bounds
                .get(*s)
                .cloned()
                .unwrap_or(default.clone());
            let new = new_source_bounds.get(*s).cloned().unwrap_or(default);
            if old != new {
                changes.push(ChangeKind::SourceBound {
                    source: (*s).clone(),
                    old,
                    new,
                });
            }
            continue;
        };
        if o != n {
            changes.push(ChangeKind::SourceBound {
                source: (*s).clone(),
                old: o.clone(),
                new: n.clone(),
            });
        }
    }

    // Per-column determinism/comparability/discriminants: matched on
    // column name, over the UNION of both sides' keys (G3,
    // `docs/outcomes/20260905-property-diff` fix round 1) — a column
    // present in one map and absent from the other, with `columns`
    // otherwise unchanged, must still surface as a change, mirroring
    // `literal_columns`'s own union-based diff just below.
    let old_det: BTreeMap<&String, &Det> = old_determinism
        .iter()
        .map(|d| (&d.output, &d.level))
        .collect();
    let new_det: BTreeMap<&String, &Det> = new_determinism
        .iter()
        .map(|d| (&d.output, &d.level))
        .collect();
    let all_det_cols: BTreeSet<&String> = old_det.keys().chain(new_det.keys()).copied().collect();
    for col in all_det_cols {
        let o = old_det.get(col).copied();
        let n = new_det.get(col).copied();
        if o != n {
            changes.push(ChangeKind::Determinism {
                column: col.clone(),
                old: o.copied(),
                new: n.copied(),
            });
        }
    }

    let old_comp: BTreeMap<&String, &Comp> = old_comparability
        .iter()
        .map(|c| (&c.output, &c.comparability))
        .collect();
    let new_comp: BTreeMap<&String, &Comp> = new_comparability
        .iter()
        .map(|c| (&c.output, &c.comparability))
        .collect();
    let all_comp_cols: BTreeSet<&String> =
        old_comp.keys().chain(new_comp.keys()).copied().collect();
    for col in all_comp_cols {
        let o = old_comp.get(col).copied();
        let n = new_comp.get(col).copied();
        if o != n {
            changes.push(ChangeKind::Comparability {
                column: col.clone(),
                old: o.copied(),
                new: n.copied(),
            });
        }
    }

    let old_disc: BTreeMap<&String, &crate::analysis::discriminants::Discriminants> =
        old_discriminants
            .iter()
            .map(|d| (&d.output, &d.discriminants))
            .collect();
    let new_disc: BTreeMap<&String, &crate::analysis::discriminants::Discriminants> =
        new_discriminants
            .iter()
            .map(|d| (&d.output, &d.discriminants))
            .collect();
    let all_disc_cols: BTreeSet<&String> =
        old_disc.keys().chain(new_disc.keys()).copied().collect();
    for col in all_disc_cols {
        let o = old_disc.get(col).copied();
        let n = new_disc.get(col).copied();
        if o != n {
            changes.push(ChangeKind::Discriminant {
                column: col.clone(),
                old: o.copied(),
                new: n.copied(),
            });
        }
    }

    // Functional dependencies: matched on (key, determines) as a whole
    // tuple (an FD is identified by its full shape, not a separate name).
    // `multiset_excess` (G7, `docs/outcomes/20260905-property-diff` fix
    // round 1) counts occurrences rather than membership — plain `.contains`
    // would silently drop a duplicate FD's removal (two copies of the same
    // FD on the old side, one on the new side, is a real removal `.contains`
    // cannot see since the value is still present).
    for fd in multiset_excess(old_fds, new_fds) {
        changes.push(ChangeKind::FdRemoved(fd));
    }
    for fd in multiset_excess(new_fds, old_fds) {
        changes.push(ChangeKind::FdAdded(fd));
    }

    // Literal columns: matched on column name.
    let old_lit: BTreeMap<&String, &String> =
        old_literal_columns.iter().map(|(k, v)| (k, v)).collect();
    let new_lit: BTreeMap<&String, &String> =
        new_literal_columns.iter().map(|(k, v)| (k, v)).collect();
    let all_lit_cols: BTreeSet<&String> = old_lit.keys().chain(new_lit.keys()).copied().collect();
    for col in all_lit_cols {
        let o = old_lit.get(col).map(|s| (*s).clone());
        let n = new_lit.get(col).map(|s| (*s).clone());
        if o != n {
            changes.push(ChangeKind::LiteralColumn {
                column: col.clone(),
                old: o,
                new: n,
            });
        }
    }

    changes
}

/// The `(group, trigger)` match key rendered as the report's own
/// `<group>@<trigger>` cell address (`docs/outcomes/20260905-property-diff/
/// phases/03-plan.md` "Cell subject").
fn cell_key(v: &CellVerdict) -> String {
    format!("{}@{}", v.group, v.trigger)
}

/// Diff two cell-verdict lists, matched on `(group, trigger)`
/// (`docs/specs/property_diff.md` §"The diff"). `still_maintained` on an
/// added/removed change is whether the *other* side's cell list is
/// non-empty (whether the model remains maintained at all).
fn diff_cell_verdicts(old: &[CellVerdict], new: &[CellVerdict]) -> Vec<ChangeKind> {
    let mut changes = Vec::new();
    let old_by_key: BTreeMap<String, &CellVerdict> = old.iter().map(|v| (cell_key(v), v)).collect();
    let new_by_key: BTreeMap<String, &CellVerdict> = new.iter().map(|v| (cell_key(v), v)).collect();

    for (key, old_v) in &old_by_key {
        match new_by_key.get(key) {
            None => changes.push(ChangeKind::CellRemoved {
                cell: key.clone(),
                old: Box::new((*old_v).clone()),
                still_maintained: !new.is_empty(),
            }),
            Some(new_v) => {
                let CellVerdict {
                    group: _,
                    trigger: _,
                    corner: old_corner,
                    technique: old_technique,
                    row_identity: old_ri,
                    contract_point: old_cp,
                    state_downgrade: old_sd,
                } = *old_v;
                let CellVerdict {
                    group: _,
                    trigger: _,
                    corner: new_corner,
                    technique: new_technique,
                    row_identity: new_ri,
                    contract_point: new_cp,
                    state_downgrade: new_sd,
                } = *new_v;
                if old_technique != new_technique {
                    changes.push(ChangeKind::CellTechnique {
                        cell: key.clone(),
                        old: *old_technique,
                        new: *new_technique,
                    });
                }
                if old_corner != new_corner {
                    changes.push(ChangeKind::CellCorner {
                        cell: key.clone(),
                        old: old_corner.clone(),
                        new: new_corner.clone(),
                    });
                }
                if old_ri != new_ri {
                    changes.push(ChangeKind::CellRowIdentity {
                        cell: key.clone(),
                        old: old_ri.clone(),
                        new: new_ri.clone(),
                    });
                }
                if old_cp != new_cp {
                    changes.push(ChangeKind::ContractPoint {
                        cell: key.clone(),
                        old: old_cp.clone(),
                        new: new_cp.clone(),
                    });
                }
                if old_sd != new_sd {
                    changes.push(ChangeKind::StateDowngrade {
                        cell: key.clone(),
                        old: old_sd.clone(),
                        new: new_sd.clone(),
                    });
                }
            }
        }
    }
    for (key, new_v) in &new_by_key {
        if !old_by_key.contains_key(key) {
            changes.push(ChangeKind::CellAdded {
                cell: key.clone(),
                new: Box::new((*new_v).clone()),
                still_maintained: !old.is_empty(),
            });
        }
    }
    changes
}

/// Diff two refusal sets, matched on `(code, text)`
/// (`docs/specs/property_diff.md` §"The diff" — a `None`-coded refusal
/// matches only another `None`-coded refusal with the same text).
fn diff_refusals(old: &[ProfileRefusal], new: &[ProfileRefusal]) -> Vec<ChangeKind> {
    let mut changes = Vec::new();
    for r in old {
        if !new.contains(r) {
            changes.push(ChangeKind::RefusalRemoved(r.clone()));
        }
    }
    for r in new {
        if !old.contains(r) {
            changes.push(ChangeKind::RefusalAdded(r.clone()));
        }
    }
    changes
}

/// Diff two probe sets, matched on `(fact, cell)`
/// (`docs/specs/property_diff.md` §"The diff"). G2 (`docs/outcomes/
/// 20260905-property-diff` fix round 1): a matched pair's `probe` field
/// (the named diagnostic) is destructured explicitly, with NO `..`, so a
/// change to it cannot be silently dropped the way it previously was — a
/// matched probe whose `probe` field changed emitted nothing at all. There
/// is no dedicated dimension for "the probe's diagnostic changed" (the JSON
/// schema names only `probe_added`/`probe_removed`), so it is reported the
/// same way a renamed column is (spec "The diff": "Renames are not
/// detected... is a removal plus an addition").
fn diff_probes(old: &[ProfileProbe], new: &[ProfileProbe]) -> Vec<ChangeKind> {
    let mut changes = Vec::new();
    let key = |p: &ProfileProbe| (p.fact.clone(), p.cell.clone());
    let old_by_key: BTreeMap<(String, String), &ProfileProbe> =
        old.iter().map(|p| (key(p), p)).collect();
    let new_by_key: BTreeMap<(String, String), &ProfileProbe> =
        new.iter().map(|p| (key(p), p)).collect();

    for (k, old_p) in &old_by_key {
        match new_by_key.get(k) {
            None => changes.push(ChangeKind::ProbeRemoved((*old_p).clone())),
            Some(new_p) => {
                let ProfileProbe {
                    fact: _,
                    probe: old_probe,
                    cell: _,
                } = *old_p;
                let ProfileProbe {
                    fact: _,
                    probe: new_probe,
                    cell: _,
                } = *new_p;
                if old_probe != new_probe {
                    changes.push(ChangeKind::ProbeRemoved((*old_p).clone()));
                    changes.push(ChangeKind::ProbeAdded((*new_p).clone()));
                }
            }
        }
    }
    for (k, new_p) in &new_by_key {
        if !old_by_key.contains_key(k) {
            changes.push(ChangeKind::ProbeAdded((*new_p).clone()));
        }
    }
    changes
}

/// Diff two [`PropertyProfile`]s, with no `..` rest pattern (mirrors
/// [`diff_property_set`]'s field-coverage guarantee for the top-level
/// profile shape).
fn diff_profile(old: &PropertyProfile, new: &PropertyProfile) -> Vec<Change> {
    let PropertyProfile {
        properties: old_properties,
        cell_verdicts: old_cells,
        refusals: old_refusals,
        probes: old_probes,
    } = old;
    let PropertyProfile {
        properties: new_properties,
        cell_verdicts: new_cells,
        refusals: new_refusals,
        probes: new_probes,
    } = new;

    let mut kinds = diff_property_set(old_properties, new_properties);
    kinds.extend(diff_cell_verdicts(old_cells, new_cells));
    // G1 (`docs/outcomes/20260905-property-diff` fix round 1): emitted ONCE
    // here, at the profile level, never derived from the per-cell
    // `cell_removed`/`cell_added` changes above — `cell_removed` stays
    // `Neutral` in this case (see its own doc comment), so this is the only
    // signal that "no longer incrementally maintained" surfaces as. This is
    // the fix for the case a plain `refresh: incremental` -> `refresh: full`
    // edit hit: `derive_model_maintenance_plan` returns `None` before any
    // refusal is constructed, so old cells/new refusals were both empty and
    // nothing downgraded.
    if !old_cells.is_empty() && new_cells.is_empty() {
        kinds.push(ChangeKind::MaintenanceLost);
    } else if old_cells.is_empty() && !new_cells.is_empty() {
        kinds.push(ChangeKind::MaintenanceGained);
    }
    kinds.extend(diff_refusals(old_refusals, new_refusals));
    kinds.extend(diff_probes(old_probes, new_probes));
    kinds.into_iter().map(Change::from_kind).collect()
}

/// Every change for a model that is `added`/`removed` in its entirety: one
/// change per profile field, with `old = null` (added) or `new = null`
/// (removed) — `docs/specs/property_diff.md` §"The diff".
fn whole_model_changes(profile: &PropertyProfile, added: bool) -> Vec<Change> {
    // Reuse the field-by-field diff against an "empty" profile so every
    // field still gets its own dimension, then null out the absent side.
    let empty = PropertyProfile {
        properties: PropertySet {
            columns: Vec::new(),
            grain: Grain::unkeyed(),
            functional_dependencies: Vec::new(),
            determinism: Vec::new(),
            comparability: Vec::new(),
            discriminants: Vec::new(),
            literal_columns: Vec::new(),
            has_set_op_barrier: false,
            has_fan_out_join: false,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            source_bounds: BTreeMap::new(),
        },
        cell_verdicts: Vec::new(),
        refusals: Vec::new(),
        probes: Vec::new(),
    };
    let mut changes = if added {
        diff_profile(&empty, profile)
    } else {
        diff_profile(profile, &empty)
    };
    for c in &mut changes {
        if added {
            c.old = None;
        } else {
            c.new = None;
        }
        // G6 (`docs/outcomes/20260905-property-diff` fix round 1):
        // per-dimension directions are noise for a model that is wholly
        // added or removed — the `cause` (`added`/`removed`) already says
        // so, and grading e.g. a new model's every `refusal_added` as
        // `Downgrade` and a deleted model's every `refusal_removed` as
        // `Upgrade` invents a signal the summary counts should not carry.
        c.direction = Direction::Neutral;
    }
    changes
}

/// The pure diff (`docs/specs/property_diff.md` §"The diff", §Constraints
/// item 2 "Diff purity"): no I/O, no ledger, no backend, no git.
pub fn diff_profiles(
    old: &BTreeMap<String, PropertyProfile>,
    new: &BTreeMap<String, PropertyProfile>,
    graph: &DiffGraph,
) -> PropertyDiff {
    let mut model_diffs: Vec<ModelDiff> = Vec::new();
    let all_names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();

    for name in &all_names {
        let (changes, cause_kind) = match (old.get(*name), new.get(*name)) {
            (None, Some(new_profile)) => (whole_model_changes(new_profile, true), CauseKind::Added),
            (Some(old_profile), None) => {
                (whole_model_changes(old_profile, false), CauseKind::Removed)
            }
            (Some(old_profile), Some(new_profile)) => {
                if old_profile == new_profile {
                    continue;
                }
                let attributed = graph.attribute(name);
                model_diffs.push(ModelDiff {
                    model: (*name).clone(),
                    cause: attributed,
                    changes: diff_profile(old_profile, new_profile),
                });
                continue;
            }
            (None, None) => continue,
        };
        model_diffs.push(ModelDiff {
            model: (*name).clone(),
            cause: Cause {
                kind: cause_kind,
                of: vec![],
                reason: None,
            },
            changes,
        });
    }

    // Order: the graph's topological order (upstream first), then name for
    // ties (`docs/specs/property_diff.md` §Surface "Text").
    let mut order_index: BTreeMap<String, usize> = BTreeMap::new();
    if let Ok(topo) = topological_order(graph) {
        for (i, n) in topo.into_iter().enumerate() {
            order_index.insert(n, i);
        }
    }
    model_diffs.sort_by(|a, b| {
        let ai = order_index.get(&a.model);
        let bi = order_index.get(&b.model);
        match (ai, bi) {
            (Some(ai), Some(bi)) => ai.cmp(bi).then_with(|| a.model.cmp(&b.model)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.model.cmp(&b.model),
        }
    });

    let mut summary = DiffSummary {
        shifted_models: model_diffs.len(),
        ..Default::default()
    };
    for m in &model_diffs {
        for c in &m.changes {
            match c.direction {
                Direction::Downgrade => summary.downgrades += 1,
                Direction::Upgrade => summary.upgrades += 1,
                Direction::Neutral => summary.neutral += 1,
            }
        }
    }

    PropertyDiff {
        models: model_diffs,
        summary,
    }
}

/// A simple upstream-first topological order over `graph.upstream`,
/// falling back to name order alone if the graph is cyclic (a cyclic graph
/// is already a `GraphError` upstream of this module — this just must not
/// hang, `docs/outcomes/20260905-property-diff/phases/03-plan.md` "Risks").
fn topological_order(graph: &DiffGraph) -> Result<Vec<String>, ()> {
    let mut all_names: BTreeSet<String> = graph.upstream.keys().cloned().collect();
    for ups in graph.upstream.values() {
        for u in ups {
            all_names.insert(u.clone());
        }
    }
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut in_progress: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();

    fn visit(
        node: &str,
        graph: &DiffGraph,
        visited: &mut BTreeSet<String>,
        in_progress: &mut BTreeSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), ()> {
        if visited.contains(node) {
            return Ok(());
        }
        if !in_progress.insert(node.to_string()) {
            return Err(());
        }
        if let Some(ups) = graph.upstream.get(node) {
            for u in ups {
                visit(u, graph, visited, in_progress, order)?;
            }
        }
        in_progress.remove(node);
        visited.insert(node.to_string());
        order.push(node.to_string());
        Ok(())
    }

    for name in &all_names {
        visit(name, graph, &mut visited, &mut in_progress, &mut order)?;
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ContractPointView;
    use crate::maintenance::{Corner, RowIdentity, Technique, Trigger};
    use smelt_core::config::RetainDeparted;

    fn set(row_identity: RowIdentityVerdict) -> PropertySet {
        PropertySet {
            columns: vec!["a".to_string()],
            grain: Grain::unkeyed(),
            functional_dependencies: Vec::new(),
            // Every mutation test below changes an existing column's
            // *value*, never its presence, so the base carries one
            // determinism/comparability/discriminant entry for "a" already;
            // the per-column diff loops below are keyed on the union but
            // need not synthesize a "missing" default.
            determinism: vec![crate::analysis::walk::ColumnDeterminism {
                output: "a".to_string(),
                level: Det::Clean,
            }],
            comparability: vec![crate::analysis::walk::ColumnComparability {
                output: "a".to_string(),
                comparability: Comp::Comparable,
            }],
            discriminants: vec![crate::analysis::walk::ColumnDiscriminant {
                output: "a".to_string(),
                discriminants: crate::analysis::discriminants::Discriminants {
                    is_monoid: false,
                    needs_inverse: false,
                    decomposable: false,
                    monotone: crate::analysis::discriminants::Monotone::None,
                },
            }],
            literal_columns: Vec::new(),
            has_set_op_barrier: false,
            has_fan_out_join: false,
            row_identity,
            source_bounds: BTreeMap::new(),
        }
    }

    fn base_set() -> PropertySet {
        set(RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        })
    }

    fn profile_from(properties: PropertySet) -> PropertyProfile {
        PropertyProfile {
            properties,
            cell_verdicts: Vec::new(),
            refusals: Vec::new(),
            probes: Vec::new(),
        }
    }

    fn cell(technique: Technique, group: &str, trigger_source: &str) -> CellVerdict {
        CellVerdict {
            group: group.to_string(),
            trigger: format!(
                "{:?}",
                Trigger::NewData {
                    source: trigger_source.to_string()
                }
            ),
            corner: format!("{:?}", Corner::FoldDelta),
            technique,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::Key(vec!["id".to_string()]),
                proven_mismatch: None,
            },
            contract_point: ContractPointView::default(),
            state_downgrade: None,
        }
    }

    fn empty_graph() -> DiffGraph {
        DiffGraph::default()
    }

    // --- Direction table ---

    #[test]
    fn technique_downgrade_walks_the_ladder_down() {
        let old = cell(Technique::KeyedFold, "{a}", "orders");
        let new = cell(Technique::DeleteInsert, "{a}", "orders");
        let old_p = profile_from(base_set());
        let mut old_p = old_p;
        old_p.cell_verdicts = vec![old];
        let mut new_p = profile_from(base_set());
        new_p.cell_verdicts = vec![new];
        let changes = diff_profile(&old_p, &new_p);
        let technique_change = changes
            .iter()
            .find(|c| c.dimension == Dimension::CellTechnique)
            .expect("technique changed");
        assert_eq!(technique_change.direction, Direction::Downgrade);

        // Reverse direction.
        let changes_up = diff_profile(&new_p, &old_p);
        let up = changes_up
            .iter()
            .find(|c| c.dimension == Dimension::CellTechnique)
            .unwrap();
        assert_eq!(up.direction, Direction::Upgrade);
    }

    #[test]
    fn ladder_is_total() {
        let ranks = [
            technique_rank(Technique::KeyedFold),
            technique_rank(Technique::ColumnScopedMerge),
            technique_rank(Technique::InPlaceUpdate),
            technique_rank(Technique::PerGroupRecompute),
            technique_rank(Technique::DeleteInsert),
        ];
        for w in ranks.windows(2) {
            assert!(w[0] > w[1], "ladder must be strictly descending: {ranks:?}");
        }
    }

    #[test]
    fn cell_removed_from_maintained_model_is_a_downgrade() {
        let old = vec![
            cell(Technique::KeyedFold, "{a}", "orders"),
            cell(Technique::KeyedFold, "{b}", "orders"),
        ];
        let new = vec![cell(Technique::KeyedFold, "{a}", "orders")];
        let changes = diff_cell_verdicts(&old, &new);
        let removed = changes
            .iter()
            .find(|c| matches!(c, ChangeKind::CellRemoved { .. }))
            .unwrap();
        assert_eq!(removed.direction(), Direction::Downgrade);
    }

    #[test]
    fn cell_added_is_an_upgrade() {
        let old = vec![cell(Technique::KeyedFold, "{a}", "orders")];
        let new = vec![
            cell(Technique::KeyedFold, "{a}", "orders"),
            cell(Technique::KeyedFold, "{b}", "orders"),
        ];
        let changes = diff_cell_verdicts(&old, &new);
        let added = changes
            .iter()
            .find(|c| matches!(c, ChangeKind::CellAdded { .. }))
            .unwrap();
        assert_eq!(added.direction(), Direction::Upgrade);
    }

    fn bounded(secs: u64) -> BoundResult {
        use crate::analysis::source_bounds::Seconds;
        BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds(secs),
            after: Seconds(0),
        }
    }

    #[test]
    fn source_bound_unbounding_is_a_downgrade() {
        let k = ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: bounded(60),
            new: BoundResult::Unbounded,
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn source_bound_widening_is_a_downgrade() {
        let k = ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: bounded(60),
            new: bounded(120),
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn source_bound_narrowing_is_an_upgrade() {
        let k = ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: bounded(120),
            new: bounded(60),
        };
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn source_bound_unbounded_to_not_derivable_is_neutral() {
        let k = ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: BoundResult::Unbounded,
            new: BoundResult::NotDerivable,
        };
        assert_eq!(k.direction(), Direction::Neutral);
        let reverse = ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: BoundResult::NotDerivable,
            new: BoundResult::Unbounded,
        };
        assert_eq!(reverse.direction(), Direction::Neutral);
    }

    #[test]
    fn grain_lost_is_a_downgrade() {
        let full = Grain {
            keys: vec![vec!["id".to_string()]],
        };
        let empty = Grain::unkeyed();
        let k = ChangeKind::Grain {
            subject: String::new(),
            old: full.clone(),
            new: empty.clone(),
        };
        assert_eq!(k.direction(), Direction::Downgrade);

        // Partial loss: had two keys, now only one — still a downgrade
        // ("lost a key column").
        let two_keys = Grain {
            keys: vec![vec!["id".to_string()], vec!["email".to_string()]],
        };
        let one_key = Grain {
            keys: vec![vec!["id".to_string()]],
        };
        let partial = ChangeKind::Grain {
            subject: String::new(),
            old: two_keys,
            new: one_key,
        };
        assert_eq!(partial.direction(), Direction::Downgrade);
    }

    #[test]
    fn grain_gained_is_an_upgrade() {
        let k = ChangeKind::Grain {
            subject: String::new(),
            old: Grain::unkeyed(),
            new: Grain {
                keys: vec![vec!["id".to_string()]],
            },
        };
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn row_identity_key_to_whole_row_is_a_downgrade() {
        let k = ChangeKind::RowIdentity {
            old: RowIdentityVerdict {
                identity: RowIdentity::Key(vec!["id".to_string()]),
                proven_mismatch: None,
            },
            new: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
        };
        assert_eq!(k.direction(), Direction::Downgrade);
        let reverse = ChangeKind::RowIdentity {
            old: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            new: RowIdentityVerdict {
                identity: RowIdentity::Key(vec!["id".to_string()]),
                proven_mismatch: None,
            },
        };
        assert_eq!(reverse.direction(), Direction::Upgrade);
    }

    fn refusal(code: Option<&str>, text: &str) -> ProfileRefusal {
        ProfileRefusal {
            code: code.map(|c| c.to_string()),
            text: text.to_string(),
        }
    }

    #[test]
    fn refusal_added_is_a_downgrade() {
        let k = ChangeKind::RefusalAdded(refusal(Some("MaintenanceScanUnbounded"), "boom"));
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn refusal_removed_is_an_upgrade() {
        let k = ChangeKind::RefusalRemoved(refusal(Some("MaintenanceScanUnbounded"), "boom"));
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn uncoded_refusals_match_on_text_not_on_a_shared_placeholder() {
        let old = vec![refusal(None, "reach not derivable: A")];
        let new = vec![refusal(None, "reach not derivable: B")];
        let changes = diff_refusals(&old, &new);
        assert_eq!(
            changes.len(),
            2,
            "expected one removed + one added: {changes:?}"
        );
        assert!(changes.iter().any(
            |c| matches!(c, ChangeKind::RefusalRemoved(r) if r.text == "reach not derivable: A")
        ));
        assert!(changes.iter().any(
            |c| matches!(c, ChangeKind::RefusalAdded(r) if r.text == "reach not derivable: B")
        ));

        // Unchanged None-coded refusal on both sides: no change at all.
        let same = vec![refusal(None, "reach not derivable: A")];
        assert!(diff_refusals(&same, &same).is_empty());
    }

    fn contract_view_default() -> ContractPointView {
        ContractPointView::default()
    }

    #[test]
    fn contract_relaxation_added_is_a_downgrade() {
        let new = ContractPointView {
            frozen_horizon: Some("90 days".to_string()),
            frozen_horizon_seconds: Some(90 * 86400),
            ..Default::default()
        };
        let k = ChangeKind::ContractPoint {
            cell: "revenue@orders".to_string(),
            old: contract_view_default(),
            new,
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn contract_horizon_widened_is_a_downgrade() {
        let old = ContractPointView {
            frozen_horizon: Some("90 days".to_string()),
            frozen_horizon_seconds: Some(90 * 86400),
            ..Default::default()
        };
        let new = ContractPointView {
            frozen_horizon: Some("180 days".to_string()),
            frozen_horizon_seconds: Some(180 * 86400),
            ..Default::default()
        };
        let k = ChangeKind::ContractPoint {
            cell: "revenue@orders".to_string(),
            old,
            new,
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn contract_relaxation_removed_is_an_upgrade() {
        let old = ContractPointView {
            frozen_horizon: Some("90 days".to_string()),
            frozen_horizon_seconds: Some(90 * 86400),
            ..Default::default()
        };
        let k = ChangeKind::ContractPoint {
            cell: "revenue@orders".to_string(),
            old,
            new: contract_view_default(),
        };
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn retain_departed_appearing_is_a_downgrade() {
        let new = ContractPointView {
            retain_departed: Some("true".to_string()),
            ..Default::default()
        };
        let k = ChangeKind::ContractPoint {
            cell: "revenue@orders".to_string(),
            old: contract_view_default(),
            new,
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn retain_departed_removed_is_an_upgrade() {
        let old = ContractPointView {
            retain_departed: Some("true".to_string()),
            ..Default::default()
        };
        let k = ChangeKind::ContractPoint {
            cell: "revenue@orders".to_string(),
            old,
            new: contract_view_default(),
        };
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn retain_departed_shape_change_is_neutral() {
        let _ = RetainDeparted::Bool(true); // sanity: type reachable in tests
        let old = ContractPointView {
            retain_departed: Some("true".to_string()),
            ..Default::default()
        };
        let new = ContractPointView {
            retain_departed: Some("tombstone: deleted_at".to_string()),
            ..Default::default()
        };
        let k = ChangeKind::ContractPoint {
            cell: "revenue@orders".to_string(),
            old,
            new,
        };
        assert_eq!(k.direction(), Direction::Neutral);
    }

    fn probe(fact: &str) -> ProfileProbe {
        ProfileProbe {
            fact: fact.to_string(),
            probe: "SomeProbe".to_string(),
            cell: "main.model (declared)".to_string(),
        }
    }

    #[test]
    fn probe_removed_is_a_downgrade() {
        let k = ChangeKind::ProbeRemoved(probe("assert_monotonic"));
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn probe_added_is_an_upgrade() {
        let k = ChangeKind::ProbeAdded(probe("assert_monotonic"));
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn determinism_clean_to_run_is_a_downgrade() {
        let k = ChangeKind::Determinism {
            column: "c".to_string(),
            old: Some(Det::Clean),
            new: Some(Det::Run),
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn determinism_run_to_row_is_a_downgrade() {
        let k = ChangeKind::Determinism {
            column: "c".to_string(),
            old: Some(Det::Run),
            new: Some(Det::Row),
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn determinism_row_to_clean_is_an_upgrade() {
        let k = ChangeKind::Determinism {
            column: "c".to_string(),
            old: Some(Det::Row),
            new: Some(Det::Clean),
        };
        assert_eq!(k.direction(), Direction::Upgrade);
    }

    #[test]
    fn column_added_removed_discriminant_and_corner_are_neutral() {
        assert_eq!(
            ChangeKind::ColumnAdded("c".to_string()).direction(),
            Direction::Neutral
        );
        assert_eq!(
            ChangeKind::ColumnRemoved("c".to_string()).direction(),
            Direction::Neutral
        );
        assert_eq!(
            ChangeKind::CellCorner {
                cell: "x".to_string(),
                old: "A".to_string(),
                new: "B".to_string()
            }
            .direction(),
            Direction::Neutral
        );
        use crate::analysis::discriminants::Discriminants;
        let d1 = Discriminants {
            is_monoid: true,
            needs_inverse: false,
            decomposable: false,
            monotone: crate::analysis::discriminants::Monotone::None,
        };
        let d2 = Discriminants {
            is_monoid: false,
            needs_inverse: false,
            decomposable: false,
            monotone: crate::analysis::discriminants::Monotone::None,
        };
        assert_eq!(
            ChangeKind::Discriminant {
                column: "c".to_string(),
                old: Some(d1),
                new: Some(d2)
            }
            .direction(),
            Direction::Neutral
        );
    }

    #[test]
    fn comparability_loss_is_a_downgrade() {
        let k = ChangeKind::Comparability {
            column: "c".to_string(),
            old: Some(Comp::Comparable),
            new: Some(Comp::Incomparable),
        };
        assert_eq!(k.direction(), Direction::Downgrade);
        let reverse = ChangeKind::Comparability {
            column: "c".to_string(),
            old: Some(Comp::Incomparable),
            new: Some(Comp::Comparable),
        };
        assert_eq!(reverse.direction(), Direction::Upgrade);
    }

    #[test]
    fn set_op_barrier_appearing_is_a_downgrade() {
        let k = ChangeKind::SetOpBarrier {
            old: false,
            new: true,
        };
        assert_eq!(k.direction(), Direction::Downgrade);
        let reverse = ChangeKind::SetOpBarrier {
            old: true,
            new: false,
        };
        assert_eq!(reverse.direction(), Direction::Upgrade);
    }

    #[test]
    fn fan_out_join_appearing_is_a_downgrade() {
        let k = ChangeKind::FanOutJoin {
            old: false,
            new: true,
        };
        assert_eq!(k.direction(), Direction::Downgrade);
    }

    #[test]
    fn fd_and_literal_changes_are_neutral() {
        let fd = DerivedFd {
            key: vec!["id".to_string()],
            determines: "amount".to_string(),
        };
        assert_eq!(
            ChangeKind::FdAdded(fd.clone()).direction(),
            Direction::Neutral
        );
        assert_eq!(ChangeKind::FdRemoved(fd).direction(), Direction::Neutral);
        assert_eq!(
            ChangeKind::LiteralColumn {
                column: "c".to_string(),
                old: None,
                new: Some("x".to_string())
            }
            .direction(),
            Direction::Neutral
        );
    }

    // --- Structural cases ---

    #[test]
    fn model_only_in_new_is_added_with_null_olds() {
        let old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        let mut new: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        new.insert("m".to_string(), profile_from(base_set()));
        let diff = diff_profiles(&old, &new, &empty_graph());
        assert_eq!(diff.models.len(), 1);
        let m = &diff.models[0];
        assert!(matches!(m.cause.kind, CauseKind::Added));
        assert!(!m.changes.is_empty());
        for c in &m.changes {
            assert_eq!(c.old, None, "added model's changes must have old=null");
            // G6 (`docs/outcomes/20260905-property-diff` fix round 1): a
            // wholly new model's changes are never graded — the `cause`
            // already says `added`.
            assert_eq!(c.direction, Direction::Neutral);
        }
    }

    #[test]
    fn model_only_in_old_is_removed() {
        let mut old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        old.insert("m".to_string(), profile_from(base_set()));
        let new: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        let diff = diff_profiles(&old, &new, &empty_graph());
        assert_eq!(diff.models.len(), 1);
        let m = &diff.models[0];
        assert!(matches!(m.cause.kind, CauseKind::Removed));
        for c in &m.changes {
            assert_eq!(c.new, None, "removed model's changes must have new=null");
            assert_eq!(c.direction, Direction::Neutral);
        }
    }

    // --- Fix round 1 (G1, G5, G7) ---

    #[test]
    fn maintenance_lost_is_a_downgrade_and_gained_is_an_upgrade() {
        assert_eq!(
            ChangeKind::MaintenanceLost.direction(),
            Direction::Downgrade
        );
        assert_eq!(
            ChangeKind::MaintenanceGained.direction(),
            Direction::Upgrade
        );
        assert_eq!(
            ChangeKind::MaintenanceLost.dimension(),
            Dimension::MaintenanceLost
        );
        assert_eq!(
            ChangeKind::MaintenanceGained.dimension(),
            Dimension::MaintenanceGained
        );
    }

    /// R8 (`docs/outcomes/20260905-property-diff/phases/04-plan.md`
    /// addendum): a cell's `state_downgrade` appearing is a downgrade,
    /// disappearing is an upgrade, and a shape-only change (different
    /// missing structure, presence unchanged) is neutral.
    #[test]
    fn state_downgrade_appearing_is_a_downgrade_disappearing_is_an_upgrade() {
        use crate::maintenance::availability::{StateDowngrade, StateStructure};

        let sd = StateDowngrade {
            original: Technique::KeyedFold,
            missing: StateStructure::FingerprintSidecar,
            reason: "no fingerprint sidecar on this target".to_string(),
        };
        assert_eq!(
            ChangeKind::StateDowngrade {
                cell: "c".to_string(),
                old: None,
                new: Some(sd.clone()),
            }
            .direction(),
            Direction::Downgrade
        );
        assert_eq!(
            ChangeKind::StateDowngrade {
                cell: "c".to_string(),
                old: Some(sd.clone()),
                new: None,
            }
            .direction(),
            Direction::Upgrade
        );
        let sd2 = StateDowngrade {
            missing: StateStructure::MergeLedger,
            ..sd.clone()
        };
        assert_eq!(
            ChangeKind::StateDowngrade {
                cell: "c".to_string(),
                old: Some(sd),
                new: Some(sd2),
            }
            .direction(),
            Direction::Neutral
        );
        assert_eq!(
            ChangeKind::StateDowngrade {
                cell: "c".to_string(),
                old: None,
                new: None,
            }
            .dimension(),
            Dimension::StateDowngrade
        );
    }

    /// A matched cell whose `state_downgrade` differs must surface as a
    /// `state_downgrade` change even when technique/corner/row-identity/
    /// contract point are unchanged — the case a plain `cell_technique`
    /// diff would miss if the ideal and degraded techniques happened to
    /// coincide on both sides for an unrelated reason.
    #[test]
    fn diff_cell_verdicts_surfaces_a_state_downgrade_on_an_otherwise_unchanged_cell() {
        use crate::maintenance::availability::{StateDowngrade, StateStructure};

        let mut old_cell = cell(Technique::DeleteInsert, "{a}", "orders");
        let mut new_cell = old_cell.clone();
        new_cell.state_downgrade = Some(StateDowngrade {
            original: Technique::KeyedFold,
            missing: StateStructure::FingerprintSidecar,
            reason: "no fingerprint sidecar on this target".to_string(),
        });
        old_cell.state_downgrade = None;

        let changes = diff_cell_verdicts(&[old_cell], &[new_cell]);
        assert!(
            changes.iter().any(|c| matches!(
                c,
                ChangeKind::StateDowngrade {
                    old: None,
                    new: Some(_),
                    ..
                }
            )),
            "expected a state_downgrade change, got {changes:?}"
        );
    }

    /// G1: `refresh: incremental` -> `refresh: full` with byte-identical SQL
    /// yields empty cells and empty refusals on the new side — before this
    /// fix, N `cell_removed` changes all graded `Neutral` and nothing
    /// downgraded. `diff_profile` must emit exactly one `maintenance_lost`
    /// change, graded `Downgrade`, alongside the (neutral) per-cell removals.
    #[test]
    fn losing_maintenance_entirely_surfaces_as_one_downgrade() {
        let mut old_profile = profile_from(base_set());
        old_profile.cell_verdicts = vec![
            cell(Technique::KeyedFold, "{a}", "orders"),
            cell(Technique::KeyedFold, "{b}", "orders"),
        ];
        let mut new_profile = old_profile.clone();
        new_profile.cell_verdicts = Vec::new();

        let changes = diff_profile(&old_profile, &new_profile);
        let lost: Vec<_> = changes
            .iter()
            .filter(|c| c.dimension == Dimension::MaintenanceLost)
            .collect();
        assert_eq!(
            lost.len(),
            1,
            "expected exactly one maintenance_lost change: {changes:?}"
        );
        assert_eq!(lost[0].direction, Direction::Downgrade);
        assert!(
            changes.iter().any(|c| c.direction == Direction::Downgrade),
            "the model must show at least one downgrade when it stops being maintained"
        );
        // The per-cell removals themselves stay neutral — the event is
        // named once, not N times.
        for c in changes
            .iter()
            .filter(|c| c.dimension == Dimension::CellRemoved)
        {
            assert_eq!(c.direction, Direction::Neutral);
        }
    }

    #[test]
    fn grain_composite_key_column_dropped_is_a_downgrade() {
        // Key(["id", "region"]) -> Key(["id"]): the composite narrowed —
        // "lost a key column" per spec §Direction. Before G5 this graded
        // Neutral (the two KeySets are unequal as values, so the prior
        // membership check saw both "lost" and "gained").
        let k = ChangeKind::Grain {
            subject: String::new(),
            old: Grain {
                keys: vec![vec!["id".to_string(), "region".to_string()]],
            },
            new: Grain {
                keys: vec![vec!["id".to_string()]],
            },
        };
        assert_eq!(k.direction(), Direction::Downgrade);

        // Symmetric: gaining a column on the composite is an upgrade.
        let reverse = ChangeKind::Grain {
            subject: String::new(),
            old: Grain {
                keys: vec![vec!["id".to_string()]],
            },
            new: Grain {
                keys: vec![vec!["id".to_string(), "region".to_string()]],
            },
        };
        assert_eq!(reverse.direction(), Direction::Upgrade);
    }

    #[test]
    fn fd_multiplicity_is_respected() {
        // Two copies of the same FD on the old side, one on the new side:
        // a plain `.contains` membership check sees the FD "still present"
        // and misses the removal of the duplicate.
        let fd = DerivedFd {
            key: vec!["id".to_string()],
            determines: "amount".to_string(),
        };
        let old = vec![fd.clone(), fd.clone()];
        let new = vec![fd.clone()];
        assert_eq!(multiset_excess(&old, &new), vec![fd.clone()]);
        assert!(multiset_excess(&new, &old).is_empty());
    }

    #[test]
    fn identical_profiles_are_unshifted() {
        let mut old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        old.insert("m".to_string(), profile_from(base_set()));
        let new = old.clone();
        let diff = diff_profiles(&old, &new, &empty_graph());
        assert!(diff.models.is_empty());
        assert_eq!(diff.summary.shifted_models, 0);
        assert_eq!(diff.summary.downgrades, 0);
        assert_eq!(diff.summary.upgrades, 0);
        assert_eq!(diff.summary.neutral, 0);
    }

    #[test]
    fn every_profile_difference_produces_at_least_one_change() {
        let base = base_set();

        let mut grain_changed = base.clone();
        grain_changed.grain = Grain {
            keys: vec![vec!["id".to_string()]],
        };
        let mut fd_changed = base.clone();
        fd_changed.functional_dependencies = vec![DerivedFd {
            key: vec![],
            determines: "c".to_string(),
        }];
        let mut det_changed = base.clone();
        det_changed.columns = vec!["a".to_string()];
        det_changed.determinism = vec![crate::analysis::walk::ColumnDeterminism {
            output: "a".to_string(),
            level: Det::Run,
        }];
        let mut comp_changed = base.clone();
        comp_changed.columns = vec!["a".to_string()];
        comp_changed.comparability = vec![crate::analysis::walk::ColumnComparability {
            output: "a".to_string(),
            comparability: Comp::Incomparable,
        }];
        let mut disc_changed = base.clone();
        disc_changed.columns = vec!["a".to_string()];
        disc_changed.discriminants = vec![crate::analysis::walk::ColumnDiscriminant {
            output: "a".to_string(),
            discriminants: crate::analysis::discriminants::Discriminants {
                is_monoid: true,
                needs_inverse: false,
                decomposable: false,
                monotone: crate::analysis::discriminants::Monotone::None,
            },
        }];
        let mut lit_changed = base.clone();
        lit_changed.literal_columns = vec![("a".to_string(), "1".to_string())];
        let mut set_op_changed = base.clone();
        set_op_changed.has_set_op_barrier = true;
        let mut fan_out_changed = base.clone();
        fan_out_changed.has_fan_out_join = true;
        let mut row_identity_changed = base.clone();
        row_identity_changed.row_identity = RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        };
        let mut source_bound_changed = base.clone();
        source_bound_changed
            .source_bounds
            .insert("raw.orders".to_string(), BoundResult::Unbounded);
        let mut columns_changed = base.clone();
        columns_changed.columns.push("extra".to_string());

        // G3/G4 (`docs/outcomes/20260905-property-diff` fix round 1): remove
        // the per-column entry entirely while `columns` stays unchanged —
        // the presence-asymmetry case the value-only mutations above cannot
        // exercise. `literal_columns` is exempted: it already diffs the
        // union of keys (`:867`), so removing its one entry is covered by
        // `lit_changed` above via a value-level round trip already.
        let mut det_removed = base.clone();
        det_removed.determinism = Vec::new();
        let mut comp_removed = base.clone();
        comp_removed.comparability = Vec::new();
        let mut disc_removed = base.clone();
        disc_removed.discriminants = Vec::new();

        let mutations = [
            grain_changed,
            fd_changed,
            det_changed,
            comp_changed,
            disc_changed,
            lit_changed,
            set_op_changed,
            fan_out_changed,
            row_identity_changed,
            source_bound_changed,
            columns_changed,
            det_removed,
            comp_removed,
            disc_removed,
        ];

        for m in mutations {
            let changed = m != base;
            let kinds = diff_property_set(&base, &m);
            assert_eq!(
                changed,
                !kinds.is_empty(),
                "PropertySet difference must produce a change (or the profile must actually be equal): {m:?} vs {base:?}"
            );
        }
    }

    /// G2/G4 (`docs/outcomes/20260905-property-diff` fix round 1): the
    /// PropertySet-level test above cannot see `cell_verdicts`/`refusals`/
    /// `probes` at all — this covers the rest of [`PropertyProfile`]. Before
    /// the G2/G1 fixes this test caught: a matched probe whose `probe` field
    /// changed produced zero changes (`diff_probes` matched on `(fact,
    /// cell)` only and never compared the third field), and a model whose
    /// cell list went from non-empty to empty produced only `Neutral`
    /// `cell_removed` changes with no dimension naming the event itself.
    #[test]
    fn every_profile_field_difference_produces_at_least_one_change() {
        let mut base = profile_from(base_set());
        base.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
        base.refusals = vec![refusal(Some("MaintenanceScanUnbounded"), "boom")];
        base.probes = vec![probe("assert_monotonic")];

        let mut probe_fact_changed = base.clone();
        probe_fact_changed.probes = vec![ProfileProbe {
            fact: "assert_other".to_string(),
            ..probe("assert_monotonic")
        }];
        let mut probe_diagnostic_changed = base.clone();
        probe_diagnostic_changed.probes = vec![ProfileProbe {
            probe: "SomeOtherProbe".to_string(),
            ..probe("assert_monotonic")
        }];
        let mut probe_cell_changed = base.clone();
        probe_cell_changed.probes = vec![ProfileProbe {
            cell: "main.other_model (declared)".to_string(),
            ..probe("assert_monotonic")
        }];
        let mut refusal_code_changed = base.clone();
        refusal_code_changed.refusals = vec![refusal(Some("MaintenanceUnsupportedGrain"), "boom")];
        let mut refusal_text_changed = base.clone();
        refusal_text_changed.refusals =
            vec![refusal(Some("MaintenanceScanUnbounded"), "different text")];
        let mut maintenance_lost = base.clone();
        maintenance_lost.cell_verdicts = Vec::new();
        let mut all_empty = profile_from(base_set());
        let mut maintenance_gained = all_empty.clone();
        maintenance_gained.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
        all_empty.refusals = Vec::new();
        all_empty.probes = Vec::new();

        let mutations = [
            probe_fact_changed,
            probe_diagnostic_changed,
            probe_cell_changed,
            refusal_code_changed,
            refusal_text_changed,
            maintenance_lost,
        ];
        for m in mutations {
            let changed = m != base;
            let changes = diff_profile(&base, &m);
            assert_eq!(
                changed,
                !changes.is_empty(),
                "PropertyProfile difference must produce a change: {m:?} vs {base:?}"
            );
        }

        // Symmetric gained-maintenance case, diffed the other way round.
        assert_ne!(all_empty, maintenance_gained);
        assert!(
            !diff_profile(&all_empty, &maintenance_gained).is_empty(),
            "a model gaining maintenance (empty cells -> non-empty) must produce a change"
        );
    }

    #[test]
    fn renamed_column_is_removal_plus_addition() {
        let mut base = base_set();
        base.columns = vec!["a".to_string()];
        let mut renamed = base_set();
        renamed.columns = vec!["b".to_string()];
        let kinds = diff_property_set(&base, &renamed);
        assert!(kinds
            .iter()
            .any(|k| matches!(k, ChangeKind::ColumnRemoved(c) if c == "a")));
        assert!(kinds
            .iter()
            .any(|k| matches!(k, ChangeKind::ColumnAdded(c) if c == "b")));
    }

    #[test]
    fn cells_match_on_group_and_trigger() {
        let old = vec![cell(Technique::KeyedFold, "{a}", "orders")];
        let new = vec![cell(Technique::KeyedFold, "{a}", "returns")];
        let changes = diff_cell_verdicts(&old, &new);
        assert!(changes
            .iter()
            .any(|c| matches!(c, ChangeKind::CellRemoved { .. })));
        assert!(changes
            .iter()
            .any(|c| matches!(c, ChangeKind::CellAdded { .. })));
        assert!(!changes
            .iter()
            .any(|c| matches!(c, ChangeKind::CellTechnique { .. })));
    }

    // --- Attribution ---

    fn graph_with(upstream: &[(&str, &[&str])], edited: &[&str]) -> DiffGraph {
        let mut g = DiffGraph::default();
        for (n, ups) in upstream {
            g.upstream
                .insert(n.to_string(), ups.iter().map(|s| s.to_string()).collect());
        }
        g.edited = edited.iter().map(|s| s.to_string()).collect();
        g
    }

    #[test]
    fn own_file_edited_is_cause_edited() {
        let g = graph_with(&[("a", &[])], &["a"]);
        let cause = g.attribute("a");
        assert!(matches!(cause.kind, CauseKind::Edited));
    }

    #[test]
    fn downstream_names_nearest_edited_ancestor() {
        // src -> a(edited) -> b
        let g = graph_with(&[("b", &["a"]), ("a", &["src"])], &["a"]);
        let cause = g.attribute("b");
        assert!(matches!(cause.kind, CauseKind::Downstream));
        assert_eq!(cause.of, vec!["a".to_string()]);
    }

    #[test]
    fn attribution_stops_at_the_first_edited_node() {
        // a(edited) -> b(edited) -> c
        let g = graph_with(&[("c", &["b"]), ("b", &["a"])], &["a", "b"]);
        let cause = g.attribute("c");
        assert_eq!(cause.of, vec!["b".to_string()]);
    }

    #[test]
    fn edited_source_is_a_valid_ancestor() {
        let g = graph_with(&[("m", &["raw.orders"])], &["raw.orders"]);
        let cause = g.attribute("m");
        assert_eq!(cause.of, vec!["raw.orders".to_string()]);
    }

    #[test]
    fn two_edited_ancestors_are_both_listed_sorted() {
        let g = graph_with(&[("c", &["a", "b"])], &["a", "b"]);
        let cause = g.attribute("c");
        assert_eq!(cause.of, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn no_edited_ancestor_yields_project_config_cause() {
        let mut g = graph_with(&[("m", &[])], &[]);
        g.project_config_changed = true;
        let cause = g.attribute("m");
        assert!(matches!(cause.kind, CauseKind::Downstream));
        assert!(cause.of.is_empty());
        assert_eq!(
            cause.reason,
            Some("project configuration changed".to_string())
        );
    }

    // --- Serialization / summary ---

    #[test]
    fn change_json_matches_the_spec_schema() {
        let k = ChangeKind::RefusalAdded(refusal(Some("MaintenanceScanUnbounded"), "boom"));
        let change = Change::from_kind(k);
        let v = serde_json::to_value(&change).unwrap();
        let obj = v.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["dimension", "direction", "new", "old", "reason", "subject"]
        );
        assert_eq!(v["dimension"], "refusal_added");
    }

    #[test]
    fn summary_counts_directions() {
        let mut old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        let mut new: BTreeMap<String, PropertyProfile> = BTreeMap::new();
        let mut old_profile = profile_from(base_set());
        old_profile.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
        let mut new_profile = old_profile.clone();
        new_profile.cell_verdicts = vec![cell(Technique::DeleteInsert, "{a}", "orders")];
        old.insert("m".to_string(), old_profile);
        new.insert("m".to_string(), new_profile);
        let diff = diff_profiles(&old, &new, &empty_graph());
        assert_eq!(diff.summary.downgrades, 1);
        assert_eq!(diff.summary.shifted_models, 1);
    }
}
