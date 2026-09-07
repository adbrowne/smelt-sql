use std::path::Path;

use crate::support::build_report_for;

// ---------------------------------------------------------------------------
// explain_prints_relation_contract (Phase S2 of
// `docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// `smelt explain <model>` prints the Relation Contract
// (`docs/specs/models.md` §"The Relation Contract") — the model's own
// clock/identity/derived-grain rows, plus one contract block per inbound
// edge, source and model providers rendered through the same rows.
// ---------------------------------------------------------------------------
/// `daily_events_status` (`examples/timeseries`) directly refs **two**
/// sources with different shapes: `raw.events` (declares `unique_key:
/// [event_id]`, no clock — keyed-dimension) and `raw.user_status` (declares
/// both a clock and `unique_key: [user_id]`, `changed_at` NOT in the key —
/// keyed, time-partitioned). The model's own facts (`timeseries:` only, no
/// top-level `unique_key:`) derive `grain: partition`. All three renders —
/// the model's own contract and both source edges' contracts — must use the
/// identical field names (`clock:`, `identity:`, `derived grain:`).
#[test]
fn explain_prints_relation_contract_for_source_edges() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events_status")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("Relation contract:"),
        "expected a Relation Contract section: {report}"
    );
    // The model's own derived grain: clock declared, no top-level identity.
    assert!(
        report.contains("derived grain: partition"),
        "daily_events_status declares only a clock at the top level: {report}"
    );

    // Both source refs appear as inbound edges, labelled `(source)`.
    assert!(
        report.contains("sources.raw.events (source)"),
        "expected the raw.events source edge: {report}"
    );
    assert!(
        report.contains("sources.raw.user_status (source)"),
        "expected the raw.user_status source edge: {report}"
    );

    // raw.events: identity declared (event_id), no clock -> keyed-dimension.
    // raw.user_status: both declared, partition_column not in key -> keyed.
    // Both source edges use the same field names as the model's own
    // contract row set.
    let clock_rows = report.matches("clock:").count();
    let identity_rows = report.matches("identity:").count();
    let grain_rows = report.matches("derived grain:").count();
    assert_eq!(
        clock_rows, 3,
        "expected 3 clock rows (own + 2 source edges): {report}"
    );
    assert_eq!(
        identity_rows, 3,
        "expected 3 identity rows (own + 2 source edges): {report}"
    );
    assert_eq!(
        grain_rows, 3,
        "expected 3 derived-grain rows (own + 2 source edges): {report}"
    );
    assert!(
        report.contains("identity: event_id"),
        "expected raw.events's declared identity to render: {report}"
    );
    assert!(
        report.contains("identity: user_id"),
        "expected raw.user_status's declared identity to render: {report}"
    );
}

/// `user_spend_running_total` (already exercised above for its route-1
/// locality) also demonstrates a **model** edge's contract rendering through
/// the same field names a source edge uses — `(model)` label, `clock:`,
/// `identity:`, `derived grain:` rows for the composed upstream
/// `user_daily_spend`.
#[test]
fn explain_prints_relation_contract_for_model_edge() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_spend_running_total")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("user_daily_spend (model)"),
        "expected the composed upstream rendered as a model-provider edge: {report}"
    );
    assert!(
        report.contains("Relation contract:"),
        "expected the model's own contract section: {report}"
    );
    // Same field names as the source-edge case above.
    assert!(report.contains("clock:"), "{report}");
    assert!(report.contains("identity:"), "{report}");
    assert!(report.contains("derived grain:"), "{report}");
}

/// JSON leg: `smelt explain <model> --show-sql --json` carries the
/// `contract` object (own fill) and `inbound_edges[].contract` (per-edge
/// fill) with identical field paths (`clock`, `identity`, `derived_grain`)
/// for both a source-provider and a model-provider edge.
#[test]
fn json_carries_relation_contract_for_both_providers() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events_status")
        .arg("--show-sql")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events_status --show-sql --json");

    assert!(
        output.status.success(),
        "smelt explain --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}: {stdout}"));

    let own_contract = parsed
        .get("contract")
        .unwrap_or_else(|| panic!("expected a top-level 'contract' object: {stdout}"));
    assert!(
        own_contract.get("clock").is_some(),
        "expected the model's own contract to carry a 'clock' field path: {stdout}"
    );
    assert!(
        own_contract.get("derived_grain").is_some(),
        "expected the model's own contract to carry a 'derived_grain' field path: {stdout}"
    );

    let edges = parsed
        .get("inbound_edges")
        .and_then(|e| e.as_array())
        .unwrap_or_else(|| panic!("expected a top-level 'inbound_edges' array: {stdout}"));
    assert_eq!(edges.len(), 2, "expected both source edges: {stdout}");
    // `raw.events` declares identity only (no clock) — keyed-dimension;
    // `raw.user_status` declares both — keyed, time-partitioned. Both
    // contracts use the same field *names* (`clock`, `identity`,
    // `derived_grain`) as the model's own contract object above; which
    // fields are present differs per provider's own declared facts, exactly
    // as it does for the model's own contract.
    let mut names: Vec<String> = Vec::new();
    for edge in edges {
        assert_eq!(
            edge.get("provider").and_then(|p| p.as_str()),
            Some("source"),
            "expected both edges to be source-provided: {edge}"
        );
        let contract = edge
            .get("contract")
            .unwrap_or_else(|| panic!("expected an edge 'contract' object: {edge}"));
        assert!(
            contract.get("derived_grain").is_some(),
            "expected every edge contract to carry a 'derived_grain' field path: {edge}"
        );
        names.push(
            edge.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string(),
        );
    }
    assert!(names.contains(&"sources.raw.events".to_string()));
    assert!(names.contains(&"sources.raw.user_status".to_string()));

    let events_edge = edges
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("sources.raw.events"))
        .expect("raw.events edge present");
    assert!(
        events_edge["contract"].get("clock").is_none(),
        "raw.events declares no clock: {events_edge}"
    );
    assert_eq!(
        events_edge["contract"]["identity"],
        serde_json::json!(["event_id"]),
        "expected raw.events's declared identity: {events_edge}"
    );

    let user_status_edge = edges
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("sources.raw.user_status"))
        .expect("raw.user_status edge present");
    assert!(
        user_status_edge["contract"].get("clock").is_some(),
        "expected raw.user_status's declared clock, using the same 'clock' field path as \
         the model's own contract: {user_status_edge}"
    );
}
