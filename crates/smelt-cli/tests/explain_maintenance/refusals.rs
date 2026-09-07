use std::process::Command;

use crate::support::build_report_for;

/// A `grain: partition` model joining a clocked, append-only driving source
/// with a second, unclocked `mutable_snapshot` source — the second source's
/// scan cannot be partition-bounded, so the plan refuses admission for it
/// (`MaintenanceScanUnbounded`) rather than silently shipping a full-table
/// write (`incremental_models.md` §"Partition-local maintenance (the K8
/// guardrail)").
fn stage_scan_unbounded_project(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create dirs");

    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: scan_unbounded_fixture\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .expect("write smelt.yml");

    std::fs::write(
        project_dir.join("models/sources/clocked.yml"),
        "description: clocked append-only source\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n",
    )
    .expect("write clocked.yml");

    std::fs::write(
        project_dir.join("models/sources/unclocked.yml"),
        "description: unclocked mutable source, no clock to bound its scan\n\
         mutation_profile: mutable_snapshot\n\
         columns:\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .expect("write unclocked.yml");

    std::fs::write(
        project_dir.join("models/joined.sql"),
        "---\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         refresh: incremental\n\
         grain: partition\n\
         ---\n\
         SELECT c.d, c.id, u.val\n\
         FROM smelt.sources.clocked c\n\
         JOIN smelt.sources.unclocked u ON c.id = u.id\n",
    )
    .expect("write joined.sql");

    project_dir
}

/// `smelt explain <model> --json`'s `refusals` array carries the same
/// admission refusal the text report's "Refusals" section prints — read
/// verbatim from the property profile
/// (`docs/specs/property_diff.md` §"The property profile", test 6 of
/// `docs/outcomes/20260905-property-diff/phases/02-plan.md`).
#[test]
fn explain_json_carries_refusals() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let project_dir = stage_scan_unbounded_project(&tmp);

    let report = build_report_for(&project_dir, "joined").expect("joined has a maintenance plan");
    assert!(
        report.contains("MaintenanceScanUnbounded") || report.contains("ScanUnbounded"),
        "expected the text report to print the ScanUnbounded refusal: {report}"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("joined")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain joined --json");
    assert!(
        output.status.success(),
        "smelt explain joined --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("parse JSON");

    let refusals = json["refusals"]
        .as_array()
        .expect("refusals must be an array");
    assert!(
        !refusals.is_empty(),
        "expected at least one refusal in --json output: {json}"
    );
    assert!(
        refusals
            .iter()
            .all(|r| r["code"].as_str() == Some("MaintenanceScanUnbounded")),
        "expected every refusal here to be MaintenanceScanUnbounded: {refusals:?}"
    );
    assert!(
        refusals.iter().any(|r| r["text"]
            .as_str()
            .is_some_and(|t| t.contains("ScanUnbounded") && t.contains("unclocked"))),
        "expected the refusal text to name the unclocked source: {refusals:?}"
    );
}
