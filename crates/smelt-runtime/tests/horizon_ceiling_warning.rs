//! `horizon_ceiling` declaration — compile-time warning wiring (DC4 of
//! `docs/plans/20260704-model-updates-l3-declarations.md`;
//! `docs/specs/incremental_models.md` §"Windowed maintenance and the
//! horizon").
//!
//! Loads the real `examples/horizon_ceiling_comfortable` and
//! `examples/horizon_ceiling_tight` fixtures' downstream model files,
//! extracts the declared `horizon_ceiling` from frontmatter via
//! `smelt_core::extract_file_metadata`, and calls `build_source_bound_map`
//! exactly as `smelt-runtime`'s incremental execute loop does — proving the
//! comfortable-ceiling fixture emits no warning and the tight-ceiling
//! fixture emits exactly one, naming both the derived reach and the
//! declared ceiling. The clamp bound itself (`before_secs`/`after_secs`) is
//! asserted unchanged by the ceiling in both cases — the "cannot narrow"
//! invariant.

use smelt_core::metadata::{extract_file_metadata, FileMetadata};
use smelt_runtime::build_source_bound_map;
use std::collections::HashMap;
use std::path::Path;

/// Load `models/rolling_amount.sql` from an example fixture and return
/// `(sql_body, horizon_ceiling)`.
fn load_fixture(example_dir: &str) -> (String, Option<smelt_core::config::DataLatency>) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .join("../..")
        .join(example_dir)
        .join("models/rolling_amount.sql");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));

    let metadata = match extract_file_metadata(&source).expect("fixture frontmatter must parse") {
        FileMetadata::Single { metadata, .. } => *metadata,
        other => panic!("expected a single-model fixture, got {other:?}"),
    };

    let sql_body = smelt_parser::strip_frontmatter(&source);
    (sql_body, metadata.horizon_ceiling)
}

fn dep_ts() -> HashMap<String, (Vec<String>, String)> {
    let mut dep_ts = HashMap::new();
    dep_ts.insert(
        "smelt.events".to_string(),
        (vec!["events".to_string()], "event_date".to_string()),
    );
    dep_ts
}

#[test]
fn comfortable_ceiling_emits_no_warning() {
    let (sql, ceiling) = load_fixture("examples/horizon_ceiling_comfortable");
    let ceiling = ceiling.expect("fixture must declare horizon_ceiling");
    assert_eq!(
        ceiling.seconds,
        30 * 86400,
        "fixture's declared ceiling must be 30 days"
    );

    let (bounds, warnings) = build_source_bound_map(&sql, &dep_ts(), Some(&ceiling));

    assert!(
        warnings.is_empty(),
        "a 2-hour derived reach is well inside a 30-day ceiling — expected no warnings, got: {warnings:?}"
    );

    let bound = bounds
        .get("smelt.events")
        .expect("smelt.events must appear in the bound map");
    assert_eq!(
        bound.before_secs,
        2 * 3600,
        "the clamp must use the derived 2-hour reach"
    );
}

#[test]
fn tight_ceiling_emits_exactly_one_warning_naming_both_values() {
    let (sql, ceiling) = load_fixture("examples/horizon_ceiling_tight");
    let ceiling = ceiling.expect("fixture must declare horizon_ceiling");
    assert_eq!(
        ceiling.seconds, 3600,
        "fixture's declared ceiling must be 1 hour"
    );

    let (bounds, warnings) = build_source_bound_map(&sql, &dep_ts(), Some(&ceiling));

    assert_eq!(
        warnings.len(),
        1,
        "a 2-hour derived reach exceeds a 1-hour ceiling — expected exactly one warning, got: {warnings:?}"
    );
    let warning = &warnings[0];
    assert!(
        warning.contains("7200") || warning.contains("2d") || warning.to_lowercase().contains("2h"),
        "warning must name the derived reach (7200s / 2h): {warning}"
    );
    assert!(
        warning.contains("3600"),
        "warning must name the declared ceiling (3600s / 1h): {warning}"
    );

    // The "cannot narrow" invariant: the clamp still uses the derived
    // 2-hour reach even though it exceeds the declared 1-hour ceiling.
    let bound = bounds
        .get("smelt.events")
        .expect("smelt.events must appear in the bound map");
    assert_eq!(
        bound.before_secs,
        2 * 3600,
        "a smaller declared ceiling must never narrow the derived clamp"
    );
}
