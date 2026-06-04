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
paths:
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
FROM smelt.raw_orders
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
paths:
  - models
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
FROM smelt.raw_orders o
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

// ─── Test 3: PASSING-clause / named-arg substitution executes ───────────────

/// Named-argument (`param => value`) calls must produce the same materialised
/// rows as the equivalent positional call, even when args are supplied in
/// reverse declaration order (order-independence at the lowering layer).
///
/// The workspace contains `safe_divide` (two positional params: `numerator`,
/// `denominator`) and two models:
///   - `raw_orders` — literal VALUES table with a divide-by-zero row.
///   - `order_margin_named` — calls safe_divide with named args in reverse
///     declaration order: `smelt.functions.safe_divide(denominator => cost, numerator => revenue)`.
///
/// The expected rows are identical to `e2e_safe_divide_executes_against_duckdb`
/// which uses positional syntax.
#[test]
fn e2e_passing_clause_substitution_executes() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_named_args
version: 1
paths:
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

    // Named args in *reverse* declaration order — verifies binding-by-name, not position.
    let order_margin_named_sql = "---
materialization: table
---
SELECT order_id, smelt.functions.safe_divide(denominator => cost, numerator => revenue) AS margin
FROM smelt.raw_orders
ORDER BY order_id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("functions/safe_divide.sql", SAFE_DIVIDE_FN),
            ("models/raw_orders.sql", RAW_ORDERS_VALUES_MODEL),
            ("models/order_margin_named.sql", order_margin_named_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    // Open the resulting DuckDB file and assert on the materialised rows.
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare("SELECT order_id, margin FROM main.order_margin_named ORDER BY order_id")
        .expect("prepare margin query");
    let rows: Vec<(i32, Option<f64>)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i32>(0)?, row.get::<_, Option<f64>>(1)?))
        })
        .expect("query margin rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    // Expected (same as the positional call):
    //   order_id=1 → revenue=100, cost=50  → 2.0
    //   order_id=2 → revenue=200, cost=80  → 2.5
    //   order_id=3 → revenue=300, cost=0   → NULL (CASE branch fired)
    //   order_id=4 → revenue=400, cost=100 → 4.0
    assert_eq!(rows.len(), 4, "expected 4 rows, got: {rows:?}");
    assert_eq!(rows[0].0, 1);
    assert!(
        (rows[0].1.unwrap() - 2.0).abs() < 1e-9,
        "row 1 (named args): {:?}",
        rows[0]
    );
    assert_eq!(rows[1].0, 2);
    assert!(
        (rows[1].1.unwrap() - 2.5).abs() < 1e-9,
        "row 2 (named args): {:?}",
        rows[1]
    );
    assert_eq!(rows[2].0, 3);
    assert!(
        rows[2].1.is_none(),
        "row 3 (cost=0, named args) should be NULL via CASE branch, got: {:?}",
        rows[2]
    );
    assert_eq!(rows[3].0, 4);
    assert!(
        (rows[3].1.unwrap() - 4.0).abs() < 1e-9,
        "row 4 (named args): {:?}",
        rows[3]
    );
}

// ─── Test 5: struct-returning fn with .* projection executes ────────────────

/// A `smelt.define` whose return type is `Expr<Struct<{…}>>` and is called
/// with `.*` projection in a SELECT list should produce `n` named columns.
#[test]
fn e2e_struct_returning_fn_dot_star_projection_executes() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_struct_dot_star
version: 1
paths:
  - models
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

    // CSV seed for raw_events (avoids the literal-VALUES type-inference
    // limitation noted in docs/plans/20260422-smelt-functions.md §1281).
    // JSON values use CSV quoting: the whole value is wrapped in double-quotes,
    // and interior double-quotes are escaped by doubling them (`""`).
    let raw_events_csv = "id,payload\n\
1,\"{\"\"event_name\"\":\"\"click\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\"\n\
2,\"{\"\"event_name\"\":\"\"view\"\",\"\"platform\"\":\"\"mobile\"\",\"\"url\"\":\"\"https://m.example.com\"\"}\"\n";

    let parse_event_fn = "---
backends: [duckdb]
---
smelt.define parse_event_payload(payload: Expr<Text>) -> Expr<Struct<{event_name: Text, platform: Text, url: Text}>>
    AS ({json_extract_string(payload, '$.event_name') AS event_name, json_extract_string(payload, '$.platform') AS platform, json_extract_string(payload, '$.url') AS url})
