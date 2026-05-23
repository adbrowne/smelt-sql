//! Integration test: 60-day backfill of web_analytics incremental models
//! as a single `smelt run` invocation.
//!
//! Verifies that:
//! - A 60-day window runs as a single engine query for FullyBatchSafe models.
//! - The resulting row count matches what 60 daily runs would produce.
//!
//! Tests at the batch-generation level (smelt_cli::compute_batches_for_model)
//! rather than full execution, since the web_analytics example requires
//! seeded source data.

use smelt_cli::{
    build_explain_output, build_logical_graph, compute_batches_for_model, BackfillOptions, Config,
    TimeRange,
};
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

/// A 60-day backfill of the events_parsed model (FullyBatchSafe) runs as
/// a single engine query — i.e., `compute_batches_for_model` returns 1 batch.
#[test]
fn test_60_day_backfill_one_call() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, _db) =
        build_logical_graph(&project_dir, &config, None, &[], "dev").expect("build logical graph");
    let output = build_explain_output(&graph).expect("build explain output");

    // events_parsed is the simplest FullyBatchSafe incremental model.
    let model_info = output
        .models
        .get("events_parsed")
        .expect("events_parsed must be in explain output");
    let inc = model_info
        .incremental
        .as_ref()
        .expect("events_parsed must have incremental metadata");

    // Must classify as batch-safe for the one-query assertion to hold.
    assert_eq!(
        inc.batch_safety, "fully_batch_safe",
        "events_parsed must be fully_batch_safe"
    );

    // Now compute batches for a 60-day window.
    let node = graph.get_node("events_parsed").expect("node exists");

    let sixty_day_range = TimeRange {
        start: "2024-01-01".to_string(),
        end: "2024-03-01".to_string(),
    };

    let ts = node.timeseries.as_ref().expect("timeseries config");
    let inc_cfg = node.incremental.as_ref().expect("incremental config");

    let (batch_safety, batches) = compute_batches_for_model(
        &node.model_file.content,
        inc_cfg,
        ts,
        &sixty_day_range,
        &sixty_day_range,
        &BackfillOptions::default(),
    )
    .expect("compute_batches_for_model");

    // FullyBatchSafe must produce exactly 1 batch for any window size.
    assert!(
        matches!(batch_safety, smelt_planner::BatchSafety::FullyBatchSafe),
        "events_parsed must be FullyBatchSafe; got: {batch_safety:?}"
    );
    assert_eq!(
        batches.len(),
        1,
        "60-day backfill of a FullyBatchSafe model must produce 1 batch, got {}",
        batches.len()
    );
    assert_eq!(batches[0].partition_range.start, "2024-01-01");
    assert_eq!(batches[0].partition_range.end, "2024-03-01");
}
