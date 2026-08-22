//! Live coverage for native pipe-SQL emission (`supports_pipe_syntax`).
//!
//! BigQuery is the only backend advertising `supports_pipe_syntax = true`, so
//! it is the only backend whose printer takes the *emit-pipes-natively* path:
//! the `PIPE_QUERY` node is printed verbatim (`|>` and all) instead of being
//! lowered to standard SQL. Every other backend takes `print_pipe_rewrite`.
//!
//! That makes one model a two-sided test. The same pipe query runs on both
//! legs and must produce the same rows:
//!   * on DuckDB it proves the **lowering** is faithful (already covered
//!     offline by `smelt-db`'s `pipe_equivalence`, here end-to-end);
//!   * on BigQuery it proves the **native emission** is accepted by a real
//!     GoogleSQL warehouse and computes the same relation.
//!
//! Without the BigQuery leg the `true` in the capability matrix is a claim, not
//! a fact — `capability_conformance` asserts the flag's *value*, never that the
//! path it enables works.
//!
//! DuckDB always runs; the BigQuery leg runs only when the binary is compiled
//! with `--features bigquery` and a live token is present.

mod common;
use common::{
    assert_table_parity, bigquery_enabled, bq_target_block, drop_bq_dataset, fetch_rows,
    targets_to_run, TargetKind,
};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Scopes this suite's BigQuery dataset (`<base>_pipe_<pid>`).
const BQ_LABEL: &str = "pipe";

/// The upstream the pipe query reads, so the pipe's `FROM` is a real
/// `smelt.<path>` reference rather than an inline subquery — the reference
/// rewrite has to survive inside a natively-emitted pipe stage too.
const BASE_MODEL: &str = "\
SELECT 'alpha' AS grp, 100 AS val
UNION ALL SELECT 'beta', 200
UNION ALL SELECT 'alpha', 50
UNION ALL SELECT 'gamma', 10";

/// A pipe query covering the stages that behave differently either side of the
/// capability flag:
/// - `|> WHERE` **before** aggregation (lowers to `WHERE`)
/// - `|> EXTEND` (lowers to a re-projection)
/// - `|> AGGREGATE … GROUP BY` (lowers to `SELECT keys, aggs … GROUP BY`)
/// - `|> WHERE` **after** aggregation (lowers to `HAVING`)
/// - `|> ORDER BY` / `|> LIMIT` (trailing clauses)
///
/// Rows surviving `val >= 50`: (alpha,100), (beta,200), (alpha,50).
/// Doubled and grouped: alpha → 300 over 2 rows, beta → 400 over 1.
/// Both clear `total_double > 100`; gamma is filtered out before aggregating.
const PIPE_MODEL: &str = "\
FROM smelt.pipe_base
|> WHERE val >= 50
|> EXTEND val * 2 AS double_val
|> AGGREGATE SUM(double_val) AS total_double, COUNT(*) AS n GROUP BY grp
|> WHERE total_double > 100
|> ORDER BY grp
|> LIMIT 10";

fn stage_pipe_workspace(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let root = tmp.path().join("pipe_proj");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(&warehouse).unwrap();

    let yml = format!(
        "name: pipe_proj\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n{bq_block}\
         default_materialization: table\n",
        bq_block = bq_target_block(BQ_LABEL)
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();
    std::fs::write(root.join("models").join("pipe_base.sql"), BASE_MODEL).unwrap();
    std::fs::write(root.join("models").join("pipe_showcase.sql"), PIPE_MODEL).unwrap();

    (root, warehouse)
}

fn run_smelt_run(project_dir: &std::path::Path, target: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

/// Columns in `AGGREGATE` output order: grouping keys, then aggregates.
fn expected_rows() -> Vec<Vec<String>> {
    vec![
        vec!["alpha".to_string(), "300".to_string(), "2".to_string()],
        vec!["beta".to_string(), "400".to_string(), "1".to_string()],
    ]
}

/// A pipe query computes the same relation whether the printer lowers it
/// (DuckDB) or emits it natively (BigQuery).
///
/// **Red on BigQuery** if GoogleSQL rejects the emitted pipe query, if the
/// `smelt.<path>` rewrite does not reach inside a pipe stage, or if native
/// pipe semantics diverge from smelt's lowering — all three are silent today.
#[cfg(feature = "duckdb")]
#[test]
fn pipe_query_agrees_across_lowered_and_native_emission() {
    let tmp = TempDir::new().unwrap();
    let (root, warehouse) = stage_pipe_workspace(&tmp);
    let db_path = root.join("target/dev.duckdb");
    let mut ran_on_bigquery = false;

    for kind in targets_to_run(BQ_LABEL) {
        let (target_name, schema) = match &kind {
            TargetKind::DuckDb => ("dev", "main"),
            TargetKind::Spark => continue,
            TargetKind::BigQuery { dataset } => {
                ran_on_bigquery = true;
                ("bq", dataset.as_str())
            }
        };

        let out = run_smelt_run(&root, target_name);
        assert!(
            out.status.success(),
            "{target_name}: `smelt run` failed.\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, "pipe_showcase");
        drop_bq_dataset(&kind);
        assert_table_parity(&actual, &expected_rows(), target_name);
    }

    // Non-vacuity: with a live token present this suite's whole point is the
    // BigQuery leg, so a silently DuckDB-only pass is a failure, not a skip.
    assert_eq!(
        ran_on_bigquery,
        bigquery_enabled(),
        "BigQuery leg did not run despite a live environment being configured"
    );
}
