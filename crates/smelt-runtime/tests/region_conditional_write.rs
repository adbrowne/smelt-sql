//! Live-equivalence coverage for the region `DeleteInsert` family's
//! change-suppressed conditional variant (`docs/outcomes/
//! 20260815-definition-delta-migrate/phases/27b-plan.md`): a real DuckDB
//! backend proves the staged write realised by `RegionWrite::Suppressed`
//! (update leg + complete delete leg + insert leg) leaves a region's
//! contents equal to a full recompute, and a row whose key departs the
//! region is actually deleted (the delete leg's own reason to exist).

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::choice::RegionWrite;
use smelt_logical::maintenance::emit::{MaintenanceDialect, Region};
use smelt_runtime::maintenance_driver::{
    execute_delete_insert_with_delta_restriction, RestrictionDeltaSource,
};
use tempfile::TempDir;

const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "region-conditional-write-test",
        model_name: "region-conditional-write-test",
        reporter: &NO_OP_REPORTER,
    }
}

fn region() -> Region {
    Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    }
}

fn suppressed() -> RegionWrite {
    RegionWrite::Suppressed {
        key: vec!["region_id".to_string()],
        compared_columns: vec!["amount".to_string()],
    }
}

/// A second run over unchanged data leaves the region's contents equal to
/// a full refresh: the update leg's `IS DISTINCT FROM` guard matches
/// nothing, the delete leg finds every stored row still present in the
/// candidate, and the insert leg finds every candidate row already stored.
#[tokio::test]
async fn a_run_over_unchanged_data_leaves_the_region_untouched() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.regions (region_id VARCHAR, region_date DATE, amount INTEGER)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.regions VALUES \
             ('r1', '2026-07-01', 10), ('r2', '2026-07-01', 20)",
        )
        .await
        .unwrap();

    let body = "SELECT region_id, region_date, amount FROM (VALUES \
                ('r1', DATE '2026-07-01', 10), ('r2', DATE '2026-07-01', 20)) \
                AS t(region_id, region_date, amount)";

    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "regions",
        "region_date",
        &region(),
        body,
        body,
        None,
        None,
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "sources.regions_raw",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        Some(&suppressed()),
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("suppressed region recompute executes");

    assert!(group.transactional);
    assert!(group
        .statements
        .iter()
        .any(|s| s.sql.contains("IS DISTINCT FROM")));

    let batches = backend
        .execute_sql("SELECT region_id, amount FROM main.regions ORDER BY region_id")
        .await
        .unwrap();
    let rows = region_amount_rows(&batches);
    assert_eq!(
        rows,
        vec![("r1".to_string(), 10), ("r2".to_string(), 20)],
        "unchanged data must leave the region's stored contents equal to a full refresh"
    );
}

/// A run whose candidate no longer contains a previously-stored key (the key
/// "departed the region") deletes exactly that row — the complete delete
/// leg's own coverage, distinct from the update leg's `IS DISTINCT FROM`
/// guard.
#[tokio::test]
async fn a_departed_key_is_deleted_a_changed_value_is_updated_a_new_key_is_inserted() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.regions (region_id VARCHAR, region_date DATE, amount INTEGER)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.regions VALUES \
             ('r1', '2026-07-01', 10), ('r2', '2026-07-01', 20), ('r3', '2026-07-01', 30)",
        )
        .await
        .unwrap();

    // r1 unchanged, r2 changed (20 -> 25), r3 departed (absent from the
    // candidate), r4 is new.
    let body = "SELECT region_id, region_date, amount FROM (VALUES \
                ('r1', DATE '2026-07-01', 10), ('r2', DATE '2026-07-01', 25), \
                ('r4', DATE '2026-07-01', 40)) AS t(region_id, region_date, amount)";

    execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "regions",
        "region_date",
        &region(),
        body,
        body,
        None,
        None,
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "sources.regions_raw",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        Some(&suppressed()),
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("suppressed region recompute executes");

    let batches = backend
        .execute_sql("SELECT region_id, amount FROM main.regions ORDER BY region_id")
        .await
        .unwrap();
    let rows = region_amount_rows(&batches);
    assert_eq!(
        rows,
        vec![
            ("r1".to_string(), 10),
            ("r2".to_string(), 25),
            ("r4".to_string(), 40),
        ],
        "r1 stays unchanged, r2's changed value is written, r3 (departed) is deleted, r4 (new) \
         is inserted: {rows:?}"
    );
}

fn region_amount_rows(batches: &[arrow::record_batch::RecordBatch]) -> Vec<(String, i32)> {
    use arrow::array::{Array, Int32Array, StringArray};
    let mut rows = Vec::new();
    for batch in batches {
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let amounts = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        for i in 0..batch.num_rows() {
            rows.push((ids.value(i).to_string(), amounts.value(i)));
        }
    }
    rows
}
