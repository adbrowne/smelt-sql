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
use std::process::Command;

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

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
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

/// `host: ${SMELT_DBX_HOSTNAME}` reads the bare-hostname variable, never
/// `SMELT_DBX_HOST` — a scheme-bearing `SMELT_DBX_HOST` sitting in the
/// environment (as `scripts/dbx-dogfood-env.sh` always leaves it, for
/// `dbx-auth.sh`'s own URL-building) must not stop the config from loading.
/// And the target's own bare-hostname contract still holds: if a scheme
/// reaches `SMELT_DBX_HOSTNAME` itself, `Config::load` still refuses it
/// (`docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` phase 6b —
/// the scheme/bare-host mismatch phase 6 worked around in the shell).
#[test]
fn databricks_target_host_is_bare() {
    // `explain --json` on a real model triggers the same whole-project
    // config load (and its `${VAR}` interpolation across every declared
    // target) as `list`/`run`, without `list`'s own unrelated discovery
    // issue over this project's root-level `sample.sql`/`setup_sources.sql`
    // scratch files.
    let out = Command::new(smelt_bin())
        .args(["explain", "--json", "silver.events_deduped"])
        .args(["--project-dir", example_dir().to_str().unwrap()])
        .env_remove("RUST_LOG")
        .env("SMELT_DBX_HOST", "https://dbc-test.cloud.databricks.com")
        .env("SMELT_DBX_HOSTNAME", "dbc-test.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt explain`: {e}"));
    assert!(
        out.status.success(),
        "config load must succeed reading the bare SMELT_DBX_HOSTNAME even though the \
         scheme-bearing SMELT_DBX_HOST is also set:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new(smelt_bin())
        .args(["explain", "--json", "silver.events_deduped"])
        .args(["--project-dir", example_dir().to_str().unwrap()])
        .env_remove("RUST_LOG")
        .env("SMELT_DBX_HOST", "https://dbc-test.cloud.databricks.com")
        .env(
            "SMELT_DBX_HOSTNAME",
            "https://dbc-test.cloud.databricks.com",
        )
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt explain`: {e}"));
    assert!(
        !out.status.success(),
        "a scheme-bearing SMELT_DBX_HOSTNAME reaching `host:` must still be refused"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bare hostname"),
        "expected the bare-hostname refusal message in stderr:\n{stderr}"
    );
}

/// `silver.actor_sessions` writes its own `CAST(... AS VARCHAR)` casts
/// (session id concatenation, plus `sessionize`'s `platform_col => CAST(NULL
/// AS VARCHAR)` argument). A bare `VARCHAR` cast target is rejected by
/// Unity Catalog with `[DATATYPE_MISSING_SIZE]`
/// (`docs/outcomes/20260912-databricks-dogfood-spine/phases/06d-plan.md`).
/// Compiling for the `databricks` target with `--dry-run` needs no live
/// workspace — it only exercises the printer's per-dialect cast-target
/// spelling (`docs/specs/multi_backend.md` §"Output-schema type
/// conformance").
#[test]
fn actor_sessions_compiles_without_bare_varchar() {
    let out = Command::new(smelt_bin())
        .args(["run", "--target", DATABRICKS_TARGET, "--dry-run"])
        .args(["--select", "silver.actor_sessions"])
        .args(["--project-dir", example_dir().to_str().unwrap()])
        .env_remove("RUST_LOG")
        .env("SMELT_DBX_HOSTNAME", "dbc-test.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run --dry-run`: {e}"));
    assert!(
        out.status.success(),
        "smelt run --target databricks --dry-run failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(" AS VARCHAR)"),
        "a bare VARCHAR cast target must be spelled STRING on the databricks target: {stdout}"
    );
}

/// `silver.actor_sessions` calls `epoch_us`, a DuckDB-only builtin with no
/// `BuiltinRegistry` entry until phase 6e — it reached Databricks printed
/// verbatim and failed `[UNRESOLVED_ROUTINE]`. Compiling with `--dry-run`
/// needs no live workspace; it only exercises the registry-driven Spark
/// emission spelling (`unix_micros`).
#[test]
fn actor_sessions_compiles_without_epoch_us() {
    let out = Command::new(smelt_bin())
        .args(["run", "--target", DATABRICKS_TARGET, "--dry-run"])
        .args(["--select", "silver.actor_sessions"])
        .args(["--project-dir", example_dir().to_str().unwrap()])
        .env_remove("RUST_LOG")
        .env("SMELT_DBX_HOSTNAME", "dbc-test.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run --dry-run`: {e}"));
    assert!(
        out.status.success(),
        "smelt run --target databricks --dry-run failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("epoch_us("),
        "a bare `epoch_us(` must not survive on the databricks target — it must lower to \
         `unix_micros(`: {stdout}"
    );
}
