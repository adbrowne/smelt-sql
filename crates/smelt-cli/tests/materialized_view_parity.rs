//! `refresh: materialized_view` against a live BigQuery warehouse.
//!
//! BigQuery is the only backend advertising `supports_native_ivm`, so it is
//! the only backend on which this mode resolves to an emission rather than
//! to the §"No silent fallback" hard error
//! (`docs/specs/materialized_view.md`). This suite is that mode's live
//! coverage.
//!
//! **Why row equality alone would prove nothing.** smelt emitting a plain
//! `CREATE TABLE` instead of `CREATE MATERIALIZED VIEW` would produce
//! *exactly the same rows* — and that substitution is precisely what
//! `materialized_view.md` §Constraints item 3 forbids ("never silently falls
//! back"). So the load-bearing assertion here is on the object's TYPE, read
//! back from `INFORMATION_SCHEMA.TABLES`, not on its contents. This is the
//! same lesson `pipe_parity` learned about `supports_pipe_syntax`: a live leg
//! that would stay green with the capability turned off is not coverage of
//! the capability (`docs/specs/multi_backend.md` §Constraints, "A capability
//! flag advertising a *path* carries live coverage of that path").
//!
//! Freshness is asserted synchronously, which a probe established is sound:
//! BigQuery combines a materialized view's stored data with a live delta over
//! the base table at query time, so a read straight after a write already
//! reflects the write — no refresh-cycle wait, no forced refresh
//! (`docs/research/20260816-bigquery-backend.md` §"Materialized views").
//!
//! Skips green with `SMELT_BQ_PROJECT`/`SMELT_BQ_ACCESS_TOKEN` unset or
//! without `--features bigquery`, matching every other BigQuery leg.

#![cfg(feature = "bigquery")]

mod common;
use common::{bigquery_enabled, bq_backend, bq_dataset, bq_target_block};
use smelt_backend::Backend;
use std::process::Command;
use tempfile::TempDir;

/// Scopes this suite's BigQuery datasets. Each test gets its OWN label, and
/// therefore its own dataset: cargo runs these two concurrently and each tears
/// its dataset down when it finishes, so a shared one would let the first
/// finisher drop the warehouse out from under the other. (It did, the first
/// time this suite ran live.)
const BQ_LABEL_OK: &str = "mv_p1";
const BQ_LABEL_INELIGIBLE: &str = "mv_p1_bad";

/// The base relation the materialized view reads. A BigQuery materialized
/// view must read a real table, so this cannot be folded into the view's own
/// SQL as a literal `WITH`.
const SOURCE_MODEL: &str = r#"---
materialization: table
---
WITH data AS (
    SELECT 'alpha' AS label, CAST(10 AS BIGINT) AS amount
    UNION ALL SELECT 'alpha', CAST(5 AS BIGINT)
    UNION ALL SELECT 'beta',  CAST(7 AS BIGINT)
)
SELECT label, amount FROM data
"#;

/// An aggregation BigQuery's incremental IVM accepts. Measured, not assumed:
/// an aggregate carrying any further transformation (even `SUM(x) * 100`) is
/// refused, so the projection outputs the aggregate directly.
const ROLLUP_MODEL: &str = r#"---
refresh: materialized_view
---
SELECT label, SUM(amount) AS total
FROM smelt.mv_source
GROUP BY label
"#;

/// A query BigQuery's IVM refuses — a top-level `ORDER BY`, which it reports
/// as "do not support the Sort operation".
const INELIGIBLE_MODEL: &str = r#"---
refresh: materialized_view
---
SELECT label, amount
FROM smelt.mv_source
ORDER BY amount
"#;

