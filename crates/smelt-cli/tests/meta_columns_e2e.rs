#![cfg(feature = "duckdb")]
//! End-to-end execution test for the in-model meta-language build-path
//! evaluator's `smelt.columns_of` reflection (BUG-006 columns_of): a model that
//! consumes `smelt.columns_of(t)` via `filter` / `map` and a spread must have
//! the reflection materialised at compile time and lowered to plain select
//! items, so it executes on DuckDB rather than `columns_of` reaching the engine
//! verbatim.
//!
//! Stages a hermetic project under a `TempDir`, shells out to the compiled
//! `smelt` binary, and opens the produced DuckDB file to assert on the
//! materialised columns — same pattern as `meta_hofs_e2e.rs` / `meta_lists_e2e.rs`.
//!
//! The `examples/meta_columns/` workspace itself stays on the `example_builds`
//! `KNOWN_UNBUILDABLE` allow-list because its `orders` model reads the unseeded
//! `raw.orders` source (it fails before reaching the reflection-using model);
//! this hermetic test is the BUG-006 columns_of regression target.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_workspace(tmp: &Path, files: &[(&str, &str)]) {
    for (rel, contents) in files {
        let path = tmp.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write workspace file");
    }
}

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

fn smelt_yml(db_path: &Path) -> String {
    format!(
        "name: e2e_meta_columns
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: table
",
        db_path.display()
    )
}

/// A model that reflects over the column list of an upstream model with
/// `smelt.columns_of`, keeps the numeric columns via `filter(c.is_numeric)`,
/// projects each to its name, and spreads the result into the SELECT list —
/// invoked through a `smelt.define` body (the `coalesce_numeric` shape from
/// `examples/meta_columns/`). The reflection must be materialised at build so
/// `picked` has exactly the numeric columns of `base`.
#[test]
fn e2e_columns_of_filter_map_spread_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            (
                // Base model with a mix of numeric (id, amount) and non-numeric
                // (label) columns, no external source so it executes standalone.
                "models/base.sql",
                "SELECT CAST(1 AS INTEGER) AS id, \
                 CAST('a' AS VARCHAR) AS label, \
                 CAST(2.5 AS DOUBLE) AS amount\n",
            ),
            (
                // smelt.columns_of(t) |> filter(numeric) |> map(name)
                "functions/numeric_names.sql",
                "smelt.define numeric_names(t: TableExpr) -> SelectItems<Scalar, t> AS (\n\
                 \x20   smelt.columns_of(t)\n\
                 \x20     |> filter(fn c => c.is_numeric)\n\
                 \x20     |> map(fn c => c.name)\n\
                 )\n",
            ),
            (
                // Spread the numeric column names of `base` into the SELECT list.
                // Expected lowering: SELECT id, amount FROM base.
                "models/picked.sql",
                "SELECT ...smelt.functions.numeric_names(smelt.base) FROM smelt.base\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    // `picked` projects exactly the numeric columns id (1) and amount (2.5).
    let id: i64 = conn
        .query_row("SELECT id FROM main.picked", [], |r| r.get(0))
        .expect("query picked.id");
    assert_eq!(id, 1, "columns_of filter(numeric) keeps id");

    let amount: f64 = conn
        .query_row("SELECT amount FROM main.picked", [], |r| r.get(0))
        .expect("query picked.amount");
    assert!(
        (amount - 2.5).abs() < f64::EPSILON,
        "columns_of filter(numeric) keeps amount"
    );

    // The non-numeric `label` column was filtered out — it must not be present.
    let label_cols: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = 'picked' AND column_name = 'label'",
            [],
            |r| r.get(0),
        )
        .expect("query information_schema for label");
    assert_eq!(
        label_cols, 0,
        "non-numeric label column is filtered out by columns_of(...) |> filter(c.is_numeric)"
    );
}
