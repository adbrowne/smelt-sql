#![cfg(feature = "duckdb")]
//! End-to-end execution test for the in-model meta-language build-path
//! evaluator's **config loader** (`smelt.config.load_yaml` / `load_json`,
//! BUG-006 loader): a model that consumes `smelt.config.load_yaml(path,
//! List<…>)` via `map` and a reducer must have the loader value materialised at
//! compile time (one record per loaded row, each field rendered as a Data-World
//! SQL literal) and the surrounding HOF chain lowered to plain SQL, so it
//! executes on DuckDB rather than the `smelt.config.load_yaml` call reaching the
//! engine verbatim (`Catalog "smelt" does not exist`).
//!
//! Stages a hermetic project under a `TempDir`, shells out to the compiled
//! `smelt` binary, and opens the produced DuckDB file to assert on the
//! materialised values — same pattern as `meta_workspace_e2e.rs`.
//!
//! The consumer model folds the loaded `region` fields with `reduce(map(...),
//! concat_with(', '))` into a single aliased scalar. The bare-spread form
//! (`SELECT ...load_yaml(...) |> map(fn c => c.region)`) lowers correctly — its
//! compile-time lowering is covered by the `meta_eval` unit tests — but produces
//! unaliased string-literal columns that trip a pre-existing, orthogonal
//! `apply_type_casts` defect (the same disposition recorded for the literal
//! list-spread case in the diagnostic-parity plan, P5). The fold form exercises
//! the same loader evaluation end-to-end on DuckDB without depending on that
//! orthogonal defect.
//!
//! The `examples/meta_config/` workspace itself stays on the `example_builds`
//! `KNOWN_UNBUILDABLE` allow-list because its `tenants` model demonstrates a
//! `Map<Text, …>`-schema loader, whose build-path lowering is not yet
//! implemented (a recorded Known Divergence in `meta_config_loading.md`); this
//! hermetic test is the BUG-006 loader regression target for the `List<…>` form.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_workspace(tmp: &Path, files: &[(&str, &str)]) {
    for (rel, contents) in files {
        let path = tmp.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write workspace file");
    }
}

fn run_smelt_build(project_dir: &Path, target: &str) {
    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    assert!(
        output.status.success(),
        "smelt build (target={target}) failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn smelt_yml(db_path: &Path) -> String {
    format!(
        "name: e2e_meta_config
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: {}
    schema: main
default_materialization: table
",
        db_path.display()
    )
}

/// A model that loads a `List<{name, region, threshold}>` from a YAML file,
/// projects each loaded record's `region`, and folds the regions into a single
/// `Text` column. The loader must be materialised at build so the fold collapses
/// to a concrete string of the loaded values.
#[test]
fn e2e_config_loader_list_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            (
                "configs/cohorts.yaml",
                "- name: us_west\n  region: us-west-2\n  threshold: 100\n\
                 - name: us_east\n  region: us-east-1\n  threshold: 200\n\
                 - name: eu\n  region: eu-west-1\n  threshold: 50\n",
            ),
            // Consumer: load the cohorts, map each to its region, fold to a
            // single scalar 'us-west-2, us-east-1, eu-west-1'.
            (
                "models/cohort_regions.sql",
                "SELECT reduce(\n  map(\n    smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>),\n    fn c => c.region\n  ),\n  concat_with(', ')\n) AS regions\n",
            ),
            // Consumer: fold the numeric thresholds with plus_chain → 350.
            (
                "models/threshold_total.sql",
                "SELECT reduce(\n  map(\n    smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>),\n    fn c => c.threshold\n  ),\n  plus_chain\n) AS total\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    let regions: String = conn
        .query_row("SELECT regions FROM main.cohort_regions", [], |r| r.get(0))
        .expect("query cohort_regions");
    assert_eq!(regions, "us-west-2, us-east-1, eu-west-1");

    let total: i64 = conn
        .query_row("SELECT total FROM main.threshold_total", [], |r| r.get(0))
        .expect("query threshold_total");
    assert_eq!(total, 350);
}

/// P7d: A model that loads a `Map<Text, {plan, threshold}>` from a YAML file
/// and consumes it via `.keys()` → `map` → `reduce`, materialising the sorted
/// keys as a comma-separated string.  Validates that:
///   1. The parser emits `MAP_METHOD_CALL` for the `.keys()` postfix on a
///      `SMELT_PATH_CALL` receiver.
///   2. The build-path evaluator lowers `Map<…>` loaders to `MetaValue::Map`
///      and `eval_map_method_call` converts `.keys()` to a `MetaValue::List`.
///   3. The HOF pipeline collapses to a concrete SQL scalar and DuckDB executes
///      it without seeing the `smelt.config.load_yaml` call verbatim.
#[test]
fn e2e_config_loader_map_keys_executes_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            (
                // Map-schema YAML: top-level keys are tenant IDs; values are records.
                "configs/tenants.yaml",
                "tenant_a:\n  plan: pro\n  threshold: 100\ntenant_b:\n  plan: free\n  threshold: 10\n",
            ),
            (
                // Consumer: load the tenant map, extract its keys, fold them.
                "models/tenant_keys.sql",
                "SELECT reduce(\n  map(\n    smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>).keys(),\n    fn k => k\n  ),\n  concat_with(', ')\n) AS tenant_keys\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    let keys: String = conn
        .query_row("SELECT tenant_keys FROM main.tenant_keys", [], |r| r.get(0))
        .expect("query tenant_keys");
    // BTreeMap returns keys in lexicographic order: tenant_a < tenant_b
    assert_eq!(keys, "tenant_a, tenant_b");
}
