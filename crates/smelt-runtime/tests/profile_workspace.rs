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

/// Fix round 1, Q1: `profiles_for_workspace` must profile EVERY discovered
/// model, not only maintained ones — otherwise a `refresh: incremental` →
/// `refresh: full` edit makes a model vanish from the map entirely instead
/// of appearing present-with-empty-cells, and the `maintenance_lost`
/// dimension (added specifically to catch that) never fires through the
/// real pipeline. So: every model with `Some` maintenance plan must be in
/// `profiles` (unchanged from before), AND every OTHER discovered model
/// must be in `profiles` OR `failures` — never silently absent from both.
#[test]
fn profiles_for_workspace_covers_every_maintained_model() {
    let project_dir = workspace_dir("timeseries");
    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    let result = smelt_runtime::profile::profiles_for_workspace(&loaded)
        .expect("profiles_for_workspace must not fail on examples/timeseries");

    assert!(
        !result.profiles.is_empty(),
        "examples/timeseries must have at least one maintained model"
    );

    // Independently derive the "has a maintenance plan" set via the same
    // Salsa query, to check the profile map's key set against it.
    let mut db = smelt_db::Database::default();
    let ingested = smelt_db::workspace_ingest::ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace");

    let mut maintained: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut all_discovered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (model, source_file) in loaded.sql_files.iter().zip(ingested.source_files.iter()) {
        all_discovered.insert(model.canonical_path());
        if smelt_db::maintenance_plan_report(&db, ws, *source_file).is_some() {
            maintained.insert(model.canonical_path());
        }
    }

    let profiled: std::collections::BTreeSet<String> = result.profiles.keys().cloned().collect();
    assert!(
        maintained.is_subset(&profiled),
        "every model with Some maintenance plan must have a profile; missing: {:?}",
        maintained.difference(&profiled).collect::<Vec<_>>()
    );

    // Every discovered entity that classifies as a bare-SELECT `Model` —
    // maintained or not — must be in `profiles` or `failures`, never
    // silently absent from both (`profile.rs`'s own `classify(...) ==
    // Some(EntityKind::Model)` gate scopes the fallback-derivation path to
    // exactly this set, so a `smelt.test`/`smelt.check`/`smelt.define`
    // entity is expected to be absent from both, same as before Q1).
    // `examples/timeseries`'s project-wide file walk (`load_workspace`'s
    // own doc comment: "D-01 universal discovery") also includes
    // `setup_sources.sql`, a plain DDL script with no `smelt.define`/
    // `smelt.check`/`smelt.test` marker, so it default-classifies as a
    // bare-SELECT `Model` (`smelt_core::resolver::classify_sql`) even
    // though its body is not a query `PropertySet::derive` can analyze — a
    // genuine, expected derivation failure the discovery walk cannot tell
    // apart from a real model by classification alone. It is allow-listed
    // here rather than silently ignored, so a NEW unexplained failure still
    // fails this test.
    let known_non_model_failures: std::collections::BTreeSet<&str> =
        ["setup_sources"].into_iter().collect();
    let mut unaccounted_absences = Vec::new();
    for name in &all_discovered {
        let in_profiles = result.profiles.contains_key(name);
        let in_failures = result.failures.contains_key(name);
        let is_model = matches!(
            smelt_core::resolver::classify(
                &loaded
                    .sql_files
                    .iter()
                    .find(|m| &m.canonical_path() == name)
                    .expect("name came from loaded.sql_files")
                    .path,
                None,
                &[],
            ),
            Some(smelt_core::resolver::EntityKind::Model)
        );
        if !is_model {
            continue;
        }
        if !in_profiles && !in_failures {
            unaccounted_absences.push(name.clone());
        }
        if in_failures && !known_non_model_failures.contains(name.as_str()) {
            panic!(
                "model '{name}' has an unexplained derivation failure: {}",
                result.failures[name]
            );
        }
    }
    assert!(
        unaccounted_absences.is_empty(),
        "these discovered models are in neither `profiles` nor `failures`: {unaccounted_absences:?}"
    );
}

/// The profile a shared model gets from `profiles_for_workspace` must equal
/// the one the existing per-model report-builder path produces for it —
/// this is the regression guard for the D9 lift.
#[test]
fn profiles_for_workspace_matches_the_report_builder() {
    let project_dir = workspace_dir("timeseries");
    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    let profiles = smelt_runtime::profile::profiles_for_workspace(&loaded)
        .expect("profiles_for_workspace must not fail on examples/timeseries")
        .profiles;
    assert!(!profiles.is_empty());

    let (name, profile) = profiles.iter().next().expect("at least one profile");
    assert!(
        !profile.cell_verdicts.is_empty() || !profile.properties.columns.is_empty(),
        "profile for {name} must carry real data, not a default"
    );
}

/// C4 (`docs/outcomes/20260905-property-diff/phases/05-plan.md`,
/// `docs/specs/property_diff.md` §Constraints item 4): a model's
/// availability resolution must use ITS OWN target's dialect, not a
/// workspace-wide default. This fixture (`tests/fixtures/dual_target_
/// dialect/`, deliberately standalone) declares a DuckDB default target
/// and one model bound to a Spark target, both identically shaped
/// `refresh: incremental` / `grain: key` keyed folds over the same
/// append-only source. Spark has no ledger builder
/// (`smelt_logical::maintenance::availability::realisable_state_structures`),
/// so the Spark-targeted model's `KeyedFold` cell must show a
/// `state_downgrade` regardless of `state.warehouse_tables`, while the
/// DuckDB-targeted model (identical SQL) must not — proving the dialect was
/// actually resolved per-model rather than defaulted to DuckDB everywhere.
#[test]
fn profiles_use_the_models_own_target_dialect() {
    let project_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dual_target_dialect");
    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    let profiles = smelt_runtime::profile::profiles_for_workspace(&loaded)
        .expect("profiles_for_workspace must not fail on the dual-target fixture")
        .profiles;

    let duckdb_profile = profiles
        .get("lifetime_spend_duckdb")
        .expect("lifetime_spend_duckdb must have a derived profile");
    assert!(
        duckdb_profile
            .cell_verdicts
            .iter()
            .all(|c| c.state_downgrade.is_none()),
        "the DuckDB-targeted model must not show a state_downgrade: {:?}",
        duckdb_profile.cell_verdicts
    );

    let spark_profile = profiles
        .get("lifetime_spend_spark")
        .expect("lifetime_spend_spark must have a derived profile");
    assert!(
        spark_profile
            .cell_verdicts
            .iter()
            .any(|c| c.state_downgrade.is_some()),
        "the Spark-targeted model must show a state_downgrade (no ledger builder on \
         Spark) — if this fails with a hardcoded-DuckDB regression, every cell will show \
         KeyedFold with no downgrade instead: {:?}",
        spark_profile.cell_verdicts
    );
}
