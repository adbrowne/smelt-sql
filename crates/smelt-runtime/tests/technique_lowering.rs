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

#[test]
fn unadmitted_cell_never_lowers_targeted_write() {
    let plan = refused_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // Pinning `rederive_columns` names a cell the plan never admitted — a
    // pin bypasses the cost model, never admission
    // (`incremental_models.md` §"Per-cell admission"). This must refuse at
    // plan-resolution time, not silently execute a targeted write with an
    // unbounded footprint.
    let err = resolve_cell_technique(&plan, &trigger, Some(CellTechnique::RederiveColumns), true)
        .expect_err("a pin naming an unadmitted cell must refuse, never lower a targeted write");
    assert!(
        err.to_string().contains("MaintenanceUnboundedFootprint"),
        "refusal must name the diagnostic: {err}"
    );

    // Without a pin, the safe default is region-recompute — no error, and
    // critically NOT `ColumnScopedMerge` (there is no runtime fallback that
    // fabricates a targeted write the plan never admitted).
    let resolved = resolve_cell_technique(&plan, &trigger, None, true)
        .expect("no pin + unadmitted cell must fall back safely, not error");
    assert_eq!(resolved, ResolvedTechnique::RegionRecompute);
}

#[test]
fn backend_capability_gap_is_indistinguishable_from_plan_refusal() {
    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // The plan admits the cell, but the backend cannot execute a
    // column-scoped MERGE at all: this must behave exactly like an
    // unadmitted cell, never a runtime surprise after the plan already
    // chose the technique.
    let resolved = resolve_cell_technique(&plan, &trigger, None, false)
        .expect("capability gap without a pin falls back safely");
    assert_eq!(resolved, ResolvedTechnique::RegionRecompute);

    let err = resolve_cell_technique(&plan, &trigger, Some(CellTechnique::RederiveColumns), false)
        .expect_err("a pin naming a capability-gapped backend must refuse, not silently downgrade");
    assert!(
        err.to_string().contains("MaintenanceUnboundedFootprint"),
        "refusal must name the diagnostic: {err}"
    );
}

#[test]
fn admitted_cell_on_capable_backend_resolves_column_scoped_merge() {
    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let resolved = resolve_cell_technique(&plan, &trigger, None, true)
        .expect("admitted cell + capable backend resolves cleanly");
    assert_eq!(resolved, ResolvedTechnique::ColumnScopedMerge);

    // The hard pin agrees — pinning a technique the plan DID admit is not
    // an override, just an explicit restatement of what the cost model
    // would already have chosen.
    let pinned =
        resolve_cell_technique(&plan, &trigger, Some(CellTechnique::RederiveColumns), true)
            .expect("pinning an admitted, runnable technique must succeed");
    assert_eq!(pinned, ResolvedTechnique::ColumnScopedMerge);
}

