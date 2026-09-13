//! Live-gated Trino smoke test — the walking-skeleton gate for the Trino
//! backend, modelled on `spark_smoke.rs` but asserting rather than collecting
//! breaks: `docs/outcomes/20260913-trino-target-spine` phase 9 makes
//! `type: trino` a target a real project can run end to end.
//!
//! Skips green when `SMELT_TRINO_URL` is unset. Run it with:
//!   bash scripts/trino-up.sh && source scripts/trino-env.sh
//!   cargo test -p smelt-cli --test trino_smoke

mod common;
use common::{drop_trino_schema, fetch_trino_rows, trino_env, trino_schema, trino_target_block};

use smelt_state::file_store::FileStore;
use smelt_state::RunOutcomeKind;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

/// Guards every read *and* mutation of `SMELT_TRINO_URL` in this binary.
/// `cargo test` runs `#[test]` functions on separate threads within one
/// process, and `trino_legs_skip_not_pass_when_url_unset` below temporarily
/// removes the var — without this lock that mutation could race
/// `trino_smoke_materializes_table_and_view`'s own read of it.
static TRINO_ENV_GUARD: Mutex<()> = Mutex::new(());

fn stage_trino_spine(tmp: &TempDir, schema: &str) -> std::path::PathBuf {
    let root = tmp.path().join("trino_spine_proj");
    fs::create_dir_all(root.join("models")).unwrap();

    let yml = format!(
        "name: trino_spine_smoke\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\nstate:\n  mode: intervals\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    let spine_sql = "SELECT CAST(1 AS BIGINT) AS id, 'alpha' AS label\n\
                      UNION ALL SELECT CAST(2 AS BIGINT), 'beta'\n\
                      UNION ALL SELECT CAST(3 AS BIGINT), 'gamma'\n";
    fs::write(
        root.join("models/spine_table.sql"),
        format!("---\nmaterialization: table\n---\n{spine_sql}"),
    )
    .unwrap();
    fs::write(
        root.join("models/spine_view.sql"),
        format!("---\nmaterialization: view\n---\n{spine_sql}"),
    )
    .unwrap();

    root
}

fn run_smelt(project_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            "trino",
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

fn expected_rows() -> Vec<Vec<String>> {
    let mut rows = vec![
        vec!["1".to_string(), "alpha".to_string()],
        vec!["2".to_string(), "beta".to_string()],
        vec!["3".to_string(), "gamma".to_string()],
    ];
    rows.sort();
    rows
}

/// The lone report file a fresh `.smelt/targets/trino/reports/` directory
/// holds after exactly one run — there is no CLI flag to pin `run_id`, so the
/// run's own manifest/report directory is the only way to find it.
fn only_report_run_id(project_dir: &Path) -> String {
    let reports_dir = project_dir.join(".smelt/targets/trino/reports");
    let mut entries: Vec<_> = fs::read_dir(&reports_dir)
        .unwrap_or_else(|e| panic!("read_dir {reports_dir:?} failed: {e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one report in {reports_dir:?}, found {}",
        entries.len()
    );
    entries
        .pop()
        .unwrap()
        .path()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Tests 5 + 6 (phase 9 plan): one `smelt run --target trino` materializes
/// both a table and a view as real Iceberg objects, readable back with the
/// expected rows, and writes a run report naming both models with a success
/// status.
#[test]
fn trino_smoke_materializes_table_and_view() {
    let has_env = {
        let _guard = TRINO_ENV_GUARD.lock().unwrap();
        trino_env().is_some()
    };
    if !has_env {
        eprintln!("SMELT_TRINO_URL unset — skipping trino_smoke_materializes_table_and_view");
        return;
    }
    let schema = trino_schema("spine_smoke");

    let tmp = TempDir::new().unwrap();
    let root = stage_trino_spine(&tmp, &schema);

    let out = run_smelt(&root);
    assert!(
        out.status.success(),
        "`smelt run --target trino` failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let table_rows = fetch_trino_rows(&schema, "spine_table");
    let view_rows = fetch_trino_rows(&schema, "spine_view");

    let expected = expected_rows();
    let mut actual_table = table_rows;
    actual_table.sort();
    let mut actual_view = view_rows;
    actual_view.sort();
    assert_eq!(actual_table, expected, "spine_table rows mismatch");
    assert_eq!(actual_view, expected, "spine_view rows mismatch");

    let run_id = only_report_run_id(&root);
    let file_store = FileStore::new(&root, "trino");
    let report = file_store
        .load_report(&run_id)
        .expect("load_report must not error")
        .expect("report must exist after a successful run");
    assert_eq!(report.outcome_counts.success, 2, "both models must succeed");
    assert_eq!(report.outcome_counts.failed, 0);
    assert!(report.failures.is_empty());

    let manifest = file_store
        .load_run(&run_id)
        .expect("load_run must not error")
        .expect("manifest must be persisted for a successful run");
    for model in ["spine_table", "spine_view"] {
        assert_eq!(
            manifest.models[model].outcome,
            RunOutcomeKind::Success,
            "{model} must be recorded as a success in the run manifest"
        );
    }

    drop_trino_schema(&schema);
}

/// Test 8 (phase 9 plan): the vacuous-pass guard. With `SMELT_TRINO_URL`
/// unset, `trino_env()` is `None` and no Trino leg runs anywhere — this test
/// pins that fact so a future refactor of the skip gate can't quietly turn a
/// skip into a silent pass.
#[test]
fn trino_legs_skip_not_pass_when_url_unset() {
    let _guard = TRINO_ENV_GUARD.lock().unwrap();
    let saved = std::env::var("SMELT_TRINO_URL").ok();
    // SAFETY: `TRINO_ENV_GUARD` serializes every reader/mutator of this var
    // in this binary, so no other thread observes the var mid-mutation.
    unsafe {
        std::env::remove_var("SMELT_TRINO_URL");
    }
    let result = trino_env();
    if let Some(url) = saved {
        // SAFETY: see above.
        unsafe {
            std::env::set_var("SMELT_TRINO_URL", url);
        }
    }
    assert!(
        result.is_none(),
        "trino_env() must be None (skip), never Some with fabricated defaults, \
         when SMELT_TRINO_URL is unset"
    );
}
