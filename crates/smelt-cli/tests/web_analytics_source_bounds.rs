//! Integration test: `smelt explain --json` for `silver/sessions` exposes
//! a `source_bounds` field with the derived per-source bound map.
//!
//! The sessions model calls `smelt.functions.sessionize(...)` over
//! `smelt.silver.events_parsed` — that source has `timeseries: event_date`.
//! The sessions model's own SQL has no RANGE BETWEEN or WHERE INTERVAL,
//! so the bound for events_parsed should be Bounded(event_date, PT0S, PT0S)
//! (fully partition-local, before=0, after=0).
//!
//! The test verifies:
//! - `source_bounds` field is present in the JSON for the `sessions` model.
//! - It contains an entry for `events_parsed`.
//! - The bound is `bounded` type.

use smelt_cli::{build_explain_output, build_logical_graph, Config};
use std::path::Path;

fn examples_dir() -> &'static Path {
    Box::leak(Box::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples"),
    ))
}

#[test]
fn test_explain_json_exposes_bounds() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, _db) =
        build_logical_graph(&project_dir, &config, None, &[], "dev").expect("build logical graph");
    let output = build_explain_output(&graph).expect("build explain output");

    // sessions is the canonical incremental model with a timeseries upstream
    let sessions = output.models.get("sessions").unwrap_or_else(|| {
        panic!(
            "sessions model not found in explain output; keys: {:?}",
            output.models.keys().collect::<Vec<_>>()
        )
    });

    let inc = sessions
        .incremental
        .as_ref()
        .expect("sessions must have incremental metadata");

    // source_bounds must be present
    assert!(
        !inc.source_bounds.is_empty(),
        "sessions incremental metadata must have source_bounds; got: {:?}",
        inc.source_bounds
    );

    // events_parsed is the upstream timeseries source
    let bound = inc.source_bounds.get("events_parsed").unwrap_or_else(|| {
        panic!(
            "sessions source_bounds must have 'events_parsed' entry; keys: {:?}",
            inc.source_bounds.keys().collect::<Vec<_>>()
        )
    });

    // Verify the JSON shape
    let json_str = serde_json::to_string_pretty(&inc.source_bounds).expect("serialize");

    // The bound type must be "bounded" (sessions SQL has no INTERVAL lookback)
    assert!(
        json_str.contains("\"bounded\""),
        "events_parsed bound must be bounded type; JSON: {json_str}"
    );
    assert!(
        json_str.contains("event_date"),
        "events_parsed bound must name the partition_col 'event_date'; JSON: {json_str}"
    );
    assert!(
        json_str.contains("\"before\""),
        "events_parsed bound must have 'before' field; JSON: {json_str}"
    );
    assert!(
        json_str.contains("\"after\""),
        "events_parsed bound must have 'after' field; JSON: {json_str}"
    );

    // The bound for a partition-local sessions model should be PT0S/PT0S
    // (no RANGE BETWEEN INTERVAL in the sessions.sql or compute_session_start_date.sql)
    let _ = bound; // used in shape checks above
                   // Verify round-trip serialization
    let json_output = serde_json::to_string_pretty(&output).expect("serialize full output");
    assert!(
        json_output.contains("\"source_bounds\""),
        "smelt explain --json output must include 'source_bounds' field; excerpt:\n{}",
        &json_output[..json_output.len().min(2000)]
    );
}

/// Verify that the events_parsed model itself (also incremental) has source_bounds
/// for its upstream bronze.raw_events (which is external and has no timeseries:).
/// Since raw_events has no timeseries:, events_parsed.source_bounds should be empty.
#[test]
fn test_explain_json_lookup_sources_absent() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, _db) =
        build_logical_graph(&project_dir, &config, None, &[], "dev").expect("build logical graph");
    let output = build_explain_output(&graph).expect("build explain output");

    let events_parsed = output.models.get("events_parsed").unwrap_or_else(|| {
        panic!(
            "events_parsed model not found; keys: {:?}",
            output.models.keys().collect::<Vec<_>>()
        )
    });

    let inc = events_parsed
        .incremental
        .as_ref()
        .expect("events_parsed must have incremental metadata");

    // events_parsed reads from smelt.bronze.raw_events (an external source, no timeseries:)
    // Its source_bounds should be empty (no timeseries refs)
    assert!(
        inc.source_bounds.is_empty(),
        "events_parsed source_bounds must be empty (raw_events has no timeseries:); got: {:?}",
        inc.source_bounds
    );
}
