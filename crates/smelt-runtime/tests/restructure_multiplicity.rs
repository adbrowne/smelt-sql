//! Row-multiplicity gate for statement-level restructuring
//! (`docs/specs/multi_backend.md` §"Statement-level lowering"): the
//! synthesised join between the bound source and a grouped CTE must be
//! null-safe, because a plain equi-join silently drops every row whose
//! partition key is `NULL` — `NULL = NULL` is `NULL`, not `TRUE`, in SQL.
//!
//! This has been measured twice on live engines and reproduced here against
//! real DuckDB (`docs/plans/20260827-statement-level-lowering.md`,
//! 2026-08-27/28 sweeps): BigQuery keeps 3 of 5 rows with a plain equi-join,
//! live Spark 4.0.0 keeps 3 of 5 likewise, where the null-safe spelling
//! keeps all 5 on both. `IS NOT DISTINCT FROM` is DuckDB's own null-safe
//! spelling (`docs/specs/multi_backend.md` §Surface capability matrix), so
//! this leg runs entirely in-process against a real DuckDB, no gating flag
//! or live warehouse needed.
//!
//! The audit's value leg (`crates/smelt-db/tests/dialect_audit`) cannot own
//! this assertion: `ANY_VALUE` is a registered nondeterministic entry probed
//! on the schema leg only (`crates/smelt-db/tests/dialect_audit/
//! overrides.rs:193`). This is the dedicated test.

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::ModelFile;
use smelt_runtime::CompilerRegistry;
use std::collections::HashMap;

fn duckdb_target() -> Target {
    Target {
        target_type: "duckdb".to_string(),
        database: Some("test.duckdb".to_string()),
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

fn registry() -> CompilerRegistry {
    let mut targets = HashMap::new();
    targets.insert("duckdb".to_string(), duckdb_target());
    let config = Config {
        name: "restructure_multiplicity".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: targets.clone(),
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    };
    CompilerRegistry::new(&config, &targets)
}

fn make_model(name: &str, sql: &str) -> ModelFile {
    let parse = smelt_parser::parse(sql);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let path = std::path::PathBuf::from(format!("models/{name}.sql"));
    ModelFile {
        name: name.to_string(),
        path: path.clone(),
        content: sql.to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(path),
        address_segments: vec![name.to_string()],
    }
}

/// A whole-partition-window occurrence of the ordered-set `PERCENTILE_CONT`
/// aggregate — DuckDB has no window form of it, so this restructures around
/// a synthesised, joined-back CTE (`RestructureId::WindowToCte`).
const DECORRELATED_SQL: &str = "SELECT id, g, \
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
    FROM src";

/// Seed data with a `NULL`-bearing partition key, mirroring the exact
/// proportion measured on BigQuery and Spark: 2 rows share a non-`NULL` key
/// (`g = 1`), 3 rows have `g IS NULL`. A plain equi-join drops the 3 `NULL`
/// rows because `NULL = NULL` is not `TRUE` — keeping 2 of 5, the same shape
/// of loss measured live (BigQuery/Spark kept 3 of 5 on their own fixtures).
fn seed(conn: &duckdb::Connection) {
    conn.execute_batch(
        "CREATE TABLE src (id INTEGER, g INTEGER, x DOUBLE);
         INSERT INTO src VALUES
             (1, 1, 10.0),
             (2, 1, 20.0),
             (3, NULL, 30.0),
             (4, NULL, 40.0),
             (5, NULL, 50.0);",
    )
    .expect("seed table");
}

fn row_count(conn: &duckdb::Connection, sql: &str) -> usize {
    let count: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM ({sql}) t"), [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|e| panic!("query failed: {e}\nsql = {sql}"));
    count as usize
}

/// The gate: the compiled, null-safe join keeps every row, including the
/// `NULL`-partition-keyed ones; the same statement with the join spelling
/// downgraded to plain `=` drops them. Both assertions run in the same test
/// so a regression that flips the printer's join spelling back to `=` fails
/// this test immediately, rather than only showing up as a silently wrong
/// row count with no baseline to compare against.
#[test]
fn null_partition_key_preserves_row_count() {
    let model = make_model("decorrelated", DECORRELATED_SQL);
    let compiled = registry()
        .get("duckdb")
        .compile(&model, "main")
        .expect("DuckDB has no window form of the ordered-set aggregate; must restructure");

    assert!(
        compiled.sql.contains("IS NOT DISTINCT FROM"),
        "DuckDB's null-safe join spelling must appear in the restructured SQL, or the \
         regression check below is comparing against a query that was never null-safe to \
         begin with: {}",
        compiled.sql
    );

    let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
    seed(&conn);

    let null_safe_rows = row_count(&conn, &compiled.sql);
    assert_eq!(
        null_safe_rows, 5,
        "the null-safe join must keep every row, NULL partition keys included: {}",
        compiled.sql
    );

    // The regression tripwire: take the exact SQL the compiler produced and
    // downgrade its null-safe join to a plain equi-join. If the printer ever
    // regresses `IS NOT DISTINCT FROM` back to `=`, this substitution becomes
    // a no-op and `broken_rows` silently equals `null_safe_rows` — which is
    // exactly why the assertion above pins that the null-safe spelling is
    // actually present first.
    let broken_sql = compiled.sql.replace("IS NOT DISTINCT FROM", "=");
    assert_ne!(
        broken_sql, compiled.sql,
        "the substitution must actually change the query, or this test cannot detect a \
         regression to a plain equi-join"
    );
    let broken_rows = row_count(&conn, &broken_sql);
    assert_eq!(
        broken_rows, 2,
        "a plain equi-join must drop the NULL-partition-keyed rows (NULL = NULL is not \
         TRUE in SQL) — if this is no longer 2, the fixture's NULL/non-NULL row split has \
         changed and this assertion needs updating alongside it: {broken_sql}"
    );
    assert!(
        broken_rows < null_safe_rows,
        "the whole point of the gate: a plain equi-join must keep strictly fewer rows than \
         the null-safe join. null_safe={null_safe_rows} broken={broken_rows}"
    );
}