";

    let events_parsed_sql = "---
materialization: table
---
SELECT id, smelt.functions.parse_event_payload(payload).*
FROM smelt.raw_events
ORDER BY id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("seeds/raw_events.csv", raw_events_csv),
            ("functions/parse_event_payload.sql", parse_event_fn),
            ("models/events_parsed.sql", events_parsed_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    // The table should have 4 columns: id, event_name, platform, url
    let mut stmt = conn
        .prepare(
            "SELECT id, event_name, platform, url \
             FROM main.events_parsed ORDER BY id",
        )
        .expect("prepare events_parsed query");
    let rows: Vec<(i64, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query events_parsed rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    assert_eq!(rows.len(), 2, "expected 2 rows, got: {rows:?}");
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, "click");
    assert_eq!(rows[0].2, "web");
    assert_eq!(rows[0].3, "https://example.com");
    assert_eq!(rows[1].0, 2);
    assert_eq!(rows[1].1, "view");
    assert_eq!(rows[1].2, "mobile");
    assert_eq!(rows[1].3, "https://m.example.com");
}

/// A struct-returning function called WITHOUT `.*` (assigned to a single
/// aliased column via `AS`) should have its body substituted verbatim —
/// the `SMELT_PATH_CALL_STAR` path is NOT triggered.
///
/// The brace-struct literal body `{expr AS name, …}` is smelt DSL, not
/// native DuckDB struct syntax, so the build is expected to fail with a
/// SQL parser error from DuckDB.  This test pins that the non-`.*` call site
/// substitutes the body (rather than silently dropping it or inserting
/// wrong SQL), so that only the `.*` projection path adds new lowering.
#[test]
fn e2e_struct_returning_fn_without_dot_star_passes_through() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_struct_no_star
version: 1
paths:
  - models
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

    // Same seed and function as the .* test above.
    // JSON values use CSV quoting: double-quoted field, interior quotes doubled.
    let raw_events_csv = "id,payload\n\
1,\"{\"\"event_name\"\":\"\"click\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\"\n";

    let parse_event_fn = "---
backends: [duckdb]
---
smelt.define parse_event_payload(payload: Expr<Text>) -> Expr<Struct<{event_name: Text, platform: Text, url: Text}>>
    AS ({json_extract_string(payload, '$.event_name') AS event_name, json_extract_string(payload, '$.platform') AS platform, json_extract_string(payload, '$.url') AS url})
";

    // No .* suffix — the function result is aliased to a single column.
    // The brace-struct literal body is NOT valid DuckDB SQL on its own, so
    // the build exits non-zero.  This is the expected behaviour for a
    // non-`.*` call site today; cross-engine struct-literal lowering (Spark,
    // Postgres) and single-field access (.field_name) on struct-returning
    // calls are not yet supported.
    let events_struct_sql = "---
materialization: table
---
SELECT id, smelt.functions.parse_event_payload(payload) AS payload_struct
FROM smelt.raw_events
ORDER BY id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("seeds/raw_events.csv", raw_events_csv),
            ("functions/parse_event_payload.sql", parse_event_fn),
            ("models/events_struct.sql", events_struct_sql),
        ],
    );

    // The build is expected to fail: the brace-struct literal `{expr AS name}`
    // is smelt DSL syntax substituted verbatim into the SQL, and DuckDB does
    // not accept it as a struct-construction expression.
    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            proj.to_str().unwrap(),
            "--target",
            "dev",
        ])
        .env("RUST_LOG", "warn")
        .output()
        .expect("failed to spawn smelt build");

    assert!(
        !output.status.success(),
        "expected smelt build to fail for non-.* struct call site; \
         struct body is not valid DuckDB SQL on its own.\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Verify the body WAS substituted (the SQL error mentions the struct
    // literal content, confirming the function was expanded rather than
    // silently dropped).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("json_extract_string") || stderr.contains("event_name"),
        "expected struct field names in error output (body was not substituted), \
         got stderr:\n{stderr}"
    );
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
paths:
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
FROM smelt.raw_orders
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

// ─── Test 7: nested table-valued function chain in FROM position (BUG-013) ──

