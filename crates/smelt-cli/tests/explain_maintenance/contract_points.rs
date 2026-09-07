use std::path::Path;
use std::process::Command;

use crate::support::build_report_for;

/// `daily_events` declares no `contract:` block — every cell's block prints
/// `contract:  default`.
#[test]
fn explain_prints_default_contract_point_per_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    assert!(
        report.contains("contract:  default"),
        "expected a default contract row per cell: {report}"
    );
}

/// `daily_event_counts_frozen` in `examples/timeseries` declares
/// `contract.frozen_horizon: '365 days'` — the report renders it on the
/// model's cell.
#[test]
fn explain_prints_frozen_horizon_contract_point() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_event_counts_frozen")
        .expect("daily_event_counts_frozen has a maintenance plan");

    assert!(
        report.contains("contract:  frozen_horizon 365 days"),
        "expected the declared frozen_horizon on the cell's contract row: {report}"
    );
}

/// `--json` carries the same effective contract per cell in a
/// `contract_point` object; a default cell omits the relaxation keys rather
/// than rendering them `null`.
#[test]
fn explain_json_carries_contract_point_per_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_event_counts_frozen")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_event_counts_frozen --json");

    assert!(
        output.status.success(),
        "smelt explain daily_event_counts_frozen --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let cells = json
        .get("cells")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level cells array: {stdout}"));
    assert!(!cells.is_empty(), "expected at least one cell: {stdout}");
    for cell in cells {
        let contract_point = cell
            .get("contract_point")
            .unwrap_or_else(|| panic!("expected contract_point on every cell: {stdout}"));
        assert_eq!(
            contract_point
                .get("frozen_horizon")
                .and_then(|v| v.as_str()),
            Some("365 days"),
            "expected the declared frozen_horizon on contract_point: {stdout}"
        );
        // No deferral is declared — those keys are omitted, never null.
        assert!(
            contract_point.get("deferral").is_none(),
            "an undeclared relaxation must be omitted, not rendered null: {stdout}"
        );
        assert!(contract_point.get("deferral_origin").is_none());
    }

    let default_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events --json");
    assert!(default_output.status.success());
    let default_stdout = String::from_utf8_lossy(&default_output.stdout);
    let default_json: serde_json::Value = serde_json::from_str(&default_stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {default_stdout}"));
    let default_cells = default_json
        .get("cells")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level cells array: {default_stdout}"));
    for cell in default_cells {
        let contract_point = cell
            .get("contract_point")
            .unwrap_or_else(|| panic!("expected contract_point on every cell: {default_stdout}"));
        assert_eq!(
            contract_point.as_object().map(|o| o.len()),
            Some(0),
            "a default cell's contract_point must be an empty object, not null-filled keys: \
             {default_stdout}"
        );
    }
}

// =============================================================================
// The repair family's surface (`docs/outcomes/20260809-repair-family/phases/
// 11-plan.md`): a `Technique::PerGroupRecompute` cell's own report stanza —
// its affected-key slice, bounded per-group read bound, affected-key
// discovery mechanism, and (for a `write: diff_patch` pin) the resolved
// write mechanism and delete-leg verdict. `smelt_maintenance_testkit::
// recipe::RepairRecipe` stages the same non-invertible-fold-over-a-mutable-
// clocked-source shape `repair_lowering.rs` hand-builds, generalized into
// typed recipe data.
// =============================================================================
