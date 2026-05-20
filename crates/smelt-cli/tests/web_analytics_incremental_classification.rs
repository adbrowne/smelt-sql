//! Verify that incremental models in `examples/web_analytics/` are classified
//! as actually incremental by the planner's batch-safety analyser, not
//! silently downgraded to full-rebuild on account of an outer-body `OVER`
//! or other safety-check rejection.
//!
//! Background: `silver/sessions` declares `incremental: enabled` but the
//! planner's safety check rejects `OVER` in the outer body and the CLI logs
//! the rejection as `warn!` and proceeds — the model is silently downgraded
//! to full-rebuild.  The web_analytics example expressed its session-start-
//! date column via `FIRST_VALUE OVER (...)` in the outer body for a long
//! time without anyone noticing.  This test prevents that regression: each
//! model that declares itself incremental must classify as `fully_batch_safe`
//! (or another non-downgraded variant) in `smelt explain --json`.

use smelt_cli::{build_explain_output, build_logical_graph, Config};
use std::path::Path;

fn examples_dir() -> &'static Path {
    // crates/smelt-cli/tests/ → repo root → examples/
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
fn web_analytics_silver_sessions_classifies_as_incremental() {
    let project_dir = examples_dir().join("web_analytics");
    let config = Config::load(&project_dir).expect("load config");
    let (graph, _db) =
        build_logical_graph(&project_dir, &config, None, &[], "dev").expect("build logical graph");
    let output = build_explain_output(&graph).expect("build explain output");

    let sessions = output.models.get("sessions").unwrap_or_else(|| {
        panic!(
            "sessions not found in explain output; keys: {:?}",
            output.models.keys().collect::<Vec<_>>()
        )
    });

    let incremental = sessions
        .incremental
        .as_ref()
        .expect("sessions must have incremental metadata");

    assert_eq!(
        incremental.batch_safety, "fully_batch_safe",
        "sessions must classify as fully_batch_safe — an outer-body OVER or other safety rejection silently downgrades it to full-rebuild"
    );
}
