//! The standing generative gate for `docs/specs/property_diff.md`
//! §Constraints item 11 ("Story coverage totality"): over a generated
//! `Vec<Change>` covering every [`Dimension`] variant with every direction
//! it can take, `narrate` (1) folds every change into exactly one story,
//! (2) never produces a `risk`/`cost` story with no folded downgrade, or a
//! downgrade folded by no `risk`/`cost` story, and (3) never panics —
//! including on adversarial edge cases (an empty change list, only
//! `other`-bound dimensions, `Unbounded`/`NotDerivable` source bounds on
//! either side, an empty-keyed grain on both sides).
//!
//! The generator builds a fixed pool of concrete, well-typed [`ChangeKind`]
//! values — one per `(dimension, direction)` combination the direction
//! table (`docs/specs/property_diff.md` §"Direction") can produce — and
//! draws random subsequences of it with `proptest::sample::subsequence`, so
//! every case exercises `Change::from_kind`'s real `direction()`/
//! `subject()`/`old_json()`/`new_json()` derivation rather than a
//! hand-rolled stand-in.

use proptest::prelude::*;
use proptest::sample::subsequence;

use smelt_logical::analysis::diff::{Cause, CauseKind, Change, ChangeKind, ModelDiff};
use smelt_logical::analysis::diff_stories::{narrate, Severity};
use smelt_logical::analysis::profile::{CellVerdict, ProfileProbe, ProfileRefusal};
use smelt_logical::analysis::source_bounds::{BoundResult, Seconds};
use smelt_logical::analysis::walk::{Comparability, DerivedFd, Determinism, Grain};
use smelt_logical::contract::ContractPointView;
use smelt_logical::maintenance::availability::{StateDowngrade, StateStructure};
use smelt_logical::maintenance::{RowIdentity, RowIdentityVerdict, Technique};

fn cell_verdict(
    group: &str,
    source: &str,
    technique: Technique,
    partition_local: bool,
) -> CellVerdict {
    CellVerdict {
        group: group.to_string(),
        trigger: format!("NewData {{ source: {source:?} }}"),
        corner: "RecomputeRegion".to_string(),
        technique,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        contract_point: ContractPointView::default(),
        state_downgrade: None,
        trigger_source: Some(source.to_string()),
        partition_local,
        locality_reason: if partition_local {
            None
        } else {
            Some((
                source.to_string(),
                "no partition_column declared".to_string(),
            ))
        },
    }
}

fn grain(cols: &[&str]) -> Grain {
    if cols.is_empty() {
        Grain::unkeyed()
    } else {
        Grain {
            keys: vec![cols.iter().map(|c| c.to_string()).collect()],
        }
    }
}

fn key_identity(cols: &[&str]) -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::Key(cols.iter().map(|c| c.to_string()).collect()),
        proven_mismatch: None,
    }
}

fn whole_row_identity() -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    }
}

fn bounded(before_days: u64, after_days: u64) -> BoundResult {
    BoundResult::Bounded {
        source_partition_col: "event_date".to_string(),
        before: Seconds(before_days * 86_400),
        after: Seconds(after_days * 86_400),
    }
}

