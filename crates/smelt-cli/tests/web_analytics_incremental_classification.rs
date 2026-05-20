//! Verify that incremental models in `examples/web_analytics/` are classified
//! as actually incremental by the planner's batch-safety analyser, not
//! silently downgraded to full-rebuild on account of an outer-body `OVER`
//! or other safety-check rejection.
//!
//! Background: a model that declares `incremental: enabled` but has an outer
//! `OVER`, `HAVING`, subquery, `LIMIT`, etc. in its body is rejected by the
//! planner's safety check; the CLI logs the rejection as `warn!` and proceeds
//! — the model is silently downgraded to full-rebuild.  The web_analytics
//! example's `silver/sessions` shipped this way for a long time without anyone
//! noticing.  This test prevents that regression: each model that declares
//! itself incremental in the example must classify as `fully_batch_safe` in
//! `smelt explain --json`.

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

fn assert_incremental_and_fully_batch_safe(
    output: &smelt_cli::explain::ExplainOutput,
    model_name: &str,
) {
    let model = output.models.get(model_name).unwrap_or_else(|| {
        panic!(
            "{} not found in explain output; keys: {:?}",
            model_name,
            output.models.keys().collect::<Vec<_>>()
        )
    });

    let incremental = model
        .incremental
        .as_ref()
        .unwrap_or_else(|| panic!("{} must have incremental metadata", model_name));

    assert_eq!(
        incremental.batch_safety, "fully_batch_safe",
        "{} must classify as fully_batch_safe — outer-body OVER/HAVING/LIMIT/etc. silently downgrades it to full-rebuild",
        model_name
    );
}

#[test]
fn web_analytics_incremental_models_classify_as_safe() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, _db) =
        build_logical_graph(&project_dir, &config, None, &[], "dev").expect("build logical graph");
    let output = build_explain_output(&graph).expect("build explain output");

    // Every model that declares `incremental: enabled` in this example must
    // pass the safety classifier.  Listed explicitly so that adding a new
    // incremental model without a classification check fails noisily here.
    for model in &["sessions", "device_user_edges", "events_parsed"] {
        assert_incremental_and_fully_batch_safe(&output, model);
    }
}
