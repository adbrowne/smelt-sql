//! Tests for `scripts/bq-dogfood-loader.sh`
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/09-plan.md`).
//!
//! The loader is a shell script that *derives* its BigQuery load SQL from
//! `examples/github_activity/sample.sql` rather than restating it. These
//! tests drive `--emit-sql`/`--emit-ddl`, which touch no network and need no
//! `bq`/`gcloud` on `PATH` — they only read two files in the repo and print
//! SQL — so "verbatim" is a gate, not a hope.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn loader_script() -> PathBuf {
    repo_root().join("scripts/bq-dogfood-loader.sh")
}

fn sample_sql_path() -> PathBuf {
    repo_root().join("examples/github_activity/sample.sql")
}

fn readme_path() -> PathBuf {
    repo_root().join("examples/github_activity/README.md")
}

fn run_loader(args: &[&str]) -> std::process::Output {
    Command::new("bash")
        .arg(loader_script())
        .args(args)
        .current_dir(repo_root())
        .env_remove("SMELT_BQ_ACCESS_TOKEN")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn bq-dogfood-loader.sh: {e}"))
}

/// `sample.sql`'s body: everything from the first bare `SELECT` line to EOF,
/// exactly as committed — the same slice the loader script itself extracts.
fn sample_body_lines() -> Vec<String> {
    let text = std::fs::read_to_string(sample_sql_path()).expect("read sample.sql");
    let mut lines = text.lines();
    for line in lines.by_ref() {
        if line == "SELECT" {
            break;
        }
    }
    let mut body = vec!["SELECT".to_string()];
    body.extend(lines.map(|l| l.to_string()));
    body
}

#[test]
fn loader_reproduces_the_sample_projection_and_filter_verbatim() {
    let out = run_loader(&["--emit-sql", "--date", "2026-08-06"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();

    let sample_lines = sample_body_lines();
    let suffix_line_idx = sample_lines
        .iter()
        .position(|l| l.starts_with("WHERE _TABLE_SUFFIX BETWEEN"))
        .expect("sample.sql has a _TABLE_SUFFIX BETWEEN line");

    // The derived base query must appear in the emitted output as one
    // contiguous run of lines, identical to sample.sql's own body line for
    // line, except the suffix line.
    let emitted_lines: Vec<&str> = emitted.lines().collect();
    let anchor = "SELECT\n  id,";
    assert!(
        emitted.contains(anchor),
        "emitted SQL does not contain the sample's SELECT list verbatim:\n{emitted}"
    );

    let start = emitted_lines
        .iter()
        .position(|l| *l == "SELECT")
        .expect("emitted SQL contains a bare SELECT line");
    let window = &emitted_lines[start..start + sample_lines.len()];

    for (i, (sample_line, emitted_line)) in sample_lines.iter().zip(window.iter()).enumerate() {
        if i == suffix_line_idx {
            assert_ne!(
                sample_line, emitted_line,
                "expected the suffix line to differ (it should be re-pinned to the requested date)"
            );
            assert!(
                emitted_line.starts_with("WHERE _TABLE_SUFFIX BETWEEN '0805' AND '0806'"),
                "unexpected suffix line: {emitted_line}"
            );
        } else {
            assert_eq!(
                sample_line, emitted_line,
                "line {i} of the derived base query diverges from sample.sql, but only the \
                 _TABLE_SUFFIX line should differ"
            );
        }
    }

    assert!(emitted.contains("FROM `githubarchive.day.2026*`"));
    assert!(emitted.contains("AND MOD(repo.id, 1000) = 0"));
    assert!(emitted.contains("INSERT INTO `raw.github_events`"));
    assert!(emitted.contains("INSERT INTO `raw.github_events_arrival`"));
}

#[test]
fn loader_suffix_range_is_bounded_to_the_requested_days() {
    let out = run_loader(&["--emit-sql", "--date", "2026-08-06"]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout);
    assert!(emitted.contains("_TABLE_SUFFIX BETWEEN '0805' AND '0806'"));
    assert!(
        !emitted.contains("day.*`") && !emitted.contains("2026*`\nWHERE\n"),
        "no unbounded day.* scan should appear"
    );
    // Every FROM clause carries the narrowed 2026* wildcard, never a bare day.*.
    for line in emitted.lines() {
        if line.contains("FROM `githubarchive") {
            assert!(line.contains("2026*`"), "unbounded wildcard: {line}");
        }
    }

    // A --date spanning a year boundary (Jan 1st) is refused, not silently wrong.
    let refused = run_loader(&["--emit-sql", "--date", "2026-01-01"]);
    assert!(
        !refused.status.success(),
        "a year-boundary date must be refused"
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.to_lowercase().contains("year"),
        "refusal message should mention the year-boundary problem: {stderr}"
    );
}

