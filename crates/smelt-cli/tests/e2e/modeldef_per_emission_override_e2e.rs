#![cfg(feature = "duckdb")]
//! End-to-end coverage for per-`ModelDef` overrides (phase 4 of
//! `docs/outcomes/20260815-partition-grain-residue/outcome.md`).
//!
//! A single generator emits two `refresh: incremental` / `grain: partition`
//! models, each carrying its own `ModelDef.timeseries` field naming a
//! differently-named source event-time column — something the old
//! file-wide-only frontmatter block could not express. `smelt build` must
//! succeed with zero diagnostics.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path) {
    std::fs::write(
        dir.join("smelt.yml"),
        r#"
name: modeldef-override-test
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
"#,
    )
    .unwrap();
}

#[test]
fn generator_emits_partition_grain_models_with_distinct_time_columns() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("models")).unwrap();
    write_smelt_yml(dir);

    // Generator emits two incremental/partition-grain models, each overriding
    // `timeseries` with a distinct event_time_column — us_west uses `order_ts`,
    // eu uses `created_at`. Neither name matches the other's source shape, so
    // this could not be expressed with a single file-wide frontmatter block.
    std::fs::write(
        dir.join("models").join("cohorts.gen.sql"),
        concat!(
            "---\n",
            "generates: models\n",
            "refresh: incremental\n",
            "grain: partition\n",
            "---\n",
            "[\n",
            "  ModelDef {\n",
            "    name: 'us_west',\n",
            "    body: SELECT CAST('2024-01-01' AS TIMESTAMP) AS order_ts, 1 AS id,\n",
            "    materialization: 'incremental',\n",
            "    timeseries: { event_time_column: 'order_ts', partition_column: 'order_ts', granularity: 'day' }\n",
            "  },\n",
            "  ModelDef {\n",
            "    name: 'eu',\n",
            "    body: SELECT CAST('2024-01-01' AS TIMESTAMP) AS created_at, 1 AS id,\n",
            "    materialization: 'incremental',\n",
            "    timeseries: { event_time_column: 'created_at', partition_column: 'created_at', granularity: 'day' }\n",
            "  }\n",
            "]\n",
        ),
    )
    .unwrap();

    let output = Command::new(smelt_bin())
        .args(["build", "--project-dir", dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .expect("smelt build failed to spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "smelt build must succeed for two partition-grain emissions with distinct \
         per-ModelDef event_time_columns.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.to_lowercase().contains("error") && !stderr.to_lowercase().contains("error"),
        "build must produce zero diagnostics.\nstdout: {stdout}\nstderr: {stderr}"
    );
}