fn stage(tmp: &TempDir, rollup: &str, label: &str) -> std::path::PathBuf {
    let root = tmp.path().join("mv_proj");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();

    let yml = format!(
        "name: mv_proj\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         {bq_block}\
         default_materialization: table\n",
        bq_block = bq_target_block(label)
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();
    std::fs::write(root.join("models").join("mv_source.sql"), SOURCE_MODEL).unwrap();
    std::fs::write(root.join("models").join("mv_rollup.sql"), rollup).unwrap();
    root
}

fn run_smelt(project_dir: &std::path::Path, target: &str) -> std::process::Output {
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

/// Reads back one scalar string from a live BigQuery query.
fn bq_scalar(dataset: &str, sql: &str) -> Option<String> {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let backend = bq_backend(dataset).await;
        let batches = backend
            .execute_sql(sql)
            .await
            .unwrap_or_else(|e| panic!("BigQuery query failed: {sql}\n{e}"));
        for batch in batches {
            if batch.num_rows() == 0 {
                continue;
            }
            let col = batch.column(0);
            let s = arrow::util::display::array_value_to_string(col, 0)
                .expect("value renders as a string");
            return Some(s);
        }
        None
    })
}

fn drop_dataset(dataset: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let backend = bq_backend(dataset).await;
        let _ = backend
            .execute_sql(&format!("DROP SCHEMA IF EXISTS `{dataset}` CASCADE"))
            .await;
    });
}

/// A `refresh: materialized_view` model is emitted as a real, engine-owned
/// MATERIALIZED VIEW on BigQuery, and serves the right rows.
#[test]
fn materialized_view_is_emitted_natively_on_bigquery() {
    if !bigquery_enabled() {
        eprintln!("BigQuery not configured — skipping");
        return;
    }
    let dataset = bq_dataset(BQ_LABEL_OK);
    let tmp = TempDir::new().unwrap();
    let root = stage(&tmp, ROLLUP_MODEL, BQ_LABEL_OK);

    let out = run_smelt(&root, "bq");
    assert!(
        out.status.success(),
        "`smelt run` failed on BigQuery.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // THE load-bearing assertion: the object is a materialized view, not a
    // table smelt quietly substituted. Row equality below would hold either
    // way, so this is what makes the leg coverage of the capability rather
    // than of the rows.
    let table_type = bq_scalar(
        &dataset,
        &format!(
            "SELECT table_type FROM `{dataset}`.INFORMATION_SCHEMA.TABLES \
             WHERE table_name = 'mv_rollup'"
        ),
    );
    assert_eq!(
        table_type.as_deref(),
        Some("MATERIALIZED VIEW"),
        "mv_rollup must be an engine-owned MATERIALIZED VIEW; a substituted \
         TABLE would serve identical rows and violate \
         docs/specs/materialized_view.md §\"No silent fallback\""
    );

    // And it serves the right aggregate. alpha = 10 + 5, beta = 7 => 22.
    let total = bq_scalar(
        &dataset,
        &format!("SELECT CAST(SUM(total) AS STRING) FROM `{dataset}`.mv_rollup"),
    );
    assert_eq!(
        total.as_deref(),
        Some("22"),
        "materialized view served the wrong aggregate"
    );

    drop_dataset(&dataset);
}

/// smelt relays BigQuery's OWN incrementalizability verdict rather than
/// masking it or pre-empting it with a smelt-side eligibility check
/// (`materialized_view.md` §"No smelt-side eligibility", §"No silent
/// fallback" item 2).
#[test]
fn ineligible_query_relays_the_engines_own_reason() {
    if !bigquery_enabled() {
        eprintln!("BigQuery not configured — skipping");
        return;
    }
    let dataset = bq_dataset(BQ_LABEL_INELIGIBLE);
    let tmp = TempDir::new().unwrap();
    let root = stage(&tmp, INELIGIBLE_MODEL, BQ_LABEL_INELIGIBLE);

    let out = run_smelt(&root, "bq");
    assert!(
        !out.status.success(),
        "a query BigQuery's IVM refuses must fail the run, never fall back to \
         a table.\nstdout: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    // Must fail for the ENGINE'S OWN eligibility verdict specifically. A
    // looser assertion would go green on any failure at all — including the
    // missing-dataset race this suite actually hit on its first live run.
    assert!(
        combined.contains("do not support the Sort operation"),
        "expected BigQuery's own incrementalizability reason relayed verbatim \
         (\"... do not support the Sort operation.\"), got:\n{combined}"
    );

    drop_dataset(&dataset);
}
