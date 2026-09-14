//! `docs/outcomes/20260913-trino-ledger/outcome.md` phase 9: `.smelt/` is
//! not correctness-bearing on Trino (`docs/specs/state.md` §"The residency
//! rule") — falsified against a live tier rather than argued from the
//! inside. Deleting `.smelt/` between two runs changes no maintained
//! table's value, and `state.mode: stateless` writes nothing under
//! `.smelt/` while producing the same table values as `state.mode:
//! intervals` for the same project.
//!
//! Covers `materialization: table` models only — Trino has no
//! `MaintenanceDialect` variant yet (phase 6's summary), so no `refresh:
//! incremental` model can complete a live run on Trino today. The
//! incremental shapes are covered offline by
//! `trino_posture_plan_invariance.rs` instead.
//!
//! Skips green when `SMELT_TRINO_URL` is unset. Run it with:
//!   bash scripts/trino-up.sh && source scripts/trino-env.sh
//!   cargo test -p smelt-cli --test trino_state_residency

mod common;
use common::{drop_trino_schema, fetch_trino_rows, trino_env, trino_schema, trino_target_block};

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

/// Guards every read/mutation of `SMELT_TRINO_URL` in this binary, mirroring
/// `trino_lock_versioning.rs`'s own `TRINO_ENV_GUARD` — this is a separate
/// test binary so it needs its own static, not a shared one.
static TRINO_ENV_GUARD: Mutex<()> = Mutex::new(());

fn has_trino_env() -> bool {
    let _guard = TRINO_ENV_GUARD.lock().unwrap();
    trino_env().is_some()
}

/// A base `table` model plus a downstream one reading it as a model edge
/// (`FROM smelt.base_table`), so the DAG has more than one node — matching
/// `dag_kchain_b`'s edge shape in `trino_explain_downgrade.rs`.
fn stage_residency_project(tmp: &TempDir, schema: &str, state_mode: &str) -> std::path::PathBuf {
    let root = tmp.path().join("trino_residency_proj");
    fs::create_dir_all(root.join("models")).unwrap();

    let yml = format!(
        "name: trino_state_residency\nversion: 1\npaths:\n  - models\ntargets:\n{}\
         default_materialization: table\nstate:\n  mode: {state_mode}\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/base_table.sql"),
        "---\nmaterialization: table\n---\n\
         SELECT CAST(1 AS BIGINT) AS id, 'alpha' AS label\n",
    )
    .unwrap();

    fs::write(
        root.join("models/downstream_table.sql"),
        "---\nmaterialization: table\n---\n\
         SELECT id, label FROM smelt.base_table\n",
    )
    .unwrap();

    root
}

