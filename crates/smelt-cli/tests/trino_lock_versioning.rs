//! Live-gated: the `.smelt/` run lock and the `meta.json` layout-version gate
//! are backend-independent (`docs/specs/run_state.md` §"Locking") — realised
//! identically on Trino as on every other target, even though Trino claims no
//! engine-resident correctness structure. This proves it against the live
//! tier: a held lock refuses a second `smelt run --target trino` by naming
//! the holding PID and writes neither `.smelt/` run artifacts nor an Iceberg
//! table; releasing the lock lets the next run proceed; and a future
//! `state_version` in `meta.json` refuses before any write.
//!
//! Skips green when `SMELT_TRINO_URL` is unset. Run it with:
//!   bash scripts/trino-up.sh && source scripts/trino-env.sh
//!   cargo test -p smelt-cli --test trino_lock_versioning

mod common;
use common::{drop_trino_schema, trino_env, trino_schema, trino_target_block};

use smelt_state::file_store::FileStore;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

/// Guards every read/mutation of `SMELT_TRINO_URL` in this binary, mirroring
/// `trino_smoke.rs`'s `TRINO_ENV_GUARD` — this is a separate test binary so
/// it needs its own static, not a shared one.
static TRINO_ENV_GUARD: Mutex<()> = Mutex::new(());

/// Resolves `schema`'s target block under the guard, or `None` (skip) when
/// `SMELT_TRINO_URL` is unset. Widened (this phase) to cover the
/// `trino_target_block` read too, not just the `trino_env().is_some()`
/// check — `trino_lock_legs_skip_not_pass_when_url_unset` removes and
/// restores the var under this same guard, and an unguarded
/// `trino_target_block` read elsewhere could observe the var mid-mutation.
fn resolve_trino_target_block(schema: &str) -> Option<String> {
    let _guard = TRINO_ENV_GUARD.lock().unwrap();
    trino_env()?;
    Some(trino_target_block(schema))
}

fn stage_lock_project(tmp: &TempDir, target_block: &str) -> std::path::PathBuf {
    let root = tmp.path().join("trino_lock_proj");
    fs::create_dir_all(root.join("models")).unwrap();

    let yml = format!(
        "name: trino_lock_versioning\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\nstate:\n  mode: intervals\n",
        target_block
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/locked_table.sql"),
        "---\nmaterialization: table\n---\nSELECT CAST(1 AS BIGINT) AS id, 'alpha' AS label\n",
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

/// `true` only if a live table exists at `schema.table` — a query error
/// (missing schema, missing table, or any other backend error) is read as
/// "does not exist", since this helper exists only to assert a run wrote
/// nothing.
fn trino_table_exists(schema: &str, table: &str) -> bool {
    use smelt_backend::Backend;
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime for trino_table_exists");
    let backend = common::trino_backend(schema);
    rt.block_on(async {
        backend
            .execute_sql(&format!("SELECT 1 FROM {schema}.{table} LIMIT 1"))
            .await
            .is_ok()
    })
}

fn run_artifacts_empty(project_dir: &Path) -> bool {
    let runs_dir = project_dir.join(".smelt/targets/trino/runs");
    let reports_dir = project_dir.join(".smelt/targets/trino/reports");
    let empty_or_missing = |dir: &Path| {
        !dir.exists()
            || fs::read_dir(dir)
                .map(|mut it| it.next().is_none())
                .unwrap_or(true)
    };
    empty_or_missing(&runs_dir) && empty_or_missing(&reports_dir)
}

#[test]
fn held_lock_refuses_a_second_trino_run_by_pid() {
    let schema = trino_schema("lock_pid");
    let Some(target_block) = resolve_trino_target_block(&schema) else {
        eprintln!("SMELT_TRINO_URL unset — skipping held_lock_refuses_a_second_trino_run_by_pid");
        return;
    };
    let tmp = TempDir::new().unwrap();
    let root = stage_lock_project(&tmp, &target_block);

    // Hold the project-wide `.smelt/lock` from this process for the duration
    // of the subprocess run below.
    let file_store = FileStore::new(&root, "trino");
    let guard = file_store
        .lock()
        .expect("this process's own lock acquisition must succeed — nothing else holds it yet");

    let out = run_smelt(&root);
    assert!(
        !out.status.success(),
        "`smelt run --target trino` must fail while the lock is held"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_pid = std::process::id().to_string();
    assert!(
        stderr.contains("state locked by PID") && stderr.contains(&expected_pid),
        "expected the refusal to name this process's PID ({expected_pid}); stderr: {stderr}"
    );

    assert!(
        run_artifacts_empty(&root),
        "a refused run must leave .smelt/targets/trino/runs and reports/ empty"
    );
    assert!(
        !trino_table_exists(&schema, "locked_table"),
        "a refused run must not create the model's Iceberg table"
    );

    drop(guard);
    drop_trino_schema(&schema);
}

#[test]
fn releasing_the_lock_lets_the_next_trino_run_proceed() {
    let schema = trino_schema("lock_release");
    let Some(target_block) = resolve_trino_target_block(&schema) else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping releasing_the_lock_lets_the_next_trino_run_proceed"
        );
        return;
    };
    let tmp = TempDir::new().unwrap();
    let root = stage_lock_project(&tmp, &target_block);

    let file_store = FileStore::new(&root, "trino");
    let guard = file_store.lock().unwrap();
    let refused = run_smelt(&root);
    assert!(
        !refused.status.success(),
        "the first run must be refused while the lock is held"
    );
    drop(guard);

    let out = run_smelt(&root);
    assert!(
        out.status.success(),
        "`smelt run --target trino` must succeed once the lock is released.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        trino_table_exists(&schema, "locked_table"),
        "the model's Iceberg table must exist after the successful run"
    );

    drop_trino_schema(&schema);
}

#[test]
fn future_state_version_refuses_a_trino_run_before_any_write() {
    let schema = trino_schema("future_ver");
    let Some(target_block) = resolve_trino_target_block(&schema) else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping future_state_version_refuses_a_trino_run_before_any_write"
        );
        return;
    };
    let tmp = TempDir::new().unwrap();
    let root = stage_lock_project(&tmp, &target_block);

    fs::create_dir_all(root.join(".smelt")).unwrap();
    fs::write(root.join(".smelt/meta.json"), r#"{"state_version": 99}"#).unwrap();

    let out = run_smelt(&root);
    assert!(
        !out.status.success(),
        "`smelt run --target trino` must refuse a future state_version"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains('9') && stderr.contains("99"),
        "expected the refusal to name the future version 99; stderr: {stderr}"
    );
    assert!(
        !trino_table_exists(&schema, "locked_table"),
        "a version-refused run must not create the model's Iceberg table"
    );
    assert!(
        run_artifacts_empty(&root),
        "a version-refused run must leave .smelt/targets/trino/runs and reports/ empty"
    );

    drop_trino_schema(&schema);
}

/// The vacuous-pass guard, mirroring `trino_smoke.rs`'s
/// `trino_legs_skip_not_pass_when_url_unset`: with `SMELT_TRINO_URL` unset,
/// `trino_env()` is `None` and no leg in this file runs anywhere — this pins
/// that fact so a future refactor of the skip gate can't quietly turn a skip
/// into a silent pass.
#[test]
fn trino_lock_legs_skip_not_pass_when_url_unset() {
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