/// End-to-end: a fact table `events_enriched(d, user_id, val, tier)` is
/// enriched from a mutable dimension `users(user_id, tier)`. A dimension
/// mutation is column-scoped MERGEd into just the `{tier}` group via
/// `execute_column_scoped_merge`, and the result matches a full-refresh
/// oracle re-joining the CURRENT dimension contents — the G-05/EX-13/EX-24
/// enrichment shape's equivalence leg (`smelt-maintenance-testkit`'s
/// `join_enrichment_mutable_dimension`), exercised directly against
/// `maintenance_driver`'s new physical primitive rather than through
/// `execute_project` (forward propagation — deciding *when* a dimension
/// mutation should trigger this cell — is MP15's job; this phase makes the
/// mechanism itself live and callable).
#[tokio::test]
async fn column_scoped_merge_matches_full_refresh_after_dimension_mutation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched (d DATE, user_id BIGINT, val DOUBLE, tier VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched VALUES \
             (DATE '2024-01-01', 1, 10.0, 'bronze'), \
             (DATE '2024-01-01', 2, 20.0, 'silver')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
        .await
        .expect("seed dim table (user_id=1 mutated bronze -> gold)");

    assert!(
        backend.capabilities().supports_column_scoped_merge,
        "DuckDB backend must advertise column-scoped MERGE capability"
    );

    // The re-derivation batch for the `{tier}` group: DuckDB's `merge_into`
    // issues `UPDATE SET *`, which requires the source projection to carry
    // the FULL target row (a column-count mismatch is a hard backend
    // error, not a silent by-name subset) — so `val` passes through
    // unchanged from the existing target row (`e.val`) while `tier`
    // re-derives from the CURRENT dimension contents (`u.tier`). This is
    // what keeps the merge column-scoped in *effect*: only `tier`'s value
    // actually changes. The horizon clamp reuses the target's own `d`
    // column as the conversion-timestamp axis.
    let dimension_batch_sql = "SELECT e.d, e.user_id, e.val, u.tier \
         FROM main.events_enriched e JOIN main.sources_users u ON e.user_id = u.user_id";

    let contribution = ContributionVerdict::Monotone;
    let bound = BoundResult::Bounded {
        source_partition_col: "d".to_string(),
        before: Seconds::ZERO,
        after: Seconds::hours(24),
    };

    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let resolved = resolve_cell_technique(
        &plan,
        &trigger,
        None,
        backend.capabilities().supports_column_scoped_merge,
    )
    .expect("admitted cell resolves");
    assert_eq!(resolved, ResolvedTechnique::ColumnScopedMerge);

    execute_column_scoped_merge(
        &backend,
        "main",
        "events_enriched",
        &["user_id".to_string()],
        &contribution,
        &bound,
        "d",
        "2024-01-01 12:00:00",
        dimension_batch_sql,
        &unconditional(),
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("column-scoped merge must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");

    let maintained_tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained tier for user 1");
    assert_eq!(
        maintained_tier_1, "gold",
        "column-scoped MERGE must pick up the mutated dimension value"
    );

    // `val` must be untouched — the merge is scoped to `{tier}` IN EFFECT,
    // even though the physical `merge_into` primitive issues an
    // `UPDATE SET *` over the full row: `dimension_batch_sql` carries `val`
    // through unchanged from the existing target row, so its value is a
    // no-op write, while `tier` re-derives from the CURRENT dimension.
    let val_1: f64 = conn
        .query_row(
            "SELECT val FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read untouched val for user 1");
    assert_eq!(
        val_1, 10.0,
        "column-scoped MERGE must not touch columns outside its group"
    );

    // Full-refresh oracle: re-join the CURRENT dimension contents from
    // scratch — no smelt compilation, no derived filter.
    let full_refresh_tier_1: String = conn
        .query_row(
            "SELECT u.tier FROM main.sources_users u WHERE u.user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("oracle read");
    assert_eq!(
        maintained_tier_1, full_refresh_tier_1,
        "column-scoped MERGE result must match the full-refresh oracle"
    );

    let unaffected_tier_2: String = conn
        .query_row(
            "SELECT tier FROM main.events_enriched WHERE user_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("read unaffected row");
    assert_eq!(
        unaffected_tier_2, "silver",
        "an unmutated dimension row's enrichment must be unchanged (still matches its own \
         full-refresh oracle value)"
    );
}

/// The `PartitionLocal::Yes` corner's physical mechanism, end-to-end against
/// a real DuckDB backend: `decide_column_merge_dispatch` selects
/// `ColumnMergeDispatch::Clamped`, `widen_horizon_for_batch` derives the
/// horizon, and `execute_column_scoped_merge`/`dimension_horizon_merge`
/// (F15) actually clamp the MERGE to `[conv_ts − H, conv_ts]` on the
/// target's own partition axis — distinct from
/// `execute_column_scoped_merge_full`'s unconditional full merge above: a
/// mutated dimension row OUTSIDE the horizon must be left untouched this
/// run (the horizon settled-delay/tail-rewrite mechanism that would later
/// catch it up is unbuilt — `model_transforms.md`'s transform catalogue).
#[tokio::test]
async fn yes_corner_clamps_the_merge_to_the_horizon_and_leaves_the_rest_untouched() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched (d DATE, user_id BIGINT, val DOUBLE, status VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched VALUES \
             (DATE '2024-01-01', 1, 10.0, 'active'), \
             (DATE '2024-01-03', 2, 20.0, 'active')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_user_status (user_id BIGINT, status VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_user_status VALUES (1, 'suspended'), (2, 'suspended')",
        )
        .await
        .expect("seed dim table (both users mutated)");

    let cell = PlanCell {
        group: "{status}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "user_status".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![ScanClamp {
            source: "user_status".to_string(),
            column: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        }],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
    };

    let dispatch = decide_column_merge_dispatch(
        &cell,
        "user_status",
        /* table_exists */ true,
        /* model_declares_unique_key */ true,
        &ContributionVerdict::Monotone,
    )
    .expect("a PartitionLocal::Yes cell with a matching scan must dispatch Clamped");
    let ColumnMergeDispatch::Clamped(scan) = dispatch else {
        panic!("expected ColumnMergeDispatch::Clamped, got {dispatch:?}");
    };

    // A 1-day batch — the widened horizon must equal the derived 24h margin
    // (neither narrows the other here).
    let bound = widen_horizon_for_batch(&scan, Seconds::days(1));
    assert_eq!(
        bound,
        BoundResult::Bounded {
            source_partition_col: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        }
    );

    let dimension_batch_sql = "SELECT e.d, e.user_id, e.val, s.status \
         FROM main.events_enriched e JOIN main.sources_user_status s ON e.user_id = s.user_id";

    execute_column_scoped_merge(
        &backend,
        "main",
        "events_enriched",
        &["user_id".to_string()],
        &ContributionVerdict::Monotone,
        &bound,
        "d",
        "2024-01-01 00:00:00",
        dimension_batch_sql,
        &unconditional(),
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("horizon-clamped column-scoped merge must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");

    let within_horizon: String = conn
        .query_row(
            "SELECT status FROM main.events_enriched WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained status for user 1");
    assert_eq!(
        within_horizon, "suspended",
        "d=2024-01-01 is within [conv_ts - 24h, conv_ts] — the mutation must be picked up"
    );

    let outside_horizon: String = conn
        .query_row(
            "SELECT status FROM main.events_enriched WHERE user_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("read status for user 2");
    assert_eq!(
        outside_horizon, "active",
        "d=2024-01-03 falls outside [conv_ts - 24h, conv_ts] — the horizon clamp must leave it \
         untouched this run, unlike execute_column_scoped_merge_full's unconditional full merge"
    );
}

/// Phase C4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the change-suppressed column-scoped MERGE (T1), real-DuckDB proof: an
/// unchanged-input re-run writes **zero** rows. `resolve_write_suppression`
/// is fed a proven `Key` row identity (P2) and an all-`Comparable` P3
/// verdict for the `{tier}` group — the admission this phase adds — so it
/// resolves `WriteSuppression::Suppressed`, and `execute_column_scoped_merge_
/// full` dispatches through `emit_column_scoped_merge_suppressed` instead of
/// the plain unconditional emitter used by the tests above.
///
/// The "zero rows written" proof reads DuckDB's own `Count` column off the
/// `MERGE`'s query result (`execute_sql`'s returned `RecordBatch`, captured
/// here directly rather than through `execute_column_scoped_merge_full`'s
/// `ExecutionResult::row_count`, which only reports the target's total row
/// count, not the number of rows the statement itself touched) — the same
/// affected-row semantics DuckDB's own `MERGE`/`UPDATE`/`INSERT` report.
#[tokio::test]
async fn suppressed_merge_writes_zero_rows_on_unchanged_rerun() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    // Dimension starts out identical to the target — the first run below is
    // itself an unchanged-input run.
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed dim table");

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";

    // The admission this phase adds: a proven key over a fully comparable
    // group resolves `Suppressed`, naming exactly the `{tier}` group.
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability = vec![smelt_logical::analysis::walk::ColumnComparability {
        output: "tier".to_string(),
        comparability: smelt_logical::analysis::walk::Comparability::Comparable,
    }];
    let suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
        &["tier".to_string()],
        &comparability,
        &row_identity,
    );
    assert_eq!(
        suppression,
        WriteSuppression::Suppressed {
            compared_columns: vec!["tier".to_string()]
        },
        "a proven key over a fully comparable group must admit the conditional variant"
    );

    // Run 1: dimension unchanged relative to the target — the suppressed
    // MERGE must match every row but write none of them.
    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &suppression,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("suppressed column-scoped merge must succeed");

    let affected = merge_affected_row_count(
        &backend,
        "main.dim_users",
        "main.sources_users",
        &["user_id"],
        &["tier"],
    )
    .await;
    assert_eq!(
        affected, 0,
        "an unchanged-input re-run of the suppressed MERGE must write zero rows"
    );

    // Mutate the dimension for user 1 — now a real change exists.
    backend
        .execute_sql("UPDATE main.sources_users SET tier = 'gold' WHERE user_id = 1")
        .await
        .expect("mutate dimension");

    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &suppression,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("suppressed column-scoped merge must succeed after mutation");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.dim_users WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained tier for user 1");
    assert_eq!(
        tier_1, "gold",
        "the suppressed MERGE must still pick up a genuine change"
    );
    let tier_2: String = conn
        .query_row(
            "SELECT tier FROM main.dim_users WHERE user_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("read unaffected tier for user 2");
    assert_eq!(tier_2, "silver", "an unmutated row must be left untouched");

    // Full-refresh oracle: the maintained state must equal a fresh join,
    // exactly like the unconditional-variant tests above.
    let oracle_tier_1: String = conn
        .query_row(
            "SELECT tier FROM main.sources_users WHERE user_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("oracle read");
    assert_eq!(tier_1, oracle_tier_1);

    // Run 3: dimension unchanged again (relative to the now-mutated state)
    // — zero rows written a second time.
    let affected_again = merge_affected_row_count(
        &backend,
        "main.dim_users",
        "main.sources_users",
        &["user_id"],
        &["tier"],
    )
    .await;
    assert_eq!(
        affected_again, 0,
        "a second unchanged-input re-run must also write zero rows"
    );
}

/// Issue the exact `emit_column_scoped_merge_suppressed`-shaped statement
/// directly and read DuckDB's own affected-row `Count` off the query
/// result — the same probe this test file's e2e proof reads its "zero rows
/// written" assertion from, kept separate from `execute_column_scoped_merge_
/// full` (whose `ExecutionResult::row_count` reports the target's total row
/// count, not the number of rows the statement itself touched).
async fn merge_affected_row_count(
    backend: &DuckDbBackend,
    target: &str,
    source: &str,
    key: &[&str],
    compared: &[&str],
) -> i64 {
    use smelt_logical::maintenance::emit::{
        emit_column_scoped_merge_suppressed, MaintenanceDialect,
    };

    let key_owned: Vec<String> = key.iter().map(|s| s.to_string()).collect();
    let compared_owned: Vec<String> = compared.iter().map(|s| s.to_string()).collect();
    let group = emit_column_scoped_merge_suppressed(
        target,
        &key_owned,
        &format!("SELECT * FROM {source}"),
        &compared_owned,
        MaintenanceDialect::DuckDb,
    );
    let batches = backend
        .execute_sql(&group.statements[0].sql)
        .await
        .expect("probe merge must succeed");
    let batch = batches.first().expect("MERGE returns one Count row");
    let counts = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("Count column is Int64");
    counts.value(0)
}

/// Phase G1 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the conditional-variant dimension enters `choice.rs`'s override ladder:
/// `resolve_write_variant` folds the first-build/definition-change-backfill
/// posture into an already-proven `WriteSuppression::Suppressed` verdict.
/// This is the interchangeability claim the plan asks for, exercised
/// end-to-end against a real DuckDB backend: a steady-state trigger
/// (`Trigger::UpstreamMutation`, no ledger catch-up) *prefers* the
/// change-suppressed matched arm, while a first-build/backfill trigger
/// (`Trigger::Backfill`, or a definition-change cell's own
/// `ledger_catch_up: true`) is *admitted but not preferred* and resolves the
/// unconditional matched arm instead — bit-identical final state either way
/// (`incremental_models.md` §"Interchangeability and choice"), never a
/// difference in which `S` is reflected.
#[tokio::test]
async fn first_build_posture_and_steady_state_preference_resolve_bit_identical_state() {
    use smelt_logical::maintenance::choice::{resolve_write_variant, VariantReason};

    let tmp = tempfile::TempDir::new().expect("tempdir");

    // Two separate real DuckDB databases, seeded identically: a target that
    // already diverges from its dimension source for user 1 (tier
    // 'bronze' → 'gold'), unchanged for user 2.
    async fn seed(path: &Path) -> DuckDbBackend {
        let backend = DuckDbBackend::new(path, "main").await.expect("open duckdb");
        backend
            .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create target table");
        backend
            .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
            .await
            .expect("seed target table");
        backend
            .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create dim table");
        backend
            .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
            .await
            .expect("seed dim table");
        backend
    }

    let steady_db_path = tmp.path().join("steady.duckdb");
    let backfill_db_path = tmp.path().join("backfill.duckdb");
    let steady_backend = seed(&steady_db_path).await;
    let backfill_backend = seed(&backfill_db_path).await;

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability = vec![smelt_logical::analysis::walk::ColumnComparability {
        output: "tier".to_string(),
        comparability: smelt_logical::analysis::walk::Comparability::Comparable,
    }];
    let proven_suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
        &["tier".to_string()],
        &comparability,
        &row_identity,
    );
    assert_eq!(
        proven_suppression,
        WriteSuppression::Suppressed {
            compared_columns: vec!["tier".to_string()]
        },
        "precondition: the compare must actually be admitted for this test to prove anything"
    );

    // Steady-state trigger: no prior-state gap — the ladder PREFERS
    // suppression.
    let steady_trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (steady_variant, steady_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        false,
        &smelt_logical::maintenance::choice::EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(steady_reason, VariantReason::SteadyStatePreference);
    assert!(matches!(
        steady_variant,
        WriteSuppression::Suppressed { .. }
    ));

    // First-build/backfill trigger: admitted, but NOT preferred — resolves
    // unconditional by default even though the same proof holds.
    let (backfill_variant, backfill_reason) = resolve_write_variant(
        &proven_suppression,
        &Trigger::Backfill,
        false,
        &smelt_logical::maintenance::choice::EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(backfill_reason, VariantReason::FirstBuildPosture);
    assert!(matches!(
        backfill_variant,
        WriteSuppression::Unconditional { .. }
    ));

    // A definition-change backfill cell (`ledger_catch_up: true`) resolves
    // the same way even on an otherwise-steady-state trigger.
    let (catch_up_variant, catch_up_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        true,
        &smelt_logical::maintenance::choice::EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(catch_up_reason, VariantReason::FirstBuildPosture);
    assert!(matches!(
        catch_up_variant,
        WriteSuppression::Unconditional { .. }
    ));

    // Execute both resolved variants against their own real, identically
    // seeded target — one via the suppressed matched arm the steady-state
    // trigger prefers, the other via the unconditional matched arm the
    // first-build/backfill posture defaults to.
    execute_column_scoped_merge_full(
        &steady_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &steady_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("steady-state suppressed merge must succeed");

    execute_column_scoped_merge_full(
        &backfill_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &backfill_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("first-build unconditional merge must succeed");

    // Bit-identical final state either way — choice may change which
    // matched-arm shape ran, never observable bits at a fixed processed-
    // input set.
    let steady_conn = duckdb::Connection::open(&steady_db_path).expect("reconnect steady");
    let backfill_conn = duckdb::Connection::open(&backfill_db_path).expect("reconnect backfill");
    for user_id in [1_i64, 2_i64] {
        let steady_tier: String = steady_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read steady tier");
        let backfill_tier: String = backfill_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read backfill tier");
        assert_eq!(
            steady_tier, backfill_tier,
            "user {user_id}: the suppressed (steady-state-preferred) and unconditional \
             (first-build-posture) matched arms must produce bit-identical state"
        );
    }
}

/// Phase G1's pin dimension: a `technique: unconditional` pin forces the
/// plain matched arm on a steady-state trigger that would otherwise prefer
/// suppression, and a `technique: suppress` pin forces the change-suppressed
/// matched arm on for a first-build trigger that would otherwise default to
/// unconditional — exercised end-to-end against a real DuckDB backend,
/// asserting bit-identical final state against the natural (unpinned)
/// resolution either way (`incremental_models.md` §"Interchangeability and
/// choice": a pin may change which matched-arm shape ran, never the bits at
/// a fixed processed-input set).
#[tokio::test]
async fn technique_pin_forces_the_variant_and_still_produces_bit_identical_state() {
    use smelt_core::config::CellTechnique;
    use smelt_logical::maintenance::choice::{
        resolve_write_variant, EffectiveOverride, VariantReason,
    };

    let tmp = tempfile::TempDir::new().expect("tempdir");

    async fn seed(path: &Path) -> DuckDbBackend {
        let backend = DuckDbBackend::new(path, "main").await.expect("open duckdb");
        backend
            .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create target table");
        backend
            .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
            .await
            .expect("seed target table");
        backend
            .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
            .await
            .expect("create dim table");
        backend
            .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
            .await
            .expect("seed dim table");
        backend
    }

    let natural_db_path = tmp.path().join("natural.duckdb");
    let pinned_db_path = tmp.path().join("pinned.duckdb");
    let natural_backend = seed(&natural_db_path).await;
    let pinned_backend = seed(&pinned_db_path).await;

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability = vec![smelt_logical::analysis::walk::ColumnComparability {
        output: "tier".to_string(),
        comparability: smelt_logical::analysis::walk::Comparability::Comparable,
    }];
    let proven_suppression = smelt_logical::maintenance::choice::resolve_write_suppression(
        &["tier".to_string()],
        &comparability,
        &row_identity,
    );

    // A steady-state trigger naturally prefers suppression — pin
    // `technique: unconditional` to force the plain matched arm instead.
    let steady_trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (natural_variant, natural_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(natural_reason, VariantReason::SteadyStatePreference);
    assert!(matches!(
        natural_variant,
        WriteSuppression::Suppressed { .. }
    ));

    let unconditional_pin = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Unconditional),
    };
    let (pinned_variant, pinned_reason) = resolve_write_variant(
        &proven_suppression,
        &steady_trigger,
        false,
        &unconditional_pin,
    )
    .expect("`technique: unconditional` is always admissible — never refuses");
    assert_eq!(pinned_reason, VariantReason::Overridden);
    assert!(matches!(
        pinned_variant,
        WriteSuppression::Unconditional { .. }
    ));

    execute_column_scoped_merge_full(
        &natural_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &natural_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("natural suppressed merge must succeed");

    execute_column_scoped_merge_full(
        &pinned_backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &pinned_variant,
        &test_window(),
        &no_retry_policy(),
    )
    .await
    .expect("pinned unconditional merge must succeed");

    let natural_conn = duckdb::Connection::open(&natural_db_path).expect("reconnect natural");
    let pinned_conn = duckdb::Connection::open(&pinned_db_path).expect("reconnect pinned");
    for user_id in [1_i64, 2_i64] {
        let natural_tier: String = natural_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read natural tier");
        let pinned_tier: String = pinned_conn
            .query_row(
                "SELECT tier FROM main.dim_users WHERE user_id = ?",
                [user_id],
                |row| row.get(0),
            )
            .expect("read pinned tier");
        assert_eq!(
            natural_tier, pinned_tier,
            "user {user_id}: the natural (suppressed) and pinned (unconditional) matched \
             arms must produce bit-identical state — a pin changes which shape ran, never \
             the bits at a fixed processed-input set"
        );
    }
}

/// Real fixture: `examples/timeseries/models/daily_events_enriched.sql`
/// (fact `raw.events` × dimension `raw.users`, the latter declared
/// `mutation_profile: mutable_snapshot`) is the MP11 shape wired into the
/// example workspace. This derives the SAME `MaintenancePlan` `smelt
/// explain` reports (`smelt-db::maintenance_plan_report`), reading the
/// model + source YAML straight off disk with no Salsa layer, and asserts
/// the `{user_name}` group's `UpstreamMutation { source: "raw.users" }`
/// cell is admitted with `Technique::ColumnScopedMerge` — the derivation
/// this phase's `resolve_cell_technique`/`execute_column_scoped_merge`
/// consume. `example_diagnostics` (`crates/smelt-cli/tests/`) is the
/// standing gate that this fixture carries no diagnostics.
#[test]
fn real_fixture_examples_timeseries_admits_column_scoped_merge_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let model_path = project_dir.join("models/daily_events_enriched.sql");
    let text = std::fs::read_to_string(&model_path).expect("read daily_events_enriched.sql");

    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("daily_events_enriched.sql must be a single-model file");
    };
    let sql_body = &text[sql_offset..];

    let config = smelt_core::Config::load(&project_dir).expect("load smelt.yml");
    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);

    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let segs: Vec<String> = stripped.split('.').map(String::from).collect();
            let info = source_infos
                .iter()
                .find(|s| s.address_segments == segs)?
                .clone();
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let sources =
        smelt_db::queries::maintenance::build_source_facts(&source_refs, model_scan_bounds, None);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql_body,
        "daily_events_enriched",
        &metadata,
        &sources,
        &explicitly_mutable,
        None,
        &[],
    )
    .expect("daily_events_enriched has a maintenance plan (refresh: incremental + grain set)");

    assert!(
        result.plan.refusals.is_empty(),
        "expected no admission refusals: {:?}",
        result.plan.refusals
    );

    let mutation_trigger = Trigger::UpstreamMutation {
        source: "raw.users".to_string(),
    };
    let cell = result.plan.cell_for(&mutation_trigger).unwrap_or_else(|| {
        panic!(
            "no cell admitted for {mutation_trigger:?}: {:#?}",
            result.plan
        )
    });
    assert_eq!(
        cell.technique,
        Technique::ColumnScopedMerge,
        "the dimension-mutation cell must admit column-scoped MERGE"
    );
    assert_eq!(cell.group, "{user_name}");
}

/// Real fixture, the `PartitionLocal::Yes` corner:
/// `examples/timeseries/models/daily_events_status.sql` (fact `raw.events` ×
/// a CLOCKED, mutable dimension `raw.user_status`, joined on an explicit
/// `changed_at BETWEEN event_timestamp - INTERVAL '1 day' AND
/// event_timestamp + INTERVAL '1 day'` predicate) derives a genuine
/// `ScanClamp` for `raw.user_status` — unlike `daily_events_enriched.sql`'s
/// unclocked `raw.users`, which only ever derives the accepted-full-scan
/// corner (`PartitionLocal::No`).
///
/// **Known production gap** (documented here, not silently worked around):
/// `smelt_db::queries::maintenance::derive_model_maintenance_plan`'s own
/// trigger-list construction only ever emits a `Trigger::UpstreamMutation`
/// for a source with `partition_col.is_none()` (see that function's doc
/// comment: "a clocked enrichment join's own scan-bound derivation is
/// deferred") — so a clocked source's `UpstreamMutation` cell is never
/// derived through the production wrapper today, regardless of how the
/// runtime dispatches on it. `crates/smelt-db` is outside this phase's
/// allowed files. This test therefore reconstructs the SAME
/// `ModelInputs` the wrapper builds (`build_source_facts`,
/// `skeleton_columns`, `derive_column_groups` — all public
/// `smelt-logical`/`smelt-db` functions, no logic reimplemented) and calls
/// `smelt_logical::maintenance::derive::derive_maintenance_plan` directly
/// with the fuller trigger list the wrapper does not yet construct,
/// proving: (a) this fixture is correctly engineered to derive
/// `PartitionLocal::Yes` once that trigger-list gate is lifted, and (b) the
/// runtime dispatch mechanism this phase wires
/// (`maintenance_driver::decide_column_merge_dispatch`/
/// `execute_column_scoped_merge`, exercised end-to-end against a real
/// DuckDB backend in `yes_corner_matches_full_refresh_after_dimension_mutation`
/// below) is fed the correct shape the moment that gap closes.
#[test]
fn real_fixture_daily_events_status_would_admit_partition_local_yes_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let model_path = project_dir.join("models/daily_events_status.sql");
    let text = std::fs::read_to_string(&model_path).expect("read daily_events_status.sql");

    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("daily_events_status.sql must be a single-model file");
    };
    let sql_body = &text[sql_offset..];

    let config = smelt_core::Config::load(&project_dir).expect("load smelt.yml");
    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);

    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let segs: Vec<String> = stripped.split('.').map(String::from).collect();
            let info = source_infos
                .iter()
                .find(|s| s.address_segments == segs)?
                .clone();
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let sources =
        smelt_db::queries::maintenance::build_source_facts(&source_refs, model_scan_bounds, None);

    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let skeleton = smelt_logical::maintenance::skeleton::skeleton_columns(
        sql_body,
        &[],
        partition_col.as_deref(),
    );
    let grouping =
        smelt_logical::maintenance::grouping::derive_column_groups(sql_body, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "expected no degenerate column-group collapses: {:?}",
        grouping.degenerate
    );

    let inputs = smelt_logical::maintenance::derive::ModelInputs {
        sql: sql_body,
        output: smelt_logical::maintenance::OutputSpec {
            table: "daily_events_status".to_string(),
            grain: smelt_logical::maintenance::Grain::Partition {
                partition_col: partition_col.clone().unwrap_or_default(),
            },
            skeleton_columns: skeleton,
        },
        sources: sources.clone(),
        column_groups: grouping.groups.clone(),
        fold: None,
        column_add_proof: None,
    };

    let plan = smelt_logical::maintenance::derive::derive_maintenance_plan(
        &inputs,
        &[
            Trigger::NewData {
                source: "raw.events".to_string(),
            },
            Trigger::NewData {
                source: "raw.user_status".to_string(),
            },
            Trigger::UpstreamMutation {
                source: "raw.user_status".to_string(),
            },
            Trigger::Backfill,
        ],
    );

    assert!(
        plan.refusals.is_empty(),
        "expected no admission refusals: {:?}",
        plan.refusals
    );

    let mutation_trigger = Trigger::UpstreamMutation {
        source: "raw.user_status".to_string(),
    };
    let cell = plan
        .cell_for(&mutation_trigger)
        .unwrap_or_else(|| panic!("no cell admitted for {mutation_trigger:?}: {plan:#?}"));
    assert_eq!(
        cell.technique,
        Technique::ColumnScopedMerge,
        "the dimension-mutation cell must admit column-scoped MERGE"
    );
    assert_eq!(cell.group, "{status}");
    assert_eq!(
        cell.partition_local,
        PartitionLocal::Yes,
        "raw.user_status is clocked with an explicit, derivable window predicate — this must \
         be the genuine scan-clamp corner, not the accepted-full-scan corner \
         daily_events_enriched.sql exercises"
    );
    let scan = cell
        .scans
        .iter()
        .find(|s| s.source == "raw.user_status")
        .unwrap_or_else(|| panic!("no scan clamp for 'raw.user_status': {:?}", cell.scans));
    assert_eq!(scan.column, "changed_at");

    // The mechanism this fixture feeds is unit-tested directly against
    // `dimension_join_contribution` (`maintenance_driver_tests` below) and
    // exercised end-to-end against a real DuckDB backend in
    // `yes_corner_matches_full_refresh_after_dimension_mutation`.
    let dimension_unique_key = source_infos
        .iter()
        .find(|s| s.address_segments == ["sources", "raw", "user_status"])
        .and_then(|s| s.unique_key.clone())
        .unwrap_or_default();
    assert_eq!(dimension_unique_key, vec!["user_id".to_string()]);
    let contribution = smelt_runtime::maintenance_driver::dimension_join_contribution(
        sql_body,
        "raw.user_status",
        &dimension_unique_key,
    );
    assert!(
        contribution.is_monotone(),
        "the fact->dimension join must be provable one-to-one: {contribution:?}"
    );
}

