//! T3 — delta-restricted compute over model edges (`docs/plans/
//! 20260715-composed-axes-conditional-maintenance.md` Phase E3).
//!
//! Real-DuckDB e2e coverage for
//! `smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction`:
//! a 3-model-chain shape (an upstream driving edge's recorded observed
//! delta feeds a downstream enrichment recompute) is exercised directly
//! against a live backend, asserting both the emitted scan predicate and
//! the actual row-level effect — only the delta's keys are touched, every
//! other row is left byte-for-byte as it was.

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::emit::{MaintenanceDialect, Region};
use smelt_logical::maintenance::{RowPreservation, SkeletonSourceClosure};
use smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction;
use tempfile::TempDir;

/// A retry policy that never retries — these tests exercise the T3
/// delta-restricted DeleteInsert dispatch directly against a real DuckDB
/// backend, outside `execute_project`, so there is no `ExecuteRequest`/run
/// reporter to derive one from (`docs/plans/20260719-prod-w2-operability.md`
/// Phase 6).
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "delta-restricted-recompute-test",
        model_name: "delta-restricted-recompute-test",
        reporter: &NO_OP_REPORTER,
    }
}

const UPSTREAM_MODEL: &str = "silver.fact";
const WINDOW_START: &str = "2026-07-01";
const WINDOW_END: &str = "2026-07-02";

async fn setup() -> (TempDir, DuckDbBackend) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

    // The downstream enrichment output: baseline rows all tagged 'OLD'.
    backend
        .execute_sql("CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.enriched VALUES \
             ('ev-1', '2026-07-01', 'OLD'), \
             ('ev-2', '2026-07-01', 'OLD'), \
             ('ev-3', '2026-07-01', 'OLD'), \
             ('ev-4', '2026-07-01', 'OLD')",
        )
        .await
        .unwrap();

    // The recompute source: every row's recomputed value is 'NEW' — an
    // unrestricted recompute would flip every row; a delta-restricted one
    // must flip only the delta's keys.
    backend
        .execute_sql(
            "CREATE TABLE main.enrichment_recompute (event_id VARCHAR, event_date DATE, tier VARCHAR)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.enrichment_recompute VALUES \
             ('ev-1', '2026-07-01', 'NEW'), \
             ('ev-2', '2026-07-01', 'NEW'), \
             ('ev-3', '2026-07-01', 'NEW'), \
             ('ev-4', '2026-07-01', 'NEW')",
        )
        .await
        .unwrap();

    (temp_dir, backend)
}

fn region() -> Region {
    Region {
        start: format!("'{WINDOW_START}'"),
        end: format!("'{WINDOW_END}'"),
    }
}

fn body() -> &'static str {
    "SELECT event_id, event_date, tier FROM main.enrichment_recompute"
}

async fn record_delta(backend: &DuckDbBackend, changed_keys: &[&str]) {
    let ensure = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend.execute_sql(&ensure).await.unwrap();
    let changed_keys_query = if changed_keys.is_empty() {
        "SELECT NULL AS delta_key, NULL AS delta_partition WHERE FALSE".to_string()
    } else {
        let keys_list = changed_keys
            .iter()
            .map(|k| format!("('{k}', NULL)"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT * FROM (VALUES {keys_list}) AS t(delta_key, delta_partition)")
    };
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        UPSTREAM_MODEL,
        WINDOW_START,
        WINDOW_END,
        &changed_keys_query,
    );
    backend.execute_sql(&upsert).await.unwrap();
}

async fn tiers(backend: &DuckDbBackend) -> Vec<(String, String)> {
    let batches = backend
        .execute_sql("SELECT event_id, tier FROM main.enriched ORDER BY event_id")
        .await
        .unwrap();
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let tiers = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            out.push((ids.value(i).to_string(), tiers.value(i).to_string()));
        }
    }
    out
}

