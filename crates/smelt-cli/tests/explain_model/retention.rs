use std::path::Path;

use crate::support::build_report_for;

// `docs/outcomes/20260906-trimmed-history-sources/phases/10-plan.md`:
// `smelt explain` renders each declared-`retention:` source's retained
// bound against the model's required reach. Fixture mirrors
// `crates/smelt-runtime/tests/retention_admission.rs`'s own shape: a
// `retention: '45 days'` source (`3_888_000`s), with three models whose
// SQL derives a bounded-within, a bounded-exceeding, and an unprovable
// (unbounded) reach against it, plus a fourth model over a plain source
// declaring no `retention:` at all.

const SMELT_YML: &str = r#"name: retention_explain_fixture
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    schema: main
default_materialization: view
"#;

const EVENTS_SOURCE: &str = r#"description: Raw events, retained 45 days.
columns:
  - name: device_id
    type: INTEGER
  - name: event_date
    type: DATE
  - name: amount
    type: DOUBLE
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
mutation_profile:
  kind: append_only
retention: '45 days'
"#;

const PLAIN_SOURCE: &str = r#"description: Raw events, no declared retention.
columns:
  - name: device_id
    type: INTEGER
  - name: event_date
    type: DATE
  - name: amount
    type: DOUBLE
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
mutation_profile:
  kind: append_only
"#;

const WITHIN_MODEL_SQL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date,
       SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date
           RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS total_amount
FROM smelt.sources.events
"#;

const EXCEEDS_MODEL_SQL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date,
       SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date
           RANGE BETWEEN INTERVAL '100 days' PRECEDING AND CURRENT ROW) AS total_amount
FROM smelt.sources.events
"#;

const DOWNGRADE_MODEL_SQL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT event_date,
       SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date
           RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running_amount
FROM smelt.sources.events
"#;

const NO_RETENTION_MODEL_SQL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, SUM(amount) AS total
FROM smelt.sources.plain
GROUP BY event_date
"#;

fn stage_project() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(tmp.path().join("models/sources/events.yml"), EVENTS_SOURCE).unwrap();
    std::fs::write(tmp.path().join("models/sources/plain.yml"), PLAIN_SOURCE).unwrap();
    std::fs::write(tmp.path().join("models/within_model.sql"), WITHIN_MODEL_SQL).unwrap();
    std::fs::write(
        tmp.path().join("models/exceeds_model.sql"),
        EXCEEDS_MODEL_SQL,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/downgrade_model.sql"),
        DOWNGRADE_MODEL_SQL,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/no_retention_model.sql"),
        NO_RETENTION_MODEL_SQL,
    )
    .unwrap();
    tmp
}

#[test]
fn explain_text_reports_the_bound_and_the_required_reach() {
    let tmp = stage_project();
    let report = build_report_for(tmp.path(), "within_model").expect("maintenance plan report");
    assert!(
        report.contains("Retention:")
            && report.contains("events: retained 3888000s, required reach 604800s — within bound"),
        "expected a within-bound retention row: {report}"
    );
}

#[test]
fn explain_text_reports_an_exceeding_reach_as_refused() {
    let tmp = stage_project();
    let report = build_report_for(tmp.path(), "exceeds_model").expect("maintenance plan report");
    assert!(
        report.contains(
            "events: retained 3888000s, required reach 8640000s — exceeds bound \
             (SourceRetentionExceeded)"
        ),
        "expected an exceeds-bound retention row: {report}"
    );
}

#[test]
fn explain_text_reports_a_recorded_downgrade() {
    let tmp = stage_project();
    let report = build_report_for(tmp.path(), "downgrade_model").expect("maintenance plan report");
    assert!(
        report.contains(
            "events: retained 3888000s, reach unprovable — downgraded \
             (SourceRetentionDowngraded):"
        ) && report.contains("unbounded"),
        "expected a recorded-downgrade retention row naming source, bound and reason: {report}"
    );
}

#[test]
fn explain_text_omits_the_retention_section_without_a_retained_source() {
    let tmp = stage_project();
    let report =
        build_report_for(tmp.path(), "no_retention_model").expect("maintenance plan report");
    assert!(
        !report.contains("Retention:"),
        "expected no Retention: section for a model over an unretained source: {report}"
    );
}

/// Re-stages the project on disk and spawns the real `smelt` binary so the
/// `--json` wiring (the new `build_maintenance_plan_json` parameters) is
/// exercised end to end, mirroring `json_output.rs`'s own pattern.
fn run_json(project_dir: &Path, model: &str) -> serde_json::Value {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg(model)
        .arg("--json")
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("spawn smelt explain {model} --json: {e}"));
    assert!(
        output.status.success(),
        "smelt explain {model} --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse --json output")
}

#[test]
fn explain_json_carries_a_retention_entry_per_bounded_source() {
    let tmp = stage_project();
    let json = run_json(tmp.path(), "within_model");
    let entry = &json["retention"][0];
    assert_eq!(entry["source"], "events");
    assert_eq!(entry["verdict"], "within");
    assert_eq!(entry["retained_secs"], 3_888_000);
    assert_eq!(entry["required_lookback_secs"], 604_800);
}

#[test]
fn explain_json_omits_retention_when_no_source_declares_a_bound() {
    let tmp = stage_project();
    let json = run_json(tmp.path(), "no_retention_model");
    assert!(
        json.get("retention").is_none(),
        "expected no `retention` key, got: {json}"
    );
}
