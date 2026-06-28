#![cfg(feature = "duckdb")]
//! Acceptance test for the per-cohort-union killer demo.
//!
//! Runs `smelt build` against `examples/per_cohort_union/`, then opens the
//! resulting DuckDB file and executes the acceptance query from
//! `tests/cohort_count.test.sql` directly.
//!
//! The acceptance criterion:
//!   (SELECT COUNT(*) FROM all_cohorts_unioned)
//!   = (SUM of per-cohort filtered counts from orders)
//!
//! This test exercises Phase B reducers + Phase C reflection + Phase E1 records
//! + Phase E2 multi-model production (generator files) end-to-end.

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn project_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/per_cohort_union")
}

#[test]
fn union_row_count_matches_per_cohort_sum() {
    let project_dir = project_dir();

    // Build the workspace using `smelt build`.
    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            "dev",
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        output.status.success(),
        "smelt build failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Open the produced DuckDB file.
    let db_path = project_dir.join("target/dev.duckdb");
    assert!(
        db_path.exists(),
        "Expected DuckDB file at {:?} after build",
        db_path
    );

    let conn = duckdb::Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("Failed to open {:?}: {e}", db_path));

    // Execute the acceptance assertion: count of all_cohorts_unioned ==
    // sum of per-cohort filtered counts from orders.
    // The cohorts.yaml defines three cohorts:
    //   us_west: region='us-west-2', min_revenue=100  → row (1, 150) qualifies
    //   us_east: region='us-east-1', min_revenue=100  → row (3, 120) qualifies
    //   eu:      region='eu-west-1', min_revenue=50   → row (5, 60) qualifies
    // orders.sql has 6 rows total; only those three pass their per-cohort filter.
    // all_cohorts_unioned unions the three emitted models → 3 rows total.
    let passes: bool = conn
        .query_row(
            "SELECT \
              (SELECT COUNT(*) FROM main.all_cohorts_unioned) \
              = \
              (SELECT SUM(cnt) FROM ( \
                SELECT COUNT(*) AS cnt FROM main.orders \
                  WHERE region = 'us-west-2' AND revenue >= 100 \
                UNION ALL \
                SELECT COUNT(*) AS cnt FROM main.orders \
                  WHERE region = 'us-east-1' AND revenue >= 100 \
                UNION ALL \
                SELECT COUNT(*) AS cnt FROM main.orders \
                  WHERE region = 'eu-west-1' AND revenue >= 50 \
              ) AS sub) AS passes",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or_else(|e| panic!("acceptance query failed: {e}"));

    assert!(
        passes,
        "Acceptance test failed: all_cohorts_unioned row count does not equal \
         the sum of per-cohort row counts. The generator may not have emitted \
         all three cohort models, or the union is incomplete."
    );

    // Also verify the absolute row count is correct (3 qualifying rows).
    let union_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.all_cohorts_unioned", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|e| panic!("count query on all_cohorts_unioned failed: {e}"));

    assert_eq!(
        union_count, 3,
        "Expected 3 qualifying rows in all_cohorts_unioned (one per cohort), got {union_count}"
    );
}
