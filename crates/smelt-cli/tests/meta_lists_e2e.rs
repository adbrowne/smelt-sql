#![cfg(feature = "duckdb")]
//! End-to-end execution tests for in-model meta-language **list spread**
//! (BUG-006 lists). A spread of an inline list literal in a SELECT list must be
//! evaluated at compile time and emit plain SQL select items, so the model
//! executes on DuckDB rather than the `...` reaching the engine verbatim.
//!
//! These stage a hermetic project under a `TempDir`, shell out to the compiled
//! `smelt` binary, and open the produced DuckDB file to assert on materialised
//! rows — the same pattern as `functions_e2e.rs`.

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

/// Self-contained base model providing the columns the spread models read.
const RAW_USERS_MODEL: &str = "---
materialization: table
---
SELECT * FROM (VALUES
    (1, 'Alice', 'alice@example.com'),
    (2, 'Bob', 'bob@example.com')
) AS t(id, name, email)
";

fn smelt_yml(db_path: &Path) -> String {
    format!(
        "name: e2e_meta_lists
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
    )
}

/// A column-ref spread (`...[name, email]`) expands at compile time to two
/// plain select items, and the model executes — where today the `...` token
/// reaches DuckDB and raises a parser error.
#[test]
fn e2e_column_ref_spread_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    // id, then spread of [name, email] → SELECT id, name, email.
    let spread_model = "---
materialization: table
---
SELECT
    id,
    ...[name, email]
FROM smelt.raw_users
ORDER BY id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            ("models/raw_users.sql", RAW_USERS_MODEL),
            ("models/with_spread.sql", spread_model),
        ],
    );

    run_smelt_build(proj, "dev");

    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    // The spread expanded into exactly three columns: id, name, email.
    assert_eq!(
        column_names(&conn, "with_spread"),
        vec!["id", "name", "email"],
        "expanded columns"
    );

    let mut stmt = conn
        .prepare("SELECT id, name, email FROM main.with_spread ORDER BY id")
        .expect("prepare rows");
    let rows: Vec<(i32, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rows");

    assert_eq!(
        rows,
        vec![
            (1, "Alice".to_string(), "alice@example.com".to_string()),
            (2, "Bob".to_string(), "bob@example.com".to_string()),
        ],
        "spread-expanded rows"
    );
}

/// Multiple spreads plus an empty-list spread in one SELECT list: the empty
/// spread elides itself and its adjacent commas, the populated spread expands
/// in place, and surrounding plain items are preserved — all at compile time.
#[test]
fn e2e_multi_and_empty_spread_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    // SELECT id, name, email, (empty elided), email again-free → id, name, email
    let spread_model = "---
materialization: table
---
SELECT
    id,
    ...[name, email],
    ...[]
FROM smelt.raw_users
ORDER BY id
";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            ("models/raw_users.sql", RAW_USERS_MODEL),
            ("models/multi_spread.sql", spread_model),
        ],
    );

    run_smelt_build(proj, "dev");

    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");
    assert_eq!(
        column_names(&conn, "multi_spread"),
        vec!["id", "name", "email"],
        "empty spread elided; populated spread expanded"
    );
}

/// Column names of `main.<table>` in ordinal order, read from DuckDB's
/// information schema (robust regardless of statement-execution state).
fn column_names(conn: &duckdb::Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .expect("prepare information_schema query");
    stmt.query_map([table], |row| row.get::<_, String>(0))
        .expect("query columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect columns")
}