/// A two-level `smelt.define` chain where each function is **table-valued**
/// (takes a `source` table and returns `SELECT * FROM …`), called in FROM
/// position.  Before the BUG-013 fix the FROM-position branch in the printer
/// reparsed the expanded body WITHOUT the synthetic `SELECT ` prefix trick used
/// by `reexpand_call_body`, so the inner `smelt.functions.passthru` node was
/// emitted verbatim → DuckDB `Catalog "smelt" does not exist`.
///
/// Chain: consumer → wrap_tbl(source) → passthru(source) → SELECT * FROM source
/// Both functions are identity pass-throughs, so the consumer table must
/// contain exactly the same rows as `raw_t`.
#[test]
fn e2e_nested_table_fn_chain_executes() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_nested_table_fn_chain
version: 1
paths:
  - models
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

    // CSV seed so the analyzer has a concrete schema for raw_t (n: Integer).
    // A VALUES model would not propagate column types through an untyped
    // TableExpr param, causing UndeclaredColumn errors in the consumer.
    let raw_t_csv = "n\n1\n2\n3\n";

    // Two untyped-param table functions forming an identity chain:
    //   passthru(source) → SELECT * FROM source
    //   wrap_tbl(source) → SELECT * FROM smelt.functions.passthru(source)
    //
    // Both are in a single `functions/` file, separated by `---` delimiters,
    // matching the style used in examples/functions_demo/functions/nested_helpers.sql.
    // Untyped params are used because `TableExpr`-typed params require typed
    // call-site args; untyped params work cleanly for this identity-passthrough shape.
    let chain_fn = "---
backends: [duckdb]
---
smelt.define passthru(source) AS (SELECT * FROM source)

---
backends: [duckdb]
---
smelt.define wrap_tbl(source) AS (SELECT * FROM smelt.functions.passthru(source))
";

    // Consumer: calls wrap_tbl in FROM position without an explicit alias.
    // The printer must wrap the expanded body in `(...) AS __smelt_t<N>` for
    // DuckDB to accept it as a derived table, and must recursively expand the
    // inner smelt.functions.passthru reference before handing to DuckDB.
    // SELECT * avoids the UndeclaredColumn check on the untyped-param chain.
    let consumer_sql = "---
materialization: table
---
SELECT * FROM smelt.functions.wrap_tbl(smelt.raw_t)
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("seeds/raw_t.csv", raw_t_csv),
            ("functions/chain.sql", chain_fn),
            ("models/consumer.sql", consumer_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    // Verify the consumer table contains exactly the seed rows — proving the
    // nested FROM-position chain expanded correctly end-to-end.
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare("SELECT n FROM main.consumer ORDER BY n")
        .expect("prepare consumer query");
    let rows: Vec<i64> = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query consumer rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    assert_eq!(
        rows,
        vec![1, 2, 3],
        "nested FROM-position table-fn chain must produce the same rows as the seed: got {rows:?}"
    );
}

// ─── Test 8: block PASSING fragment binding executes (BUG-018) ──────────────

/// A `smelt.define` with a `SelectItems<Agg>` fragment parameter, called with
/// the block `PASSING <name> AS (<body>)` syntax, must bind the PASSING body to
/// the named fragment parameter and substitute it into the function body before
/// codegen.
///
/// Before the BUG-018 fix the printer collected only the inline `arg_list`
/// (positional + `param => value`) bindings and ignored the trailing
/// `PASSING_CLAUSE`s, so the `metrics` parameter fell back to its default `()`
/// and the body emitted `SELECT group_col, () FROM …` → DuckDB syntax error.
///
/// Chain: `category_rollup(smelt.events, group_col => category) PASSING metrics AS (COUNT(*))`
/// The body `SELECT group_col, metrics FROM source GROUP BY group_col` becomes
/// `SELECT category, COUNT(*) FROM events GROUP BY category`, so the result is
/// one row per category with its event count.
#[test]
fn e2e_block_passing_fragment_executes() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_block_passing
version: 1
paths:
  - models
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

    // CSV seed so the analyzer has a concrete schema for `events`.
    let events_csv = "category,amount\na,10\na,20\nb,5\n";

    // A fragment-parameterised rollup. `metrics: SelectItems<Agg>` defaults to
    // `()`; the caller supplies it via a PASSING block. `group_col: Expr<Text>`
    // is the grouping key, passed by name at the call site.
    let category_rollup_fn = "---
backends: [duckdb]
---
smelt.define category_rollup(
    source: TableExpr,
    group_col: Expr<Text>,
    metrics: SelectItems<Agg> = ()
) -> TableExpr AS (
    SELECT group_col, metrics
    FROM source
    GROUP BY group_col
)
";

    // Consumer: block PASSING syntax supplies the `metrics` fragment.
    let dashboard_sql = "---