/// MP11's real end-to-end proof: drive `examples/timeseries/models/
/// daily_events_enriched.sql` through `execute_project` itself — never a
/// direct call to `resolve_cell_technique`/`execute_column_scoped_merge` —
/// and observe the regular incremental execution loop
/// (`crates/smelt-runtime/src/execute.rs`) dispatch to a column-scoped
/// `MERGE` when a dimension mutation makes the `Trigger::UpstreamMutation`
/// cell live.
mod column_scoped_merge_e2e {
    use std::path::Path;
    use std::sync::Arc;

    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_core::config::Config;
    use smelt_core::graph::DependencyGraph;
    use smelt_core::ModelDiscovery;
    use smelt_runtime::execute::{BackendFactory, BackendFuture};
    use smelt_runtime::types::ExecuteRequest;
    use smelt_runtime::{execute_project, NoOpReporter};
    use tokio_util::sync::CancellationToken;

    /// `BackendFactory` that always opens the same on-disk DuckDB file,
    /// mirroring `crates/smelt-runtime/tests/execute_parity.rs`'s harness.
    struct DuckDbBackendFactory {
        db_path: std::path::PathBuf,
    }

    impl BackendFactory for DuckDbBackendFactory {
        fn create<'a>(
            &'a self,
            _target_name: &'a str,
            target_config: &'a smelt_core::config::Target,
            _project_dir: &'a Path,
        ) -> BackendFuture<'a> {
            let path = self.db_path.clone();
            let schema = target_config.schema.clone();
            Box::pin(async move {
                let backend = DuckDbBackend::new(&path, &schema)
                    .await
                    .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
                Ok(Box::new(backend) as Box<dyn Backend>)
            })
        }
    }

    /// Copy `examples/timeseries` into a scratch directory so the run's
    /// `.smelt/` state (`FileStore::new(project_dir, target)`) never lands
    /// inside the checked-in example.
    fn copy_dir_recursive(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).expect("create dst dir");
        for entry in std::fs::read_dir(src).expect("read src dir") {
            let entry = entry.expect("dir entry");
            let file_type = entry.file_type().expect("file type");
            let dst_path = dst.join(entry.file_name());
            if file_type.is_dir() {
                copy_dir_recursive(&entry.path(), &dst_path);
            } else {
                std::fs::copy(entry.path(), &dst_path).expect("copy file");
            }
        }
    }

    fn build_db_and_graph(
        project_dir: &Path,
        config: &Config,
    ) -> (
        Arc<tokio::sync::Mutex<smelt_db::Database>>,
        Arc<tokio::sync::Mutex<DependencyGraph>>,
    ) {
        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
        let sql_models = discovery.discover_models().expect("discover_models");

        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(project_dir.to_path_buf(), String::new());
        let source_files: Vec<_> = sql_models
            .iter()
            .map(|m| {
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf())
            })
            .collect();
        db.set_workspace(source_files, vec![project]);
        db.set_active_target(Some(std::sync::Arc::from("dev")));

        let graph = DependencyGraph::build(sql_models, None).expect("build graph");

        (
            Arc::new(tokio::sync::Mutex::new(db)),
            Arc::new(tokio::sync::Mutex::new(graph)),
        )
    }

    fn request_for_day() -> ExecuteRequest {
        ExecuteRequest {
            target: "dev".to_string(),
            select: vec!["daily_events_enriched".to_string()],
            exclude: vec![],
            start: Some("2025-01-10".to_string()),
            end: Some("2025-01-11".to_string()),
            batch_size_days: None,
            per_partition: false,
            full_refresh: false,
            dry_run: false,
            enforce_safety: false,
            allow_column_removal: false,
            allow_full_refresh: false,
            ephemeral_seed_ctes: vec![],
            run_checks: false,
            checks: vec![],
            jobs: None,
            retry_max: None,
            retry_backoff_ms: None,
            resume: false,
            technique_overrides: vec![],
        }
    }

    /// First run creates the target via the normal `Trigger::NewData`
    /// region-recompute path (the table doesn't exist yet). A dimension
    /// mutation is then applied directly to the staged `raw.users` source
    /// table, and a SECOND `execute_project` call over the SAME window must
    /// route through `execute.rs`'s regular incremental batch-execution
    /// branch to the live `Trigger::UpstreamMutation` cell's
    /// `ColumnScopedMerge` technique
    /// (`maintenance_driver::resolve_live_column_scoped_cell` +
    /// `execute_column_scoped_merge_full`) — reported back as
    /// `RunOutcome.models["daily_events_enriched"].strategy ==
    /// "column_scoped_merge"`, never the default region-recompute path a
    /// plain incremental run would otherwise take for every batch.
    #[tokio::test]
    async fn column_scoped_merge_dispatches_through_execute_project() {
        let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/timeseries")
            .canonicalize()
            .expect("examples/timeseries exists");

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        copy_dir_recursive(&source_dir, &project_dir);

        let db_path = tmp.path().join("run.duckdb");
        let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
        let backend_factory = DuckDbBackendFactory {
            db_path: db_path.clone(),
        };

        // Stage the two source tables `execute_project` reads —
        // `smelt.sources.raw.events` / `smelt.sources.raw.users` resolve to
        // `main.sources_raw_events` / `main.sources_raw_users` under the
        // unified default source-name mapping (no `name:` override in
        // either source YAML) — directly via raw SQL. The CSV seed loader
        // is a separate CLI-level step `execute_project` itself does not
        // perform.
        {
            let backend = DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            backend
                .execute_sql(
                    "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
                     event_type VARCHAR, event_timestamp TIMESTAMP)",
                )
                .await
                .expect("create events source table");
            backend
                .execute_sql(
                    "INSERT INTO main.sources_raw_events VALUES \
                     (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
                     (2, 2, 'login', TIMESTAMP '2025-01-10 09:00:00')",
                )
                .await
                .expect("seed events");
            backend
                .execute_sql(
                    "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                     signup_date DATE)",
                )
                .await
                .expect("create users source table");
            backend
                .execute_sql(
                    "INSERT INTO main.sources_raw_users VALUES \
                     (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
                )
                .await
                .expect("seed users");
        }

        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = execute_project(
                "run-1".to_string(),
                request_for_day(),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &backend_factory,
                &NoOpReporter,
                CancellationToken::new(),
            )
            .await
            .expect("first run must succeed");
            let record = outcome
                .models
                .get("daily_events_enriched")
                .expect("daily_events_enriched ran");
            assert_ne!(
                record.strategy, "column_scoped_merge",
                "the creation run must not take the column-scoped merge path — the target \
                 doesn't exist yet"
            );
        }

        // Mutate the dimension in place — `raw.users` is declared
        // `mutation_profile: mutable_snapshot`; renaming user 1 broadcasts
        // to every fact row referencing them (the `{user_name}` group).
        {
            let backend = DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            backend
                .execute_sql(
                    "UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1",
                )
                .await
                .expect("mutate dimension");
        }

        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-2".to_string(),
            request_for_day(),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("second run must succeed");
        let record = outcome
            .models
            .get("daily_events_enriched")
            .expect("daily_events_enriched ran");
        assert_eq!(
            record.strategy, "column_scoped_merge",
            "a dimension mutation must dispatch the regular incremental run through the \
             column-scoped MERGE technique (MP11), not the default region-recompute path"
        );

        let conn = duckdb::Connection::open(&db_path).expect("reconnect");
        let maintained_user_name: String = conn
            .query_row(
                "SELECT user_name FROM main.daily_events_enriched WHERE user_id = 1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("read maintained user_name");
        assert_eq!(
            maintained_user_name, "Alicia",
            "column-scoped MERGE must pick up the mutated dimension value"
        );

        let untouched_user_name: String = conn
            .query_row(
                "SELECT user_name FROM main.daily_events_enriched WHERE user_id = 2 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("read untouched user_name");
        assert_eq!(
            untouched_user_name, "Bob",
            "an unmutated dimension row's enrichment must be unchanged"
        );
    }
}

