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
//! The consumer models use the bare-spread form (`SELECT ...map(smelt.models
//! .with_tag(t), fn m => m.name)`) directly: its compile-time lowering (to one
//! string-literal select item per reflected entry) is covered by the
//! `meta_eval` unit tests, and each unaliased literal item receives a real,
//! bound `_smelt_col{n}` alias (`docs/specs/multi_backend.md` §"Output-schema
//! type conformance") — so the spread executes end-to-end on DuckDB without
//! needing to fold the results into a single scalar first.
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
/// (`smelt.models.with_tag`), projects each `ModelRef` to its `name`, and
/// bare-spreads the names directly into the select list; and a model that
/// does the same over sources tagged `audit` (`smelt.sources.with_tag`). The
/// reflection must be materialised at build so each spread lowers to one
/// concrete string-literal column per matching entry.
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
            // Consumer 1: bare-spread the cohort-tagged model names
            // (path-sorted: cohort_a, cohort_b; zzz_other tagged `misc` is
            // excluded by with_tag) into two select items.
            (
                "models/cohort_names.sql",
                "SELECT ...map(smelt.models.with_tag('cohort'), fn m => m.name)\n",
            ),
            // Consumer 2: bare-spread the audit-tagged source names (one
            // match: 'events') into a single select item.
            (
                "models/audit_source_names.sql",
                "SELECT ...map(smelt.sources.with_tag('audit'), fn s => s.name)\n",
            ),
        ],
    );

    run_smelt_build(proj, "dev");
    let conn = duckdb::Connection::open(&db_path).expect("open dev.duckdb");

    // cohort_names spreads to two unaliased literal items; each receives a
    // real, bound `_smelt_col{n}` alias (position-derived, per
    // `docs/specs/multi_backend.md` §"Output-schema type conformance").
    let (cohort_a, cohort_b): (String, String) = conn
        .query_row(
            "SELECT _smelt_col1, _smelt_col2 FROM main.cohort_names",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("query cohort_names");
    assert_eq!(cohort_a, "cohort_a");
    assert_eq!(cohort_b, "cohort_b");

    // audit_source_names spreads to a single unaliased literal item.
    let audit_name: String = conn
        .query_row("SELECT _smelt_col1 FROM main.audit_source_names", [], |r| {
            r.get(0)
        })
        .expect("query audit_source_names");
    assert_eq!(audit_name, "events");
}
