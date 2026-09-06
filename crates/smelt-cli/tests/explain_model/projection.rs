use std::path::Path;

use crate::support::build_report_for;

/// A composed model under locality route 1 (key-embedded) reports an
/// *exact* observed-delta projection — no widening, since a stored row's
/// partition value is a per-key constant under this route.
#[test]
fn explain_prints_exact_projection_for_a_route_one_composed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report =
        build_report_for(&project_dir, "user_daily_spend").expect("model has a maintenance plan");

    assert!(
        report.contains("observed-delta projection: exact (key-embedded)"),
        "expected an exact projection line for route 1: {report}"
    );
}

/// A composed model under locality route 3 (recurrence-bounded) reports a
/// *widened* observed-delta projection — a key's partition value may move
/// under this route, so the projected dirt widens backward by `r` plus the
/// route's own margins (`silver.events_deduped`, the flagship composed
/// dedupe fixture).
#[test]
fn explain_prints_widened_projection_for_a_route_three_composed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.events_deduped")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("observed-delta projection: widened by `r` + margins"),
        "expected a widened projection line for route 3: {report}"
    );
}

/// A bare keyed model (identity, no established key temporal locality) has
/// no partition axis to project observed deltas onto at all — the report
/// must show no projection row, distinct from a composed model's exact or
/// widened form.
#[test]
fn explain_shows_no_projection_row_for_a_bare_keyed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.device_user_edges")
        .expect("model has a maintenance plan");

    assert!(
        !report.contains("Key temporal locality:"),
        "silver.device_user_edges is bare keyed — no locality section expected: {report}"
    );
    assert!(
        !report.contains("observed-delta projection:"),
        "a bare keyed model must print no projection row at all: {report}"
    );
}

// ---------------------------------------------------------------------------
// Write variant (`docs/plans/20260715-composed-axes-conditional-
// maintenance.md` Phase G1; `docs/specs/incremental_models.md` §"Windowed
// maintenance and the horizon" category 2, §"Interchangeability and
// choice"): `smelt explain` shows which matched-arm shape a suppressible
// cell's conditional-variant dimension resolves to, and why. Real fixtures
// today only ever derive a steady-state trigger for `ColumnScopedMerge`/
// `KeyedFold` cells (`derive_backfill` always emits `Technique::DeleteInsert`
// for `Trigger::Backfill`, and `Trigger::ColumnAdded` cells are only
// constructed from an explicit `ModelDiff` a plain `smelt explain` never
// supplies) — so the first-build-posture branch is exercised directly
// against `build_maintenance_plan_report` with a hand-built
// `MaintenancePlanResult`, the same way `crates/smelt-runtime/tests/
// technique_lowering.rs` hand-builds `PlanCell`s to reach shapes no real
// fixture derives yet.
// ---------------------------------------------------------------------------