/// W10 Phase 4 (`docs/plans/20260720-prod-w10-keyed-mutable-admission.md`):
/// the keyed run path's own live `ColumnScopedMerge` dispatch — the
/// `plan_is_keyed` branch of `execute.rs` now consults
/// `resolve_live_column_scoped_cell` exactly as the non-keyed incremental
/// branch already does (`column_scoped_merge_e2e` above), reached now that
/// W10 Phases 1-3 narrow the key-grain `NewData` append-only obligation to
/// admit a keyed model that consumes an `explicitly_mutable` dimension.
///
/// **The fixture shape this module is forced into, and why.** A `grain:
/// key` body must satisfy `classify_cumulative`'s aggregate/`GROUP BY`
/// grammar (`incremental_models.md` §"Key-grain declaration"): every
/// non-aggregate `SELECT` item must be a literal `GROUP BY` key. That
/// makes a mutable dimension's own attribute column unreachable as a plain
/// enrich-only payload group two ways at once — selecting it forces it
/// into `GROUP BY` (`maintenance::skeleton::skeleton_roles` classifies
/// every `GROUP BY` column `Grouping`-role, and
/// `maintenance::grouping::derive_column_groups` excludes every skeleton
/// column from column-group derivation entirely, so no cell can ever
/// mention it), while wrapping it in an aggregate makes it fold-contributing
/// (`source_contributes_to_fold`), which W10 Phase 3's narrowed
/// `derive_new_data` refuses outright for a mutable source (`both fold and
/// enrich stays refused`, the safety carve-out
/// `crates/smelt-logical/tests/maintenance_new_data_enrich_only_waiver.rs`
/// pins). The one shape that DOES reach a live cell today is a fold
/// aggregate whose own argument does not mention the dimension at all
/// (`COUNT(t.transaction_id)`, reading only the append-only fact) joined against
/// the mutable dimension purely for row admission — `maintenance::grouping`'s
/// per-column provenance walk has a pre-existing gap (documented at
/// `examples/timeseries/models/daily_events_enriched.sql`'s own comment on
/// `model_property_vector`/skeleton-closure: "a bare `FUNCTION_CALL`-shaped
/// [reference] once 2+ FROM sources are in scope... the call's own name
/// token is misread as an unqualified column reference") that also affects
/// `derive_column_groups`'s `collect_column_refs`: an aggregate's own
/// function-name token is misresolved as an ambiguous unqualified column
/// reference once 2+ sources are joined, and the fail-closed collapse this
/// triggers (`degenerate_whole_model` — "never silently narrower", the
/// classifier's own documented safety net) widens the fold column's
/// mutation-sensitivity to every source the model references, including
/// the dimension it never actually reads. That collapse is what
/// legitimately (if conservatively) admits the `ColumnScopedMerge` cell
/// this module exercises. It is real, reachable, deterministic
/// `derive_model_maintenance_plan` output — not a fabricated shape — but it
/// also means the merged column's TRUE value never depends on the
/// dimension's own data, only on the fact: a dimension mutation can never
/// make the compared column's value diverge from what full-refresh would
/// already produce, so every dispatched merge in this fixture is a genuine
/// `WriteSuppression::Suppressed` no-op by construction, not merely by
/// coincidence of the staged data. `grouping.rs`/`derive.rs` are both
/// outside this phase's critical files (fixing the collapse would remove
/// the only reachable live cell, not produce a "cleaner" one — there is no
/// currently-implemented shape where a mutable dimension's own attribute is
/// both non-skeleton and non-fold-contributing); this is flagged here for
/// review rather than silently worked around.
mod keyed_column_scoped_merge_e2e {
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::Arc;

    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_core::config::Config;
    use smelt_core::graph::DependencyGraph;
    use smelt_core::ModelDiscovery;
    use smelt_logical::maintenance::choice::WriteSuppression;
    use smelt_logical::maintenance::{MutationProfile, SourceFacts};
    use smelt_runtime::execute::{BackendFactory, BackendFuture};
    use smelt_runtime::maintenance_driver::resolve_live_column_scoped_cell;
    use smelt_runtime::types::ExecuteRequest;
    use smelt_runtime::{execute_project, NoOpReporter};
    use tokio_util::sync::CancellationToken;

    /// The keyed model body this whole module exercises: `raw.transactions`
    /// (append-only, and — unlike `raw.events` — clocked via its OWN
    /// source-YAML `timeseries:` block, the window-forward run shape's
    /// admission precondition, `KeyedSnapshotPostureUnsupported` otherwise)
    /// folded per `user_id` via `COUNT`, inner-joined to `raw.users`
    /// (unclocked `mutation_profile: mutable_snapshot`, `allow_full_scan`
    /// declared) purely for row admission — see the module doc comment
    /// above for why the dimension's own attribute cannot itself be a
    /// selected payload column today.
    const MODEL_SQL: &str = "SELECT t.user_id AS user_id, COUNT(t.transaction_id) AS event_count \
         FROM smelt.sources.raw.transactions t \
         JOIN smelt.sources.raw.users u ON t.user_id = u.user_id \
         GROUP BY t.user_id";

    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: user_id\n\
         maintenance:\n  \
           scan_bounds:\n    \
             per_source:\n      \
               raw.users:\n        \
                 allow_full_scan: true\n\
         ---\n";

    fn model_file_text() -> String {
        format!("{MODEL_FILE}{MODEL_SQL}\n")
    }

    /// Unit-level proof (no backend): `resolve_live_column_scoped_cell` —
    /// the exact resolver `execute.rs`'s `plan_is_keyed` branch now calls —
    /// resolves this model's `raw.users` `UpstreamMutation` cell to
    /// `Technique::ColumnScopedMerge` with `WriteSuppression::Suppressed`
    /// (P3 comparability holds for `event_count`, an INTEGER column, and
    /// this is a steady-state trigger with no ledger catch-up — the
    /// suppressed arm is preferred over unconditional per
    /// `choice::resolve_write_variant`).
    #[test]
    fn resolves_suppressed_column_scoped_merge_for_keyed_dimension_cell() {
        let text = model_file_text();
        let smelt_core::FileMetadata::Single {
            metadata,
            sql_offset,
        } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
        else {
            panic!("single-model file");
        };
        let sql_body = &text[sql_offset..];

        let sources = vec![
            SourceFacts {
                name: "raw.transactions".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: None,
                unique_key: vec![],
                allow_full_scan: false,
            },
            SourceFacts {
                name: "raw.users".to_string(),
                mutation: MutationProfile::MutableSnapshot,
                partition_col: None,
                unique_key: vec![],
                allow_full_scan: true,
            },
        ];
        let mut explicitly_mutable = HashSet::new();
        explicitly_mutable.insert("raw.users".to_string());

        let (source, cell, suppression) = resolve_live_column_scoped_cell(
            sql_body,
            "user_lifetime_status",
            &metadata,
            &sources,
            &explicitly_mutable,
            true,
            &[],
        )
        .expect("resolver must not error")
        .expect("a live ColumnScopedMerge cell must resolve for raw.users");

        assert_eq!(source, "raw.users");
        assert_eq!(
            cell.technique,
            smelt_logical::maintenance::Technique::ColumnScopedMerge
        );
        assert!(
            matches!(suppression, WriteSuppression::Suppressed { .. }),
            "expected the change-suppressed matched arm, got {suppression:?}"
        );
    }

