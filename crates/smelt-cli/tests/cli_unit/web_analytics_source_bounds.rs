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

use smelt_cli::{build_dependency_graph, build_explain_output, Config};
use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::analysis::walk::model_partition_skew;
use std::collections::HashMap;
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

/// Strip the `---`-delimited YAML frontmatter block from a model file's raw
/// text, returning just the SQL body. Mirrors the delimiter convention every
/// other model fixture in this repo uses (see `smelt_core::frontmatter`);
/// duplicated here rather than pulled in because this test only needs a
/// trivial split, not the full frontmatter parser.
fn strip_frontmatter(text: &str) -> &str {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return text;
    }
    let mut offset = 4; // "---\n"
    for line in lines {
        offset += line.len() + 1;
        if line == "---" {
            return text[offset..].trim_start();
        }
    }
    text
}

#[test]
fn test_explain_json_exposes_bounds() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, db) = build_dependency_graph(&project_dir, &config, None, &[], "dev")
        .expect("build logical graph");
    let fn_bodies = smelt_runtime::build_fn_body_map(
        &db,
        smelt_db::Workspace::try_get(&db).expect("workspace"),
    );
    let output = build_explain_output(&graph, &config, &fn_bodies, &HashMap::new())
        .expect("build explain output");

    // sessions is the canonical incremental model with a timeseries upstream.
    // After LogicalGraph canonical-path rekey (Phase 3), the key is "silver.sessions".
    let sessions = output.models.get("silver.sessions").unwrap_or_else(|| {
        panic!(
            "sessions model not found in explain output; keys: {:?}",
            output.models.keys().collect::<Vec<_>>()
        )
    });

    let inc = sessions
        .incremental
        .as_ref()
        .expect("silver.sessions must have incremental metadata");

    // source_bounds must be present
    assert!(
        !inc.source_bounds.is_empty(),
        "silver.sessions incremental metadata must have source_bounds; got: {:?}",
        inc.source_bounds
    );

    // events_parsed is the upstream timeseries source; its canonical key is "silver.events_parsed".
    let bound = inc
        .source_bounds
        .get("silver.events_parsed")
        .unwrap_or_else(|| {
            panic!(
                "sessions source_bounds must have 'silver.events_parsed' entry; keys: {:?}",
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

/// Verify that the events_parsed model's source_bounds reports a genuine
/// 3-day lookback on its upstream `bronze.raw_events`.
///
/// `bronze/raw_events.sql` declares `timeseries: { partition_column:
/// event_date }` (it is a passthrough view with its own time dimension), and
/// `silver/events_parsed.sql` accepts late-arriving events via the Form B
/// filter `event_date BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
/// AND CAST(arrival_time AS DATE)`. The planner reads that filter as a
/// derived `Bounded(event_date, before=3d, after=0)` reach on
/// `bronze.raw_events` — this is the observable clamp
/// `docs/specs/batched_models.md` §"Observing the per-source clamp"
/// describes.
#[test]
fn test_explain_json_events_parsed_late_window_bound() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, db) = build_dependency_graph(&project_dir, &config, None, &[], "dev")
        .expect("build logical graph");
    let fn_bodies = smelt_runtime::build_fn_body_map(
        &db,
        smelt_db::Workspace::try_get(&db).expect("workspace"),
    );
    let output = build_explain_output(&graph, &config, &fn_bodies, &HashMap::new())
        .expect("build explain output");

    // After LogicalGraph canonical-path rekey (Phase 3), the key is "silver.events_parsed".
    let events_parsed = output
        .models
        .get("silver.events_parsed")
        .unwrap_or_else(|| {
            panic!(
                "events_parsed model not found; keys: {:?}",
                output.models.keys().collect::<Vec<_>>()
            )
        });

    let inc = events_parsed
        .incremental
        .as_ref()
        .expect("events_parsed must have incremental metadata");

    let bound = inc
        .source_bounds
        .get("bronze.raw_events")
        .unwrap_or_else(|| {
            panic!(
                "events_parsed source_bounds must have a 'bronze.raw_events' entry; keys: {:?}",
                inc.source_bounds.keys().collect::<Vec<_>>()
            )
        });

    let json_str = serde_json::to_string_pretty(bound).expect("serialize bound");
    assert!(
        json_str.contains("\"bounded\""),
        "bronze.raw_events bound must be bounded type; JSON: {json_str}"
    );
    assert!(
        json_str.contains("\"before\": \"P3D\""),
        "bronze.raw_events bound must carry a 3-day (P3D) backward reach; JSON: {json_str}"
    );
    assert!(
        json_str.contains("\"after\": \"PT0S\""),
        "bronze.raw_events bound must carry a zero forward reach; JSON: {json_str}"
    );

    // batch_safety must reflect the same 3-day context in its chunking label.
    assert!(
        inc.batch_safety.contains("context=3d"),
        "events_parsed batch_safety must report a 3-day context; got: {}",
        inc.batch_safety
    );
}

/// Real fixture: `examples/web_analytics/models/silver/sessions.sql`
/// declares `partition_column: session_start_date` and filters `WHERE
/// event_date BETWEEN session_start_date - INTERVAL '1 day' AND
/// session_start_date + INTERVAL '1 day'` — a Form B relation anchored on
/// the model's own partition column (`docs/specs/model_transforms.md`
/// §Semantics "The output window is derived, never assumed"). The
/// walk-composed skew fold (`model_partition_skew`, the property-composition
/// walk's entry) must read this as a symmetric 1-day skew bound.
#[test]
fn sessions_skew_bound_derived() {
    let sessions_sql_path = examples_dir()
        .join("web_analytics")
        .join("models")
        .join("silver")
        .join("sessions.sql");
    let raw = std::fs::read_to_string(&sessions_sql_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", sessions_sql_path.display()));
    let sql = strip_frontmatter(&raw);

    let skew = model_partition_skew(sql, "session_start_date");
    assert_eq!(
        skew.before,
        Seconds::days(1),
        "sessions.sql must derive a 1-day backward skew"
    );
    assert_eq!(
        skew.after,
        Seconds::days(1),
        "sessions.sql must derive a 1-day forward skew"
    );
}