#[test]
fn loader_redelivery_matches_the_duckdb_replay_driver() {
    let run_incremental =
        std::fs::read_to_string(repo_root().join("examples/github_activity/run_incremental.py"))
            .expect("read run_incremental.py");
    let modulus_line = run_incremental
        .lines()
        .find(|l| l.trim_start().starts_with("REDELIVERY_MODULUS"))
        .expect("run_incremental.py declares REDELIVERY_MODULUS");
    let expected_modulus: u32 = modulus_line
        .split('=')
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("parse REDELIVERY_MODULUS value")
        .parse()
        .expect("REDELIVERY_MODULUS is an integer");

    let out = run_loader(&["--emit-sql", "--date", "2026-08-06"]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout);
    let expected_predicate = format!(
        "CAST(created_at AS DATE) = DATE '2026-08-05' AND MOD(CAST(id AS BIGINT), {expected_modulus}) = 0"
    );
    assert!(
        emitted.contains(&expected_predicate),
        "expected redelivery predicate `{expected_predicate}` not found in:\n{emitted}"
    );
}

#[test]
fn loader_stamps_ingested_date_on_the_arrival_table_only() {
    let out = run_loader(&["--emit-sql", "--date", "2026-08-06"]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout);

    let events_stmt_start = emitted
        .find("INSERT INTO `raw.github_events`")
        .expect("event-time INSERT present");
    let arrival_stmt_start = emitted
        .find("INSERT INTO `raw.github_events_arrival`")
        .expect("arrival INSERT present");
    assert!(events_stmt_start < arrival_stmt_start);

    let events_stmt = &emitted[events_stmt_start..arrival_stmt_start];
    let arrival_stmt = &emitted[arrival_stmt_start..];

    assert!(
        !events_stmt.contains("ingested_date"),
        "the event-time table must not receive an ingested_date column:\n{events_stmt}"
    );
    assert!(
        arrival_stmt.contains("DATE '2026-08-06' AS ingested_date"),
        "the arrival table must stamp ingested_date with the run day:\n{arrival_stmt}"
    );
}

