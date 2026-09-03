//! Coverage-matrix conformance sweep (`docs/plans/20260707-maintenance-plan-impl.md`
//! phase MP17): lifts named cells of the research coverage matrix
//! (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`
//! §"Coverage matrix") into grounded, executable assertions against the REAL
//! pure derivation (`smelt_logical::maintenance::derive`/`grouping`/
//! `granularity`) — no hand-waved technique, every assertion either:
//!
//! - **HOLDS** — the derived technique matches the catalogue's expected cell,
//!   and (where the technique is actually wired to run SQL, per MP11/MP12)
//!   the emitted maintenance is multiset-equal to a full refresh; or
//! - **refuses by name** — `plan.refusals` contains the specific `Refusal`
//!   variant the catalogue's honest-today verdict predicts.
//!
//! No test here asserts a third, silent outcome. Each test's doc comment
//! names its catalogue id and cites the exact catalogue verdict it pins.
//!
//! This file complements (does not replace) the reduced-scope
//! `maintenance_plan_conformance.rs` (EX-02, EX-24) already in this crate —
//! together they are the `coverage_matrix_is_inhabited` meta-test's `claimed`
//! registry for the cells listed below. See that meta-test
//! (`maintenance_plan_conformance.rs::coverage_matrix_is_inhabited`) for the
//! full inhabited-cell inventory and the explicit list of cells this phase
//! did NOT reach (the "known gap" set — never silently omitted).

use std::collections::BTreeSet;

use smelt_core::Granularity;
use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::granularity::check_declared_granularity;
use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts, Technique,
    Trigger,
};
use smelt_types::SqlFunction;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// EX-12 — currency-converted revenue: two mutable inputs merge into one
// column group (`07-example-catalogue.md` EX-12).
//
// Catalogue framing: "merged column group → recompute-region only
// (factoring degenerates)" — a *definitional* fact about column-group
// provenance, not a runtime behaviour to falsify (probe-status:
// not-probe-worthy). What IS falsifiable, and what this test pins is that
// the merged group's cell is a whole-row `DeleteInsert` recompute, never a
// `ColumnScopedMerge` — both `orders` and `fx_rates` are `MutableSnapshot`
// and both are read in the `JOIN`'s `ON` predicate, so both are
// membership-sensitive (`docs/specs/model_properties.md` §"Per-column
// mutation-sensitivity / column provenance", membership paragraph): either
// source's churn can retroactively add or remove a matched row, which only
// the recompute family can repair (`docs/specs/incremental_models.md`
// §"The plan matrix"). "Degenerates" here names the *grouping* fact (both
// sources collapse onto the SAME undifferentiated column group, `grouping.
// rs`'s own vocabulary for a merged/shared provenance set).
// ---------------------------------------------------------------------------

