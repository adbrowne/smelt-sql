//! `smelt explain <model>` probe-set rendering
//! (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report":
//! "Declared-fact probes"; `docs/outcomes/20260809-probe-backed-facts/
//! phases/08-plan.md`). Drives the real `smelt` binary over a staged
//! fixture declaring `functional_dependencies:` — the report is offline
//! (no backend connection is required for these assertions to hold, and
//! none is configured to exist).

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// Stages a project whose one model declares `functional_dependencies:` —
/// enough for `declared_model_probes` to build exactly one probe, with no
/// database ever created (`smelt explain` never connects).
fn stage_project(tmp: &tempfile::TempDir) -> PathBuf {
    let root = tmp.path().join("explain_probes");
    write_file(
        &root.join("smelt.yml"),
        "name: explain_probes\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n",
    );
    write_file(
        &root.join("models/sources/raw/subs.yml"),
        "description: Raw subscription rows\n\
         name: raw.subs\n\
         columns:\n\
         \x20 - name: signup_ts\n    type: TIMESTAMP\n\
         \x20 - name: customer_id\n    type: INTEGER\n\
         \x20 - name: region\n    type: VARCHAR\n",
    );
    write_file(
        &root.join("models/subscriptions.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         functional_dependencies:\n\
         \x20 - key: [customer_id]\n    determines: region\n\
         timeseries:\n\
         \x20 event_time_column: signup_ts\n  partition_column: signup_date\n  granularity: day\n\
         ---\n\
         SELECT CAST(signup_ts AS DATE) AS signup_date, customer_id, region \
         FROM smelt.sources.raw.subs\n",
    );
    root
}

#[test]
fn explain_text_report_lists_declared_probes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = stage_project(&tmp);

    let out = Command::new(smelt_bin())
        .args(["explain", "subscriptions"])
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt explain`: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "smelt explain failed.\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("Probes (1):"), "{stdout}");
    assert!(
        stdout.contains("fact: functional_dependencies:"),
        "{stdout}"
    );
    assert!(
        stdout.contains("probe: DeclaredFunctionalDependencyViolated"),
        "{stdout}"
    );
    assert!(stdout.contains("cadence: per_run"), "{stdout}");
    assert!(
        stdout.contains("cost: +1 query per consuming run"),
        "{stdout}"
    );
}

#[test]
fn explain_json_carries_probes_array() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = stage_project(&tmp);

    for extra_args in [vec![], vec!["--show-sql".to_string()]] {
        let out = Command::new(smelt_bin())
            .args(["explain", "subscriptions", "--json"])
            .args(&extra_args)
            .args(["--project-dir", project_dir.to_str().unwrap()])
            .env_remove("RUST_LOG")
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn `smelt explain --json`: {e}"));
        assert!(
            out.status.success(),
            "smelt explain --json {extra_args:?} failed.\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("valid JSON output");
        let probes = json
            .get("probes")
            .unwrap_or_else(|| panic!("missing `probes` key in {json}"))
            .as_array()
            .expect("`probes` must be an array");
        assert_eq!(probes.len(), 1, "{probes:?}");
        let entry = &probes[0];
        assert_eq!(entry["fact"], "functional_dependencies:");
        assert_eq!(entry["probe"], "DeclaredFunctionalDependencyViolated");
        assert_eq!(entry["cadence"], "per_run");
        assert_eq!(entry["cost"], "+1 query per consuming run");
        assert!(entry.get("cell").is_some());
    }
}

/// The probe set is built without any backend connection: a project whose
/// `smelt.yml` names a target with no reachable database still gets a full
/// probe-set report — the report only *builds* probe SQL, never executes
/// it (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
/// report").
#[test]
fn explain_probe_set_is_offline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = stage_project(&tmp);
    // No `target/dev.duckdb` file or directory exists anywhere under
    // `project_dir` — if the report ever opened a connection, it would
    // either fail or silently create one; neither happens here.
    let db_path = project_dir.join("target/dev.duckdb");
    assert!(!db_path.exists());

    let out = Command::new(smelt_bin())
        .args(["explain", "subscriptions", "--json"])
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt explain --json`: {e}"));
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !db_path.exists(),
        "smelt explain must never create a database file"
    );
}