/// The flagship case: `P1` Closed + a non-empty recorded delta on 2 of 4
/// keys — only those 2 rows flip to 'NEW'; the scan predicate names exactly
/// those keys on both the DELETE and the INSERT.
#[tokio::test]
async fn closed_with_exact_delta_touches_only_the_delta_keys() {
    let (_dir, backend) = setup().await;
    record_delta(&backend, &["ev-1", "ev-2"]).await;

    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::JoinShape,
    };
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region(),
        body(),
        Some("event_id"),
        Some(&closure),
        UPSTREAM_MODEL,
        WINDOW_START,
        WINDOW_END,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
    )
    .await
    .expect("restricted recompute executes");

    // `ARRAY_AGG(DISTINCT ...)` does not guarantee element order, so assert
    // membership rather than a fixed literal-order substring.
    for stmt in &group.statements {
        assert!(stmt.sql.contains("event_id IN ("), "{}", stmt.sql);
        assert!(stmt.sql.contains("'ev-1'"), "{}", stmt.sql);
        assert!(stmt.sql.contains("'ev-2'"), "{}", stmt.sql);
        assert!(!stmt.sql.contains("'ev-3'"), "{}", stmt.sql);
        assert!(!stmt.sql.contains("'ev-4'"), "{}", stmt.sql);
    }

    let rows = tiers(&backend).await;
    assert_eq!(
        rows,
        vec![
            ("ev-1".to_string(), "NEW".to_string()),
            ("ev-2".to_string(), "NEW".to_string()),
            ("ev-3".to_string(), "OLD".to_string()),
            ("ev-4".to_string(), "OLD".to_string()),
        ],
        "only the delta's 2 keys must flip to NEW; ev-3/ev-4 must be untouched"
    );

    // End state equals what a full refresh over the same processed input
    // would produce for the touched keys — i.e. the restricted write did
    // not desync ev-1/ev-2 from the recompute source's own value.
    let full_refresh_equivalent: Vec<(String, String)> = vec![
        ("ev-1".to_string(), "NEW".to_string()),
        ("ev-2".to_string(), "NEW".to_string()),
    ];
    let restricted_rows: Vec<(String, String)> = rows
        .into_iter()
        .filter(|(id, _)| id == "ev-1" || id == "ev-2")
        .collect();
    assert_eq!(restricted_rows, full_refresh_equivalent);
}

/// `Open` closure never restricts, even with an exact delta recorded —
/// every row flips to 'NEW', matching the ordinary widened scan.
#[tokio::test]
async fn open_closure_falls_back_to_the_widened_scan() {
    let (_dir, backend) = setup().await;
    record_delta(&backend, &["ev-1", "ev-2"]).await;

    let closure = SkeletonSourceClosure::Open {
        reason: "test".to_string(),
    };
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region(),
        body(),
        Some("event_id"),
        Some(&closure),
        UPSTREAM_MODEL,
        WINDOW_START,
        WINDOW_END,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
    )
    .await
    .expect("unrestricted recompute executes");

    assert!(!group.statements[0].sql.contains("IN ("));
    let rows = tiers(&backend).await;
    assert!(
        rows.iter().all(|(_, tier)| tier == "NEW"),
        "an Open closure must fall back to the widened scan — every row recomputes; got {rows:?}"
    );
}

/// Fallback: no observed delta was ever recorded for this window (the
/// pre-D2-upstream case) — widen-never-narrow, every row recomputes even
/// though closure is Closed.
#[tokio::test]
async fn absent_observed_delta_runs_the_ordinary_widened_scan() {
    let (_dir, backend) = setup().await;
    // No `record_delta` call — the window was never recorded.

    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::JoinShape,
    };
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region(),
        body(),
        Some("event_id"),
        Some(&closure),
        UPSTREAM_MODEL,
        WINDOW_START,
        WINDOW_END,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
    )
    .await
    .expect("unrestricted recompute executes");

    assert!(!group.statements[0].sql.contains("IN ("));
    let rows = tiers(&backend).await;
    assert!(
        rows.iter().all(|(_, tier)| tier == "NEW"),
        "an absent delta must fall back to the widened scan; got {rows:?}"
    );
}

/// A present-but-empty recorded delta (a fully-suppressed upstream run —
/// nothing changed) also falls back to the widened scan in this phase: the
/// true zero-write no-op cascade is a later phase's payoff, not this one's.
#[tokio::test]
async fn empty_observed_delta_falls_back_to_the_widened_scan() {
    let (_dir, backend) = setup().await;
    record_delta(&backend, &[]).await;

    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::JoinShape,
    };
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region(),
        body(),
        Some("event_id"),
        Some(&closure),
        UPSTREAM_MODEL,
        WINDOW_START,
        WINDOW_END,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
    )
    .await
    .expect("unrestricted recompute executes");

    assert!(!group.statements[0].sql.contains("IN ("));
    let rows = tiers(&backend).await;
    assert!(rows.iter().all(|(_, tier)| tier == "NEW"));
}
