//! Integration test: web_analytics incremental model batch-safety classification.
//!
//! Verifies that `silver.events_parsed` is classified as `FullyBatchSafe` so
//! that a time-range run (e.g. a 60-day backfill) can execute as a single
//! engine query rather than many per-partition chunks.
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

/// `silver.events_parsed` must be classified as `FullyBatchSafe` — any
/// time-range run can execute it as a single engine query.
#[test]
fn test_events_parsed_is_fully_batch_safe() {
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

    let model_info = output
        .models
        .get("silver.events_parsed")
        .expect("silver.events_parsed must be in explain output");
    let inc = model_info
        .incremental
        .as_ref()
        .expect("silver.events_parsed must have incremental metadata");

    assert_eq!(
        inc.batch_safety, "fully_batch_safe",
        "silver.events_parsed must be fully_batch_safe so a 60-day backfill \
         runs as a single engine query"
    );
}