    /// `BackendFactory` that always opens the same on-disk DuckDB file,
    /// mirroring `column_scoped_merge_e2e` above.
    struct DuckDbBackendFactory {
        db_path: std::path::PathBuf,
    }

    impl BackendFactory for DuckDbBackendFactory {
        fn create<'a>(
            &'a self,
            _target_name: &'a str,
            target_config: &'a smelt_core::config::Target,
            _project_dir: &'a Path,
        ) -> BackendFuture<'a> {
            let path = self.db_path.clone();
            let schema = target_config.schema.clone();
            Box::pin(async move {
                let backend = DuckDbBackend::new(&path, &schema)
                    .await
                    .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
                Ok(Box::new(backend) as Box<dyn Backend>)
            })
        }
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).expect("create dst dir");
        for entry in std::fs::read_dir(src).expect("read src dir") {
            let entry = entry.expect("dir entry");
            let file_type = entry.file_type().expect("file type");
            let dst_path = dst.join(entry.file_name());
            if file_type.is_dir() {
                copy_dir_recursive(&entry.path(), &dst_path);
            } else {
                std::fs::copy(entry.path(), &dst_path).expect("copy file");
            }
        }
    }

    fn build_db_and_graph(
        project_dir: &Path,
        config: &Config,
    ) -> (
        Arc<tokio::sync::Mutex<smelt_db::Database>>,
        Arc<tokio::sync::Mutex<DependencyGraph>>,
    ) {
        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
        let sql_models = discovery.discover_models().expect("discover_models");

        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(project_dir.to_path_buf(), String::new());
        let source_files: Vec<_> = sql_models
            .iter()
            .map(|m| {
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf())
            })
            .collect();
        db.set_workspace(source_files, vec![project]);
        db.set_active_target(Some(std::sync::Arc::from("dev")));

        let graph = DependencyGraph::build(sql_models, None).expect("build graph");

        (
            Arc::new(tokio::sync::Mutex::new(db)),
            Arc::new(tokio::sync::Mutex::new(graph)),
        )
    }

    /// `start`/`end` advance by one day per call — the windowed-keyed-
    /// maintenance driver's reconciliation ledger refuses re-folding the
    /// SAME partition twice (`docs/specs/incremental_models.md`
    /// §"Reprocessing" — never-fold-twice), independent of whether the
    /// column-scoped-merge dispatch this module tests fires; each of this
    /// test's three runs therefore needs its own fresh day. No transaction
    /// rows are staged for days after the first, so the fold contributes
    /// nothing new on runs 2/3 — `event_count` stays exactly what run 1
    /// computed throughout.
    fn request_for_day(start: &str, end: &str) -> ExecuteRequest {
        ExecuteRequest {
            target: "dev".to_string(),
            select: vec!["user_lifetime_status".to_string()],
            exclude: vec![],
            start: Some(start.to_string()),
            end: Some(end.to_string()),
            batch_size_days: None,
            per_partition: false,
            full_refresh: false,
            dry_run: false,
            enforce_safety: false,
            allow_column_removal: false,
            allow_full_refresh: false,
            ephemeral_seed_ctes: vec![],
            run_checks: false,
            checks: vec![],
            jobs: None,
            retry_max: None,
            retry_backoff_ms: None,
            resume: false,
            technique_overrides: vec![],
        }
    }

    /// The real-fixture requirement: drive the model above through
    /// `execute_project` itself (root `CLAUDE.md` §"Run pipeline parity
    /// rule"). First run creates the target via the ordinary `KeyedFold`
    /// creation path (the table doesn't exist yet — the creation run must
    /// never take the column-scoped-merge path). A SECOND run over the
    /// SAME window, with the table now present, must route through
    /// `execute.rs`'s `plan_is_keyed` branch's new live-cell dispatch to
    /// `Technique::ColumnScopedMerge` (`RunOutcome.models["user_lifetime_
    /// status"].strategy == "column_scoped_merge"`) — never the default
    /// `cumulative_aggregate` fold label a plain keyed run would otherwise
    /// report every time. A THIRD run (no data changes at all since the
    /// second) must still dispatch the same technique (the known "fires on
    /// every run, not gated on a genuine change" divergence,
    /// `incremental_models.md` §Known Divergences) and must write ZERO
    /// affected rows — the change-suppressed `IS DISTINCT FROM` arm
    /// actually suppressing, read directly off DuckDB via the SAME
    /// `emit_column_scoped_merge_suppressed`-shaped probe
    /// `column_scoped_merge_e2e`'s siblings already use
    /// (`super::merge_affected_row_count`).
    #[tokio::test]
    async fn keyed_run_loop_dispatches_column_scoped_merge_through_execute_project() {
        let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/timeseries")
            .canonicalize()
            .expect("examples/timeseries exists");

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        copy_dir_recursive(&source_dir, &project_dir);
        std::fs::write(
            project_dir.join("models/user_lifetime_status.sql"),
            model_file_text(),
        )
        .expect("write keyed model fixture");

        let db_path = tmp.path().join("run.duckdb");
        let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
        let backend_factory = DuckDbBackendFactory {
            db_path: db_path.clone(),
        };

        // Stage the two source tables `execute_project` reads (mirrors
        // `column_scoped_merge_e2e`'s own staging, but over `raw.transactions`
        // — clocked via its OWN source-YAML `timeseries:` block, unlike
        // `raw.events`, whose clock is only ever declared on a downstream
        // MODEL's frontmatter — this fixture declares none) — the CSV seed
        // loader is a separate CLI-level step `execute_project` itself does
        // not perform.
        {
            let backend = DuckDbBackend::new(&db_path, "main")
                .await
                .expect("open duckdb");
            backend
                .execute_sql(
                    "CREATE TABLE main.sources_raw_transactions (transaction_id INTEGER, \
                     user_id INTEGER, amount DECIMAL(10,2), transaction_timestamp TIMESTAMP, \
                     transaction_type VARCHAR)",
                )
                .await
                .expect("create transactions source table");
            backend
                .execute_sql(
                    "INSERT INTO main.sources_raw_transactions VALUES \
                     (1, 1, 10.00, TIMESTAMP '2025-01-10 08:00:00', 'purchase'), \
                     (2, 2, 20.00, TIMESTAMP '2025-01-10 09:00:00', 'purchase')",
                )
                .await
                .expect("seed transactions");
            backend
                .execute_sql(
                    "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                     signup_date DATE)",
                )
                .await
                .expect("create users source table");
            backend
                .execute_sql(
                    "INSERT INTO main.sources_raw_users VALUES \
                     (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
                )
                .await
                .expect("seed users");
        }

        // First run: creation. Must not take the column-scoped-merge path.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = execute_project(
                "run-1".to_string(),
                request_for_day("2025-01-10", "2025-01-11"),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &backend_factory,
                &NoOpReporter,
                CancellationToken::new(),
            )
            .await
            .expect("first run must succeed");
            let record = outcome
                .models
                .get("user_lifetime_status")
                .expect("user_lifetime_status ran");
            assert_ne!(
                record.strategy, "column_scoped_merge",
                "the creation run must not take the column-scoped merge path — the target \
                 doesn't exist yet"
            );
        }

        // Mutate the dimension in place — mirrors `column_scoped_merge_e2e`'s
        // own narrative even though (per the module doc comment) this
        // fixture's merged column never actually depends on the mutated
        // value.
        {
            let backend = DuckDbBackend::new(&db_path, "main")
                .await
                .expect("reopen duckdb");
            backend
                .execute_sql(
                    "UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1",
                )
                .await
                .expect("mutate dimension");
        }

        // Second run: the live cell dispatches.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = execute_project(
                "run-2".to_string(),
                request_for_day("2025-01-11", "2025-01-12"),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &backend_factory,
                &NoOpReporter,
                CancellationToken::new(),
            )
            .await
            .expect("second run must succeed");
            let record = outcome
                .models
                .get("user_lifetime_status")
                .expect("user_lifetime_status ran");
            assert_eq!(
                record.strategy, "column_scoped_merge",
                "a live UpstreamMutation cell must dispatch the keyed run loop through the \
                 column-scoped MERGE technique (W10 Phase 4), not the default cumulative-fold \
                 label a plain keyed run would otherwise report"
            );
        }

        let conn = duckdb::Connection::open(&db_path).expect("reconnect");
        let (user1_count, user2_count): (i64, i64) = {
            let c1: i64 = conn
                .query_row(
                    "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 1",
                    [],
                    |row| row.get(0),
                )
                .expect("read user 1 event_count");
            let c2: i64 = conn
                .query_row(
                    "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 2",
                    [],
                    |row| row.get(0),
                )
                .expect("read user 2 event_count");
            (c1, c2)
        };
        assert_eq!(
            (user1_count, user2_count),
            (1, 1),
            "the column-scoped MERGE must not corrupt the fact-derived event_count — both \
             users still show exactly the one event staged for them"
        );

        // Third run: no data change since run 2 at all. The dispatch still
        // fires (known divergence — unconditional per-run dispatch), but
        // the change-suppressed arm must write zero affected rows.
        {
            let (db, graph) = build_db_and_graph(&project_dir, &config);
            let outcome = execute_project(
                "run-3".to_string(),
                request_for_day("2025-01-12", "2025-01-13"),
                Arc::clone(&config),
                graph,
                db,
                &project_dir,
                &backend_factory,
                &NoOpReporter,
                CancellationToken::new(),
            )
            .await
            .expect("third run must succeed");
            let record = outcome
                .models
                .get("user_lifetime_status")
                .expect("user_lifetime_status ran");
            assert_eq!(record.strategy, "column_scoped_merge");
        }

        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb for probe");
        let recompute_sql = MODEL_SQL
            .replace(
                "smelt.sources.raw.transactions",
                "main.sources_raw_transactions",
            )
            .replace("smelt.sources.raw.users", "main.sources_raw_users");
        let source_subquery = format!("({recompute_sql}) recomputed");
        let affected = super::merge_affected_row_count(
            &backend,
            "main.user_lifetime_status",
            &source_subquery,
            &["user_id"],
            &["event_count"],
        )
        .await;
        assert_eq!(
            affected, 0,
            "an unchanged redelivery must write zero rows through the change-suppressed arm"
        );
    }
}

/// The open write-pattern registry's `write:` pin, end-to-end
/// (`docs/specs/incremental_models.md` §"Per-cell write addressing" →
/// "User pins"; `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase R1). Unlike an earlier version of this module,
/// these tests do not call [`resolve_write_pin`] and
/// `emit_delete_insert`/`resolve_cell_technique_with_write_pin` as two
/// disconnected function calls that happen to agree by construction — the
/// resolved [`smelt_logical::maintenance::WritePattern`] returned by
/// [`resolve_write_pin`] is fed directly into
/// [`smelt_logical::maintenance::choice::resolve_cell_choice`] (the real
/// technique-choice resolver `choice.rs` documents as the module a
/// `write:` pin must constrain) and into
/// [`resolve_cell_technique_with_write_pin`] (the real runtime driver
/// resolver `maintenance_driver.rs` documents the same way) — the same two
/// functions the review flagged as validating-and-discarding the pin. Each
/// fixture is chosen so the pin changes what these resolvers pick relative
/// to their own unpinned default: a non-vacuous proof the pin is actually
/// consulted, not merely accepted.
mod write_pattern_registry_pin {
    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_logical::maintenance::choice::{
        effective_override, resolve_cell_choice, ChosenTechnique,
    };
    use smelt_logical::maintenance::emit::{emit_delete_insert, MaintenanceDialect, Region};
    use smelt_logical::maintenance::{
        lookup_write_pattern, resolve_write_pin, BackendWriteCapabilities, Corner, MaintenancePlan,
        OutputContractFacts, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique,
        Trigger,
    };