#[test]
fn ddl_declares_day_partitioning_and_the_documented_retention_bound() {
    let readme = std::fs::read_to_string(readme_path()).expect("read README.md");
    let documented: u32 = readme
        .lines()
        .find_map(|l| {
            l.split_once("partition_expiration_days = ")
                .and_then(|(_, rest)| {
                    rest.split_whitespace()
                        .next()
                        .map(|tok| tok.trim_matches(|c: char| !c.is_ascii_digit()))
                        .and_then(|digits| digits.parse().ok())
                })
        })
        .expect("README.md documents partition_expiration_days = N");

    let out = run_loader(&["--emit-ddl"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ddl = String::from_utf8_lossy(&out.stdout);

    assert!(ddl.contains("`raw.github_events`"));
    assert!(ddl.contains("`raw.github_events_arrival`"));
    assert!(ddl.contains("PARTITION BY DATE(created_at)"));
    assert!(ddl.contains("PARTITION BY ingested_date"));
    assert_eq!(
        ddl.matches(&format!("partition_expiration_days = {documented}"))
            .count(),
        2,
        "both tables should declare the README-documented retention bound:\n{ddl}"
    );
    // No dataset/table-level `expiration_time`/default table expiry is set here —
    // criterion 1's "no default table expiration" stays a dataset-provisioning concern.
    assert!(!ddl.to_lowercase().contains("default_table_expiration"));
    assert!(!ddl.contains("expiration_timestamp"));
}

/// The `retention:` a source YAML declares must equal the loader's real
/// `partition_expiration_days` (`docs/specs/incremental_models.md`
/// §"The equivalence invariant", trimmed-history paragraph): an inert value
/// that disagrees with what the loader actually keeps is exactly the silent
/// under-read this outcome exists to prevent.
#[test]
fn source_yaml_retention_matches_the_loader_expiration() {
    let readme = std::fs::read_to_string(readme_path()).expect("read README.md");
    let documented: u32 = readme
        .lines()
        .find_map(|l| {
            l.split_once("partition_expiration_days = ")
                .and_then(|(_, rest)| {
                    rest.split_whitespace()
                        .next()
                        .map(|tok| tok.trim_matches(|c: char| !c.is_ascii_digit()))
                        .and_then(|digits| digits.parse().ok())
                })
        })
        .expect("README.md documents partition_expiration_days = N");

    for source_file in [
        "examples/github_activity/models/sources/raw/github_events.yml",
        "examples/github_activity/models/sources/raw/github_events_arrival.yml",
    ] {
        let path = repo_root().join(source_file);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {source_file}: {e}"));
        let retention_line = text
            .lines()
            .find(|l| l.starts_with("retention:"))
            .unwrap_or_else(|| panic!("{source_file} declares retention:"));
        let expected = format!("retention: '{documented} days'");
        assert_eq!(
            retention_line, expected,
            "{source_file} must declare the loader's real retention bound"
        );
    }
}

#[test]
fn emit_sql_touches_no_cloud() {
    let stripped_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|p| {
            let lower = p.to_lowercase();
            !lower.contains("gcloud") && !lower.contains("google-cloud-sdk")
        })
        .collect::<Vec<_>>()
        .join(":");

    for (args, marker) in [
        (vec!["--emit-sql", "--date", "2026-08-06"], "SELECT"),
        (vec!["--emit-ddl"], "CREATE TABLE"),
    ] {
        let out = Command::new("bash")
            .arg(loader_script())
            .args(&args)
            .current_dir(repo_root())
            .env_remove("SMELT_BQ_ACCESS_TOKEN")
            .env("PATH", &stripped_path)
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn bq-dogfood-loader.sh: {e}"));
        assert!(
            out.status.success(),
            "args {args:?} should succeed with no bq/gcloud on PATH and no token; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(marker),
            "expected SQL output for {args:?}"
        );
    }
}

#[test]
fn loader_script_is_shellcheck_clean() {
    if Command::new("shellcheck")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("shellcheck not on PATH — skipping");
        return;
    }
    let out = Command::new("shellcheck")
        .arg(loader_script())
        .output()
        .expect("run shellcheck");
    assert!(
        out.status.success(),
        "shellcheck findings:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// ---------------------------------------------------------------------------
// Tests for `examples/github_activity/load_day.sh` — the DuckDB-native day
// loader, declared as an external step
// (`models/sources/raw/github_loader.yml`) and invoked by `smelt run` on the
// run path (phase 7 of `docs/outcomes/20260906-external-dag-steps`). Distinct
// from `bq-dogfood-loader.sh` above: that script derives BigQuery load SQL
// and never touches a database; this one loads directly into a real DuckDB
// database and is exercised by spawning it, not by inspecting emitted SQL.
// ---------------------------------------------------------------------------

fn day_loader_script() -> PathBuf {
    repo_root().join("examples/github_activity/load_day.sh")
}

fn day_loader_sample_parquet() -> PathBuf {
    repo_root().join("examples/github_activity/seeds/github_events_sample.parquet")
}

fn run_day_loader(database: &Path, date: &str) -> Output {
    Command::new("bash")
        .arg(day_loader_script())
        .args(["--date", date, "--database"])
        .arg(database)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn load_day.sh: {e}"))
}

fn duckdb_scalar(database: &Path, sql: &str) -> i64 {
    let out = Command::new("duckdb")
        .arg(database)
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

/// Test 1: running `load_day.sh --date D` twice against one database yields
/// the same row counts in both relations as running it once — the per-day
/// ledger (`main._loader_days`) makes the load idempotent, required because
/// a run may legitimately invoke the step more than once for the same day.
#[test]
fn load_day_script_is_idempotent() {
    let tmp = TempDir::new().expect("tempdir");
    let db = tmp.path().join("dev.duckdb");

    let out = run_day_loader(&db, "2026-08-06");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events_once = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events",
    );
    let arrival_once = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events_arrival",
    );

    let out = run_day_loader(&db, "2026-08-06");
    assert!(
        out.status.success(),
        "second invocation should also exit 0 (idempotent skip); stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events_twice = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events",
    );
    let arrival_twice = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events_arrival",
    );

    assert_eq!(
        events_once, events_twice,
        "re-running the same day must not duplicate rows"
    );
    assert_eq!(
        arrival_once, arrival_twice,
        "re-running the same day must not duplicate rows"
    );
    assert!(
        events_once > 0,
        "the first invocation should have loaded some rows"
    );
}

