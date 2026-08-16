//! Regression coverage for the Phase 6 reviewer finding
//! (`docs/plans/20260809-sensitivity-precision.md` Phase 6): the
//! definition-change backfill must be atomic with its schema migration.
//!
//! Before this fix, `smelt-runtime::execute_project` ran the
//! `ALTER TABLE ... ADD COLUMN` (via `schema_evolution::check_and_migrate`,
//! which also saves the deployed-schema snapshot immediately afterward) and
//! the `Technique::InPlaceUpdate` backfill `UPDATE`
//! (`maintenance_driver::execute_in_place_update`) as two INDEPENDENT
//! `StatementGroup`s. A crash between the two left the column physically
//! present but permanently `NULL`: the next run's deployed-schema snapshot
//! already includes the column, so `diff_deployed_columns` sees no delta,
//! the `Trigger::ColumnAdded` cell never re-derives, and there is no repair
//! path.
//!
//! The fix folds the `InPlaceUpdate` cell's derived backfill assignment
//! into the SAME `backfill_exprs` map the pre-existing declared
//! (`backfill:`/`default:` frontmatter) mechanism already threads through
//! `check_and_migrate` — which already emits `ADD COLUMN` + `UPDATE` into
//! ONE `StatementGroup`, executed via `Backend::execute_statement_group`,
//! with `save_schema` only called after that group returns `Ok`. Routing
//! the derived assignment through the same map makes it atomic with the
//! migration for free, and — critically — means a failure anywhere in the
//! group (transactional DDL backends) leaves NEITHER the physical column
//! NOR the saved schema snapshot changed, preserving the repair path: the
//! next run's diff still sees the column missing and retries the whole
//! migration+backfill from scratch.
//!
//! These tests exercise `schema_evolution::check_and_migrate` directly —
//! the exact function `smelt-runtime::execute.rs` now calls with the
//! `Trigger::ColumnAdded` cell's assignments folded into `backfill_exprs`
//! (see `execute.rs`'s "Definition-change trigger" comment block) — using
//! the SAME production cell-resolution entry point
//! (`maintenance_driver::resolve_live_in_place_update_cell`) execute.rs
//! calls, so the assignment under test is the real one production would
//! compute, not a hand-rolled stand-in.

use std::collections::HashMap;
use std::path::Path;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Grain, Materialization, RefreshStrategy, StateMode, TimeseriesConfig};
use smelt_core::{Granularity, ModelMetadata};
use smelt_logical::maintenance::SourceFacts;
use smelt_runtime::maintenance_driver::resolve_live_in_place_update_cell;
use smelt_runtime::schema_evolution::{check_and_migrate, ddl_backend_for_dialect};
use smelt_runtime::{NoOpReporter, RetryPolicy};
use smelt_state::file_store::FileStore;
use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};

const NO_OP_REPORTER: NoOpReporter = NoOpReporter;
fn no_retry_policy() -> RetryPolicy<'static> {
    RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "schema-migration-backfill-atomicity",
        model_name: "events_enriched",
        reporter: &NO_OP_REPORTER,
    }
}

/// `val_doubled AS val * 2` reads only the already-stored `val` column —
/// `classify_definition_change` must admit `DefinitionChangeClass::
/// PureBackfill` and the derived plan must carry a `Technique::
/// InPlaceUpdate` cell for it, the same fixture `technique_lowering.rs`'s
/// `in_place_update_lowering` module uses.
const SQL: &str =
    "SELECT event_date, user_id, val, val * 2 AS val_doubled FROM smelt.sources.events";

fn metadata() -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Partition),
        materialization: Some(Materialization::Table),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    }
}

fn sources() -> Vec<SourceFacts> {
    vec![SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }]
}

/// The OLD deployed-schema snapshot (before `val_doubled` existed) — what
/// `execute.rs` reads as `deployed_column_names` before this run's own
/// migration.
fn old_column_names() -> Vec<String> {
    vec![
        "event_date".to_string(),
        "user_id".to_string(),
        "val".to_string(),
    ]
}

fn inferred_columns() -> Vec<DeployedColumn> {
    vec![
        DeployedColumn {
            name: "event_date".to_string(),
            data_type: "DATE".to_string(),
            nullable: false,
        },
        DeployedColumn {
            name: "user_id".to_string(),
            data_type: "BIGINT".to_string(),
            nullable: false,
        },
        DeployedColumn {
            name: "val".to_string(),
            data_type: "BIGINT".to_string(),
            nullable: false,
        },
        DeployedColumn {
            name: "val_doubled".to_string(),
            data_type: "BIGINT".to_string(),
            nullable: true,
        },
    ]
}

