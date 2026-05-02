#![cfg(feature = "duckdb")]
//! End-to-end regression scaffold for the three critical bugs from
//! `~/smelt_shop/FINDINGS.md` (see
//! `docs/plans/20260417-smelt-shop-0.3-followup.md`, Phase A2).
//!
//! This test shells out to the compiled `smelt` binary (NOT the library) and
//! runs `smelt build` twice against a smelt-shop-shaped workspace. Library-
//! level tests caught none of these bugs in the prior plan, so the rule for
//! this scaffold is: invoke the binary, assert behaviour from the outside.
//!
//! It is expected to FAIL on `main`. Each fix phase (B1, B2, B3) turns one
//! assertion green. When all assertions pass, the regression is closed.
//!
//! Baseline failure on main (captured 2026-04-17, before any B-phase fixes):
//!
//!     test smelt_shop_min_idempotency_and_types ... FAILED
//!
//!     thread 'smelt_shop_min_idempotency_and_types' panicked at
//!     `smelt build` (run #1) failed (exit ExitStatus(unix_wait_status(256)));
//!     stderr:
//!     Error: Dependency validation failed
//!
//!     Caused by:
//!         Dependency resolution failed:
//!           Model 'stg_orders' references undefined model/source 'order_statuses'
//!
//! That is bug #2 firing first (the dependency validator never sees the
//! seed). Once B2 lands, the second invocation will surface bug #1
//! (`DROP VIEW IF EXISTS` against an existing Table on the rebuild). Once B1
//! lands, the type assertions will surface bug #3
//! (`typeof(net_revenue) = 'BIGINT'` instead of `'DOUBLE'`).
//!
//! See also:
//!   * docs/research/20260417-0.3-regression-triage.md (root-cause analysis)
//!   * docs/plans/20260417-smelt-shop-0.3-followup-findings/A2-baseline-failures.md
//!     (verbatim failure capture against the current main)

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Number of seeded raw rows. Large enough that a SMALLINT (max 32_767)
/// COUNT(DISTINCT order_id) would visibly truncate.
const SEED_ROW_COUNT: u64 = 50_000;

/// Path to the bundled minimal example workspace.
fn workspace_template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/smelt_shop_min")
}

/// Recursively copy `src` into `dst`. Both directories must already exist.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Pre-populate `raw.orders` in the DuckDB file so the source
/// (`smelt.sources.raw.orders`) resolves at run time. Uses the embedded
/// `duckdb` crate directly — equivalent to a `load_raw.py` step.
fn seed_raw_orders(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")?;
    // Distribution: 80% OK, 15% RT, 5% CN. Gross 1..10000 (so the sum is
    // far above i32::MAX once row count climbs), qty 1..5.
    let sql = format!(
        "CREATE OR REPLACE TABLE raw.orders AS
         SELECT
             CAST(i AS INTEGER)                              AS order_id,
             CAST((i % 1000) + 1 AS INTEGER)                 AS customer_id,
             CASE
                 WHEN (i % 100) < 80 THEN 'OK'
                 WHEN (i % 100) < 95 THEN 'RT'
                 ELSE 'CN'
             END                                             AS status_code,
             CAST(((i % 9999) + 1) + 0.37 AS DOUBLE)         AS gross_amount,
             CAST((i % 5) + 1 AS INTEGER)                    AS qty
         FROM range({0}) AS t(i);",
        SEED_ROW_COUNT
    );
    conn.execute_batch(&sql)?;
    Ok(())
}

/// Compute the expected `SUM(gross_amount)` from the seeded data so the test
/// can compare against a fractional value (and detect bug #3 narrowing the
/// double to an integer that loses the cents).
fn expected_net_revenue() -> f64 {
    let mut total = 0.0f64;
    for i in 0..SEED_ROW_COUNT {
        total += ((i % 9999) as f64) + 1.0 + 0.37;
    }
    total
}

