#![cfg(feature = "duckdb")]
//! Tests for `scripts/dbx-dogfood-loader.py`/`.sh`, the pinned
//! `databricks-connect` client venv, and `scripts/dbx-dogfood-env.sh`
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/03-plan.md`).
//!
//! The loader is external to smelt — smelt's source declaration is the
//! contract it is trusted against, exactly as `scripts/bq-dogfood-loader.sh`
//! is for BigQuery (`crates/smelt-cli/tests/github_activity_loader.rs`). Its
//! `--emit-ddl`/`--emit-sql`/`--emit-slice-sql` modes touch no network and
//! need no `databricks-connect` package or workspace, so this file proves
//! the loader is byte-for-byte faithful to
//! `examples/github_activity/load_day.sh`'s own redelivery rule with no
//! cloud in the loop.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

mod github_activity_support;
use github_activity_support::{load_day, stage_workspace, FIXTURE_DAYS};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn loader_py() -> PathBuf {
    repo_root().join("scripts/dbx-dogfood-loader.py")
}

fn load_day_sh() -> PathBuf {
    repo_root().join("examples/github_activity/load_day.sh")
}

fn run_loader(args: &[&str]) -> Output {
    Command::new("python3")
        .arg(loader_py())
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn dbx-dogfood-loader.py: {e}"))
}

fn duckdb_scalar(sql: &str) -> i64 {
    let out = Command::new("duckdb")
        .args([":memory:", "-json", "-c", sql])
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn duckdb: {e}"));
    assert!(
        out.status.success(),
        "duckdb query failed: {}\nsql: {sql}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    rows[0]
        .as_object()
        .and_then(|obj| obj.values().next())
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("expected one scalar column in query result: {rows}"))
}

fn duckdb_scalar_against(db: &Path, sql: &str) -> i64 {
    let out = Command::new("duckdb")
        .arg(db)
        .args(["-json", "-c", sql])
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn duckdb: {e}"));
    assert!(
        out.status.success(),
        "duckdb query failed: {}\nsql: {sql}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    rows[0]
        .as_object()
        .and_then(|obj| obj.values().next())
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("expected one scalar column in query result: {rows}"))
}

/// Splits `--emit-slice-sql`'s output into (events_query, arrival_query),
/// delimited by the `-- events` / `-- arrival` markers the loader prints.
fn split_slice_sql(emitted: &str) -> (String, String) {
    let events_marker = "-- events";
    let arrival_marker = "-- arrival";
    let events_start = emitted
        .find(events_marker)
        .unwrap_or_else(|| panic!("missing '{events_marker}' marker in:\n{emitted}"));
    let arrival_start = emitted
        .find(arrival_marker)
        .unwrap_or_else(|| panic!("missing '{arrival_marker}' marker in:\n{emitted}"));
    assert!(events_start < arrival_start);
    let events = emitted[events_start + events_marker.len()..arrival_start]
        .trim()
        .trim_end_matches(';')
        .to_string();
    let arrival = emitted[arrival_start + arrival_marker.len()..]
        .trim()
        .trim_end_matches(';')
        .to_string();
    (events, arrival)
}

fn parsed_modulus() -> u32 {
    let text = std::fs::read_to_string(load_day_sh()).expect("read load_day.sh");
    let line = text
        .lines()
        .find(|l| l.contains("MOD(CAST(id AS BIGINT),"))
        .expect("load_day.sh names its redelivery modulus");
    let after = line
        .split("MOD(CAST(id AS BIGINT),")
        .nth(1)
        .expect("modulus follows MOD(CAST(id AS BIGINT),");
    after
        .trim_start()
        .split(')')
        .next()
        .expect("modulus is followed by a closing paren")
        .trim()
        .parse()
        .expect("modulus is an integer")
}

#[test]
fn slice_matches_load_day_rows_for_every_fixture_day() {
    for day in FIXTURE_DAYS {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, db, sample) = stage_workspace(tmp.path());
        load_day(&db, &sample, day, None);

        let out = run_loader(&["--emit-slice-sql", "--date", day]);
        assert!(
            out.status.success(),
            "day {day} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let emitted = String::from_utf8_lossy(&out.stdout).to_string();
        let (events_sql, _) = split_slice_sql(&emitted);

        let left = duckdb_scalar_against(
            &db,
            &format!(
                "SELECT count(*) AS c FROM (SELECT * FROM main.sources_raw_github_events \
                 EXCEPT ALL ({events_sql})) t"
            ),
        );
        let right = duckdb_scalar_against(
            &db,
            &format!(
                "SELECT count(*) AS c FROM (({events_sql}) \
                 EXCEPT ALL SELECT * FROM main.sources_raw_github_events) t"
            ),
        );
        assert_eq!(
            left, 0,
            "day {day}: loaded rows not covered by the emitted slice"
        );
        assert_eq!(
            right, 0,
            "day {day}: emitted slice rows not present in load_day.sh's output"
        );

        let loaded_count = duckdb_scalar_against(
            &db,
            "SELECT count(*) AS c FROM main.sources_raw_github_events",
        );
        let slice_count =
            duckdb_scalar_against(&db, &format!("SELECT count(*) AS c FROM ({events_sql}) t"));
        assert_eq!(loaded_count, slice_count, "day {day}: row counts diverge");
    }
}

#[test]
fn arrival_slice_stamps_ingested_date_like_load_day() {
    let day = "2026-08-06";
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, db, sample) = stage_workspace(tmp.path());
    load_day(&db, &sample, day, None);

    let out = run_loader(&["--emit-slice-sql", "--date", day]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();
    let (_, arrival_sql) = split_slice_sql(&emitted);

    let left = duckdb_scalar_against(
        &db,
        &format!(
            "SELECT count(*) AS c FROM (SELECT * FROM main.sources_raw_github_events_arrival \
             EXCEPT ALL ({arrival_sql})) t"
        ),
    );
    let right = duckdb_scalar_against(
        &db,
        &format!(
            "SELECT count(*) AS c FROM (({arrival_sql}) \
             EXCEPT ALL SELECT * FROM main.sources_raw_github_events_arrival) t"
        ),
    );
    assert_eq!(left, 0);
    assert_eq!(right, 0);

    let stamped = duckdb_scalar_against(
        &db,
        &format!("SELECT count(*) AS c FROM ({arrival_sql}) t WHERE ingested_date <> DATE '{day}'"),
    );
    assert_eq!(
        stamped, 0,
        "every row in the arrival slice must be stamped ingested_date = {day}"
    );
}

#[test]
fn first_fixture_day_has_an_empty_redelivery_arm() {
    let day = FIXTURE_DAYS[0];
    let out = run_loader(&["--emit-slice-sql", "--date", day]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();
    let (events_sql, _) = split_slice_sql(&emitted);

    let sample = repo_root().join("examples/github_activity/seeds/github_events_sample.parquet");
    let plain_day_count = duckdb_scalar(&format!(
        "SELECT count(*) AS c FROM read_parquet('{}') WHERE CAST(created_at AS DATE) = DATE '{day}'",
        sample.display()
    ));
    let slice_count = duckdb_scalar(&format!("SELECT count(*) AS c FROM ({events_sql}) t"));
    assert_eq!(
        plain_day_count, slice_count,
        "the earliest fixture day has no D-1 rows in the sample, so its redelivery arm must \
         match zero rows and the loader needs no special case"
    );
}

#[test]
fn redelivery_modulus_is_parsed_from_load_day_not_restated() {
    let expected = parsed_modulus();
    let out = run_loader(&["--emit-slice-sql", "--date", "2026-08-06"]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        emitted.contains(&format!("MOD(CAST(id AS BIGINT), {expected})")),
        "expected the loader to use load_day.sh's own modulus ({expected}) in:\n{emitted}"
    );
}

#[test]
fn loader_is_idempotent_per_day() {
    let tmp = TempDir::new().expect("tempdir");
    let store = tmp.path().join("store");
    std::fs::create_dir_all(&store).expect("mkdir store");

    let out = run_loader(&[
        "--date",
        "2026-08-06",
        "--dry-run-store",
        store.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let first_stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(
        first_stdout.contains("loaded"),
        "first run should report it loaded: {first_stdout}"
    );
    let ledger = store.join("loader_days.txt");
    assert!(
        ledger.exists(),
        "first run should record the day in the store's ledger"
    );
    let ledger_after_first = std::fs::read_to_string(&ledger).expect("read ledger");
    assert!(ledger_after_first.contains("2026-08-06"));

    let out = run_loader(&[
        "--date",
        "2026-08-06",
        "--dry-run-store",
        store.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "second invocation should also exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout).to_lowercase(),
        String::from_utf8_lossy(&out.stderr).to_lowercase()
    );
    assert!(
        combined.contains("already loaded"),
        "second run should report already loaded: {combined}"
    );
    let ledger_after_second = std::fs::read_to_string(&ledger).expect("read ledger");
    assert_eq!(
        ledger_after_first, ledger_after_second,
        "the second, already-loaded run must write nothing further to the ledger"
    );
}

#[test]
fn emitted_load_statements_reference_no_host_path() {
    for (args, _label) in [
        (vec!["--emit-sql", "--date", "2026-08-06"], "emit-sql"),
        (vec!["--emit-ddl"], "emit-ddl"),
    ] {
        let out = run_loader(&args);
        assert!(
            out.status.success(),
            "{args:?} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let emitted = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !emitted.contains("seeds/github_events_sample.parquet"),
            "{args:?} must not reference the fixture's host path:\n{emitted}"
        );
        assert!(
            !emitted.to_lowercase().contains(".parquet"),
            "{args:?} must not reference any parquet file path:\n{emitted}"
        );
        assert!(
            emitted.contains("smelt_dogfood.github_events"),
            "{args:?} should name the catalog-qualified target table:\n{emitted}"
        );
    }
}

#[test]
fn client_env_is_pinned_and_disjoint_from_the_spark_venv() {
    let pin_file = repo_root().join("scripts/dbx-dogfood-requirements.txt");
    let pins = std::fs::read_to_string(&pin_file).expect("read dbx-dogfood-requirements.txt");
    assert!(
        pins.lines().any(|l| {
            let l = l.trim();
            l.starts_with("databricks-connect==")
        }),
        "expected an exact databricks-connect== pin in:\n{pins}"
    );
    assert!(
        !pins.lines().any(|l| {
            let l = l.trim();
            l == "pyspark" || l.starts_with("pyspark==") || l.starts_with("pyspark>=")
        }),
        "the databricks-connect client must not co-install bare pyspark:\n{pins}"
    );

    let venv_script = repo_root().join("scripts/dbx-dogfood-venv.sh");
    let script = std::fs::read_to_string(&venv_script).expect("read dbx-dogfood-venv.sh");
    assert!(
        script.contains(".smelt-dbx-venv"),
        "expected the dbx venv script to target .smelt-dbx-venv"
    );
    assert!(
        !script.contains(".smelt-spark-venv"),
        "the dbx venv script must not target the local-Spark venv"
    );
}

#[test]
fn dbx_dogfood_env_exports_the_dbx_venv_and_no_secret() {
    let env_script = repo_root().join("scripts/dbx-dogfood-env.sh");
    assert!(env_script.exists());

    // A real credential config now lives at the default
    // $HOME/.config/databricks-smelt-dogfood (phase 4b's live provisioning), so
    // merely removing the three env vars falls back to reading it and the "no
    // credential config on disk" assertion below would see a real host. Point
    // SMELT_DBX_CONFIG_DIR at an empty tempdir to genuinely isolate this case.
    let tmp = TempDir::new().expect("tempdir");
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "set -e; source {}; echo \"PYTHONPATH=$PYTHONPATH\"; echo \"GATE=${{SMELT_DBX_HOST-UNSET}}\"",
            env_script.display()
        ))
        .current_dir(repo_root())
        .env_remove("SMELT_DBX_HOST")
        .env_remove("SMELT_DBX_TOKEN")
        .env("SMELT_DBX_CONFIG_DIR", tmp.path())
        .output()
        .unwrap_or_else(|e| panic!("failed to source dbx-dogfood-env.sh: {e}"));
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains(".smelt-dbx-venv"),
        "expected the dbx venv's site-packages on PYTHONPATH:\n{stdout}"
    );
    assert!(
        !stdout.contains(".smelt-spark-venv"),
        "must never put the local-Spark venv on PYTHONPATH:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("{}", repo_root().join("python").display())),
        "expected the repo python/ package on PYTHONPATH:\n{stdout}"
    );
    assert!(
        stdout.contains("GATE=UNSET"),
        "with no credential config on disk, the live gate variable must stay unset:\n{stdout}"
    );

    let combined = format!("{}{}", stdout, String::from_utf8_lossy(&out.stderr));
    assert!(
        !combined.to_lowercase().contains("dapi"),
        "must never print a token-shaped value:\n{combined}"
    );
}
