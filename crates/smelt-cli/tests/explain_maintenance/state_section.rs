use std::process::Command;

use crate::support::build_report_for;
use crate::support::STATE_COLUMNS_EVENTS_SOURCE;
use crate::support::STATE_COLUMNS_SMELT_YML;

/// `smelt explain <model>` for a keyed `AVG` model prints an internal-state
/// section naming both hidden state columns and says they are not in the
/// model's public schema (`docs/outcomes/20260809-rung2-state-shapes` row 9).
#[test]
fn explain_renders_internal_state_section() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/avg_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, AVG(amount) AS avg_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let report =
        build_report_for(tmp.path(), "avg_amount").expect("avg_amount has a maintenance plan");

    assert!(
        report.contains("State columns:"),
        "expected an internal-state section: {report}"
    );
    assert!(
        report.contains("avg_amount__sum") && report.contains("avg_amount__count"),
        "expected both hidden state columns named: {report}"
    );
    assert!(
        report.contains("not part of the model's public schema"),
        "expected the state section to say it is not part of the public schema: {report}"
    );
}

/// A keyed `SUM` model has no decomposed state — the report has no state
/// section at all (no empty header).
#[test]
fn explain_omits_state_section_when_no_state() {
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
        !report.contains("State columns:"),
        "a stateless model must print no state section: {report}"
    );
}

/// `--json` carries the same state-column information as the text section,
/// in a top-level `state_columns` array.
#[test]
fn explain_json_reports_state_columns() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/avg_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, AVG(amount) AS avg_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("avg_amount")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain avg_amount --json");

    assert!(
        output.status.success(),
        "smelt explain avg_amount --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let state_columns = json
        .get("state_columns")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level state_columns array: {stdout}"));
    assert_eq!(state_columns.len(), 1, "state_columns: {stdout}");
    let entry = &state_columns[0];
    assert_eq!(entry["presented_column"], "avg_amount");
    assert_eq!(
        entry["state_columns"],
        serde_json::json!(["avg_amount__sum", "avg_amount__count"])
    );
    assert_eq!(
        entry["presentation_expr"],
        "avg_amount__sum / avg_amount__count"
    );
}
