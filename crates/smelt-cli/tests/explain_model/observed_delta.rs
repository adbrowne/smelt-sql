use std::path::Path;

use smelt_cli::build_maintenance_plan_report;

use crate::support::{build_report_for, synthetic_profile};

// ---------------------------------------------------------------------------
// Observed-delta recording + projection surface
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase D4;
// `docs/specs/incremental_models.md` §"The graph layer" — "Observed deltas
// on model edges", §"What the composed shape uniquely enables" — "Exact
// key→partition dirt projection").
//
// `examples/timeseries/models/daily_events_enriched.sql` USED to be the
// real fixture exercising a `Technique::ColumnScopedMerge` cell (a
// single-input mutable dimension enrichment) — the only technique family D2
// wired observed-delta recording for. As of `docs/plans/
// 20260808-membership-sensitivity.md` Phase 1, `raw.users` being read in
// that model's `JOIN`'s own `ON` predicate makes it membership-sensitive
// instead (`Technique::DeleteInsert`), so NO fixture in this workspace
// reaches `ColumnScopedMerge` anymore (Phase 2's own reachability verdict);
// the recording-status test below is now built over a synthetic
// `MaintenancePlan`, not a real fixture — see its own doc comment. The
// projection-form assertion still uses real fixtures (route 1's
// `user_daily_spend`, route 3's `silver.events_deduped`), unaffected by
// this change.
///
/// **Post-`docs/plans/20260808-membership-sensitivity.md` Phase 3 note:**
/// `daily_events_enriched` (the real fixture) no longer derives ANY
/// `Technique::ColumnScopedMerge` cell at all — `raw.users` is read in the
/// enrichment `JOIN`'s own `ON` predicate, a row-admission read, which
/// makes EVERY column group's cell for that trigger membership-sensitive
/// (`Technique::DeleteInsert`), never `ColumnScopedMerge` (Phase 1's review
/// checklist: "membership cells cannot receive ColumnScopedMerge"). Per
/// Phase 2's own reachability verdict, no fixture in this workspace reaches
/// `ColumnScopedMerge` anymore — so this test (which exists to check the
/// EXPLAIN PRINTING logic for a `ColumnScopedMerge` cell with `WholeRow` row
/// identity, `crates/smelt-cli/src/explain.rs` lines ~353-364) is rewritten
/// to build its `MaintenancePlan` synthetically, mirroring
/// `write_variant_explain_surface`'s own pattern below — the printing logic
/// is independent of whether real SQL derivation can currently produce this
/// shape, and constructing a fictitious SQL fixture to keep the technique
/// artificially reachable would misrepresent what the derivation actually
/// admits today.
#[test]
fn explain_prints_observed_delta_recording_status_for_a_conditional_cell() {
    use std::collections::BTreeSet;

    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::maintenance::{
        ColumnGroup, Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity,
        RowIdentityVerdict, Technique, Trigger,
    };

    let cell = PlanCell {
        group: "{user_name}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![cell],
            refusals: vec![],
            key_locality: None,
        },
        column_groups: vec![ColumnGroup {
            columns: vec!["user_name".to_string()],
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability: vec![],
        succession_advisories: vec![],
        succession_recipe: None,
    };
    let __profile = synthetic_profile(&result, "daily_events_enriched");
    let report = build_maintenance_plan_report(
        "daily_events_enriched",
        &result,
        &RelationContractView::from_facts(None, None),
        &[],
        &[],
        None,
        None,
        &[],
        &[],
        smelt_core::config::ProbeCadence::PerRun,
        &[],
        None,
        None,
        &__profile,
    )
    .expect("build_maintenance_plan_report");

    assert_eq!(
        report
            .matches("observed-delta recording: yes (change-suppressed column-scoped MERGE)")
            .count(),
        0,
        "a ColumnScopedMerge cell with WholeRow row identity must never claim recording: yes: \
         {report}"
    );
    assert_eq!(
        report.matches("observed-delta recording: no").count(),
        1,
        "expected the negative recording row exactly once, on the model's one \
         ColumnScopedMerge cell: {report}"
    );
}

/// A `ColumnScopedMerge` cell whose P2 row identity resolves `WholeRow`
/// must print "no" for observed-delta recording, never "yes" — a
/// `WholeRow` cell has no per-row join identity to compare on, so
/// `choice::resolve_write_suppression` always fail-closes to
/// `Unconditional` for it (`crates/smelt-logical/src/maintenance/
/// choice.rs`'s `whole_row_identity_refuses_regardless_of_comparability`
/// unit test covers the same fail-closed rule at the derivation layer; this
/// covers the `smelt explain` reporting surface). The plan carries a
/// SIBLING `Technique::DeleteInsert` cell alongside the `ColumnScopedMerge`
/// cell under test, proving the "no" line is correctly isolated to the
/// `ColumnScopedMerge` cell's own block and never leaks onto (or is
/// swallowed by) an unrelated sibling cell's report lines.
///
/// **Post-`docs/plans/20260808-membership-sensitivity.md` Phase 3 note:**
/// originally built over a real fact+dimension enrichment fixture (mirroring
/// `examples/timeseries/models/daily_events_enriched.sql`); rewritten to a
/// synthetic `MaintenancePlan` for the same reason as
/// `explain_prints_observed_delta_recording_status_for_a_conditional_cell`
/// above — no fixture in this workspace derives a `ColumnScopedMerge` cell
/// anymore (Phase 1's membership-sensitivity derivation), so the EXPLAIN
/// PRINTING logic under test needs a hand-built plan to reach it at all.
#[test]
fn explain_prints_no_recording_for_a_whole_row_identity_conditional_cell() {
    use std::collections::BTreeSet;

    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::maintenance::{
        ColumnGroup, Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity,
        RowIdentityVerdict, Technique, Trigger,
    };

    let merge_cell = PlanCell {
        group: "{user_name}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "users".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let sibling_cell = PlanCell {
        group: "{event_type, user_id}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "users".to_string(),
        },
        corner: Corner::RecomputeRegion,
        technique: Technique::DeleteInsert,
        partition_local: PartitionLocal::No {
            source: "users".to_string(),
            why: "unclocked source is read in full on every recompute".to_string(),
        },
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![merge_cell, sibling_cell],
            refusals: vec![],
            key_locality: None,
        },
        column_groups: vec![
            ColumnGroup {
                columns: vec!["user_name".to_string()],
                mutation_sensitivity: Default::default(),
                membership_sensitivity: BTreeSet::new(),
            },
            ColumnGroup {
                columns: vec!["event_type".to_string(), "user_id".to_string()],
                mutation_sensitivity: Default::default(),
                membership_sensitivity: BTreeSet::new(),
            },
        ],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability: vec![],
        succession_advisories: vec![],
        succession_recipe: None,
    };
    let __profile = synthetic_profile(&result, "events_enriched");
    let report = build_maintenance_plan_report(
        "events_enriched",
        &result,
        &RelationContractView::from_facts(None, None),
        &[],
        &[],
        None,
        None,
        &[],
        &[],
        smelt_core::config::ProbeCadence::PerRun,
        &[],
        None,
        None,
        &__profile,
    )
    .expect("build_maintenance_plan_report");

    assert!(
        report.contains("region key: WholeRow"),
        "fixture must actually exercise the WholeRow identity case: {report}"
    );
    // Each cell's block starts at its own "  - group ..." header line; split
    // on that marker to isolate the ColumnScopedMerge cell's own lines from
    // the sibling DeleteInsert cell.
    let cell_block = report
        .split("  - group ")
        .find(|block| block.contains("technique: ColumnScopedMerge"))
        .expect("expected the admitted ColumnScopedMerge cell");
    assert!(
        cell_block.contains("observed-delta recording: no"),
        "a WholeRow-identity ColumnScopedMerge cell must never claim recording: yes: {cell_block}\n\nfull report:\n{report}"
    );
    assert!(
        !cell_block.contains("observed-delta recording: yes"),
        "a WholeRow-identity ColumnScopedMerge cell must never claim recording: yes: {cell_block}"
    );
    let sibling_block = report
        .split("  - group ")
        .find(|block| block.contains("technique: DeleteInsert"))
        .expect("expected the sibling DeleteInsert cell");
    assert!(
        !sibling_block.contains("observed-delta recording"),
        "a DeleteInsert cell must never print an observed-delta recording line at all — that \
         reporting family is wired only for ColumnScopedMerge: {sibling_block}"
    );
}