    use super::{admitted_plan, resolve_cell_technique_with_write_pin, ResolvedTechnique};

    /// A composed model's mutation-trigger cell whose derived plan admits
    /// `Technique::KeyedFold` (the fold-a-delta corner — `RowIdentity::
    /// Key`, the shape a `grain: key` + `timeseries:` composed model
    /// derives for an upstream source mutation) — the default,
    /// UNPINNED choice this fixture exists to be overridden away from.
    fn composed_keyed_fold_plan(source: &str) -> MaintenancePlan {
        MaintenancePlan {
            cells: vec![PlanCell {
                group: "{*}".to_string(),
                trigger: Trigger::UpstreamMutation {
                    source: source.to_string(),
                },
                corner: Corner::FoldDelta,
                technique: Technique::KeyedFold,
                partition_local: PartitionLocal::Yes,
                scans: vec![],
                ledger_catch_up: false,
                row_identity: RowIdentityVerdict {
                    identity: RowIdentity::Key(vec!["id".to_string()]),
                    proven_mismatch: None,
                },
                skeleton_source_closure: None,
                fingerprint_projections: std::collections::BTreeMap::new(),
            }],
            refusals: vec![],
            key_locality: None,
        }
    }

    /// Pinning `write: region` on a composed model's mutation cell —
    /// admitted by the plan as `KeyedFold`, not `region`'s
    /// `DeleteInsert`/region-recompute corner — resolves against the open
    /// registry (the pattern only requires a declared partition axis, which
    /// a composed key+timeseries output declares) and then, fed into the
    /// real `resolve_cell_choice`, overrides the cell's own admitted
    /// technique: the pin changes the resolved choice from `Admitted
    /// (KeyedFold)` (what an unpinned resolution picks) to
    /// `RegionRecompute` — proving the pin is actually consulted, not
    /// merely validated and discarded. The resolved `RegionRecompute`
    /// choice is then lowered through the SAME `emit_delete_insert` emitter
    /// `Technique::DeleteInsert` cells use and actually executed against a
    /// real DuckDB backend, matching a hand-written full-refresh oracle.
    #[tokio::test]
    async fn pinning_region_on_composed_mutation_cell_overrides_keyed_fold_to_delete_insert() {
        let source = "sources.raw_events";
        let plan = composed_keyed_fold_plan(source);
        let trigger = Trigger::UpstreamMutation {
            source: source.to_string(),
        };

        // 0. Prove the fixture is non-vacuous: absent a write pin, the real
        //    resolver picks the cell's own admitted `KeyedFold`, not region
        //    recompute.
        let unpinned = resolve_cell_choice(
            &plan,
            &trigger,
            &effective_override(None, &[], "unused", &[]),
            None,
            true,
        )
        .expect("unpinned resolution must not refuse");
        assert_eq!(
            unpinned,
            ChosenTechnique::Admitted(Technique::KeyedFold),
            "the unpinned default must be the cell's own admitted technique, not region \
             recompute — otherwise the pin below can't be proven to have changed anything"
        );

        // 1. Resolve the pin against the registry: a composed (key +
        //    partition-axis) output admits `region` (it only requires a
        //    declared partition axis).
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: true,
        };
        let backend_caps = BackendWriteCapabilities {
            supports_merge: true,
            supports_column_scoped_merge: true,
        };
        let resolved_pattern = resolve_write_pin(
            "UpstreamMutation",
            "region",
            "duckdb",
            facts,
            backend_caps,
            |_pattern| Ok(()),
        )
        .expect("a `region` pin on a partition-axis output must resolve");
        assert_eq!(resolved_pattern.name, "region");
        // `lookup_write_pattern` is the same registry lookup
        // `resolve_cell_technique`'s/`resolve_cell_choice`'s production
        // call sites use to turn a stored `cells[].write: String` back into
        // a `&'static WritePattern` — exercised here instead of just
        // reusing `resolved_pattern` directly, so this test also proves the
        // production lookup path resolves to the identical entry.
        let looked_up = lookup_write_pattern("region").expect("registered pattern");
        assert_eq!(looked_up.name, resolved_pattern.name);

        // 2. Feed the resolved, validated pattern into the REAL
        //    technique-choice resolver — this is the wiring the review
        //    found missing: the pin must actually change what this
        //    function picks, not just have been checked upstream.
        let pinned = resolve_cell_choice(
            &plan,
            &trigger,
            &effective_override(None, &[], "unused", &[]),
            Some(looked_up),
            true,
        )
        .expect("a region pin on a partition-axis output must resolve, not refuse");
        assert_eq!(
            pinned,
            ChosenTechnique::RegionRecompute,
            "the write pin must override the cell's own admitted KeyedFold technique"
        );
        assert_ne!(
            pinned, unpinned,
            "the pin must actually change the outcome relative to the unpinned default — \
             otherwise it is validated and ignored, not consulted"
        );

        // 3. `ChosenTechnique::RegionRecompute` lowers to the SAME
        //    `emit_delete_insert` emitter `Technique::DeleteInsert` cells
        //    use (`incremental_models.md` §"Statement emission (single
        //    owner)") — actually executed against a real DuckDB backend and
        //    checked against a hand-written full-refresh oracle.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");

        backend
            .execute_sql("CREATE TABLE main.daily_totals (d DATE, total DOUBLE)")
            .await
            .expect("create target table");
        backend
            .execute_sql(
                "INSERT INTO main.daily_totals VALUES \
                 (DATE '2024-01-01', 999.0), \
                 (DATE '2024-01-02', 20.0)",
            )
            .await
            .expect("seed target table with a stale 2024-01-01 row");
        backend
            .execute_sql("CREATE TABLE main.raw_events (d DATE, amount DOUBLE)")
            .await
            .expect("create source table");
        backend
            .execute_sql(
                "INSERT INTO main.raw_events VALUES \
                 (DATE '2024-01-01', 5.0), \
                 (DATE '2024-01-01', 7.0)",
            )
            .await
            .expect("seed source table");

        let region = Region {
            start: "DATE '2024-01-01'".to_string(),
            end: "DATE '2024-01-02'".to_string(),
        };
        let body =
            "SELECT d, SUM(amount) AS total FROM main.raw_events WHERE d >= DATE '2024-01-01' \
             AND d < DATE '2024-01-02' GROUP BY d";
        let group = match pinned {
            ChosenTechnique::RegionRecompute => emit_delete_insert(
                "main.daily_totals",
                "d",
                &region,
                body,
                MaintenanceDialect::DuckDb,
            ),
            ChosenTechnique::Admitted(_) => {
                panic!("the pin must have resolved to RegionRecompute — asserted above")
            }
        };
        assert_eq!(
            group.statements.len(),
            2,
            "the region pattern's physical mechanism is exactly one DELETE + one INSERT"
        );
        assert!(group.statements[0].sql.starts_with("DELETE FROM"));
        assert!(group.statements[1].sql.starts_with("INSERT INTO"));
        assert!(group.transactional, "DELETE+INSERT must be one transaction");

        backend
            .execute_statement_group(&group)
            .await
            .expect("DELETE+INSERT region rewrite must succeed");

        let conn = duckdb::Connection::open(&db_path).expect("reconnect");
        let recomputed_total: f64 = conn
            .query_row(
                "SELECT total FROM main.daily_totals WHERE d = DATE '2024-01-01'",
                [],
                |row| row.get(0),
            )
            .expect("read recomputed total");
        assert_eq!(
            recomputed_total, 12.0,
            "the pinned region rewrite must replace the stale row with the recomputed total \
             (5.0 + 7.0), matching a full-refresh oracle over the same region"
        );

        let untouched_total: f64 = conn
            .query_row(
                "SELECT total FROM main.daily_totals WHERE d = DATE '2024-01-02'",
                [],
                |row| row.get(0),
            )
            .expect("read untouched row");
        assert_eq!(
            untouched_total, 20.0,
            "a region-scoped DELETE+INSERT must not touch rows outside the pinned region"
        );
    }

    /// A `write: keyed` pin on an identity-free output refuses at
    /// resolution time — never silently falls back to `region` or any
    /// other addressing (no substituted technique).
    #[test]
    fn pinning_keyed_on_identity_free_output_refuses_never_substitutes() {
        let facts = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let backend_caps = BackendWriteCapabilities {
            supports_merge: true,
            supports_column_scoped_merge: true,
        };
        let err = resolve_write_pin(
            "Backfill",
            "keyed",
            "duckdb",
            facts,
            backend_caps,
            |_pattern| Ok(()),
        )
        .expect_err("keyed must refuse on an identity-free output");
        assert!(err
            .to_string()
            .contains("MaintenanceWriteAddressingRefused"));
    }

    /// A pin that resolves cleanly against the registry (structural facts +
    /// backend capability both satisfied) can still be refused one level
    /// deeper by `resolve_cell_choice`, when the validated pattern's
    /// selection isn't what THIS cell's derived plan actually admitted —
    /// e.g. `write: column` validated fine against an identity-bearing
    /// output, but the cell in hand only ever admitted `KeyedFold`, not
    /// `ColumnScopedMerge`. Never a silent substitution to whatever WAS
    /// admitted.
    #[test]
    fn pinning_column_on_a_keyed_fold_cell_refuses_at_the_choice_layer() {
        let source = "sources.raw_events";
        let plan = composed_keyed_fold_plan(source);
        let trigger = Trigger::UpstreamMutation {
            source: source.to_string(),
        };

        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: true,
        };
        let backend_caps = BackendWriteCapabilities {
            supports_merge: true,
            supports_column_scoped_merge: true,
        };
        let resolved_pattern = resolve_write_pin(
            "UpstreamMutation",
            "column",
            "duckdb",
            facts,
            backend_caps,
            |_pattern| Ok(()),
        )
        .expect("`column` resolves fine against the registry for an identity-bearing output");

        let err = resolve_cell_choice(
            &plan,
            &trigger,
            &effective_override(None, &[], "unused", &[]),
            Some(resolved_pattern),
            true,
        )
        .expect_err(
            "a registry-valid pin whose selection the cell never admitted must still refuse",
        );
        assert!(
            err.to_string().contains("MaintenanceUnboundedFootprint"),
            "refusal must name the diagnostic family: {err}"
        );
    }

    /// The narrower runtime driver resolver
    /// (`maintenance_driver::resolve_cell_technique_with_write_pin`) — the
    /// second function the review named — is consulted the same way: a
    /// `write: region` pin overrides a live `ColumnScopedMerge` cell's own
    /// default to region recompute, a `write: column` pin reaffirms it, and
    /// a pin selecting a technique this narrow (`ColumnScopedMerge` vs
    /// region-only) resolver has no lowering for (`keyed`) refuses rather
    /// than silently falling back.
    #[test]
    fn driver_resolve_cell_technique_consults_the_write_pin() {
        let plan = admitted_plan("users");
        let trigger = Trigger::UpstreamMutation {
            source: "users".to_string(),
        };

        // Unpinned default: the live, admitted ColumnScopedMerge cell.
        let unpinned = resolve_cell_technique_with_write_pin(&plan, &trigger, None, None, true)
            .expect("unpinned resolution must not refuse");
        assert_eq!(unpinned, ResolvedTechnique::ColumnScopedMerge);

        // `write: region` overrides that default to region recompute — a
        // real, non-vacuous behaviour change caused by the pin.
        let region_pattern = lookup_write_pattern("region").expect("registered pattern");
        let region_pinned = resolve_cell_technique_with_write_pin(
            &plan,
            &trigger,
            None,
            Some(region_pattern),
            true,
        )
        .expect("a region pin must resolve, not refuse");
        assert_eq!(region_pinned, ResolvedTechnique::RegionRecompute);
        assert_ne!(region_pinned, unpinned);

        // `write: column` reaffirms the admitted, live technique.
        let column_pattern = lookup_write_pattern("column").expect("registered pattern");
        let column_pinned = resolve_cell_technique_with_write_pin(
            &plan,
            &trigger,
            None,
            Some(column_pattern),
            true,
        )
        .expect("a column pin on an admitted, live cell must resolve");
        assert_eq!(column_pinned, ResolvedTechnique::ColumnScopedMerge);

        // `write: keyed` selects a technique (`KeyedFold`) this narrow
        // resolver has no lowering for — refuses fail-loud rather than
        // silently substituting a different technique than the one pinned.
        let keyed_pattern = lookup_write_pattern("keyed").expect("registered pattern");
        let err =
            resolve_cell_technique_with_write_pin(&plan, &trigger, None, Some(keyed_pattern), true)
                .expect_err("a pin selecting a technique this resolver can't lower must refuse");
        assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));
    }

    /// Pins the resolvable set member `write: keyed` (selects
    /// `Technique::KeyedFold`, this cell's own admitted technique) directly
    /// through `lookup_write_pattern` + `resolve_cell_choice`: proves
    /// `admits_write_selection`'s equality check on the exact-technique arm
    /// (`selection == Technique(t)`, `t != ColumnScopedMerge`) admits when
    /// the pinned technique equals the plan's admitted technique. Kills the
    /// `admits_write_selection` `==` → `!=` mutant on that arm together with
    /// `pinning_update_on_a_keyed_fold_cell_refuses` below.
    #[test]
    fn pinning_keyed_on_a_keyed_fold_cell_admits() {
        let source = "sources.raw_events";
        let plan = composed_keyed_fold_plan(source);
        let trigger = Trigger::UpstreamMutation {
            source: source.to_string(),
        };

        let keyed_pattern = lookup_write_pattern("keyed").expect("registered pattern");
        let chosen = resolve_cell_choice(
            &plan,
            &trigger,
            &effective_override(None, &[], "unused", &[]),
            Some(keyed_pattern),
            true,
        )
        .expect("a keyed pin matching the cell's own admitted KeyedFold technique must resolve");
        assert_eq!(chosen, ChosenTechnique::Admitted(Technique::KeyedFold));
    }

    /// Pins `write: update` (selects `Technique::InPlaceUpdate`) against the
    /// same `KeyedFold`-admitted plan: the pinned technique does NOT equal
    /// the plan's admitted technique, so `admits_write_selection` must
    /// refuse. With the `==` → `!=` mutant this would wrongly admit,
    /// substituting a technique the plan never derived.
    #[test]
    fn pinning_update_on_a_keyed_fold_cell_refuses() {
        let source = "sources.raw_events";
        let plan = composed_keyed_fold_plan(source);
        let trigger = Trigger::UpstreamMutation {
            source: source.to_string(),
        };

        let update_pattern = lookup_write_pattern("update").expect("registered pattern");
        let err = resolve_cell_choice(
            &plan,
            &trigger,
            &effective_override(None, &[], "unused", &[]),
            Some(update_pattern),
            true,
        )
        .expect_err(
            "an update pin selecting a technique the plan never admitted must refuse, not \
             substitute the admitted technique",
        );
        assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));
    }
}

