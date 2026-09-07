//! `smelt explain <model>`'s succession-grain rendering, text and `--json`
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/08-plan.md`,
//! `docs/specs/cli.md` §"Succession grain").

use std::path::Path;

use crate::support::{
    build_report_for, stage_succession_project, stage_succession_project_clamped,
    stage_succession_project_event_time_partitioned,
};

/// Test 1: `succession_cell_prints_grain_identity_and_technique` —
/// `grain: succession`, `identity: (customer_id, effective_ts)`,
/// `technique: succession-patch`, in that order.
#[test]
fn succession_cell_prints_grain_identity_and_technique() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");

    let grain_pos = report.find("grain: succession").unwrap_or_else(|| {
        panic!("expected `grain: succession` in report: {report}");
    });
    let identity_pos = report
        .find("identity: (customer_id, effective_ts)")
        .unwrap_or_else(|| {
            panic!("expected `identity: (customer_id, effective_ts)` in report: {report}");
        });
    let technique_pos = report
        .find("technique: succession-patch")
        .unwrap_or_else(|| {
            panic!("expected `technique: succession-patch` in report: {report}");
        });
    assert!(
        grain_pos < identity_pos && identity_pos < technique_pos,
        "expected grain, identity, technique in that order: {report}"
    );
}

/// Test 2: `succession_cell_prints_run_axis_and_clock_for_an_arrival_partitioned_source`
/// — `run axis: ingested_date (arrival-partitioned)` and
/// `clock: effective_ts`.
#[test]
fn succession_cell_prints_run_axis_and_clock_for_an_arrival_partitioned_source() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");

    assert!(
        report.contains("run axis: ingested_date (arrival-partitioned)"),
        "expected the arrival-partitioned run axis line: {report}"
    );
    assert!(
        report.contains("clock: effective_ts"),
        "expected the clock line: {report}"
    );
}

/// Test 3: `succession_cell_prints_event_time_partitioning_when_axis_equals_clock`
/// — the same project with the source's `partition_column ==
/// event_time_column` renders `(event-time-partitioned)`.
#[test]
fn succession_cell_prints_event_time_partitioning_when_axis_equals_clock() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project_event_time_partitioned(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");

    assert!(
        report.contains("run axis: effective_ts (event-time-partitioned)"),
        "expected the event-time-partitioned run axis line: {report}"
    );
}

/// Test 4: `succession_cell_prints_fixed_execution_postures` — `posture:
/// re-run tolerant; order-independent but serial`.
#[test]
fn succession_cell_prints_fixed_execution_postures() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");

    assert!(
        report.contains("posture: re-run tolerant; order-independent but serial"),
        "expected the fixed succession posture line: {report}"
    );
}

/// Test 5: `succession_cell_prints_pre_window_filter_only_when_declared` —
/// the clamped model prints `pre-window filter: <sql>`; the unclamped one
/// prints no such line.
#[test]
fn succession_cell_prints_pre_window_filter_only_when_declared() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");
    assert!(
        !report.contains("pre-window filter:"),
        "the unclamped model must print no pre-window filter line: {report}"
    );

    let clamped_tmp = tempfile::TempDir::new().expect("tempdir");
    let clamped_project_dir = stage_succession_project_clamped(&clamped_tmp);
    let clamped_report = build_report_for(&clamped_project_dir, "customer_history")
        .expect("customer_history has a plan");
    assert!(
        clamped_report.contains("pre-window filter: effective_ts >= DATE '2026-01-01'"),
        "expected the declared pre-window filter: {clamped_report}"
    );
}

/// Test 6: `succession_cell_prints_the_tombstone_ledger_as_internal_state` —
/// `internal state: tombstone ledger customer_history__tombstones
/// (customer_id, effective_ts) — not part of the model's public schema`.
#[test]
fn succession_cell_prints_the_tombstone_ledger_as_internal_state() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");

    assert!(
        report.contains(
            "internal state: tombstone ledger customer_history__tombstones (customer_id, \
             effective_ts) — not part of the model's public schema"
        ),
        "expected the tombstone-ledger internal-state line: {report}"
    );
}

/// Test 7: `succession_headline_is_event_addressed` — the report's first
/// line reads `(emits: event history keyed by [customer_id],
/// event-addressed by (customer_id, effective_ts); grain: …)`.
#[test]
fn succession_headline_is_event_addressed() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_history").expect("customer_history has a plan");
    let first_line = report.lines().next().expect("report has a first line");

    // `customer_history` declares no `timeseries:`/`unique_key:` of its own
    // (the succession grain is recognised, not declared), so its own
    // Relation Contract `derived_grain` is `None` and the headline's
    // trailing `; grain: …` clause is correctly absent — the report's own
    // `derived grain:` row would print `(unclassified)` for the same reason.
    assert!(
        first_line.contains(
            "(emits: event history keyed by [customer_id], event-addressed by (customer_id, \
             effective_ts))"
        ),
        "expected the event-addressed headline: {first_line}"
    );
}