#[test]
fn ex12_multi_input_merge_degenerates_to_recompute() {
    let sql = "SELECT o.order_date, o.order_id, o.amount * fx.rate AS amount_usd \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.fx_rates fx \
                 ON fx.ccy = o.ccy AND fx.fixing_date = o.order_date";
    let sources = vec![
        SourceFacts {
            name: "orders".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: Some("order_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "fx_rates".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        },
    ];
    let skeleton = set(&["order_date", "order_id"]);
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "EX-12's qualified column references should resolve cleanly, not hit the \
         fail-closed whole-model collapse: {:?}",
        grouping.degenerate
    );
    assert_eq!(grouping.groups.len(), 1, "one merged group: {{amount_usd}}");
    let merged = &grouping.groups[0];
    assert_eq!(merged.columns, vec!["amount_usd".to_string()]);
    assert_eq!(
        merged.mutation_sensitivity,
        set(&["orders", "fx_rates"]),
        "the group's provenance is the UNION of both mutable inputs — no per-input isolation"
    );
    assert_eq!(
        merged.membership_sensitivity,
        set(&["orders", "fx_rates"]),
        "both sources are read in the JOIN's ON predicate — both are membership-sensitive"
    );

    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "revenue_usd".to_string(),
            grain: Grain::Partition {
                partition_col: "order_date".to_string(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    // Both triggering sources land on the SAME merged group with the SAME
    // technique — no per-source targeted isolation exists for a merged
    // group today. Membership sensitivity forces the recompute family for
    // both: a `ColumnScopedMerge` could rewrite `amount_usd` in place but
    // cannot create or delete the row a churned join match would.
    for source in ["orders", "fx_rates"] {
        let plan = derive_maintenance_plan(
            &inputs,
            &[Trigger::UpstreamMutation {
                source: source.to_string(),
            }],
        );
        assert!(
            plan.refusals.is_empty(),
            "mutation trigger for {source} refused unexpectedly: {:?}",
            plan.refusals
        );
        assert_eq!(plan.cells.len(), 1);
        assert_eq!(plan.cells[0].group, "{amount_usd}");
        assert_eq!(plan.cells[0].corner, Corner::RecomputeRegion);
        assert_eq!(plan.cells[0].technique, Technique::DeleteInsert);
    }
}

// ---------------------------------------------------------------------------
// EX-14 — additive SUM over CDC with retractions (`07-example-catalogue.md`
// EX-14; candidate probe cell). Catalogue hypothesis: "UNSUPPORTED-TODAY;
// probe what `change_feed` does today (likely re-scan or refusal)".
//
// `change_feed` maps to the stricter `MutationProfile::MutableSnapshot`
// posture for admission purposes (`incremental_models.md` §Known Divergences,
// `derive.rs::source_shape`'s own doc comment). A `NewData` fold over a
// `MutableSnapshot` source always fails the faithful-fold source-posture
// condition (obligation 2) regardless of the combiner's algebra — SUM is a
// perfectly good monoid, but no un-fold mechanism exists to undo an
// already-folded contribution once a retraction (delete image) arrives, so
// the fold family refuses. Only the whole-table recompute family
// (`Backfill`) remains admissible — this is the honest "recompute only"
// verdict this test pins.
// ---------------------------------------------------------------------------

#[test]
fn ex14_change_feed_sum_recompute_only() {
    let sql = "SELECT user_id, SUM(amount) AS lifetime_spend FROM smelt.sources.ledger_cdc \
               GROUP BY user_id";
    let sources = vec![SourceFacts {
        name: "ledger_cdc".to_string(),
        // Represents a `change_feed` source's admission-time posture
        // (stricter than append-only — see module doc above).
        mutation: MutationProfile::MutableSnapshot,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources,
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["ledger_cdc"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let fold_plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "ledger_cdc".to_string(),
        }],
    );
    assert!(
        fold_plan.cells.is_empty(),
        "no fold cell should be admitted over a change-feed (retracting) source: {:?}",
        fold_plan.cells
    );
    // The repair narrowing also attempts a per-group recompute over the
    // posture failure; `ledger_cdc` declares no `unique_key`, so affected-key
    // discovery fails closed too, pushing an additive `RepairKeysNotDiscoverable`
    // refusal alongside the pre-existing one (`incremental_models.md`
    // §"The repair family" — fail-closed refusal is additive, never a
    // replacement).
    assert_eq!(fold_plan.refusals.len(), 2, "{:?}", fold_plan.refusals);
    assert!(fold_plan.refusals.iter().any(|r| matches!(
        r,
        Refusal::NoAdmissibleTechnique { why, .. } if why.contains("faithful-fold source-posture")
    )));
    assert!(fold_plan
        .refusals
        .iter()
        .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { .. })));

    // The recompute family (Backfill) is unaffected — a keyed-grain backfill
    // is a whole-table rebuild, always admissible.
    let backfill_plan = derive_maintenance_plan(&inputs, &[Trigger::Backfill]);
    assert!(backfill_plan.refusals.is_empty());
    assert_eq!(backfill_plan.cells[0].technique, Technique::DeleteInsert);
}

// ---------------------------------------------------------------------------
// EX-18 — GROUP BY week over day partitions (`07-example-catalogue.md`
// EX-18; candidate probe cell). Catalogue hypothesis: "HOLDS iff write
// window rounds up; sharp footprint-map probe".
//
// This test pins ONLY the write-window-rounds-up *precondition* — today's
// declared-granularity check (`granularity.rs::check_declared_granularity`,
// MP14): declaring `timeseries.granularity: week` while the model's own
// `partition_column` projection only actually truncates to `day` is a safe
// WIDEN (coarser than or equal to the derived unit) — never flagged,
// because the graph layer schedules at a grid no finer than the data
// supports (widen-never-narrow, P3). This is the mechanism that makes "the
// write window rounds up to the week boundary" true by construction rather
// than by convention.
//
// It does NOT run the recompute technique or prove multiset-equivalence
// against a full refresh — that equivalence leg (the catalogue's actual
// "HOLDS" verdict) lives in
// `maintenance_plan_conformance.rs::described_technique_matches_execution_ex18_group_by_coarser_write_window`,
// which derives the plan, emits `DeleteInsert` over the week-rounded
// region, and asserts it against a real DuckDB. The `CLAIMED` entries for
// this matrix cell point at that test, not this one.
// ---------------------------------------------------------------------------

#[test]
fn ex18_group_by_coarser_write_window_rounds_up() {
    let sql = "SELECT date_trunc('week', order_ts) AS order_week, SUM(amount) AS total \
               FROM smelt.sources.orders GROUP BY 1";
    // Declaring the coarser `week` grain the construct actually groups by
    // is a safe widen — HOLDS, no mismatch.
    assert!(check_declared_granularity(sql, "order_week", Granularity::Week).is_none());

    // The construct's OWN daily-partitioned upstream still shows up if a
    // caller mistakenly declares something FINER than the model's actual
    // weekly grouping (e.g. hour) — refused as a narrowing hazard, not
    // silently accepted.
    let mismatch = check_declared_granularity(sql, "order_week", Granularity::Day)
        .expect("declaring day while the model only truncates to week is a narrowing error");
    assert_eq!(mismatch.declared, Granularity::Day);
    assert_eq!(mismatch.actual, Granularity::Week);
}

// ---------------------------------------------------------------------------
// EX-26 — MAX_BY latest-status over a change feed (`07-example-catalogue.md`
// EX-26; candidate probe cell). Catalogue hypothesis: "HOLDS" under the
// proposed framework's order-monotone overwrite fold — but that fold
// mechanism does not exist today (`incremental_models.md` §Known Divergences:
// "no live fold machinery consumes a change feed's delta shape yet"). Same
// source-posture refusal as EX-14 applies regardless of combiner algebra:
// only the recompute family is reachable — this test pins that "recompute
// only" is today's honest verdict, not the catalogue's aspirational fold.
// ---------------------------------------------------------------------------

#[test]
fn ex26_change_feed_latest_writer_recompute_only() {
    let sql = "SELECT order_id, MAX(status_ts) AS status FROM smelt.sources.status_cdc \
               GROUP BY order_id";
    let sources = vec![SourceFacts {
        name: "status_cdc".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: Some("status_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "order_status".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["order_id"]),
            },
            skeleton_columns: set(&["order_id"]),
        },
        sources,
        column_groups: vec![ColumnGroup {
            columns: strings(&["status"]),
            mutation_sensitivity: set(&["status_cdc"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        // MAX is order-monotone in principle, but the source-posture
        // condition is checked FIRST and independently (obligation 2) —
        // it refuses before combiner algebra (obligation 3) is even
        // consulted, so the choice of monoid combiner here doesn't matter.
        fold: Some(FoldSpec {
            add_columns: vec![("status".to_string(), SqlFunction::Max)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let fold_plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "status_cdc".to_string(),
        }],
    );
    assert!(fold_plan.cells.is_empty());
    // Additive repair refusal, same rationale as EX-14 above: `status_cdc`
    // declares no `unique_key`, so the repair narrowing's own affected-key
    // discovery fails closed too.
    assert_eq!(fold_plan.refusals.len(), 2, "{:?}", fold_plan.refusals);
    assert!(fold_plan.refusals.iter().any(|r| matches!(
        r,
        Refusal::NoAdmissibleTechnique { why, .. } if why.contains("faithful-fold source-posture")
    )));
    assert!(fold_plan
        .refusals
        .iter()
        .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { .. })));

    let backfill_plan = derive_maintenance_plan(&inputs, &[Trigger::Backfill]);
    assert!(backfill_plan.refusals.is_empty());
    assert_eq!(backfill_plan.cells[0].technique, Technique::DeleteInsert);
}

// ---------------------------------------------------------------------------
// EX-27 — keyed collapse dedupe / dedup-to-latest (`07-example-catalogue.md`
// EX-27; the ROW_NUMBER-dedup and dedup-to-latest matrix rows' single named
// example). Catalogue hypothesis: "HOLDS once locality gate lands; today
// refuses".
//
// No `FoldSpec` exists that expresses "keep the latest row per key" for a
// `ROW_NUMBER() OVER (PARTITION BY key ORDER BY ts DESC) = 1` dedup — there
// is no locality-pruned windowed-merge technique in this v0 tracer at all
// (`incremental_models.md` §Known Divergences: "keyed dirt-sets ... designed
// ... and unbuilt"). A caller for this construct has nothing honest to
// supply for `fold`, so the keyed-grain creation trigger refuses outright.
// ---------------------------------------------------------------------------

#[test]
fn ex27_row_number_dedup_refuses_today() {
    let inputs = ModelInputs {
        sql: "SELECT user_id, event_id, event_ts FROM ( \
                SELECT user_id, event_id, event_ts, \
                       ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_ts DESC) AS rn \
                FROM smelt.sources.events_redelivered) t WHERE rn = 1",
        output: OutputSpec {
            table: "latest_event_per_user".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![SourceFacts {
            name: "events_redelivered".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["event_id", "event_ts"]),
            mutation_sensitivity: set(&["events_redelivered"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        // No windowed-merge/locality-pruned dedup fold exists — nothing
        // honest to supply.
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "events_redelivered".to_string(),
        }],
    );
    assert!(plan.cells.is_empty());
    assert_eq!(plan.refusals.len(), 1);
    assert!(matches!(
        &plan.refusals[0],
        Refusal::NoAdmissibleTechnique { why, .. } if why.contains("no fold specification")
    ));
}

// ---------------------------------------------------------------------------
// EX-35 — correlated MIN_BY first-value pick (`07-example-catalogue.md`
// EX-35; candidate probe cell, the EX-01/EX-11 three-way ledger-grade
// contrast). Original catalogue hypothesis (pre-`docs/plans/
// 20260809-keyed-frontier.md` Phase 1): "UNSUPPORTED-TODAY; recompute arm
// HOLDS (EX-01 analogue, order-sensitive combiner)".
//
// Phase 1 lands the order-monotone overwrite family (`MAX_BY`/`MIN_BY` —
// `ArgMax`/`ArgMin`, `Monotone::Order`, `analysis/discriminants.rs`):
// `faithful_fold`'s obligation-3 combiner-algebra condition now admits it
// alongside `is_monoid` monoids — a semilattice fold is well-defined under
// the same sequential-application discipline the window-forward driver
// already enforces (`incremental_shapes.md` §"The two run shapes"). The
// catalogue verdict updates: EX-35 now HOLDS (fold cell admitted), not
// recompute-only.
// ---------------------------------------------------------------------------

#[test]
fn ex35_correlated_first_value_fold_admitted() {
    let sql = "SELECT user_id, ARG_MAX(event_ts, event_ts) AS first_seen \
               FROM smelt.sources.events GROUP BY user_id";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "first_seen_per_user".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![SourceFacts {
            name: "events".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["first_seen"]),
            mutation_sensitivity: set(&["events"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("first_seen".to_string(), SqlFunction::ArgMax)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let fold_plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "events".to_string(),
        }],
    );
    assert!(
        fold_plan.refusals.is_empty(),
        "expected the order-monotone combiner to admit: {:?}",
        fold_plan.refusals
    );
    assert_eq!(fold_plan.cells[0].technique, Technique::KeyedFold);

    let backfill_plan = derive_maintenance_plan(&inputs, &[Trigger::Backfill]);
    assert!(backfill_plan.refusals.is_empty());
    assert_eq!(backfill_plan.cells[0].technique, Technique::DeleteInsert);
}

// ---------------------------------------------------------------------------
// Phase 28b fixture pin (`docs/outcomes/20260815-definition-delta-migrate`):
// the group-merge-provenance rule (`incremental_models.md` §"The plan
// matrix") driven through the REAL grouping derivation
// (`grouping::derive_column_groups`), not a hand-built `ColumnGroup` —
// `maintenance_merged_group.rs` already pins the guard's own logic
// (`derive::derive_mutation`'s mutation-capable-input count) directly
// against hand-built `ModelInputs`; this fixture instead confirms the two
// real-world provenance derivations that actually feed it (per-column
// mutation-sensitivity AND JOIN-admission membership-sensitivity) land on
// the SAME merged group for a two-mutable-dimension enrichment model, and
// that the derived plan reports region recompute for it — never a
// column-scoped merge.
// ---------------------------------------------------------------------------

#[test]
fn merged_group_fixture_plans_region_recompute() {
    let sql = "SELECT e.event_id, e.d1_id, e.d2_id, \
               COALESCE(dim1.value, 0) + COALESCE(dim2.value, 0) AS combined \
               FROM smelt.sources.events e \
               LEFT JOIN smelt.sources.dim1 dim1 ON e.d1_id = dim1.id \
               LEFT JOIN smelt.sources.dim2 dim2 ON e.d2_id = dim2.id";
    let sources = vec![
        SourceFacts {
            name: "events".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        },
        SourceFacts {
            name: "dim1".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: strings(&["id"]),
            allow_full_scan: true,
        },
        SourceFacts {
            name: "dim2".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: strings(&["id"]),
            allow_full_scan: true,
        },
    ];
    let skeleton = set(&["event_id", "d1_id", "d2_id"]);
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "degenerate: {:?}",
        grouping.degenerate
    );
    assert_eq!(grouping.groups.len(), 1, "one merged group: {{combined}}");
    let merged = &grouping.groups[0];
    assert_eq!(merged.columns, vec!["combined".to_string()]);
    assert_eq!(
        merged.mutation_sensitivity,
        set(&["dim1", "dim2"]),
        "the merged group's value provenance spans both mutable dimensions"
    );

    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "enriched_events".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["event_id"]),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    for source in ["dim1", "dim2"] {
        let plan = derive_maintenance_plan(
            &inputs,
            &[Trigger::UpstreamMutation {
                source: source.to_string(),
            }],
        );
        assert!(
            plan.refusals.is_empty(),
            "mutation trigger for {source} refused unexpectedly: {:?}",
            plan.refusals
        );
        assert_eq!(plan.cells.len(), 1);
        assert_eq!(plan.cells[0].group, "{combined}");
        assert_eq!(plan.cells[0].corner, Corner::RecomputeRegion);
        assert_eq!(
            plan.cells[0].technique,
            Technique::DeleteInsert,
            "merged group must never take ColumnScopedMerge, got {:?}",
            plan.cells[0]
        );
    }
}
