#![cfg(feature = "duckdb")]
//! Phase 57 — end-to-end execution tests for `smelt.functions.*` and
//! `smelt.as_struct()`.
//!
//! These tests stage a hermetic project under a `TempDir`, shell out to the
//! compiled `smelt` binary (Choice A: subprocess invocation, see the plan
//! deferral note for rationale), and then open the produced DuckDB file with
//! the `duckdb` crate to assert on materialised rows.
//!
//! Why subprocess instead of calling `commands::build::build()` directly?
//! `mod commands` is private to `crates/smelt-cli/src/main.rs` (see
//! `main.rs:1` — `mod commands;`, not in `lib.rs`), so it cannot be invoked
//! from an integration test without restructuring the binary. Spawning the
//! built `smelt` binary is closer to what users actually do, and matches the
//! existing pattern in `crates/smelt-cli/tests/smelt_shop_idempotency.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ─── Helpers ────────────────────────────────────────────────────────────────

/// The compiled `smelt` binary location. Cargo sets this env var for
/// integration tests in the package that defines the binary.
fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Stage a workspace under `tmp` from a list of `(relative_path, contents)`
/// pairs. Parent directories are created as needed.
fn write_workspace(tmp: &Path, files: &[(&str, &str)]) {
    for (rel, contents) in files {
        let path = tmp.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write workspace file");
    }
}

/// Run `smelt build` against a staged workspace. Panics with a descriptive
/// message (including stdout/stderr) if the build does not exit 0.
fn run_smelt_build(project_dir: &Path, target: &str) {
    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    assert!(
        output.status.success(),
        "smelt build (target={target}) failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Canonical safe_divide function body, copied from
/// `examples/functions_demo/functions/safe_divide.sql`. Kept as a literal
/// here so the test is self-contained and obvious; the canonical fixture
/// remains the regression target for `example_diagnostics::functions_demo_no_diagnostics`.
const SAFE_DIVIDE_FN: &str = "---
backends: [duckdb]
---
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double>
    AS (CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE) END)
";

/// `raw_orders` model body — a literal table with a divide-by-zero row so
/// the safe_divide CASE branch is exercised.
const RAW_ORDERS_VALUES_MODEL: &str = "---
materialization: table
---
SELECT * FROM (VALUES
    (1, 100, 50),
    (2, 200, 80),
    (3, 300, 0),
    (4, 400, 100)
) AS t(order_id, revenue, cost)
";

// ─── Test 1: safe_divide executes end-to-end against DuckDB ─────────────────

#[test]
fn e2e_safe_divide_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_safe_divide
version: 1
model_paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: view
",
        db_path.display()
    );

    let order_margin_sql = "---
materialization: table
---
SELECT order_id, smelt.functions.safe_divide(revenue, cost) AS margin
FROM smelt.models.raw_orders
ORDER BY order_id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("functions/safe_divide.sql", SAFE_DIVIDE_FN),
            ("models/raw_orders.sql", RAW_ORDERS_VALUES_MODEL),
            ("models/order_margin.sql", order_margin_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    // Open the resulting DuckDB file and assert on the materialised rows.
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare("SELECT order_id, margin FROM main.order_margin ORDER BY order_id")
        .expect("prepare margin query");
    let rows: Vec<(i32, Option<f64>)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i32>(0)?, row.get::<_, Option<f64>>(1)?))
        })
        .expect("query margin rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    // Expected:
    //   order_id=1 → 100/50 = 2.0
    //   order_id=2 → 200/80 = 2.5
    //   order_id=3 → cost = 0 → NULL (CASE branch fired)
    //   order_id=4 → 400/100 = 4.0
    assert_eq!(rows.len(), 4, "expected 4 rows, got: {rows:?}");
    assert_eq!(rows[0].0, 1);
    assert!(
        (rows[0].1.unwrap() - 2.0).abs() < 1e-9,
        "row 1: {:?}",
        rows[0]
    );
    assert_eq!(rows[1].0, 2);
    assert!(
        (rows[1].1.unwrap() - 2.5).abs() < 1e-9,
        "row 2: {:?}",
        rows[1]
    );
    assert_eq!(rows[2].0, 3);
    assert!(
        rows[2].1.is_none(),
        "row 3 (cost=0) should be NULL via CASE branch, got: {:?}",
        rows[2]
    );
    assert_eq!(rows[3].0, 4);
    assert!(
        (rows[3].1.unwrap() - 4.0).abs() < 1e-9,
        "row 4: {:?}",
        rows[3]
    );
}

// ─── Test 2: smelt.as_struct emits an executable struct literal ─────────────

#[test]
fn e2e_as_struct_emits_executable_struct_literal() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_as_struct
version: 1
model_paths:
  - models
seed_paths:
  - seeds
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: view
",
        db_path.display()
    );

    // CSV seed so smelt-core's `discover_seed_infos` produces typed columns
    // (order_id, customer_id, total, tax) for `smelt.seeds.raw_orders`.
    // A literal-VALUES model body would NOT populate UpstreamSchemas because
    // `resolved_model_schema` doesn't infer types through VALUES clauses, so
    // the as_struct emitter would fall back to pass-through.
    let raw_orders_csv = "order_id,customer_id,total,tax