/// T3 over external sources — the point-lookup enrichment recompute
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// F5): `examples/timeseries/models/daily_events_enriched.sql`'s
/// `raw.users` source now declares `unique_key: [user_id]` +
/// `referential_integrity: [user_id]`
/// (`examples/timeseries/models/sources/raw/users.yml`), so its
/// `{user_name}` `UpstreamMutation` cell's enrichment join now closes P1
/// (`skeleton_closure_pinned.rs`'s discriminating pair) and the fingerprint
/// sidecar's synthesized changed-key set (F3/F4) licenses a delta-restricted
/// recompute (`choice::resolve_recompute_restriction` — the SAME gate E3
/// built for model edges, unioned onto this external-source cell by
/// `derive::mutation_enrichment_closure`).
///
/// **Known production gap** (documented here, not silently worked around):
/// this restriction is not yet dispatched live by `execute.rs`'s regular
/// incremental batch loop — `resolve_live_column_scoped_cell`/
/// `execute_column_scoped_merge_full`'s call site in `crates/smelt-runtime/
/// src/execute.rs` is outside this phase's allowed files (only `crates/
/// smelt-logical/src/maintenance/{derive,choice,emit}.rs` and this test
/// file are). Mirroring `real_fixture_daily_events_status_would_admit_
/// partition_local_yes_cell` above and `fingerprint_sidecar.rs`'s own
/// `apply_changed_keys` doc comment ("this is the minimal per-key delta
/// application the T3 licence union (Phase F5) will later wire into the
/// real maintenance driver; here it is test-local scaffolding"), these
/// tests prove the mechanism — the derived plan cell, the sidecar-derived
/// exact delta, the licence decision, and the emitted delta-restricted
/// statement — is correctly engineered end to end against a real DuckDB
/// backend, driven directly rather than through `execute_project`.
mod external_source_point_lookup_recompute {
    use std::collections::BTreeMap;
    use std::path::Path;

    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_logical::analysis::fingerprint::Projection;
    use smelt_logical::maintenance::choice::{
        enrichment_restrict_column, resolve_recompute_restriction, RecomputeRestriction,
    };
    use smelt_logical::maintenance::derive::{
        derive_maintenance_plan_with_referential_integrity, ModelInputs,
    };
    use smelt_logical::maintenance::emit::{
        emit_count_preservation_probe, emit_delete_insert_delta_restricted, MaintenanceDialect,
        Region,
    };
    use smelt_logical::maintenance::{
        Grain, MutationProfile, OutputSpec, SkeletonSourceClosure, SourceFacts, Trigger,
    };
    use smelt_runtime::maintenance_driver::{
        diff_fingerprint_sidecar_changed_keys, refresh_fingerprint_sidecar,
    };

    /// The real fixture's SQL body (frontmatter stripped), read straight off
    /// disk so this suite can never silently drift from the file
    /// `skeleton_closure_pinned.rs` also pins.
    fn model_sql_body() -> (smelt_core::ModelMetadata, String) {
        let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/timeseries")
            .canonicalize()
            .expect("examples/timeseries exists");
        let text = std::fs::read_to_string(project_dir.join("models/daily_events_enriched.sql"))
            .expect("read daily_events_enriched.sql");
        let smelt_core::FileMetadata::Single {
            metadata,
            sql_offset,
        } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
        else {
            panic!("daily_events_enriched.sql must be a single-model file");
        };
        (*metadata, text[sql_offset..].to_string())
    }

    /// `raw.users`' declared `unique_key: [user_id]` doubles as
    /// `SourceFacts::unique_key` (P1 conjunct 3's one-to-one fact) — a
    /// SourceFacts list built by hand, not `smelt-db::build_source_facts`
    /// (which does not populate `unique_key` yet, per `execute.rs`'s own
    /// documented gap), matching `real_fixture_daily_events_status_would_
    /// admit_partition_local_yes_cell`'s established pattern above.
    fn source_facts() -> Vec<SourceFacts> {
        vec![
            SourceFacts {
                name: "raw.events".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: None,
                unique_key: vec!["event_id".to_string()],
                allow_full_scan: false,
            },
            SourceFacts {
                name: "raw.users".to_string(),
                mutation: MutationProfile::MutableSnapshot,
                partition_col: None,
                unique_key: vec!["user_id".to_string()],
                allow_full_scan: true,
            },
        ]
    }