fn run_smelt(project_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            "trino",
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

/// `docs/outcomes/20260913-trino-ledger/phases/09-plan.md` test 1: deleting
/// `.smelt/` between two runs changes no maintained table's value.
#[test]
fn deleting_smelt_between_runs_changes_no_trino_table() {
    if !has_trino_env() {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping deleting_smelt_between_runs_changes_no_trino_table"
        );
        return;
    }
    let schema = trino_schema("residency_delete");
    let tmp = TempDir::new().unwrap();
    let root = stage_residency_project(&tmp, &schema, "intervals");

    let first = run_smelt(&root);
    assert!(
        first.status.success(),
        "first `smelt run --target trino` must succeed: stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );

    let base_before = fetch_trino_rows(&schema, "base_table");
    let downstream_before = fetch_trino_rows(&schema, "downstream_table");
    assert!(!base_before.is_empty(), "base_table must be non-empty");
    assert!(
        !downstream_before.is_empty(),
        "downstream_table must be non-empty"
    );

    assert!(
        root.join(".smelt").exists(),
        ".smelt/ must exist after an intervals-posture run"
    );
    fs::remove_dir_all(root.join(".smelt")).unwrap();

    let second = run_smelt(&root);
    assert!(
        second.status.success(),
        "second `smelt run --target trino` (after deleting .smelt/) must succeed: stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );

    let base_after = fetch_trino_rows(&schema, "base_table");
    let downstream_after = fetch_trino_rows(&schema, "downstream_table");
    assert_eq!(
        base_before, base_after,
        "deleting .smelt/ between runs must not change base_table's value"
    );
    assert_eq!(
        downstream_before, downstream_after,
        "deleting .smelt/ between runs must not change downstream_table's value"
    );

    drop_trino_schema(&schema);
}

/// Guards test 1 against passing because the second run silently no-opped:
/// `.smelt/` is recreated after the delete, and the second run's exit
/// status is success.
#[test]
fn the_second_run_really_reran_after_the_delete() {
    if !has_trino_env() {
        eprintln!("SMELT_TRINO_URL unset — skipping the_second_run_really_reran_after_the_delete");
        return;
    }
    let schema = trino_schema("residency_rerun");
    let tmp = TempDir::new().unwrap();
    let root = stage_residency_project(&tmp, &schema, "intervals");

    let first = run_smelt(&root);
    assert!(first.status.success(), "first run must succeed");
    fs::remove_dir_all(root.join(".smelt")).unwrap();

    let second = run_smelt(&root);
    assert!(
        second.status.success(),
        "second `smelt run --target trino` must succeed after the delete: stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        root.join(".smelt").exists(),
        ".smelt/ must be recreated by the second run"
    );

    drop_trino_schema(&schema);
}

/// `docs/outcomes/20260913-trino-ledger/phases/09-plan.md` test 3:
/// `state.mode: stateless` writes nothing under `.smelt/` on Trino.
#[test]
fn stateless_mode_writes_nothing_under_smelt_on_trino() {
    if !has_trino_env() {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping stateless_mode_writes_nothing_under_smelt_on_trino"
        );
        return;
    }
    let schema = trino_schema("residency_stateless_nowrite");
    let tmp = TempDir::new().unwrap();
    let root = stage_residency_project(&tmp, &schema, "stateless");

    let out = run_smelt(&root);
    assert!(
        out.status.success(),
        "`smelt run --target trino` under state.mode: stateless must succeed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !root.join(".smelt").exists(),
        "state.mode: stateless must leave no .smelt/ directory on Trino"
    );

    drop_trino_schema(&schema);
}

/// `docs/outcomes/20260913-trino-ledger/phases/09-plan.md` test 4: the rows
/// a `state.mode: stateless` run produces equal the rows a `state.mode:
/// intervals` run produces for the same project.
#[test]
fn stateless_mode_changes_no_trino_table_value() {
    if !has_trino_env() {
        eprintln!("SMELT_TRINO_URL unset — skipping stateless_mode_changes_no_trino_table_value");
        return;
    }
    let intervals_schema = trino_schema("residency_intervals_cmp");
    let stateless_schema = trino_schema("residency_stateless_cmp");
    let tmp = TempDir::new().unwrap();

    let intervals_root = stage_residency_project(&tmp, &intervals_schema, "intervals");
    let intervals_out = run_smelt(&intervals_root);
    assert!(
        intervals_out.status.success(),
        "intervals-posture run must succeed: stderr={}",
        String::from_utf8_lossy(&intervals_out.stderr)
    );

    let stateless_root = stage_residency_project(&tmp, &stateless_schema, "stateless");
    let stateless_out = run_smelt(&stateless_root);
    assert!(
        stateless_out.status.success(),
        "stateless-posture run must succeed: stderr={}",
        String::from_utf8_lossy(&stateless_out.stderr)
    );

    let intervals_base = fetch_trino_rows(&intervals_schema, "base_table");
    let stateless_base = fetch_trino_rows(&stateless_schema, "base_table");
    let intervals_downstream = fetch_trino_rows(&intervals_schema, "downstream_table");
    let stateless_downstream = fetch_trino_rows(&stateless_schema, "downstream_table");

    assert!(!intervals_base.is_empty(), "base_table must be non-empty");
    assert_eq!(
        intervals_base, stateless_base,
        "base_table's value must be identical under both postures"
    );
    assert_eq!(
        intervals_downstream, stateless_downstream,
        "downstream_table's value must be identical under both postures"
    );

    drop_trino_schema(&intervals_schema);
    drop_trino_schema(&stateless_schema);
}

/// The vacuous-pass guard, mirroring `trino_lock_versioning.rs`'s own leg:
/// with `SMELT_TRINO_URL` unset, every test in this file must skip, never
/// silently pass.
#[test]
fn trino_residency_legs_skip_not_pass_when_url_unset() {
    let _guard = TRINO_ENV_GUARD.lock().unwrap();
    let saved = std::env::var("SMELT_TRINO_URL").ok();
    // SAFETY: `TRINO_ENV_GUARD` serializes every reader/mutator of this var
    // in this binary, so no other thread observes the var mid-mutation.
    unsafe {
        std::env::remove_var("SMELT_TRINO_URL");
    }
    let result = trino_env();
    if let Some(url) = saved {
        // SAFETY: see above.
        unsafe {
            std::env::set_var("SMELT_TRINO_URL", url);
        }
    }
    assert!(
        result.is_none(),
        "trino_env() must be None (skip), never Some with fabricated defaults, \
         when SMELT_TRINO_URL is unset"
    );
}
