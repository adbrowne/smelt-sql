use std::path::Path;
use std::process::Command;

use crate::support::build_report_for;
use crate::support::{STATE_COLUMNS_EVENTS_SOURCE, STATE_COLUMNS_SMELT_YML};

/// `smelt explain <model>` for a keyed model prints an `Execution postures:`
/// block naming the run shape and all three derived verdicts
/// (`docs/outcomes/20260815-keyed-grain-residue` phase 4).
#[test]
fn explain_prints_execution_postures_for_keyed_model() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/total_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, SUM(amount) AS total_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let report =
        build_report_for(tmp.path(), "total_amount").expect("total_amount has a maintenance plan");

    assert!(
        report.contains("Execution postures:"),
        "expected an execution-postures section: {report}"
    );
    assert!(
        report.contains("run shape: window-forward"),
        "expected the window-forward run shape (clocked source): {report}"
    );
    assert!(
        report.contains("re-run tolerance: no"),
        "SUM is additive, not re-run tolerant: {report}"
    );
    assert!(
        report.contains("order-independence: yes"),
        "SUM's `+` is order-independent: {report}"
    );
    assert!(
        report.contains("reprocessing: refused"),
        "reprocessing refusal is unconditional: {report}"
    );
}

/// `--json` carries the same three verdicts as the text section, in a
/// top-level `execution_postures` object.
#[test]
fn explain_json_carries_execution_postures() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/total_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, SUM(amount) AS total_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("total_amount")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain total_amount --json");

    assert!(
        output.status.success(),
        "smelt explain total_amount --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let postures = json
        .get("execution_postures")
        .unwrap_or_else(|| panic!("expected a top-level execution_postures object: {stdout}"));
    assert_eq!(postures["run_shape"], "window-forward");
    assert_eq!(postures["rerun_tolerant"]["holds"], false);
    assert_eq!(postures["order_independent"]["holds"], true);
    assert_eq!(postures["reprocessing_refused"]["holds"], true);
}

/// A `grain: partition` model never classifies through the keyed
/// classifier, so `result.execution_postures` is `None` — the report
/// prints no `Execution postures:` block at all.
#[test]
fn explain_omits_execution_postures_for_non_keyed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    assert!(
        !report.contains("Execution postures:"),
        "a grain: partition model must print no execution-postures section: {report}"
    );
}

// =============================================================================
// The contract lattice's `smelt explain` surface (`docs/outcomes/20260809-
// contract-lattice-v1/phases/07-plan.md`): the effective contract per cell —
// default or a relaxed point with its declared parameters — resolved
// through the single-owner `smelt_logical::contract::effective_contract`,
// never a local model-vs-cell ladder.
// =============================================================================
