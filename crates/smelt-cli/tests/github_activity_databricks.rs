#![cfg(feature = "duckdb")]
//! Offline preconditions for the first live Databricks run
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/06-plan.md`).
//! These tests need no workspace and no credential: they assert the
//! `databricks` target's source resolution, its non-interference with the
//! no-`--target` default, and that its config parses without touching the
//! network. Mirrors `github_activity_bq_oracle::
//! the_oracle_target_resolves_sources_to_the_shared_tables` /
//! `adding_the_oracle_target_does_not_move_the_default`.

use std::path::PathBuf;

/// The committed project's Databricks dogfood target.
const DATABRICKS_TARGET: &str = "databricks";
/// Phase 5's loader landed both physical tables in this Unity Catalog schema.
const DOGFOOD_SCHEMA: &str = "smelt_dogfood";

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(2)
        .expect("crates/smelt-cli is two levels below the repo root")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repo_root().join("examples/github_activity")
}

fn example_config() -> smelt_core::config::Config {
    let path = example_dir().join("smelt.yml");
    serde_yaml::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// The `databricks` target must resolve `smelt.sources.raw.github_events` and
/// its arrival-partitioned twin to the physical tables phase 5's loader
/// actually wrote (`smelt_dogfood.github_events` /
/// `smelt_dogfood.github_events_arrival`), not the default
/// `<schema>.sources_raw_*` mapping — otherwise every model run against this
/// target would read (or try to create) a table that does not exist.
#[test]
fn the_databricks_target_resolves_sources_to_the_loaded_tables() {
    let config = example_config();
    let infos = smelt_core::discover_source_infos(&example_dir(), &config.paths);

    for (segments, expected) in [
        (
            ["sources", "raw", "github_events"],
            "smelt_dogfood.github_events",
        ),
        (
            ["sources", "raw", "github_events_arrival"],
            "smelt_dogfood.github_events_arrival",
        ),
    ] {
        let info = infos
            .iter()
            .find(|s| s.address_segments == segments)
            .unwrap_or_else(|| panic!("no source declared at {segments:?}"));

        let resolved = info.db_name_for_target(DATABRICKS_TARGET, DOGFOOD_SCHEMA);
        assert_eq!(
            resolved,
            expected,
            "the `{DATABRICKS_TARGET}` target must resolve `{}` to the table phase 5's loader \
             wrote — otherwise the model set reads from a table nothing populated",
            segments.join(".")
        );

        let default_mapping = format!("{DOGFOOD_SCHEMA}.{}", segments.join("_"));
        assert_ne!(
            resolved, default_mapping,
            "the databricks target fell back to the default source mapping — this is exactly \
             the mismatch the `databricks:` name-map entry exists to prevent"
        );
    }
}

/// Adding a third real target must not move the no-`--target` default.
/// `target: dev` is pinned in `smelt.yml` precisely because the fallback when
/// it is unset is the alphabetically-first target name
/// (`smelt-runtime/src/profile.rs`), and `databricks` sorts ahead of both
/// `bigquery` and `dev`.
#[test]
fn adding_the_databricks_target_does_not_move_the_default() {
    let config = example_config();

    assert!(
        config.targets.contains_key(DATABRICKS_TARGET),
        "the committed project must declare the `{DATABRICKS_TARGET}` target"
    );

    let alphabetically_first = {
        let mut names: Vec<&String> = config.targets.keys().collect();
        names.sort();
        names
            .first()
            .map(|s| s.to_string())
            .expect("the project declares at least one target")
    };
    let resolved = config
        .target
        .clone()
        .unwrap_or_else(|| alphabetically_first.clone());

    assert_eq!(
        resolved, "dev",
        "the no-`--target` default must stay `dev`; the databricks target is only ever \
         selected with `--target {DATABRICKS_TARGET}`"
    );
    assert_ne!(
        alphabetically_first, "dev",
        "the `target: dev` pin has stopped being load-bearing — if `dev` ever sorts first on \
         its own, this test no longer proves the pin holds the default in place"
    );
}

/// The `databricks` target block parses to `BackendType::Databricks`, carries
/// no `warehouse`/`format` key (refused by `smelt-core::config`), and needs no
/// network to load — it only reads and validates the committed YAML.
#[test]
fn the_databricks_target_parses_and_needs_no_credential_to_load() {
    let config = example_config();
    let target = config
        .targets
        .get(DATABRICKS_TARGET)
        .unwrap_or_else(|| panic!("`{DATABRICKS_TARGET}` target must be declared"));

    assert_eq!(
        target
            .backend_type()
            .expect("the databricks target resolves to a known backend type"),
        smelt_core::config::BackendType::Databricks,
        "`type: databricks` must resolve to `BackendType::Databricks`"
    );
    assert_eq!(
        target.catalog.as_deref(),
        Some("workspace"),
        "the dogfood target must be catalog-qualified"
    );
    assert_eq!(
        target.schema, DOGFOOD_SCHEMA,
        "the dogfood target must point at the schema phase 5's loader populated"
    );
    assert!(
        target.warehouse.is_none(),
        "a `databricks` target must carry no `warehouse` key (config.rs refuses it; Free \
         Edition has no host-visible warehouse path)"
    );
}