/// The fixed pool: one [`ChangeKind`] per `(dimension, direction)`
/// combination the direction table can produce, plus a handful of
/// adversarial/edge-case entries (empty grain, `Unbounded`/`NotDerivable`
/// source bounds, one-sided determinism/comparability). Every `Dimension`
/// variant appears at least once.
fn change_kind_pool() -> Vec<ChangeKind> {
    vec![
        // grain
        ChangeKind::Grain {
            subject: String::new(),
            old: grain(&[]),
            new: grain(&["a"]),
        }, // upgrade (unkeyed -> keyed)
        ChangeKind::Grain {
            subject: String::new(),
            old: grain(&["a"]),
            new: grain(&[]),
        }, // downgrade (keyed -> unkeyed)
        ChangeKind::Grain {
            subject: String::new(),
            old: grain(&["a", "b"]),
            new: grain(&["a", "b", "c"]),
        }, // downgrade (widened)
        ChangeKind::Grain {
            subject: String::new(),
            old: grain(&["a", "b", "c"]),
            new: grain(&["a", "b"]),
        }, // upgrade (narrowed)
        ChangeKind::Grain {
            subject: String::new(),
            old: grain(&["a", "b"]),
            new: grain(&["a", "c"]),
        }, // neutral (different key)
        ChangeKind::Grain {
            subject: String::new(),
            old: grain(&[]),
            new: grain(&[]),
        }, // adversarial no-op
        // row_identity
        ChangeKind::RowIdentity {
            old: key_identity(&["a"]),
            new: whole_row_identity(),
        }, // downgrade
        ChangeKind::RowIdentity {
            old: whole_row_identity(),
            new: key_identity(&["a"]),
        }, // upgrade
        ChangeKind::RowIdentity {
            old: key_identity(&["a"]),
            new: key_identity(&["b"]),
        }, // neutral
        // source_bound
        ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: bounded(1, 1),
            new: BoundResult::Unbounded,
        }, // downgrade
        ChangeKind::SourceBound {
            source: "raw.orders".to_string(),
            old: BoundResult::Unbounded,
            new: bounded(1, 1),
        }, // upgrade
        ChangeKind::SourceBound {
            source: "raw.devices".to_string(),
            old: bounded(1, 1),
            new: bounded(7, 7),
        }, // downgrade (widened)
        ChangeKind::SourceBound {
            source: "silver.sessions".to_string(),
            old: bounded(7, 7),
            new: bounded(1, 1),
        }, // upgrade (narrowed)
        ChangeKind::SourceBound {
            source: "raw.events".to_string(),
            old: BoundResult::Unbounded,
            new: BoundResult::NotDerivable,
        }, // adversarial neutral
        // cell_technique — both a source-bearing trigger (NewData) and a
        // source-less one (Backfill), so `group`/`trigger_source` cover
        // `Some`/`None`.
        ChangeKind::CellTechnique {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            group: "{amount}".to_string(),
            trigger_source: Some("raw.orders".to_string()),
            old: Technique::KeyedFold,
            new: Technique::DeleteInsert,
        }, // downgrade
        ChangeKind::CellTechnique {
            cell: "{amount}@Backfill".to_string(),
            group: "{amount}".to_string(),
            trigger_source: None,
            old: Technique::DeleteInsert,
            new: Technique::KeyedFold,
        }, // upgrade
        // cell_corner (always neutral)
        ChangeKind::CellCorner {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            group: "{amount}".to_string(),
            trigger_source: Some("raw.orders".to_string()),
            old: "RmwRegion".to_string(),
            new: "RecomputeRegion".to_string(),
        },
        ChangeKind::CellCorner {
            cell: "{amount}@Backfill".to_string(),
            group: "{amount}".to_string(),
            trigger_source: None,
            old: "RecomputeRegion".to_string(),
            new: "RmwRegion".to_string(),
        },
        // cell_row_identity
        ChangeKind::CellRowIdentity {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            old: key_identity(&["a"]),
            new: whole_row_identity(),
        }, // downgrade
        ChangeKind::CellRowIdentity {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            old: whole_row_identity(),
            new: key_identity(&["a"]),
        }, // upgrade
        // cell_added / cell_removed
        ChangeKind::CellAdded {
            cell: "{device_type}@NewData { source: \"raw.devices\" }".to_string(),
            new: Box::new(cell_verdict(
                "{device_type}",
                "raw.devices",
                Technique::DeleteInsert,
                false,
            )),
            still_maintained: true,
        }, // downgrade (not partition-local)
        ChangeKind::CellAdded {
            cell: "{device_type}@NewData { source: \"raw.devices\" }".to_string(),
            new: Box::new(cell_verdict(
                "{device_type}",
                "raw.devices",
                Technique::DeleteInsert,
                true,
            )),
            still_maintained: true,
        }, // neutral (partition-local)
        ChangeKind::CellAdded {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            new: Box::new(cell_verdict(
                "{amount}",
                "raw.orders",
                Technique::KeyedFold,
                true,
            )),
            still_maintained: false,
        }, // neutral (first cell — maintenance_gained's upgrade)
        ChangeKind::CellRemoved {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            old: Box::new(cell_verdict(
                "{amount}",
                "raw.orders",
                Technique::KeyedFold,
                true,
            )),
            source_survives: true,
        }, // downgrade
        ChangeKind::CellRemoved {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            old: Box::new(cell_verdict(
                "{amount}",
                "raw.orders",
                Technique::KeyedFold,
                true,
            )),
            source_survives: false,
        }, // neutral
        // refusal_added / refusal_removed
        ChangeKind::RefusalAdded(ProfileRefusal {
            code: Some("MaintenanceScanUnbounded".to_string()),
            text: "ScanUnbounded".to_string(),
        }), // downgrade
        ChangeKind::RefusalRemoved(ProfileRefusal {
            code: None,
            text: "SomeRefusal".to_string(),
        }), // upgrade
        // contract_point
        ChangeKind::ContractPoint {
            cell: "{amount}@NewData".to_string(),
            old: ContractPointView::default(),
            new: ContractPointView {
                frozen_horizon: Some("90 days".to_string()),
                frozen_horizon_seconds: Some(90 * 86_400),
                ..Default::default()
            },
        }, // downgrade
        ChangeKind::ContractPoint {
            cell: "{amount}@NewData".to_string(),
            old: ContractPointView {
                frozen_horizon: Some("90 days".to_string()),
                frozen_horizon_seconds: Some(90 * 86_400),
                ..Default::default()
            },
            new: ContractPointView::default(),
        }, // upgrade
        ChangeKind::ContractPoint {
            cell: "{amount}@NewData".to_string(),
            old: ContractPointView {
                retain_departed: Some("true".to_string()),
                ..Default::default()
            },
            new: ContractPointView {
                retain_departed: Some("tombstone: deleted_at".to_string()),
                ..Default::default()
            },
        }, // neutral (shape-only)
        // probe_added / probe_removed
        ChangeKind::ProbeAdded(ProfileProbe {
            fact: "assert_monotonic".to_string(),
            probe: "MonotonicityViolated".to_string(),
            cell: "main.orders (declared)".to_string(),
        }), // upgrade
        ChangeKind::ProbeRemoved(ProfileProbe {
            fact: "assert_monotonic".to_string(),
            probe: "MonotonicityViolated".to_string(),
            cell: "main.orders (declared)".to_string(),
        }), // downgrade
        // column_added / column_removed (always neutral)
        ChangeKind::ColumnAdded("device_type".to_string()),
        ChangeKind::ColumnRemoved("legacy_flag".to_string()),
        // determinism (both present + one-sided)
        ChangeKind::Determinism {
            column: "amount".to_string(),
            old: Some(Determinism::Clean),
            new: Some(Determinism::Row),
        }, // downgrade
        ChangeKind::Determinism {
            column: "amount".to_string(),
            old: Some(Determinism::Row),
            new: Some(Determinism::Clean),
        }, // upgrade
        ChangeKind::Determinism {
            column: "device_type".to_string(),
            old: None,
            new: Some(Determinism::Clean),
        }, // neutral, schema-eligible (added column)
        // comparability (both present + one-sided)
        ChangeKind::Comparability {
            column: "amount".to_string(),
            old: Some(Comparability::Comparable),
            new: Some(Comparability::Incomparable),
        }, // downgrade
        ChangeKind::Comparability {
            column: "amount".to_string(),
            old: Some(Comparability::Incomparable),
            new: Some(Comparability::Comparable),
        }, // upgrade
        ChangeKind::Comparability {
            column: "device_type".to_string(),
            old: None,
            new: Some(Comparability::Comparable),
        }, // neutral, schema-eligible
        // discriminant (always neutral)
        ChangeKind::Discriminant {
            column: "total".to_string(),
            old: None,
            new: None,
        },
        // fd_added / fd_removed (always neutral)
        ChangeKind::FdAdded(DerivedFd {
            key: vec!["a".to_string()],
            determines: "b".to_string(),
        }),
        ChangeKind::FdRemoved(DerivedFd {
            key: vec!["a".to_string()],
            determines: "c".to_string(),
        }),
        // literal_column (always neutral)
        ChangeKind::LiteralColumn {
            column: "source_system".to_string(),
            old: None,
            new: Some("'web'".to_string()),
        },
        // set_op_barrier / fan_out_join
        ChangeKind::SetOpBarrier {
            old: false,
            new: true,
        }, // downgrade
        ChangeKind::SetOpBarrier {
            old: true,
            new: false,
        }, // upgrade
        ChangeKind::FanOutJoin {
            old: false,
            new: true,
        }, // downgrade
        ChangeKind::FanOutJoin {
            old: true,
            new: false,
        }, // upgrade
        // maintenance_lost / maintenance_gained
        ChangeKind::MaintenanceLost,
        ChangeKind::MaintenanceGained,
        // state_downgrade — one source-bearing cell, one Backfill cell.
        ChangeKind::StateDowngrade {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            group: "{amount}".to_string(),
            trigger_source: Some("raw.orders".to_string()),
            old: None,
            new: Some(StateDowngrade {
                original: Technique::KeyedFold,
                missing: StateStructure::MergeLedger,
                reason: "no merge ledger realised".to_string(),
            }),
        }, // downgrade
        ChangeKind::StateDowngrade {
            cell: "{amount}@Backfill".to_string(),
            group: "{amount}".to_string(),
            trigger_source: None,
            old: Some(StateDowngrade {
                original: Technique::KeyedFold,
                missing: StateStructure::MergeLedger,
                reason: "no merge ledger realised".to_string(),
            }),
            new: None,
        }, // upgrade
    ]
}