/// Locate the compiled `smelt` binary. Cargo sets `CARGO_BIN_EXE_<name>` for
/// integration tests in the package that defines the binary.
fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Run `smelt build` against the prepared workspace. Captures stdout/stderr
/// so the assertion message is informative when the bug fires.
fn run_smelt_build(project_dir: &Path, db_path: &Path, label: &str) -> std::process::Output {
    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
        ])
        // Surface backend logs for easier triage when the bug fires.
        .env("RUST_LOG", "info")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build` ({label}): {e}"));
    if !output.status.success() {
        panic!(
            "`smelt build` ({label}) failed (exit {:?}); stderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
    }
    output
}

/// Single column of a single row, returned as an owned String. Used for
/// `typeof(...)` probes.
fn duckdb_string(conn: &duckdb::Connection, sql: &str) -> anyhow::Result<String> {
    let value: String = conn.query_row(sql, [], |row| row.get(0))?;
    Ok(value)
}

#[test]
fn smelt_shop_min_idempotency_and_types() {
    // 1. Stage the workspace in a tempdir so the test never mutates the
    //    repo's `examples/` tree (the `target/dev.duckdb` ends up there
    //    otherwise).
    let tmp = TempDir::new().expect("create tempdir");
    let workspace = tmp.path().join("workspace");
    copy_dir_all(&workspace_template_dir(), &workspace).expect("copy example workspace");

    // 2. Use an explicit DB path under the tempdir so the test is hermetic
    //    and the second `smelt build` run sees a populated catalog
    //    (which is what triggers bug #1 — DROP VIEW IF EXISTS against an
    //    existing Table).
    let db_path = tmp.path().join("smelt_shop_min.duckdb");
    seed_raw_orders(&db_path).expect("populate raw.orders");

    // 3. First build. Currently fails on main with bug #2:
    //    "Model 'stg_orders' references undefined model/source 'order_statuses'".
    run_smelt_build(&workspace, &db_path, "run #1");

    // 4. Second build. Once B2 lands this is what trips bug #1 —
    //    `DROP VIEW IF EXISTS stg_orders` against a Table.
    run_smelt_build(&workspace, &db_path, "run #2");

    // 5. Open the materialised database and assert structural + type
    //    invariants. Once B1+B2 land this is where bug #3 surfaces.
    let conn = duckdb::Connection::open(&db_path).expect("open duckdb");

    let stg_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.stg_orders", [], |row| row.get(0))
        .expect("count stg_orders");
    assert_eq!(
        stg_rows, SEED_ROW_COUNT as i64,
        "stg_orders should have one row per raw order (proves seed JOIN executed)"
    );

    let mart_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.mart_revenue", [], |row| {
            row.get(0)
        })
        .expect("count mart_revenue");
    assert_eq!(mart_rows, 1, "mart_revenue is a single-row aggregate");

    // Bug #3: SUM(DOUBLE) should stay DOUBLE.
    let net_revenue_type = duckdb_string(
        &conn,
        "SELECT typeof(net_revenue) FROM main.mart_revenue LIMIT 1",
    )
    .expect("typeof(net_revenue)");
    assert_eq!(
        net_revenue_type, "DOUBLE",
        "bug #3: SUM(DOUBLE) was narrowed by the _smelt_typed wrapper \
         (expected DOUBLE, got {net_revenue_type})"
    );

    // Bug #3 (cousin): COUNT(DISTINCT) should be BIGINT.
    let unique_orders_type = duckdb_string(
        &conn,
        "SELECT typeof(unique_orders) FROM main.mart_revenue LIMIT 1",
    )
    .expect("typeof(unique_orders)");
    assert_eq!(
        unique_orders_type, "BIGINT",
        "bug #3: COUNT(DISTINCT) was narrowed by the _smelt_typed wrapper \
         (expected BIGINT, got {unique_orders_type})"
    );

    // Bug #3 (cousin): SUM(DOUBLE * INTEGER) should stay DOUBLE.
    let gross_value_type = duckdb_string(
        &conn,
        "SELECT typeof(gross_value) FROM main.mart_revenue LIMIT 1",
    )
    .expect("typeof(gross_value)");
    assert_eq!(
        gross_value_type, "DOUBLE",
        "bug #3: SUM(DOUBLE * INTEGER) was narrowed by the _smelt_typed wrapper \
         (expected DOUBLE, got {gross_value_type})"
    );

    // Numerical assertion: if SUM(DOUBLE) is narrowed to BIGINT the cents
    // (~$0.37 per row × 50_000 rows = ~$18 500) disappear. Compare against
    // the expected fractional total.
    let actual_net_revenue: f64 = conn
        .query_row("SELECT net_revenue FROM main.mart_revenue", [], |row| {
            row.get::<_, f64>(0)
        })
        .expect("read net_revenue");
    let expected = expected_net_revenue();
    assert!(
        (actual_net_revenue - expected).abs() < 0.01,
        "bug #3: net_revenue lost precision \
         (expected ~{expected:.4}, got {actual_net_revenue:.4})"
    );

    // Unique order count: with order_id = i for i in [0, SEED_ROW_COUNT),
    // there are exactly SEED_ROW_COUNT distinct order ids.
    let actual_unique_orders: i64 = conn
        .query_row("SELECT unique_orders FROM main.mart_revenue", [], |row| {
            row.get(0)
        })
        .expect("read unique_orders");
    assert_eq!(
        actual_unique_orders, SEED_ROW_COUNT as i64,
        "unique_orders should equal the number of distinct seeded order ids"
    );

    // B8: qualified column refs over a bare upstream MODEL table. After
    // `smelt.models.stg_orders` is resolved to `main.stg_orders AS o`, the
    // alias `o` was not registered in the TypeContext, so `o.line_revenue`
    // (a derived column in the upstream model — not in sources.yml)
    // resolved to Unknown and SUM was narrowed to BIGINT. This is what
    // iter-2 of the smelt-shop validation loop hit on every monetary
    // aggregate.
    let qualified_line_rev_type = duckdb_string(
        &conn,
        "SELECT typeof(qualified_line_revenue) FROM main.mart_revenue_qualified LIMIT 1",
    )
    .expect("typeof(qualified_line_revenue)");
    assert_eq!(
        qualified_line_rev_type, "DOUBLE",
        "B8: SUM(o.line_revenue) where o aliases a bare upstream MODEL was \
         narrowed (expected DOUBLE, got {qualified_line_rev_type})"
    );

    let qualified_gross_rev_type = duckdb_string(
        &conn,
        "SELECT typeof(qualified_gross_revenue) FROM main.mart_revenue_qualified LIMIT 1",
    )
    .expect("typeof(qualified_gross_revenue)");
    assert_eq!(
        qualified_gross_rev_type, "DOUBLE",
        "B8: SUM(o.gross_amount) where o aliases a bare upstream MODEL was \
         narrowed (expected DOUBLE, got {qualified_gross_rev_type})"
    );

    // B9: CASE WHEN ... THEN <upstream DOUBLE> ELSE 0.0 END — without B8
    // resolving o.line_revenue to DOUBLE, the CASE collapsed to the
    // ELSE branch's DECIMAL(2,1), and DuckDB threw a runtime CAST error
    // the moment a THEN value exceeded 9.9. The wrapper must cast to
    // DOUBLE.
    let qualified_returned_rev_type = duckdb_string(
        &conn,
        "SELECT typeof(qualified_returned_revenue) FROM main.mart_revenue_qualified LIMIT 1",
    )
    .expect("typeof(qualified_returned_revenue)");
    assert_eq!(
        qualified_returned_rev_type, "DOUBLE",
        "B9: SUM(CASE ... ELSE 0.0) over qualified DOUBLE was narrowed \
         (expected DOUBLE, got {qualified_returned_rev_type})"
    );

    let qualified_coalesced_rev_type = duckdb_string(
        &conn,
        "SELECT typeof(qualified_coalesced_revenue) FROM main.mart_revenue_qualified LIMIT 1",
    )
    .expect("typeof(qualified_coalesced_revenue)");
    assert_eq!(
        qualified_coalesced_rev_type, "DOUBLE",
        "B9: SUM(COALESCE(.., 0.0)) over qualified DOUBLE was narrowed \
         (expected DOUBLE, got {qualified_coalesced_rev_type})"
    );
}
