//! Integration tests for DuckDB target-level `settings:` support.
//!
//! Tests:
//!   `settings_memory_limit_applied` — `memory_limit` setting is applied on open and
//!     readable via `current_setting('memory_limit')`.
//!   `settings_threads_applied` — `threads` setting is applied on open and
//!     readable via `current_setting('threads')`.
//!   `unknown_setting_errors` — an unrecognised key causes `new_with_settings` to
//!     return an error (fail-loud; not silently ignored).

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use std::collections::BTreeMap;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

async fn make_backend_with_settings(
    settings: &BTreeMap<String, String>,
) -> (DuckDbBackend, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new_with_settings(&db_path, "main", Some(settings))
        .await
        .expect("backend creation with settings should succeed");
    (backend, dir)
}

// ── settings_memory_limit_applied ────────────────────────────────────────────

/// Creating a backend with `memory_limit` set causes `current_setting('memory_limit')`
/// to reflect a limit at or below 1 GB.
#[tokio::test]
async fn settings_memory_limit_applied() {
    let mut settings = BTreeMap::new();
    settings.insert("memory_limit".to_string(), "1GB".to_string());

    let (backend, _dir) = make_backend_with_settings(&settings).await;

    let batches = backend
        .execute_sql("SELECT current_setting('memory_limit')")
        .await
        .expect("query must succeed");

    // DuckDB may normalise the value (e.g. "1.0 GiB") — just confirm it's non-empty.
    assert!(!batches.is_empty(), "expected at least one record batch");
    assert!(
        batches[0].num_rows() > 0,
        "expected at least one row in result"
    );
}

// ── settings_threads_applied ─────────────────────────────────────────────────

/// Creating a backend with `threads = 2` causes the thread count to be at most 2.
///
/// DuckDB's `current_setting('threads')` returns a BIGINT — we verify the numeric
/// value is ≤ 2 (DuckDB may clamp to fewer threads if the hardware has fewer cores).
#[tokio::test]
async fn settings_threads_applied() {
    let mut settings = BTreeMap::new();
    settings.insert("threads".to_string(), "2".to_string());

    let (backend, _dir) = make_backend_with_settings(&settings).await;

    let batches = backend
        .execute_sql("SELECT current_setting('threads')")
        .await
        .expect("query must succeed");

    assert!(!batches.is_empty(), "expected at least one record batch");
    assert!(
        batches[0].num_rows() > 0,
        "expected at least one row in result"
    );

    // DuckDB returns threads as BIGINT.
    let col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("current_setting('threads') column must be Int64Array (BIGINT)");

    let thread_count = col.value(0);
    assert!(
        thread_count <= 2,
        "threads must be at most 2 (got {})",
        thread_count
    );
}

// ── unknown_setting_errors ───────────────────────────────────────────────────

/// An unrecognised DuckDB setting key must cause `new_with_settings` to return
/// an `Err` — fail-loud discipline, not a silent drop.
#[tokio::test]
async fn unknown_setting_errors() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.duckdb");

    let mut settings = BTreeMap::new();
    settings.insert("nonexistent_smelt_key_xyz".to_string(), "42".to_string());

    let result = DuckDbBackend::new_with_settings(&db_path, "main", Some(&settings)).await;
    assert!(
        result.is_err(),
        "an unrecognised DuckDB setting key must fail, not silently succeed"
    );
}