fn model_from(kinds: Vec<ChangeKind>) -> ModelDiff {
    ModelDiff {
        model: "gold.eventstream_with_identity".to_string(),
        cause: Cause {
            kind: CauseKind::Edited,
            of: vec![],
            reason: None,
        },
        changes: kinds.into_iter().map(Change::from_kind).collect(),
        stories: Vec::new(),
    }
}

fn changes_strategy() -> impl Strategy<Value = Vec<ChangeKind>> {
    subsequence(change_kind_pool(), 0..=change_kind_pool().len())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_change_is_folded_by_exactly_one_story(kinds in changes_strategy()) {
        let model = model_from(kinds);
        let stories = narrate(&model);

        let mut seen = vec![false; model.changes.len()];
        for story in &stories {
            for &idx in &story.changes {
                prop_assert!(idx < model.changes.len(), "story references out-of-range index {idx}");
                prop_assert!(!seen[idx], "change {idx} folded by more than one story");
                seen[idx] = true;
            }
        }
        prop_assert!(
            seen.iter().all(|&s| s),
            "not every change was folded by a story: {seen:?}"
        );
    }

    #[test]
    fn risk_and_cost_stories_fold_downgrades_and_vice_versa(kinds in changes_strategy()) {
        use smelt_logical::analysis::diff::Direction;

        let model = model_from(kinds);
        let stories = narrate(&model);

        let mut downgrade_covered = vec![false; model.changes.len()];
        for story in &stories {
            let folds_downgrade = story
                .changes
                .iter()
                .any(|&i| model.changes[i].direction == Direction::Downgrade);
            let folds_upgrade = story
                .changes
                .iter()
                .any(|&i| model.changes[i].direction == Direction::Upgrade);
            match story.severity {
                Severity::Risk | Severity::Cost => {
                    prop_assert!(
                        folds_downgrade,
                        "a {:?} story ({:?}) folds no downgrade",
                        story.severity,
                        story.kind
                    );
                }
                Severity::Improvement => {
                    prop_assert!(folds_upgrade, "an improvement story folds no upgrade");
                    prop_assert!(!folds_downgrade, "an improvement story folds a downgrade");
                }
                Severity::Info => {}
            }
            for &i in &story.changes {
                if model.changes[i].direction == Direction::Downgrade {
                    downgrade_covered[i] = true;
                    prop_assert!(
                        matches!(story.severity, Severity::Risk | Severity::Cost),
                        "change {i} is a downgrade but its story ({:?}) is {:?}",
                        story.kind,
                        story.severity
                    );
                }
            }
        }
        for (i, c) in model.changes.iter().enumerate() {
            if c.direction == Direction::Downgrade {
                prop_assert!(
                    downgrade_covered[i],
                    "downgrade at index {i} is not folded by any risk/cost story"
                );
            }
        }
    }

    #[test]
    fn narrate_never_panics(kinds in changes_strategy()) {
        let model = model_from(kinds);
        let _ = narrate(&model);
    }
}

#[test]
fn narrate_never_panics_on_adversarial_edge_cases() {
    // Empty change list.
    let _ = narrate(&model_from(vec![]));

    // Only `other`-bound dimensions.
    let _ = narrate(&model_from(vec![
        ChangeKind::CellCorner {
            cell: "{a}@NewData".to_string(),
            group: "{a}".to_string(),
            trigger_source: None,
            old: "RmwRegion".to_string(),
            new: "RecomputeRegion".to_string(),
        },
        ChangeKind::Discriminant {
            column: "total".to_string(),
            old: None,
            new: None,
        },
    ]));

    // `source_bound` `Unbounded`/`NotDerivable` on either side.
    let _ = narrate(&model_from(vec![ChangeKind::SourceBound {
        source: "raw.orders".to_string(),
        old: BoundResult::NotDerivable,
        new: BoundResult::Unbounded,
    }]));

    // Grain with empty keys on both sides.
    let _ = narrate(&model_from(vec![ChangeKind::Grain {
        subject: String::new(),
        old: Grain::unkeyed(),
        new: Grain::unkeyed(),
    }]));
}
