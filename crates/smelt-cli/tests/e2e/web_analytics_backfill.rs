//! Integration test: web_analytics incremental model batch-safety classification.
//!
//! Verifies that `silver.events_parsed` is classified as `BoundedSafe` with a
//! 3-day context — its 3-day late-arrival acceptance window (`event_date
//! BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days' AND
//! CAST(arrival_time AS DATE)` in `silver/events_parsed.sql`) is a genuine
//! derived reach on its upstream `bronze.raw_events`, so a time-range backfill
//! chunks rather than running as a single unbounded engine query.
//!
//! Tests at the explain-output level since the per-partition batch-counting
//! logic is now internal to `smelt-runtime::execute_project`.

use smelt_cli::{build_dependency_graph, build_explain_output, Config};
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

/// `silver.events_parsed` must be classified `BoundedSafe` with `context=3d`
/// — the derived reach from its 3-day late-arrival acceptance filter.
#[test]
fn test_events_parsed_is_bounded_safe_with_3day_context() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, db) = build_dependency_graph(&project_dir, &config, None, &[], "dev")
        .expect("build logical graph");
    let fn_bodies = smelt_runtime::build_fn_body_map(
        &db,
        smelt_db::Workspace::try_get(&db).expect("workspace"),
    );
    let output = build_explain_output(&graph, &config, &fn_bodies, &HashMap::new(), None)
        .expect("build explain output");

    let model_info = output
        .models
        .get("silver.events_parsed")
        .expect("silver.events_parsed must be in explain output");
    let inc = model_info
        .incremental
        .as_ref()
        .expect("silver.events_parsed must have incremental metadata");

    assert!(
        inc.batch_safety.starts_with("bounded_safe("),
        "silver.events_parsed must be bounded_safe (its 3-day late-arrival \
         acceptance filter is a genuine derived reach, not a transparent \
         zero-margin slice); got: {}",
        inc.batch_safety
    );
    assert!(
        inc.batch_safety.contains("context=3d"),
        "silver.events_parsed's batch_safety must report the 3-day late-arrival \
         context; got: {}",
        inc.batch_safety
    );
}
