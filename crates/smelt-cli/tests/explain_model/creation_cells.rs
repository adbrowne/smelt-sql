use std::path::Path;

use crate::support::build_report_for;

#[test]
fn eventstream_shows_creation_cell_for_silver_upstream() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "gold.eventstream_with_identity")
        .expect("eventstream_with_identity has a maintenance plan");

    assert!(
        report.contains("NewData { source: \"silver.events_deduped\" }"),
        "expected a creation cell for the model upstream silver.events_deduped: {report}"
    );
}

/// `silver.events_enriched` (`docs/plans/20260710-web-analytics-maintenance-demo.md`
/// Phase 7) refs **two** maintained-model upstreams in the same body:
/// `silver.events_deduped` (the composed keyed+timeseries dedupe stage,
/// read 1:1) and `silver.sessions` (clocked by `session_start_date`, joined
/// across the session boundary via the 1-day session-cap Form B filter).
/// The maintenance-plan report must show a creation cell — each with its
/// own derived scan clamp — for BOTH upstreams, demonstrating that the
/// model-upstream edge derivation (`incremental_models.md` §"Upstream model
/// edges") composes across more than one model-to-model ref.
#[test]
fn events_enriched_shows_creation_cells_for_both_model_upstreams() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.events_enriched")
        .expect("events_enriched has a maintenance plan");

    assert!(
        report.contains("NewData { source: \"silver.events_deduped\" }"),
        "expected a creation cell for the model upstream silver.events_deduped: {report}"
    );
    assert!(
        report.contains("NewData { source: \"silver.sessions\" }"),
        "expected a creation cell for the model upstream silver.sessions: {report}"
    );
}
