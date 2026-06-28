#![cfg(feature = "duckdb")]
//! End-to-end execution tests for the in-model meta-language build-path
//! evaluator (BUG-006 hofs/polish): HOFs (`map`/`filter`/`reduce`), the pipe
//! operator, lambdas, the meta-world ternary, and `smelt.config.var` must be
//! evaluated at compile time and lowered to plain SQL, so a model using them
//! executes on DuckDB rather than the construct reaching the engine verbatim.
//!
//! Stages a hermetic project under a `TempDir`, shells out to the compiled
//! `smelt` binary, and opens the produced DuckDB file to assert on the
//! materialised values — same pattern as `meta_lists_e2e.rs` / `functions_e2e.rs`.

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

/// `smelt.yml` with a `vars:` block for `smelt.config.var` and a ternary.
fn smelt_yml(db_path: &Path) -> String {
    format!(
        "name: e2e_meta_hofs
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: table
vars:
  env: dev
  region: us-west-2
",
        db_path.display()
    )
}

/// Reducers, lambdas, and a pipe chain fold to plain SQL and execute.
#[test]
fn e2e_reducers_and_pipe_execute_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            (
                // reduce([1,2,3], plus_chain) → 1 + 2 + 3 = 6
                "models/total.sql",
                "SELECT reduce([1, 2, 3], plus_chain) AS total\n",
            ),
            (
                // reduce([true,false,true], and_all) → true AND false AND true = false
                "models/all_true.sql",
                "SELECT reduce([true, false, true], and_all) AS all_true\n",
            ),
            (
                // reduce(['alpha','beta','gamma'], concat_with(' OR ')) →
                // 'alpha' || ' OR ' || 'beta' || ' OR ' || 'gamma'
                "models/joined.sql",
                "SELECT reduce(['alpha', 'beta', 'gamma'], concat_with(' OR ')) AS joined\n",
            ),
            (
                // [1,2,3] |> filter(c>0) |> map(c*2) = [2,4,6]; plus_chain → 12
                "models/doubled_sum.sql",
                "SELECT reduce([1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2), plus_chain) AS doubled_sum\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    let total: i64 = conn
        .query_row("SELECT total FROM main.total", [], |r| r.get(0))
        .expect("query total");
    assert_eq!(total, 6, "reduce(plus_chain) folds [1,2,3] to 6");

    let all_true: bool = conn
        .query_row("SELECT all_true FROM main.all_true", [], |r| r.get(0))
        .expect("query all_true");
    assert!(!all_true, "reduce(and_all) of [true,false,true] is false");

    let joined: String = conn
        .query_row("SELECT joined FROM main.joined", [], |r| r.get(0))
        .expect("query joined");
    assert_eq!(joined, "alpha OR beta OR gamma", "concat_with(' OR ') fold");

    let doubled_sum: i64 = conn
        .query_row("SELECT doubled_sum FROM main.doubled_sum", [], |r| r.get(0))
        .expect("query doubled_sum");
    assert_eq!(
        doubled_sum, 12,
        "filter(>0) then map(*2) then plus_chain = 12"
    );
}

/// `smelt.config.var` resolves to its `Text` value, and a ternary driven by a
/// var selects the reached branch at compile time.
#[test]
fn e2e_config_var_and_ternary_execute_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            (
                "models/region.sql",
                "SELECT smelt.config.var('region') AS region\n",
            ),
            (
                // env = dev → the else branch ('permissive') is selected.
                "models/mode.sql",
                "SELECT if smelt.config.var('env') = 'prod' then 'strict' else 'permissive' AS mode\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    let region: String = conn
        .query_row("SELECT region FROM main.region", [], |r| r.get(0))
        .expect("query region");
    assert_eq!(
        region, "us-west-2",
        "config.var('region') resolves to vars value"
    );

    let mode: String = conn
        .query_row("SELECT mode FROM main.mode", [], |r| r.get(0))
        .expect("query mode");
    assert_eq!(
        mode, "permissive",
        "ternary picks else branch when env != prod"
    );
}

/// `smelt.config.var` inside a `smelt.define` body resolves at compile time
/// when the function is inlined at a call site (BUG-067). Previously the
/// inlined body's `config.var(...)` call reached DuckDB verbatim
/// (`Catalog "smelt" does not exist`) even though the LSP reported the
/// workspace clean.
#[test]
fn e2e_config_var_inside_define_body_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    let tag_fn = "smelt.define region_tag(prefix) AS (prefix || smelt.config.var('region'))\n";

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            ("functions/region_tag.sql", tag_fn),
            (
                "models/tagged.sql",
                "SELECT smelt.functions.region_tag('r-') AS tag\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    let tag: String = conn
        .query_row("SELECT tag FROM main.tagged", [], |r| r.get(0))
        .expect("query tagged");
    assert_eq!(
        tag, "r-us-west-2",
        "config.var inside the inlined define body resolves to the vars value"
    );
}