fn extract_i64(batches: &[duckdb::arrow::array::RecordBatch]) -> i64 {
    batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<duckdb::arrow::array::Int64Array>()
        .expect("count column must be Int64")
        .value(0)
}

async fn setup_backend(db_path: &Path) -> DuckDbBackend {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.events_enriched (
            event_date DATE,
            user_id BIGINT,
            val BIGINT
        );
        INSERT INTO main.events_enriched VALUES
            (DATE '2024-01-01', 1, 10),
            (DATE '2024-01-01', 2, 20),
            (DATE '2024-01-02', 1, 30);
        "#,
    )
    .expect("create + seed events_enriched");
    drop(conn);
    DuckDbBackend::new(db_path, "main")
        .await
        .expect("open backend")
}

fn save_old_schema(file_store: &FileStore) {
    // Match the "old" columns' types/nullability to what `inferred_columns`
    // (below) reports for them, so `diff_schemas` sees ONLY the
    // `val_doubled` addition — never a spurious type/nullability change on
    // `event_date`/`user_id`/`val` that would divert `plan_migration_for_
    // backend` into `FullRefresh` instead of `AlterTable`.
    let inferred = inferred_columns();
    file_store
        .save_schema(&DeployedSchema {
            model: "events_enriched".to_string(),
            version: 1,
            deployed_at: chrono::Utc::now(),
            model_hash: "sha256:old".to_string(),
            definition_sql: String::new(),
            columns: old_column_names()
                .into_iter()
                .map(|name| {
                    let matched = inferred
                        .iter()
                        .find(|c| c.name == name)
                        .expect("old column name must be present in inferred_columns");
                    DeployedColumn {
                        name,
                        data_type: matched.data_type.clone(),
                        nullable: matched.nullable,
                    }
                })
                .collect(),
        })
        .expect("save old schema");
}

/// The real production entry point (`resolve_live_in_place_update_cell`)
/// resolves the `PureBackfill` cell and its ready-to-execute `(column,
/// expression)` assignments — this is EXACTLY what `execute.rs` computes
/// before folding it into `backfill_exprs`.
fn derived_assignments() -> Vec<(String, String)> {
    let md = metadata();
    let (_cell, assignments) = resolve_live_in_place_update_cell(
        SQL,
        "events_enriched",
        &md,
        &sources(),
        &old_column_names(),
        smelt_dialect::SqlDialect::DuckDB,
    )
    .expect("PureBackfill column-add must resolve a live InPlaceUpdate cell");
    assert_eq!(
        assignments.len(),
        1,
        "expected exactly one derived assignment: {assignments:?}"
    );
    assert_eq!(assignments[0].0, "val_doubled");
    assert_eq!(
        assignments[0].1.trim(),
        "val * 2",
        "expected the derived expression to be val's own defining expression"
    );
    assignments
}