materialization: table
---
SELECT *
FROM smelt.functions.category_rollup(
    smelt.events,
    group_col => category
) PASSING metrics AS (COUNT(*)) AS r
ORDER BY 1
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("seeds/events.csv", events_csv),
            ("functions/category_rollup.sql", category_rollup_fn),
            ("models/category_dashboard.sql", dashboard_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    // Read both output columns by index (the aggregate column is auto-named by
    // DuckDB, so we don't depend on its name).
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare("SELECT * FROM main.category_dashboard ORDER BY 1")
        .expect("prepare dashboard query");
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .expect("query dashboard rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    // category a → 2 events, category b → 1 event.
    assert_eq!(
        rows,
        vec![("a".to_string(), 2), ("b".to_string(), 1)],
        "PASSING fragment was not substituted: got {rows:?}"
    );
}

// ─── Test 6: nested smelt.define 3-level chain executes (BUG-013) ───────────

/// A three-level `smelt.define` chain where each function calls the previous
/// via `smelt.functions.*`.  Before the BUG-013 fix, the printer's reparse
/// step did not recognise `SMELT_PATH_CALL` nodes inside a bare fragment, so
/// the inner `smelt.functions.*` text reached DuckDB verbatim and the build
/// failed with `Binder Error: Catalog "smelt" does not exist`.
///
/// Chain:  outer_call(y) → middle(z) → inner_unary(x) → (x + 1)
/// So outer_call(10) should produce 10 + 1 = 11.
#[test]
fn e2e_nested_define_chain_executes() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_nested_chain
version: 1
paths:
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

    // Three functions in a single file using `---` delimiters, matching the
    // style of `examples/functions_demo/functions/nested_helpers.sql`.
    // Parameters are intentionally untyped so they type-check cleanly.
    let chain_fn = "---
backends: [duckdb]
---
smelt.define inner_unary(x) AS (x + 1)

---
backends: [duckdb]
---
smelt.define middle(z) AS (smelt.functions.inner_unary(z))

---
backends: [duckdb]
---
smelt.define outer_call(y) AS (smelt.functions.middle(y))
";

    // Seed values model: a single-column table with a known value so we can
    // assert the arithmetic.
    let seed_model = "---
materialization: table
---
SELECT * FROM (VALUES
    (1, 10),
    (2, 20),
    (3, 30)
) AS t(row_id, val)
";

    // Consumer model: call outer_call on each row.
    // outer_call(10) → middle(10) → inner_unary(10) → (10 + 1) = 11
    let consumer_model = "---
materialization: table
---
SELECT row_id, smelt.functions.outer_call(val) AS result
FROM smelt.seed_data
ORDER BY row_id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("functions/chain.sql", chain_fn),
            ("models/seed_data.sql", seed_model),
            ("models/consumer.sql", consumer_model),
        ],
    );

    run_smelt_build(proj, "dev");

    // Open the resulting DuckDB file and verify the materialised values.
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare("SELECT row_id, result FROM main.consumer ORDER BY row_id")
        .expect("prepare consumer query");
    let rows: Vec<(i32, i32)> = stmt
        .query_map([], |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)?)))
        .expect("query consumer rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    // outer_call(10) = middle(10) = inner_unary(10) = 10 + 1 = 11
    // outer_call(20) = 21, outer_call(30) = 31
    assert_eq!(rows.len(), 3, "expected 3 rows, got: {rows:?}");
    assert_eq!(
        rows[0],
        (1, 11),
        "outer_call(10) should be 11, got: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[1],
        (2, 21),
        "outer_call(20) should be 21, got: {:?}",
        rows[1]
    );
    assert_eq!(
        rows[2],
        (3, 31),
        "outer_call(30) should be 31, got: {:?}",
        rows[2]
    );
}

// ─── Test 10: TableExpr source.* with smelt.<path> arg (BUG-009) ────────────

/// A `TableExpr` function body that projects `source.*` must produce valid SQL
/// when called with a `smelt.<path>` argument that resolves to a qualified
/// table name (e.g. `main.raw_data`).
///
/// Before the BUG-009 fix the parameter `source` was text-replaced everywhere,
/// turning `source.*` into `main.raw_data.*` — a multi-part wildcard that
/// DuckDB rejects ("syntax error at or near *").
///
/// After the fix the substitution aliases the argument at the FROM clause
/// (`FROM main.raw_data AS source`) and leaves all other `source.*` and
/// `source.col` references unchanged so they resolve through the alias.
#[test]
fn e2e_tableexpr_source_star_path_ref_executes() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_tableexpr_star
version: 1
paths:
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

    // Function with `source.*` in body — the problematic pattern.
    let add_margin_fn = "---