/// Test 8: `non_succession_model_prints_no_succession_lines` — a
/// keyed-upsert model in the same project prints none of the seven
/// succession-only lines (no leakage).
#[test]
fn non_succession_model_prints_no_succession_lines() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let report =
        build_report_for(&project_dir, "customer_counts").expect("customer_counts has a plan");

    for needle in [
        "grain: succession",
        "identity: (customer_id, effective_ts)",
        "technique: succession-patch",
        "run axis:",
        "posture: re-run tolerant; order-independent but serial",
        "pre-window filter:",
        "internal state: tombstone ledger",
    ] {
        assert!(
            !report.contains(needle),
            "non-succession model must not print `{needle}`: {report}"
        );
    }
}

/// Spawn the real `smelt explain <model> --json` binary against a staged
/// project and parse its stdout as JSON.
fn explain_json(project_dir: &Path, model_name: &str) -> serde_json::Value {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg(model_name)
        .arg("--json")
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .expect("spawn smelt explain --json");
    assert!(
        output.status.success(),
        "smelt explain --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("expected JSON, got: {e}\n{stdout}"))
}

/// Test 9: `succession_json_object_carries_every_field` —
/// `succession.key_columns`, `clock_column`, `run_axis`, `partitioning ==
/// "arrival"`, `lead_columns`, `lag_columns`, `delete_flag`,
/// `pre_window_filter`, `tombstone_ledger.{table,columns}`,
/// `rerun_tolerant/order_independent/concurrent`.
#[test]
fn succession_json_object_carries_every_field() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project_clamped(&tmp);
    let json = explain_json(&project_dir, "customer_history");
    let succession = &json["succession"];

    assert_eq!(
        succession["key_columns"],
        serde_json::json!(["customer_id"])
    );
    assert_eq!(
        succession["clock_column"],
        serde_json::json!("effective_ts")
    );
    assert_eq!(succession["run_axis"], serde_json::json!("ingested_date"));
    assert_eq!(succession["partitioning"], serde_json::json!("arrival"));
    assert_eq!(succession["lead_columns"], serde_json::json!(["valid_to"]));
    assert_eq!(succession["lag_columns"], serde_json::json!([]));
    assert_eq!(
        succession["pre_window_filter"],
        serde_json::json!("effective_ts >= DATE '2026-01-01'")
    );
    assert_eq!(
        succession["tombstone_ledger"]["table"],
        serde_json::json!("customer_history__tombstones")
    );
    assert_eq!(
        succession["tombstone_ledger"]["columns"],
        serde_json::json!(["customer_id", "effective_ts"])
    );
    assert_eq!(succession["rerun_tolerant"], serde_json::json!(true));
    assert_eq!(succession["order_independent"], serde_json::json!(true));
    assert_eq!(succession["concurrent"], serde_json::json!(false));
}

/// Test 10: `succession_json_omits_absent_optional_fields` — no
/// `pre_window_filter` and no `delete_flag` key at all (not `null`) for a
/// model declaring neither.
#[test]
fn succession_json_omits_absent_optional_fields() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let json = explain_json(&project_dir, "customer_history");
    let succession = json["succession"]
        .as_object()
        .expect("succession object present");

    assert!(
        !succession.contains_key("pre_window_filter"),
        "expected no pre_window_filter key: {succession:#?}"
    );
    assert!(
        !succession.contains_key("delete_flag"),
        "expected no delete_flag key: {succession:#?}"
    );
}

/// Test 11: `succession_json_delta_signature_is_keyed_succession` —
/// `delta_signature.shape == "keyed_succession"`, `addressing == "event"`,
/// `keys`, `axis == run axis`.
#[test]
fn succession_json_delta_signature_is_keyed_succession() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let json = explain_json(&project_dir, "customer_history");
    let delta_signature = &json["delta_signature"];

    assert_eq!(
        delta_signature["shape"],
        serde_json::json!("keyed_succession")
    );
    assert_eq!(delta_signature["addressing"], serde_json::json!("event"));
    assert_eq!(delta_signature["keys"], serde_json::json!(["customer_id"]));
    assert_eq!(delta_signature["axis"], serde_json::json!("ingested_date"));
}

/// Test 12: `non_succession_json_omits_the_succession_key` — the key is
/// absent, never `null`.
#[test]
fn non_succession_json_omits_the_succession_key() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = stage_succession_project(&tmp);
    let json = explain_json(&project_dir, "customer_counts");
    let obj = json.as_object().expect("top-level JSON object");

    assert!(
        !obj.contains_key("succession"),
        "non-succession model must omit the succession key entirely: {obj:#?}"
    );
}
