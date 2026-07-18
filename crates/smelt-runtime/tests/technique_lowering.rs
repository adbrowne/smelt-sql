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
    resolve_cell_technique, widen_horizon_for_batch, ColumnMergeDispatch, ResolvedTechnique,
};

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
        backend.supports_column_scoped_merge(),
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
        backend.supports_column_scoped_merge(),
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
    /// `.smelt/` state (`FileStore::new(project_dir)`) never lands inside
    /// the checked-in example.
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
