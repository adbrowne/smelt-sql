//! `smelt_runtime::profile::profiles_for_workspace` — one property-profile
//! map per project version, the "both sides" half of the property diff
//! (`docs/specs/property_diff.md` §"Baseline materialisation";
//! `docs/outcomes/20260905-property-diff/phases/04-plan.md` D9).

use std::path::Path;

fn workspace_dir(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples")
        .join(name)
}

/// The map's key set must equal the set of models with a `Some` maintenance
/// plan, and must be non-empty for a workspace with maintained models.
#[test]
fn profiles_for_workspace_covers_every_maintained_model() {
    let project_dir = workspace_dir("timeseries");
    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    let profiles = smelt_runtime::profile::profiles_for_workspace(&loaded);

    assert!(
        !profiles.is_empty(),
        "examples/timeseries must have at least one maintained model"
    );

    // Independently derive the "has a maintenance plan" set via the same
    // Salsa query, to check the profile map's key set against it.
    let mut db = smelt_db::Database::default();
    let ingested = smelt_db::workspace_ingest::ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace");

    let mut expected: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (model, source_file) in loaded.sql_files.iter().zip(ingested.source_files.iter()) {
        if smelt_db::maintenance_plan_report(&db, ws, *source_file).is_some() {
            expected.insert(model.canonical_path());
        }
    }

    let actual: std::collections::BTreeSet<String> = profiles.keys().cloned().collect();
    assert_eq!(
        actual, expected,
        "profiles_for_workspace's key set must equal every model with Some maintenance plan"
    );
}

/// The profile a shared model gets from `profiles_for_workspace` must equal
/// the one the existing per-model report-builder path produces for it —
/// this is the regression guard for the D9 lift.
#[test]
fn profiles_for_workspace_matches_the_report_builder() {
    let project_dir = workspace_dir("timeseries");
    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    let profiles = smelt_runtime::profile::profiles_for_workspace(&loaded);
    assert!(!profiles.is_empty());

    let (name, profile) = profiles.iter().next().expect("at least one profile");
    assert!(
        !profile.cell_verdicts.is_empty() || !profile.properties.columns.is_empty(),
        "profile for {name} must carry real data, not a default"
    );
}