/// GREEN: folding the derived assignment into `backfill_exprs` (exactly
/// what `execute.rs`'s merge does) makes `check_and_migrate` emit the
/// `ADD COLUMN` and the derived `UPDATE` into ONE `StatementGroup` —
/// `backfilled_columns` names it, and every existing row is correctly
/// backfilled by the end of the SAME call that added the column. No
/// separate, later, independently-fallible dispatch is needed.
#[tokio::test]
async fn derived_backfill_folds_into_migration_group_and_backfills_every_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.duckdb");
    let backend = setup_backend(&db_path).await;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);
    save_old_schema(&file_store);

    let mut backfill_exprs: HashMap<String, String> = HashMap::new();
    for (col, expr) in derived_assignments() {
        backfill_exprs.insert(col, expr);
    }
    let column_defaults: HashMap<String, String> = HashMap::new();

    let ddl_backend = ddl_backend_for_dialect(backend.dialect(), None, None);
    let retry = no_retry_policy();
    let result = check_and_migrate(
        &backend,
        &file_store,
        "events_enriched",
        SQL,
        "main",
        &inferred_columns(),
        false,
        false,
        false,
        &column_defaults,
        &backfill_exprs,
        Some(&ddl_backend),
        &retry,
    )
    .await
    .expect("migration must succeed");

    match &result {
        smelt_runtime::schema_evolution::SchemaEvolutionResult::Migrated {
            backfilled_columns,
            ..
        } => {
            assert_eq!(
                backfilled_columns,
                &vec!["val_doubled".to_string()],
                "the derived column must be reported as backfilled by the migration group: {result:?}"
            );
        }
        other => panic!("expected Migrated, got {other:?}"),
    }

    // Every existing row must already be backfilled — the physical ADD
    // COLUMN and the UPDATE ran in the SAME call, no separate dispatch.
    let total = extract_i64(
        &backend
            .execute_sql("SELECT COUNT(*) FROM main.events_enriched")
            .await
            .expect("count total"),
    );
    assert_eq!(total, 3, "expected 3 seeded rows");
    let mismatched = extract_i64(
        &backend
            .execute_sql(
                "SELECT COUNT(*) FROM main.events_enriched \
                 WHERE val_doubled IS NULL OR val_doubled != val * 2",
            )
            .await
            .expect("count mismatched"),
    );
    assert_eq!(
        mismatched, 0,
        "no row may be left with a NULL/incorrect val_doubled after the migration group"
    );

    // The saved schema snapshot must already reflect val_doubled — it was
    // only saved AFTER the group (ADD COLUMN + UPDATE) committed.
    let saved = file_store
        .load_schema("events_enriched")
        .expect("load schema")
        .expect("schema must be saved");
    assert!(
        saved.columns.iter().any(|c| c.name == "val_doubled"),
        "saved schema must include val_doubled: {saved:?}"
    );
}

/// RED-pinning the atomicity guarantee the fix depends on: if the group's
/// backfill statement fails (a DuckDB transactional-DDL backend), NEITHER
/// the `ADD COLUMN` NOR `save_schema` may have taken effect. This is the
/// exact property that closes the reviewer's finding — under the OLD
/// two-independent-groups design, the `ADD COLUMN` + `save_schema` step ran
/// and committed UNCONDITIONALLY before the (formerly separate) backfill
/// ran at all, so a failure here could only ever be discovered AFTER the
/// column was already permanently orphaned in the saved snapshot. With the
/// merge, a failing backfill expression fails the whole group before
/// `save_schema` is ever reached, leaving the column genuinely absent so
/// the next run's diff retries the whole migration+backfill from scratch —
/// there is no window in which the schema snapshot outruns the physical
/// column's real values.
#[tokio::test]
async fn failing_backfill_expression_rolls_back_the_whole_group_no_orphan_column() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.duckdb");
    let backend = setup_backend(&db_path).await;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);
    save_old_schema(&file_store);

    assert!(
        backend.capabilities().supports_transactional_ddl,
        "this test's atomicity claim requires a transactional-DDL backend"
    );

    let mut backfill_exprs: HashMap<String, String> = HashMap::new();
    // A deliberately-invalid backfill expression — referencing a column
    // that does not exist — simulating a failure partway through the
    // group's execution.
    backfill_exprs.insert(
        "val_doubled".to_string(),
        "nonexistent_column * 2".to_string(),
    );
    let column_defaults: HashMap<String, String> = HashMap::new();

    let ddl_backend = ddl_backend_for_dialect(backend.dialect(), None, None);
    let retry = no_retry_policy();
    let result = check_and_migrate(
        &backend,
        &file_store,
        "events_enriched",
        SQL,
        "main",
        &inferred_columns(),
        false,
        false,
        false,
        &column_defaults,
        &backfill_exprs,
        Some(&ddl_backend),
        &retry,
    )
    .await;

    assert!(
        result.is_err(),
        "a failing backfill expression must fail the whole migration: {result:?}"
    );

    // The ADD COLUMN must have been rolled back with the failed UPDATE —
    // querying the (nonexistent) column must itself fail.
    let query = backend
        .execute_sql("SELECT val_doubled FROM main.events_enriched")
        .await;
    assert!(
        query.is_err(),
        "val_doubled must NOT exist on the table after a rolled-back migration group: {query:?}"
    );

    // The saved schema snapshot must be untouched — still the OLD columns,
    // so the next run's diff still sees val_doubled missing and retries.
    let saved = file_store
        .load_schema("events_enriched")
        .expect("load schema")
        .expect("old schema must still be present");
    assert_eq!(saved.version, 1, "save_schema must never have been called");
    assert!(
        !saved.columns.iter().any(|c| c.name == "val_doubled"),
        "the stale-but-saved-orphan-column bug: schema snapshot must not advance without the \
         physical column being real: {saved:?}"
    );
}