backends: [duckdb]
---
smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
";

    // Seed model: two rows with integer revenue and cost (INTEGER is Numeric,
    // satisfying the add_margin row-requirement check).
    let raw_data_sql = "---
materialization: table
---
SELECT
  CAST(1 AS INTEGER) AS id,
  CAST(100 AS INTEGER) AS revenue,
  CAST(30 AS INTEGER) AS cost
UNION ALL
SELECT
  CAST(2 AS INTEGER),
  CAST(200 AS INTEGER),
  CAST(80 AS INTEGER)
";

    // Consumer: calls add_margin with a smelt-path ref that resolves to
    // `main.raw_data` — a qualified name that previously produced `main.raw_data.*`.
    let with_margin_sql = "---
materialization: table
---
SELECT * FROM smelt.functions.add_margin(smelt.raw_data)
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("functions/add_margin.sql", add_margin_fn),
            ("models/raw_data.sql", raw_data_sql),
            ("models/with_margin.sql", with_margin_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    // Verify the materialised rows: source.* (id, revenue, cost) plus the
    // computed margin column.
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT id, revenue, cost, margin \
             FROM main.with_margin ORDER BY id",
        )
        .expect("prepare with_margin query");
    let rows: Vec<(i64, f64, f64, f64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .expect("query with_margin rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    assert_eq!(rows.len(), 2, "expected 2 rows, got: {rows:?}");
    // row 1: id=1, revenue=100, cost=30, margin=70
    assert_eq!(rows[0].0, 1, "row 1 id: {:?}", rows[0]);
    assert!(
        (rows[0].1 - 100.0).abs() < 1e-6,
        "row 1 revenue: {:?}",
        rows[0]
    );
    assert!((rows[0].2 - 30.0).abs() < 1e-6, "row 1 cost: {:?}", rows[0]);
    assert!(
        (rows[0].3 - 70.0).abs() < 1e-6,
        "row 1 margin (100-30=70): {:?}",
        rows[0]
    );
    // row 2: id=2, revenue=200, cost=80, margin=120
    assert_eq!(rows[1].0, 2, "row 2 id: {:?}", rows[1]);
    assert!(
        (rows[1].1 - 200.0).abs() < 1e-6,
        "row 2 revenue: {:?}",
        rows[1]
    );
    assert!((rows[1].2 - 80.0).abs() < 1e-6, "row 2 cost: {:?}", rows[1]);
    assert!(
        (rows[1].3 - 120.0).abs() < 1e-6,
        "row 2 margin (200-80=120): {:?}",
        rows[1]
    );
}

/// Regression: a `TableExpr` function called with a **CTE-name** argument
/// (no dot in the arg) must still produce valid SQL — the text-replacement
/// path (`replace_identifier`) must be used, not the FROM-aliasing path.
///
/// Before BUG-009 the CTE form already worked; this test guards that the fix
/// does not regress it.
#[test]
fn e2e_tableexpr_cte_arg_regression() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let smelt_yml = format!(
        "name: e2e_tableexpr_cte_regression
version: 1
paths:
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

    let add_margin_fn = "---
backends: [duckdb]
---
smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
";

    // Consumer: passes a CTE (no dot in arg name) to the TableExpr function.
    let margin_via_cte_sql = "---
materialization: table
---
WITH x AS (
  SELECT
    CAST(100 AS DECIMAL(18,2)) AS revenue,
    CAST(30 AS DECIMAL(18,2)) AS cost
)
SELECT margin FROM smelt.functions.add_margin(x)
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml.as_str()),
            ("functions/add_margin.sql", add_margin_fn),
            ("models/margin_via_cte.sql", margin_via_cte_sql),
        ],
    );

    run_smelt_build(proj, "dev");

    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    let mut stmt = conn
        .prepare("SELECT margin FROM main.margin_via_cte")
        .expect("prepare margin_via_cte query");
    let rows: Vec<f64> = stmt
        .query_map([], |row| row.get::<_, f64>(0))
        .expect("query margin rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    assert_eq!(rows.len(), 1, "expected 1 row, got: {rows:?}");
    assert!(
        (rows[0] - 70.0).abs() < 1e-6,
        "margin should be 100-30=70, got: {:?}",
        rows[0]
    );
}
