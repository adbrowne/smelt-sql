#![cfg(feature = "duckdb")]
//! End-to-end execution test for the in-model meta-language build-path
//! evaluator's **wide reflection** (`smelt.models.*` / `smelt.sources.*`,
//! BUG-006 wide reflection): a model that consumes `smelt.models.with_tag(t)`
//! or `smelt.sources.with_tag(t)` via `map` and a spread must have the
//! reflection materialised at compile time and lowered to plain select items
//! (string literals of the reflected names), so it executes on DuckDB rather
//! than the `smelt.models.*` / `smelt.sources.*` accessor reaching the engine
//! verbatim.
//!
//! Stages a hermetic project under a `TempDir`, shells out to the compiled
//! `smelt` binary, and opens the produced DuckDB file to assert on the
//! materialised values — same pattern as `meta_columns_e2e.rs`.
//!
//! The consumer models fold the reflected names with `reduce(map(...),
//! concat_with(', '))` into a single aliased scalar. The bare-spread form
//! (`SELECT ...map(smelt.models.with_tag(t), fn m => m.name)`) lowers correctly
//! — its compile-time lowering is covered by the `meta_eval` unit tests — but
//! produces unaliased string-literal columns that trip a pre-existing,
//! orthogonal `apply_type_casts` defect (a non-identifier select item wrapped as
//! `_colN` the inner query does not expose), exactly the disposition recorded
//! for the literal list-spread case in the diagnostic-parity plan (P5). The
//! fold form exercises the same wide-reflection evaluation end-to-end on DuckDB
//! without depending on that orthogonal defect.
//!
//! The `examples/meta_workspace/` workspace itself stays on the
//! `example_builds` `KNOWN_UNBUILDABLE` allow-list because its cohort models
//! read the unseeded `raw.events` source (they fail before reaching the
//! reflection-using models); this hermetic test is the BUG-006 wide-reflection
//! regression target.

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
        "name: e2e_meta_workspace
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

/// A model that reflects over the workspace's models tagged `cohort`
/// (`smelt.models.with_tag`), projects each `ModelRef` to its `name`, and folds
/// the names into a single `Text` column; and a model that does the same over
/// sources tagged `audit` (`smelt.sources.with_tag`). The reflection must be
/// materialised at build so each fold collapses to a concrete string.
#[test]
fn e2e_wide_reflection_models_and_sources_execute_against_duckdb() {
    let tmp = TempDir::new().expect("tempdir");
    let proj = tmp.path();
    let db_path = proj.join("dev.duckdb");

    write_workspace(
        proj,
        &[
            ("smelt.yml", smelt_yml(&db_path).as_str()),
            // Two cohort-tagged models (path-sorted: cohort_a, cohort_b) and one
            // untagged model that must NOT appear in the cohort reflection.
            (
                "models/cohort_a.sql",
                "---\ntags: [cohort]\n---\nSELECT CAST(1 AS INTEGER) AS id\n",
            ),
            (
                "models/cohort_b.sql",
                "---\ntags: [cohort]\n---\nSELECT CAST(2 AS INTEGER) AS id\n",
            ),
            (
                "models/zzz_other.sql",
                "---\ntags: [misc]\n---\nSELECT CAST(3 AS INTEGER) AS id\n",
            ),
            // A source tagged `audit` (metadata only — no table is read, so no
            // seeding is required; the reflection projects its name).
            (
                "models/sources/raw/events.yml",
                "description: Raw events\ntags: [audit]\ncolumns:\n- name: id\n  type: INTEGER\n",
            ),
            // Consumer 1: fold the cohort-tagged model names → 'cohort_a, cohort_b'
            // (path-sorted; zzz_other tagged `misc` is excluded by with_tag).
            (
                "models/cohort_names.sql",
                "SELECT reduce(\n  map(smelt.models.with_tag('cohort'), fn m => m.name),\n  concat_with(', ')\n) AS names\n",
            ),
            // Consumer 2: fold the audit-tagged source names → 'events'.
            (
                "models/audit_source_names.sql",
                "SELECT reduce(\n  map(smelt.sources.with_tag('audit'), fn s => s.name),\n  concat_with(', ')\n) AS names\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    // cohort_names folds the two cohort model names, path-sorted; zzz_other
    // (tag misc) is excluded by with_tag.
    let cohort_names: String = conn
        .query_row("SELECT names FROM main.cohort_names", [], |r| r.get(0))
        .expect("query cohort_names");
    assert_eq!(cohort_names, "cohort_a, cohort_b");

    // audit_source_names folds the single audit source's name.
    let audit_names: String = conn
        .query_row("SELECT names FROM main.audit_source_names", [], |r| {
            r.get(0)
        })
        .expect("query audit_source_names");
    assert_eq!(audit_names, "events");
}
