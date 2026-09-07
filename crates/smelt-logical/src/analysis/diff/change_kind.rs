use super::*;

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
        /// The cell's column-group display name, read structurally off the
        /// matched [`CellVerdict`] rather than split back out of `cell`'s
        /// own `{group}@{trigger:?}` join text.
        group: String,
        /// The source a `NewData`/`UpstreamMutation` trigger names, `None`
        /// for `Backfill`/`ColumnAdded` — mirrors [`CellVerdict::trigger_source`].
        trigger_source: Option<String>,
        old: Technique,
        new: Technique,
    },
    CellCorner {
        cell: String,
        group: String,
        trigger_source: Option<String>,
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
        /// Whether the removed cell's trigger source is still read by
        /// another surviving cell — the case `docs/specs/property_diff.md`
        /// §"Direction" grades a downgrade ("a column group lost its
        /// maintenance route for a source the model still depends on").
        /// `false` both when the source was dropped altogether and when the
        /// whole model lost maintenance (no surviving cell reads anything).
        source_survives: bool,
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
        group: String,
        trigger_source: Option<String>,
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
                    // A grain that widens is a downgrade because a proof
                    // only ever gets weaker by needing more columns: proving
                    // one row per `(date, user)` implies one row per
                    // `(date, user, name)`, never the reverse, and every
                    // downstream join written against the old key may now
                    // fan out (`docs/specs/property_diff.md` §"Direction" —
                    // the paragraph following the table). The narrowing case
                    // (the new key's column set is a strict subset of the
                    // old one — a *stronger* uniqueness claim) is the mirror
                    // image and is an upgrade. Compare the UNION of columns
                    // each side's keys cover, not key-set membership, so a
                    // composite key losing/gaining one column is seen as a
                    // subset/superset relationship rather than two disjoint
                    // `KeySet` values.
                    let old_cols: BTreeSet<&String> = old.keys.iter().flatten().collect();
                    let new_cols: BTreeSet<&String> = new.keys.iter().flatten().collect();
                    if new_cols == old_cols {
                        Direction::Neutral
                    } else if old_cols.is_subset(&new_cols) {
                        // new_cols is a strict superset of old_cols: widened.
                        Direction::Downgrade
                    } else if new_cols.is_subset(&old_cols) {
                        // new_cols is a strict subset of old_cols: narrowed.
                        Direction::Upgrade
                    } else {
                        // Neither a subset nor a superset of the other — a
                        // different key, not a weaker/stronger one
                        // (surfaced by the `row_key` story, not this
                        // dimension's direction).
                        Direction::Neutral
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
            ChangeKind::CellAdded {
                new,
                still_maintained,
                ..
            } => {
                // A new dependency is a cost, not an upgrade
                // (`docs/specs/property_diff.md` §Design). A model
                // *becoming* maintained (`still_maintained == false` — this
                // is the first cell it ever admitted) is `maintenance_
                // gained`'s upgrade to report, not this dimension's; on an
                // already-maintained model, a non-partition-local cell reads
                // its trigger source in full on every run and rebuilds the
                // whole model on any change to it — a downgrade. A new
                // partition-local cell is `neutral`: a dependency gained,
                // reported by the `dependency` story, not a proof that got
                // better.
                if *still_maintained && !new.partition_local {
                    Direction::Downgrade
                } else {
                    Direction::Neutral
                }
            }
            ChangeKind::CellRemoved {
                source_survives, ..
            } => {
                // A cell removed while another surviving cell still reads
                // the same trigger source lost a maintenance route
                // (`docs/specs/property_diff.md` §"Direction"): a downgrade.
                // `false` both when the source was dropped altogether (the
                // model shed a dependency — `neutral`, reported by the
                // `dependency` story) and when the whole model stopped being
                // maintained (visible via `maintenance_lost` instead, so
                // this dimension would otherwise double-count it).
                if *source_survives {
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
    pub(super) fn old_json(&self) -> Option<Value> {
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
    pub(super) fn new_json(&self) -> Option<Value> {
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
    pub(super) fn reason(&self) -> Option<String> {
        match self {
            ChangeKind::RefusalAdded(r) | ChangeKind::RefusalRemoved(r) => Some(r.text.clone()),
            ChangeKind::StateDowngrade { old, new, .. } => {
                new.as_ref().or(old.as_ref()).map(|sd| sd.reason.clone())
            }
            _ => None,
        }
    }
}
