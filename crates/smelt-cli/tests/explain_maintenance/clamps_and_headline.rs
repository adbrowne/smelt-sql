use std::path::Path;
use std::process::Command;

use crate::support::{build_report_for, stage_delta_type_project};

/// `daily_events` in `examples/timeseries` is `refresh: incremental` +
/// `grain: partition` reading a single unclocked-partition source
/// (`raw.events` has no source-level `timeseries:` declaration). The report
/// must name the cell (trigger/corner/technique), print the locality verdict
/// and a scan-clamps section, and the `ledger_catch_up` flag — as data
/// directly readable off `MaintenancePlanResult`, not fabricated.
#[test]
fn explain_prints_cells_clamps_locality() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    assert!(
        report.contains("Maintenance plan: daily_events"),
        "{report}"
    );
    assert!(
        report.contains("Cells ("),
        "expected a Cells section: {report}"
    );
    assert!(
        report.contains("trigger"),
        "expected trigger info: {report}"
    );
    assert!(
        report.contains("corner:") && report.contains("technique:"),
        "expected corner/technique per cell: {report}"
    );
    assert!(
        report.contains("locality:"),
        "expected a partition-locality verdict per cell: {report}"
    );
    assert!(
        report.contains("scan clamps"),
        "expected a scan-clamps section per cell: {report}"
    );
    assert!(
        report.contains("ledger_catch_up"),
        "expected the ledger_catch_up flag per cell: {report}"
    );
    assert!(
        report.contains("Inbound edges"),
        "expected an inbound-edges section: {report}"
    );
    // `daily_events` has fully resolved column provenance — the degenerate-
    // collapse callout is a false-positive risk if it were still keyed off
    // "one group spanning 2+ sources" instead of the real `degenerate` signal.
    assert!(
        !report.contains("could not distinguish"),
        "daily_events has no ambiguous provenance; the collapse callout must not fire: {report}"
    );
    assert!(
        report.contains("admissible write patterns: region"),
        "expected the admissible write-pattern registry listing, leading with `region` (the \
         only structural fact this cell's declared facts satisfy first in registry order): \
         {report}"
    );
}

/// `docs/outcomes/20260904-decision-residue` phase 5: a source's declared
/// `mutation_profile.lateness` prints as an orchestration-only fact — never a
/// plan input — on its inbound-edge block. `daily_events` reads
/// `raw.events`, which declares `mutation_profile.lateness: '2 hours'`
/// (`examples/timeseries/models/sources/raw/events.yml`); `user_daily_spend`
/// reads `raw.transactions`, which declares none, so its report carries no
/// such line.
#[test]
fn explain_prints_lateness_as_orchestration_only() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");
    assert!(
        report.contains("orchestration-only fact: lateness = 2 hours (never a plan input)"),
        "expected the orchestration-only lateness line for raw.events: {report}"
    );

    let report = build_report_for(&project_dir, "user_daily_spend")
        .expect("user_daily_spend has a maintenance plan");
    assert!(
        !report.contains("orchestration-only fact: lateness"),
        "raw.transactions declares no lateness; none should be printed: {report}"
    );
}

/// `--json`'s `inbound_edges[].lateness` carries the same append-stable fact
/// as the text report's orchestration-only line (`docs/outcomes/
/// 20260904-decision-residue` phase 5).
#[test]
fn explain_json_carries_lateness() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events --json");
    assert!(
        output.status.success(),
        "smelt explain daily_events --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let edges = json
        .get("inbound_edges")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level inbound_edges array: {stdout}"));
    let events_edge = edges
        .iter()
        .find(|e| e["name"].as_str().unwrap_or_default().contains("events"))
        .unwrap_or_else(|| panic!("expected an inbound edge for raw.events: {stdout}"));
    assert_eq!(events_edge["lateness"], "2 hours");
}

/// The model's own delta signature (`incremental_models.md` §Surface "CLI",
/// Headline bullet) is the report's first line: `examples/timeseries`'
/// `user_spend_rollup` is a bare `grain: key` model (no key-temporal-
/// locality slice bound) whose own SQL keyed-aggregates its clocked
/// upstream — own headline `keyed upsert over [user_id, spend_date],
/// key-addressed`.
#[test]
fn headline_is_the_reports_first_line() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_spend_rollup")
        .expect("user_spend_rollup has a maintenance plan");

    let first_line = report.lines().next().expect("report has a first line");
    assert!(
        first_line.starts_with("model user_spend_rollup  (emits: keyed upsert over ["),
        "expected the delta-signature headline as the report's first line: {first_line}"
    );
    assert!(
        first_line.contains("key-addressed"),
        "expected the key-addressed claim in the headline: {first_line}"
    );
}

