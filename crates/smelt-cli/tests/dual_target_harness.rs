#![cfg(feature = "duckdb")]
//! Self-test for the dual-target harness (`tests/common`).
//!
//! Runs a trivial model on every available target. DuckDB always runs;
//! Spark runs only when `SPARK_CONNECT_URL` is set in the environment.
//! When unset the test still passes — it just covers DuckDB only.

mod common;

use common::{run_smelt_on, stage_dual_workspace, targets_to_run, TargetKind};
use tempfile::TempDir;

#[test]
fn harness_runs_trivial_model_on_every_available_target() {
    let tmp = TempDir::new().unwrap();
    let warehouse = tmp.path().join("warehouse");
    let models = &[("one.sql", "select 1 as x")];
    let root = stage_dual_workspace(&tmp, "w1_harness", models, &warehouse);

    for kind in targets_to_run() {
        let target_name = match kind {
            TargetKind::DuckDb => "dev",
            TargetKind::Spark => "spark",
        };
        let out = run_smelt_on(&root, target_name, &[]);
        assert!(
            out.status.success(),
            "smelt run failed on {target_name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