/// Test 2: after days D and D+1, the event-time relation carries the 2%
/// D-slice a second time under its original `created_at`, and the arrival
/// relation carries those same ids stamped `ingested_date = D+1`.
#[test]
fn load_day_script_redelivers_the_previous_day() {
    let tmp = TempDir::new().expect("tempdir");
    let db = tmp.path().join("dev.duckdb");

    let out = run_day_loader(&db, "2026-08-05");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_day_loader(&db, "2026-08-06");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let sample = day_loader_sample_parquet();
    let real_day05 = duckdb_scalar(
        &db,
        &format!(
            "SELECT count(*) AS c FROM read_parquet('{}') \
             WHERE CAST(created_at AS DATE) = DATE '2026-08-05'",
            sample.display()
        ),
    );
    let expected_redelivered = duckdb_scalar(
        &db,
        &format!(
            "SELECT count(*) AS c FROM read_parquet('{}') \
             WHERE CAST(created_at AS DATE) = DATE '2026-08-05' \
             AND MOD(CAST(id AS BIGINT), 50) = 0",
            sample.display()
        ),
    );
    assert!(
        expected_redelivered > 0,
        "the fixture must carry a redeliverable slice for 08-05"
    );

    // The redelivered rows keep their original `created_at` (day 05), so
    // they land in the *same* date group as the real day-05 rows — the
    // event-time relation's day-05 count is the real total plus the
    // redelivered slice, a second physical (duplicate-id) copy.
    let events_day05 = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events \
         WHERE CAST(created_at AS DATE) = DATE '2026-08-05'",
    );
    assert_eq!(
        events_day05,
        real_day05 + expected_redelivered,
        "the event-time relation's day-05 rows must be the real total plus the day-06 \
         redelivery of the same date (byte-identical id and created_at)"
    );

    let arrival_redelivered_on_open_partition = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events_arrival \
         WHERE ingested_date = DATE '2026-08-06' \
         AND CAST(created_at AS DATE) = DATE '2026-08-05'",
    );
    assert_eq!(
        arrival_redelivered_on_open_partition, expected_redelivered,
        "the arrival relation must carry the redelivered day-05 ids stamped ingested_date=2026-08-06"
    );
}

/// Test 3: a fresh database needs no `setup_sources.sql` pre-pass for the
/// step to succeed — `load_day.sh` creates both raw tables itself when
/// absent.
#[test]
fn load_day_script_creates_the_raw_tables_when_absent() {
    let tmp = TempDir::new().expect("tempdir");
    let db = tmp.path().join("dev.duckdb");
    assert!(!db.exists());

    let out = run_day_loader(&db, "2026-08-05");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(db.exists());

    let events = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events",
    );
    let arrival = duckdb_scalar(
        &db,
        "SELECT count(*) AS c FROM main.sources_raw_github_events_arrival",
    );
    assert!(events > 0, "expected rows to be loaded on first invocation");
    assert_eq!(
        events, arrival,
        "both relations should carry the same day-05 rows"
    );
}