/// A partition-grain model whose own SQL is a straightforward passthrough
/// of a clocked append-only source's columns (no aggregation, no
/// `unique_key:`) derives `AppendOnlyWindow` — its headline must read
/// `append-only within a window, window-addressed by <axis>`.
#[test]
fn partition_grain_headline_is_window_addressed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "plain_passthrough")
        .expect("plain_passthrough has a maintenance plan");

    let first_line = report.lines().next().expect("report has a first line");
    assert!(
        first_line.contains("append-only within a window, window-addressed by d"),
        "expected a window-addressed headline naming the clock axis: {first_line}"
    );
}

/// A composed model (`grain: key` + `timeseries:`, key temporal locality
/// admitted) additionally prints its slice bound and settle bound —
/// `examples/timeseries`' `user_daily_spend` is the worked composed-shape
/// example (mirrors `incremental_models.md`'s `order_facts` illustration).
#[test]
fn composed_headline_appends_slice_bound() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_daily_spend")
        .expect("user_daily_spend has a maintenance plan");

    let first_line = report.lines().next().expect("report has a first line");
    assert!(
        first_line.contains("slice-bounded by spend_date under key temporal locality"),
        "expected a slice-bound clause in the composed headline: {first_line}"
    );
    assert!(
        first_line.contains("settle bound:"),
        "expected a settle bound in the composed headline: {first_line}"
    );
}

/// A model whose own SQL degrades to `General` (a window-function output
/// column the walk cannot classify) names the degrading construct and
/// claims no addressing — `daily_events` (`COUNT(*)` with no column
/// reference) exercises the same fail-closed default.
#[test]
fn general_headline_names_the_degrading_construct() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    let first_line = report.lines().next().expect("report has a first line");
    assert!(
        first_line.contains("general (degraded by:")
            && first_line.contains("not delta-addressable"),
        "expected a general, non-addressable headline: {first_line}"
    );
}

/// The headline's `grain:` clause is the SAME string the report's own
/// `derived grain:` row prints under "Relation contract:" — never a second
/// label.
#[test]
fn headline_grain_label_matches_derived_grain_row() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_spend_rollup")
        .expect("user_spend_rollup has a maintenance plan");

    let first_line = report.lines().next().expect("report has a first line");
    let headline_grain = first_line
        .rsplit("grain: ")
        .next()
        .and_then(|s| s.strip_suffix(')'))
        .expect("headline has a grain: clause");
    let derived_grain_row = report
        .lines()
        .find(|l| l.trim_start().starts_with("derived grain:"))
        .expect("report has a derived grain: row");
    let row_grain = derived_grain_row
        .trim_start()
        .strip_prefix("derived grain: ")
        .expect("derived grain row has a value");
    assert_eq!(
        headline_grain, row_grain,
        "headline grain must match the derived grain: row exactly: {report}"
    );
}

/// A model whose own SQL yields no output-delta shape at all still gets a
/// (degraded) headline rather than crashing or omitting it — `general`
/// covers the None case, never a fabricated shape.
#[test]
fn explain_json_delta_signature_matches_text_headline() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "explain",
            "user_spend_rollup",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run smelt explain --json");
    assert!(
        output.status.success(),
        "smelt explain --json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON output");
    let delta_signature = &json["delta_signature"];
    assert_eq!(delta_signature["shape"], "keyed_upsert");
    assert_eq!(delta_signature["addressing"], "key");
    let keys = delta_signature["keys"]
        .as_array()
        .expect("keys is an array");
    let keys: Vec<&str> = keys.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(keys, vec!["user_id", "spend_date"]);

    let text_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "explain",
            "user_spend_rollup",
            "--project-dir",
            project_dir.to_str().unwrap(),
        ])
        .output()
        .expect("run smelt explain");
    let text = String::from_utf8_lossy(&text_output.stdout);
    let first_line = text.lines().next().expect("report has a first line");
    assert!(
        first_line.contains("keyed upsert over [user_id, spend_date]"),
        "expected the text headline to name the same keys as --json: {first_line}"
    );
}