1,10,100,7
2,11,200,14
3,12,300,21
";

    let orders_packed_sql = "---
materialization: table
---
SELECT smelt.as_struct(o EXCEPT customer_id) AS order_record
FROM smelt.seeds.raw_orders o
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("seeds/raw_orders.csv", raw_orders_csv),
            ("models/orders_packed.sql", orders_packed_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    // Positive assertion: struct field access yields the right values for
    // the included fields. This proves the struct literal was both valid
    // SQL and built from the right columns.
    let mut stmt = conn
        .prepare(
            "SELECT order_record.order_id, order_record.total, order_record.tax \
             FROM main.orders_packed ORDER BY order_record.order_id",
        )
        .expect("prepare struct-field query");
    let rows: Vec<(i32, i32, i32)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, i32>(2)?,
            ))
        })
        .expect("query struct rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    assert_eq!(
        rows,
        vec![(1, 100, 7), (2, 200, 14), (3, 300, 21)],
        "struct field access mismatch: {rows:?}"
    );

    // Type assertion: typeof(order_record) should be a STRUCT type with the
    // expected field names but NOT customer_id (excluded by EXCEPT).
    let typeof_str: String = conn
        .query_row(
            "SELECT typeof(order_record) FROM main.orders_packed LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("typeof query");
    assert!(
        typeof_str.to_uppercase().contains("STRUCT"),
        "expected STRUCT in typeof, got: {typeof_str}"
    );
    assert!(
        typeof_str.contains("order_id")
            && typeof_str.contains("total")
            && typeof_str.contains("tax"),
        "STRUCT type should include order_id/total/tax fields, got: {typeof_str}"
    );
    assert!(
        !typeof_str.contains("customer_id"),
        "STRUCT type should NOT contain customer_id (EXCEPT), got: {typeof_str}"
    );
}

// ─── Test 3: PASSING-clause / TableExpr substitution executes ───────────────

#[test]
#[ignore = "Phase 57 deferred: SmeltFnExpander drops named args (compiler.rs:337 — `_named` is unused), \
            so PASSING-clause / TableExpr substitution does not yet substitute correctly. \
            Extending the expander is out of Phase 57's scope (would widen scope into the next \
            functions phase). See Deferred entry in docs/plans/20260422-smelt-functions.md."]
fn e2e_passing_clause_substitution_executes() {
    // Intentionally empty — the deferral message above explains why this
    // is gated. Once the expander handles named args / PASSING substitution
    // (separate work), un-ignore this test and write the fixture.
}

// ─── Test 4: function call works across multiple targets ────────────────────

#[test]
fn e2e_cross_target_function_call() {
    // A single-engine cross-target check: the same workspace builds against
    // two DuckDB targets (different schemas, different DB files). Asserts
    // that `set_function_bodies_all` propagated the safe_divide body into
    // both compilers — same row sets must materialise in both.
    //
    // Out of scope: a true cross-engine assertion (Spark snapshot) — Spark
    // is not available in this environment. See Deferred entry in
    // docs/plans/20260422-smelt-functions.md.
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let dev_db = proj.join("dev.duckdb");
    let prod_db = proj.join("prod.duckdb");

    let smelt_yml = format!(
        "name: e2e_cross_target
version: 1
model_paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: dev_schema
  prod:
    type: duckdb
    database: {}
    schema: prod_schema
default_materialization: view
",
        dev_db.display(),
        prod_db.display()
    );

    let order_margin_sql = "---
materialization: table
---
SELECT order_id, smelt.functions.safe_divide(revenue, cost) AS margin
FROM smelt.models.raw_orders
ORDER BY order_id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("functions/safe_divide.sql", SAFE_DIVIDE_FN),
            ("models/raw_orders.sql", RAW_ORDERS_VALUES_MODEL),
            ("models/order_margin.sql", order_margin_sql),
        ],
    );

    run_smelt_build(proj, "dev");
    run_smelt_build(proj, "prod");

    fn fetch_rows(db_path: &Path, schema: &str) -> Vec<(i32, Option<f64>)> {
        let conn = duckdb::Connection::open(db_path).expect("open db");
        let sql = format!("SELECT order_id, margin FROM {schema}.order_margin ORDER BY order_id");
        let mut stmt = conn.prepare(&sql).expect("prepare");
        stmt.query_map([], |row| {
            Ok((row.get::<_, i32>(0)?, row.get::<_, Option<f64>>(1)?))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
    }

    let dev_rows = fetch_rows(&dev_db, "dev_schema");
    let prod_rows = fetch_rows(&prod_db, "prod_schema");

    assert_eq!(
        dev_rows, prod_rows,
        "dev and prod row sets diverged — function-body wiring did not \
         propagate identically: dev={dev_rows:?} prod={prod_rows:?}"
    );

    // Sanity: at least one row, with the divide-by-zero NULL preserved.
    assert!(!dev_rows.is_empty(), "expected non-empty result set");
    let zero_row = dev_rows.iter().find(|(id, _)| *id == 3).expect("row 3");
    assert!(
        zero_row.1.is_none(),
        "divide-by-zero row should be NULL in cross-target build, got: {zero_row:?}"
    );
}
