use std::path::Path;

use crate::support::build_report_for;

// ---------------------------------------------------------------------------
// Region row identity (P2, `docs/specs/model_properties.md` §"Region row
// identity") — `smelt explain` prints each cell's row identity alongside its
// technique (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
// Phase C3).
// ---------------------------------------------------------------------------
/// `user_daily_spend` (`examples/timeseries`) is a `grain: key` model whose
/// key is the outermost `GROUP BY user_id, spend_date` — no top-level
/// `unique_key:` is written, so the row identity is the walk's own proven
/// grain key, printed right alongside the cell's technique line.
#[test]
fn explain_prints_key_row_identity_for_a_group_by_keyed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report =
        build_report_for(&project_dir, "user_daily_spend").expect("model has a maintenance plan");

    assert!(
        report.contains("technique:"),
        "expected a technique line to anchor the row-identity line against: {report}"
    );
    assert!(
        report.contains("region key: Key"),
        "expected a Key(...) region key for the GROUP BY-keyed output: {report}"
    );
    assert!(
        report.contains("user_id") && report.contains("spend_date"),
        "expected the proven grain key's own columns named in the report: {report}"
    );
}

/// `daily_events_status` (`examples/timeseries`) is a `grain: partition`
/// model with no top-level `unique_key:` and no `GROUP BY` — no key can be
/// established, so every cell's row identity falls back to the
/// identity-free `WholeRow` multiset diff.
#[test]
fn explain_prints_whole_row_identity_for_a_keyless_partition_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events_status")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("region key: WholeRow"),
        "expected the keyless partition-grain fallback to be WholeRow: {report}"
    );
}