    /// Derive the plan and return the `{user_name}` `UpstreamMutation`
    /// cell's own `skeleton_source_closure` verdict — the P1 wiring this
    /// phase's `derive::mutation_enrichment_closure` adds.
    fn user_name_cell_closure() -> Option<SkeletonSourceClosure> {
        let (metadata, sql_body) = model_sql_body();
        let sources = source_facts();
        let partition_col = metadata
            .timeseries
            .as_ref()
            .map(|t| t.partition_column.clone());
        let skeleton = smelt_logical::maintenance::skeleton::skeleton_columns(
            &sql_body,
            &[],
            partition_col.as_deref(),
        );
        let grouping = smelt_logical::maintenance::grouping::derive_column_groups(
            &sql_body, &sources, &skeleton,
        );
        assert!(
            grouping.degenerate.is_empty(),
            "expected no degenerate column-group collapses: {:?}",
            grouping.degenerate
        );

        let inputs = ModelInputs {
            sql: &sql_body,
            output: OutputSpec {
                table: "daily_events_enriched".to_string(),
                grain: Grain::Partition {
                    partition_col: partition_col.unwrap_or_default(),
                },
                skeleton_columns: skeleton,
            },
            sources,
            column_groups: grouping.groups,
            fold: None,
            column_add_proof: None,
        };

        let mut source_ri = BTreeMap::new();
        source_ri.insert("raw.users".to_string(), vec!["user_id".to_string()]);

        let trigger = Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        };
        let plan = derive_maintenance_plan_with_referential_integrity(
            &inputs,
            std::slice::from_ref(&trigger),
            &source_ri,
        );
        assert!(
            plan.refusals.is_empty(),
            "expected no admission refusals: {:?}",
            plan.refusals
        );
        let cell = plan
            .cell_for(&trigger)
            .unwrap_or_else(|| panic!("no cell admitted for {trigger:?}: {plan:#?}"));
        cell.skeleton_source_closure.clone()
    }

    /// The declared-facts variant of the real fixture's `{user_name}` cell
    /// closes P1 through `derive_maintenance_plan_with_referential_
    /// integrity` — the same verdict `skeleton_closure_pinned.rs` proves
    /// directly against `skeleton_source_closure`, now reached through the
    /// full plan-derivation path (`ModelInputs` → `derive_mutation` →
    /// `mutation_enrichment_closure`) a real caller would use.
    #[test]
    fn closure_admits_and_restrict_column_resolves() {
        let closure = user_name_cell_closure();
        assert_eq!(closure, Some(SkeletonSourceClosure::Closed));

        let dimension_key = ["user_id".to_string()];
        let restrict_column = enrichment_restrict_column(&dimension_key);
        assert_eq!(restrict_column, Some("user_id"));

        // Absent an RI fact (`derive_maintenance_plan`'s own default path),
        // the SAME cell shape must still carry no closure verdict at all —
        // proving the opt-in wiring is additive, never a default-on change.
        let (metadata, sql_body) = model_sql_body();
        let sources = source_facts();
        let partition_col = metadata
            .timeseries
            .as_ref()
            .map(|t| t.partition_column.clone());
        let skeleton = smelt_logical::maintenance::skeleton::skeleton_columns(
            &sql_body,
            &[],
            partition_col.as_deref(),
        );
        let grouping = smelt_logical::maintenance::grouping::derive_column_groups(
            &sql_body, &sources, &skeleton,
        );
        let inputs = ModelInputs {
            sql: &sql_body,
            output: OutputSpec {
                table: "daily_events_enriched".to_string(),
                grain: Grain::Partition {
                    partition_col: partition_col.unwrap_or_default(),
                },
                skeleton_columns: skeleton,
            },
            sources,
            column_groups: grouping.groups,
            fold: None,
            column_add_proof: None,
        };
        let trigger = Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        };
        let default_plan = smelt_logical::maintenance::derive::derive_maintenance_plan(
            &inputs,
            std::slice::from_ref(&trigger),
        );
        let default_cell = default_plan.cell_for(&trigger).expect("cell admitted");
        assert_eq!(
            default_cell.skeleton_source_closure, None,
            "derive_maintenance_plan (no RI facts supplied) must stay byte-identical to its \
             pre-F5 behaviour — None, never an attempted-and-open verdict"
        );
    }

    /// The digest columns (`user_name` only, per `analysis::fingerprint::
    /// fingerprint_projection`'s P4 derivation) a fingerprint sidecar
    /// digests over `raw.users` for this model.
    fn projection() -> Projection {
        Projection::Columns(["user_name".to_string()].into_iter().collect())
    }

    fn all_users_columns() -> Vec<String> {
        vec![
            "user_id".to_string(),
            "user_name".to_string(),
            "signup_date".to_string(),
        ]
    }

    fn empty_write_group() -> smelt_backend::StatementGroup {
        smelt_backend::StatementGroup {
            statements: vec![],
            transactional: false,
        }
    }

    async fn seed(backend: &DuckDbBackend) {
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
                 event_type VARCHAR, event_timestamp TIMESTAMP)",
            )
            .await
            .expect("create events source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_events VALUES \
                 (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
                 (2, 1, 'click', TIMESTAMP '2025-01-10 09:00:00'), \
                 (3, 2, 'login', TIMESTAMP '2025-01-10 10:00:00'), \
                 (4, 2, 'click', TIMESTAMP '2025-01-10 11:00:00'), \
                 (5, 3, 'login', TIMESTAMP '2025-01-10 12:00:00'), \
                 (6, 3, 'click', TIMESTAMP '2025-01-10 13:00:00')",
            )
            .await
            .expect("seed events");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), \
                 (2, 'Bob', DATE '2025-01-02'), \
                 (3, 'Carol', DATE '2025-01-03')",
            )
            .await
            .expect("seed users");
    }

    fn enrichment_select(events_table: &str, users_table: &str) -> String {
        format!(
            "SELECT e.event_id, CAST(e.event_timestamp AS DATE) AS event_date, e.user_id, \
             e.event_type, u.user_name FROM {events_table} e JOIN {users_table} u ON \
             e.user_id = u.user_id"
        )
    }

    async fn user_names(backend: &DuckDbBackend) -> Vec<(i64, String)> {
        let batches = backend
            .execute_sql(
                "SELECT user_id, user_name FROM main.daily_events_enriched ORDER BY user_id, \
                 event_id",
            )
            .await
            .expect("read maintained table");
        let mut out = Vec::new();
        for batch in &batches {
            use arrow::array::{Array, Int32Array, StringArray};
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .expect("user_id is INTEGER");
            let names = batch
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("user_name is VARCHAR");
            for i in 0..batch.num_rows() {
                out.push((ids.value(i) as i64, names.value(i).to_string()));
            }
        }
        out
    }

    async fn except_all_count(backend: &DuckDbBackend, left: &str, right: &str) -> i64 {
        let sql = format!("SELECT count(*) FROM (({left}) EXCEPT ALL ({right})) AS d");
        let batches = backend.execute_sql(&sql).await.expect("except all query");
        use arrow::array::Int64Array;
        let batch = &batches[0];
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("COUNT(*) is BIGINT");
        col.value(0)
    }

    /// One renamed user out of three: the delta-restricted recompute's
    /// emitted statements carry the semi-join predicate on `user_id`, touch
    /// only that user's 2 fact rows, leave the other 4 rows byte-identical,
    /// and the maintained table still matches a from-scratch full-refresh
    /// oracle over the source's current state. The count-preservation
    /// tripwire also passes (clean data, no dangling `user_id`).
    #[tokio::test]
    async fn point_lookup_recompute_touches_only_the_renamed_users_rows() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("point_lookup.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        seed(&backend).await;

        let body = enrichment_select("main.sources_raw_events", "main.sources_raw_users");
        backend
            .execute_sql(&format!(
                "CREATE TABLE main.daily_events_enriched AS {body}"
            ))
            .await
            .expect("baseline full refresh");

        // Populate the sidecar against the ORIGINAL (pre-rename) content —
        // the baseline every subsequent diff compares against.
        let (_, sql_body) = model_sql_body();
        refresh_fingerprint_sidecar(
            &backend,
            "main",
            "smelt.sources.raw.users",
            "main.sources_raw_users",
            &["user_id".to_string()],
            &projection(),
            &all_users_columns(),
            &sql_body,
            &empty_write_group(),
        )
        .await
        .expect("populate baseline sidecar");

        // Rename user 1 — the ONLY declared-projection column that changed.
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("rename user 1");

        let changed_keys = diff_fingerprint_sidecar_changed_keys(
            &backend,
            "main",
            "smelt.sources.raw.users",
            "main.sources_raw_users",
            &["user_id".to_string()],
            &projection(),
            &all_users_columns(),
            &sql_body,
        )
        .await
        .expect("diff sidecar");
        assert_eq!(
            changed_keys,
            vec!["1".to_string()],
            "renaming exactly 1 of 3 users must synthesize exactly that user's changed-key set"
        );

        let closure = user_name_cell_closure();
        let restriction = resolve_recompute_restriction(closure.as_ref(), Some(&changed_keys));
        let RecomputeRestriction::Restricted { delta_keys } = restriction else {
            panic!("expected Restricted, got {restriction:?}");
        };
        let dimension_key = ["user_id".to_string()];
        let restrict_column =
            enrichment_restrict_column(&dimension_key).expect("single-column key");

        let region = Region {
            start: "'2025-01-10'".to_string(),
            end: "'2025-01-11'".to_string(),
        };
        let group = emit_delete_insert_delta_restricted(
            "main.daily_events_enriched",
            "event_date",
            &region,
            &body,
            restrict_column,
            &delta_keys,
            MaintenanceDialect::DuckDb,
        );
        assert!(
            group.statements[0].sql.contains("user_id IN ('1')"),
            "DELETE must carry the semi-join predicate: {}",
            group.statements[0].sql
        );
        assert!(
            group.statements[1].sql.contains("user_id IN ('1')"),
            "INSERT must carry the semi-join predicate: {}",
            group.statements[1].sql
        );

        backend
            .execute_statement_group(&group)
            .await
            .expect("execute delta-restricted recompute");

        let names = user_names(&backend).await;
        assert_eq!(
            names,
            vec![
                (1, "Alicia".to_string()),
                (1, "Alicia".to_string()),
                (2, "Bob".to_string()),
                (2, "Bob".to_string()),
                (3, "Carol".to_string()),
                (3, "Carol".to_string()),
            ],
            "only user 1's 2 rows change; users 2 and 3's rows are untouched"
        );

        // End state equals a from-scratch full refresh of the CURRENT
        // source state — the row-count-preserving semi-join restriction
        // did not silently under- or over-write.
        let oracle = enrichment_select("main.sources_raw_events", "main.sources_raw_users");
        let maintained = "SELECT * FROM main.daily_events_enriched".to_string();
        let left_only = except_all_count(&backend, &maintained, &oracle).await;
        let right_only = except_all_count(&backend, &oracle, &maintained).await;
        assert_eq!(
            (left_only, right_only),
            (0, 0),
            "the delta-restricted recompute must match a full-refresh oracle exactly"
        );

        // The RI count-preservation tripwire: clean data (every fact
        // user_id has a matching dimension row) — no violation.
        let driving_select = "SELECT event_id FROM main.sources_raw_events WHERE CAST(\
             event_timestamp AS DATE) >= '2025-01-10' AND CAST(event_timestamp AS DATE) < \
             '2025-01-11'"
            .to_string();
        let enriched_select = format!(
            "{} WHERE CAST(e.event_timestamp AS DATE) >= '2025-01-10' AND CAST(\
             e.event_timestamp AS DATE) < '2025-01-11'",
            enrichment_select("main.sources_raw_events", "main.sources_raw_users")
        );
        let probe = emit_count_preservation_probe(&driving_select, &enriched_select);
        let (driving_count, enriched_count) = run_count_preservation_probe(&backend, &probe).await;
        assert_eq!(
            driving_count, enriched_count,
            "clean data: the count-preservation tripwire must not fire"
        );
    }

    /// Execute an [`emit_count_preservation_probe`] statement and read back
    /// its `(driving_count, enriched_count)` pair.
    async fn run_count_preservation_probe(
        backend: &DuckDbBackend,
        probe: &smelt_logical::maintenance::emit::MaintenanceStatement,
    ) -> (i64, i64) {
        let batches = backend
            .execute_sql(&probe.sql)
            .await
            .expect("execute count-preservation probe");
        use arrow::array::Int64Array;
        let batch = &batches[0];
        let driving = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("driving_count is BIGINT")
            .value(0);
        let enriched = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("enriched_count is BIGINT")
            .value(0);
        (driving, enriched)
    }

    /// The count-preservation tripwire's negative leg: a dangling fact key
    /// (an event whose `user_id` has no matching dimension row) makes the
    /// inner-join enrichment's row count fall short of the driving side's —
    /// the declared `referential_integrity` is disproven, and the check
    /// (mirroring the not-yet-wired `SourceCountPreservationViolated`
    /// runtime failure — see this module's own doc comment) fails loudly
    /// rather than silently trusting a violated declaration.
    #[tokio::test]
    async fn violated_referential_integrity_fails_the_tripwire_loudly() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("violated_ri.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
                 event_type VARCHAR, event_timestamp TIMESTAMP)",
            )
            .await
            .expect("create events source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_events VALUES \
                 (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
                 (2, 99, 'login', TIMESTAMP '2025-01-10 09:00:00')",
            )
            .await
            .expect("seed events (event 2 has a dangling user_id 99)");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES (1, 'Alice', DATE '2025-01-01')",
            )
            .await
            .expect(
                "seed users (no row for user 99 — the declared referential_integrity is false)",
            );

        let driving_select =
            "SELECT event_id FROM main.sources_raw_events WHERE CAST(event_timestamp AS DATE) \
             >= '2025-01-10' AND CAST(event_timestamp AS DATE) < '2025-01-11'"
                .to_string();
        let enriched_select = format!(
            "{} WHERE CAST(e.event_timestamp AS DATE) >= '2025-01-10' AND CAST(\
             e.event_timestamp AS DATE) < '2025-01-11'",
            enrichment_select("main.sources_raw_events", "main.sources_raw_users")
        );
        let probe = emit_count_preservation_probe(&driving_select, &enriched_select);
        let (driving_count, enriched_count) = run_count_preservation_probe(&backend, &probe).await;

        assert_eq!(driving_count, 2, "both events are the driving side");
        assert_eq!(
            enriched_count, 1,
            "the inner join drops event 2 — its user_id 99 has no dimension row"
        );

        let result = check_count_preservation(driving_count, enriched_count, "raw.users");
        assert!(
            result.is_err(),
            "a violated referential_integrity must fail the tripwire, not pass it silently"
        );
        assert!(result
            .unwrap_err()
            .contains("SourceCountPreservationViolated"));
    }

    /// Test-local scaffolding for the not-yet-wired runtime consumer of
    /// [`emit_count_preservation_probe`]'s result (`docs/specs/sources.md`
    /// §"Referential integrity" — the tripwire "fails the run loudly,
    /// transactionally"). `crates/smelt-runtime/src/maintenance_driver.rs`
    /// is outside this phase's allowed files (see this module's own doc
    /// comment); this mirrors exactly what that future call site must do.
    fn check_count_preservation(
        driving_count: i64,
        enriched_count: i64,
        source: &str,
    ) -> Result<(), String> {
        if enriched_count < driving_count {
            Err(format!(
                "SourceCountPreservationViolated: '{source}' declares referential_integrity, but \
                 an enrichment join over the touched region returned {enriched_count} row(s) \
                 against {driving_count} driving row(s) — some driving row's join key has no \
                 match in the dimension"
            ))
        } else {
            Ok(())
        }
    }
}
