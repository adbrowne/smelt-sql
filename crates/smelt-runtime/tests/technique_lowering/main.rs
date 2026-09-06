//! MP11 (`docs/plans/20260707-maintenance-plan-impl.md` "First targeted-write
//! cell — column-scoped merge behind admission (M5)"): plan-driven technique
//! lowering. `maintenance_driver::resolve_cell_technique` is the one place
//! that turns an admitted `MaintenancePlan` cell + operator override
//! (`maintenance.cells[].technique`) + backend capability into an
//! executable choice — it never re-derives admission itself
//! (`docs/specs/architecture.md` §"Maintenance-plan purity"). This suite
//! asserts:
//! - a cell the plan did not admit never lowers to a targeted write, pinned
//!   or not (`unadmitted_cell_never_lowers_targeted_write`);
//! - a capability gap on the backend behaves identically to a plan-level
//!   refusal — dropped from admission at plan time, never a runtime
//!   surprise;
//! - an admitted, runnable cell resolves to `ColumnScopedMerge` and
//!   `execute_column_scoped_merge` actually performs the targeted `MERGE`
//!   against a real DuckDB backend, matching a hand-written full-refresh
//!   oracle over the fact+dimension enrichment shape (`G-05`/EX-13/EX-24
//!   family, `smelt-maintenance-testkit`'s `join_enrichment_mutable_dimension`).
//!
//! The `column_scoped_merge_e2e` module below is the phase's real-fixture
//! requirement: it drives the SAME shape through `execute_project` (the
//! sanctioned single run entrypoint, root `CLAUDE.md` §"Run pipeline parity
//! rule") — `crate::maintenance_driver::resolve_live_column_scoped_cell` +
//! `execute_column_scoped_merge_full`, wired into `execute.rs`'s regular
//! incremental batch-execution branch — never a direct unit call to
//! `resolve_cell_technique`/`execute_column_scoped_merge` as above.
//!
//! The `write_pattern_registry_pin` module (bottom of this file,
//! `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase R1)
//! adds the open write-pattern registry's own end-to-end leg: a valid
//! `maintenance.cells[].write` pin selects among admissible mechanisms and
//! actually lowers to the pinned addressing against a real DuckDB backend
//! (`pinning_region_on_backfill_cell_yields_delete_insert`). Every
//! `Backend::capabilities().supports_column_scoped_merge` call site in this
//! file (above and below) is proof by construction that the old
//! `Backend::supports_column_scoped_merge()` trait method no longer exists —
//! `crates/smelt-backend/src/lib.rs` deleted it, so a call site written the
//! old way would not compile; the whole workspace's clean build after that
//! deletion is the compile-time assertion the phase's TDD list asks for.

use std::path::Path;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::CellTechnique;
use smelt_logical::analysis::join_shape::ContributionVerdict;
use smelt_logical::analysis::source_bounds::{BoundResult, Seconds};
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::{
    Corner, MaintenancePlan, PartitionLocal, PlanCell, Refusal, RowIdentity, RowIdentityVerdict,
    ScanClamp, Technique, Trigger,
};
use smelt_runtime::maintenance_driver::{
    decide_column_merge_dispatch, execute_column_scoped_merge, execute_column_scoped_merge_full,
    resolve_cell_technique, resolve_cell_technique_with_write_pin, widen_horizon_for_batch,
    ColumnMergeDispatch, ResolvedTechnique,
};

/// A retry policy that never retries — these tests exercise the
/// column-scoped MERGE write directly against a real DuckDB backend,
/// outside `execute_project`, so there is no `ExecuteRequest`/run reporter
/// to derive one from (`docs/plans/20260719-prod-w2-operability.md` Phase
/// 6).
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "technique-lowering-test",
        model_name: "technique-lowering-test",
        reporter: &NO_OP_REPORTER,
    }
}

/// These two physical-mechanism tests below exercise `execute_column_scoped_
/// merge` directly (not through the derived plan's own `WriteSuppression`
/// resolution, `maintenance_driver::resolve_live_column_scoped_cell`'s job)
/// — they always pass the unconditional variant so the pre-Phase-C4
/// `UPDATE SET *` behaviour they assert on is unchanged by C4's suppression
/// machinery.
fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "test exercises the physical mechanism directly, not suppression admission"
            .to_string(),
    }
}

/// A run-window identity for `execute_column_scoped_merge`/`_full`'s
/// observed-delta record (T5) — these dimension-shape fixtures have no
/// partition axis, so `column` is empty (the record's `partitions` array is
/// always empty for this shape).
fn test_window() -> smelt_backend::PartitionRange {
    smelt_backend::PartitionRange {
        column: String::new(),
        start: "2024-01-01".to_string(),
        end: "2024-01-02".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    }
}

/// A plan whose only cell is an admitted `ColumnScopedMerge` for `source`'s
/// mutation trigger over the `{tier}` column group — the enrichment shape's
/// live cell.
fn admitted_plan(source: &str) -> MaintenancePlan {
    MaintenancePlan {
        cells: vec![PlanCell {
            group: "{tier}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: source.to_string(),
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
            fingerprint_projections: std::collections::BTreeMap::new(),
            key_scope: None,
            state_downgrade: None,
        }],
        refusals: vec![],
        key_locality: None,
    }
}

/// A plan that refused `source`'s mutation trigger entirely (bounded-scan
/// admission failed) — no cell exists for the trigger at all.
fn refused_plan(source: &str) -> MaintenancePlan {
    MaintenancePlan {
        cells: vec![],
        refusals: vec![Refusal::ScanUnbounded {
            source: source.to_string(),
            why: "derived scan is unbounded".to_string(),
        }],
        key_locality: None,
    }
}

mod basic;
/// MP11's real end-to-end proof: drive `examples/timeseries/models/
/// daily_events_enriched.sql` through `execute_project` itself — never a
/// direct call to `resolve_cell_technique`/`execute_column_scoped_merge` —
/// and observe the regular incremental execution loop
/// (`crates/smelt-runtime/src/execute.rs`) dispatch to a column-scoped
/// `MERGE` when a dimension mutation makes the `Trigger::UpstreamMutation`
/// cell live.
mod column_scoped_merge_e2e;
mod external_source_point_lookup_recompute;
mod fixture_based;
mod in_place_update_lowering;
mod keyed_membership_recompute_e2e;
mod write_pattern_registry_pin;
