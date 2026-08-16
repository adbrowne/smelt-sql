//! Phase 4 of `docs/plans/20260719-prod-w7-bakeoff.md` — the `smelt bakeoff`
//! command: measured per-cell × per-technique cost report over replayed
//! windows of real data (`incremental_models.md` §"CLI" bakeoff flags).
//! Phase 5 adds `--pin`: the winning technique per measured cell, emitted as
//! a ready-to-paste YAML fragment — emit-only, no file on disk is ever
//! modified.
//!
//! Reuses the same fact (`events`) + dimension (`users`) enrichment shape as
//! `bakeoff_seam.rs`/`maintenance_pins.rs` — its `{user_name}`
//! `UpstreamMutation` cell admits exactly `{rederive_columns, recompute}`,
//! so it is a genuine bakeoff candidate (2+ admissible techniques). All
//! DuckDB-backed tests skip loudly when `DUCKDB_LIB_DIR` is unset (matching
//! every other DuckDB-backed test in this crate).

use std::path::Path;
use std::process::Command;

use smelt_cli::bakeoff::{run_bakeoff, BakeoffOptions};
use smelt_cli::Config;
use smelt_logical::maintenance::Trigger;

const EVENTS_SOURCE: &str = r#"description: events source (fact)
columns:
  - name: event_id
    type: INTEGER
  - name: event_timestamp
    type: TIMESTAMP
  - name: user_id
    type: INTEGER
  - name: event_type
    type: VARCHAR
"#;

const USERS_SOURCE: &str = r#"description: users source (dimension)
mutation_profile:
  kind: mutable_snapshot
unique_key: [user_id]
referential_integrity: [user_id]
columns:
  - name: user_id
    type: INTEGER
  - name: user_name
    type: VARCHAR
"#;

/// Duplicates enough of `smelt-maintenance-testkit`'s `LinkCProject` fixture
/// shape (fact + dimension enrichment, no `maintenance.cells[]` frontmatter)
/// to give the `{user_name}` cell a genuine 2-technique resolvable set:
/// `{rederive_columns (ColumnScopedMerge), recompute}`.
fn stage_multi_technique_project(project_dir: &Path, db_path: &Path, project_name: &str) {
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create models/sources");

    // The `batched:` sub-block is retired everywhere; the MERGE-dedup-only
    // `merge_key:` (this row-shaped join can't become the composed
    // key+clock shape — no `GROUP BY`) is declared via the `smelt.yml`
    // model override instead (`docs/specs/models.md` §"Batched sub-block
    // retirement").
    let smelt_yml = format!(
        "name: {project_name}\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    \
         type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n\
         models:\n  daily_events_enriched:\n    merge_key: [event_id]\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).expect("write smelt.yml");

    let model_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      users:
        allow_full_scan: true
---
SELECT
    e.event_id,
    CAST(e.event_timestamp AS DATE) AS event_date,
    e.user_id,
    e.event_type,
    u.user_name
FROM smelt.sources.events e
JOIN smelt.sources.users u ON e.user_id = u.user_id
"#;
    std::fs::write(
        project_dir.join("models/daily_events_enriched.sql"),
        model_sql,
    )
    .expect("write model");

    std::fs::write(project_dir.join("models/sources/events.yml"), EVENTS_SOURCE)
        .expect("write events source");
    std::fs::write(project_dir.join("models/sources/users.yml"), USERS_SOURCE)
        .expect("write users source");
}

/// A plain single-source incremental model — its only cell is the creation
/// trigger (`NewData`), which never admits a second technique — so bakeoff
/// has nothing to measure by default.
fn stage_single_technique_project(project_dir: &Path, db_path: &Path, project_name: &str) {
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create models/sources");

    let smelt_yml = format!(
        "name: {project_name}\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    \
         type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).expect("write smelt.yml");

    let model_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
---
SELECT
    e.event_id,
    CAST(e.event_timestamp AS DATE) AS event_date,
    e.user_id,
    e.event_type
FROM smelt.sources.events e
"#;
    std::fs::write(project_dir.join("models/daily_events.sql"), model_sql).expect("write model");
    std::fs::write(project_dir.join("models/sources/events.yml"), EVENTS_SOURCE)
        .expect("write events source");
}

/// Seed source tables covering a wide-enough event-time range that slicing
/// into several windows produces genuinely non-overlapping days.
fn seed_tables(db_path: &Path, schema: &str) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(&format!(
        r#"
        CREATE SCHEMA IF NOT EXISTS {schema};
        CREATE OR REPLACE TABLE {schema}.sources_events (
            event_id INTEGER, event_timestamp TIMESTAMP, user_id INTEGER, event_type VARCHAR
        );
        INSERT INTO {schema}.sources_events VALUES
            (1, TIMESTAMP '2025-01-10 08:00:00', 1, 'login'),
            (2, TIMESTAMP '2025-01-11 09:00:00', 2, 'login'),
            (3, TIMESTAMP '2025-01-12 10:00:00', 1, 'logout'),
            (4, TIMESTAMP '2025-01-13 11:00:00', 2, 'logout'),
            (5, TIMESTAMP '2025-01-14 12:00:00', 1, 'login'),
            (6, TIMESTAMP '2025-01-15 13:00:00', 2, 'login');
        CREATE OR REPLACE TABLE {schema}.sources_users (user_id INTEGER, user_name VARCHAR);
        INSERT INTO {schema}.sources_users VALUES (1, 'Alice'), (2, 'Bob');
        "#
    ))
    .expect("seed source tables");
}

fn skip_without_duckdb_lib() -> bool {
    if std::env::var("DUCKDB_LIB_DIR").is_err() {
        eprintln!("skipping: DUCKDB_LIB_DIR not set (bakeoff tests require DuckDB)");
        return true;
    }
    false
}

/// Rewrite of the original `bakeoff_reports_measured_cost_per_admissible_
/// technique` (`docs/plans/20260808-membership-sensitivity.md` Phase 3):
/// `stage_multi_technique_project`'s `{user_name}` cell — and every sibling
/// column group's own cell for the SAME `UpstreamMutation(users)` trigger,
/// membership sensitivity being row-scoped, not per-column
/// (`incremental_models.md` §"The plan matrix") — is now derived
/// `Technique::DeleteInsert`: `users` is read in the `JOIN`'s own `ON`
/// predicate, a row-admission read, so `ColumnScopedMerge` is inadmissible
/// (Phase 1's review checklist: "membership cells cannot receive
/// ColumnScopedMerge"). `admitted_family` (`src/bakeoff.rs`) maps
/// `Technique::DeleteInsert` to `None` — a membership-sensitive cell is not
/// a bakeoff candidate at all (there is nothing to bake off: the recompute
/// family IS the cell's only admissible technique). This is a genuinely
/// different "nothing to measure" reason than
/// `bakeoff_with_no_multi_technique_cells_says_so`'s fixture (which has NO
/// `UpstreamMutation` cell whatsoever) — this model's derived plan DOES
/// carry `UpstreamMutation` cells, they are just all single-technique. The
/// direct `maintenance_plan_report` check below proves the "nothing to
/// measure" verdict is for that reason, not merely "no cell existed."
#[tokio::test]
async fn bakeoff_reports_nothing_to_measure_for_membership_sensitive_cells() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_multi_technique_project(&project_dir, &db_path, "bakeoff_report_fixture");
    seed_tables(&db_path, "main");

    let config = Config::load(&project_dir).expect("load config");

    // Prove the derived plan DOES carry a membership-sensitive
    // UpstreamMutation(users) cell — the "nothing to measure" verdict below
    // is because it's single-technique, not because no cell exists at all.
    {
        let discovery = smelt_cli::ModelDiscovery::new(project_dir.clone(), config.paths.clone());
        let models = discovery.discover_models().expect("discover models");
        let model = models
            .iter()
            .find(|m| m.name == "daily_events_enriched")
            .expect("model exists")
            .clone();
        let mut db = smelt_cli::init_db(&project_dir, &models);
        db.set_active_target(Some(std::sync::Arc::from("dev")));
        let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
        let file = db.source_file(&model.path).expect("model file registered");
        let result =
            smelt_db::maintenance_plan_report(&db, ws, file).expect("maintenance plan report");
        assert!(
            result.plan.cells.iter().any(|c| matches!(
                &c.trigger,
                Trigger::UpstreamMutation { source } if source == "users"
            ) && c.technique
                == smelt_logical::maintenance::Technique::DeleteInsert),
            "expected a membership-sensitive UpstreamMutation(users) DeleteInsert cell in the \
             derived plan, got: {:#?}",
            result.plan
        );
    }

    let opts = BakeoffOptions {
        cells: vec![],
        runs: 2,
        target: "dev".to_string(),
        keep: false,
        pin: false,
    };

    let report = run_bakeoff(
        &project_dir,
        std::sync::Arc::new(config),
        "daily_events_enriched",
        opts,
    )
    .await
    .expect("bakeoff must succeed (exit-success 'nothing to measure' report, not an error)");

    assert!(
        report.cells.is_empty(),
        "a membership-sensitive (DeleteInsert) cell is never a bakeoff candidate — it has \
         only one admissible technique"
    );
    assert!(
        report
            .message
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains("nothing to measure"),
        "report must clearly say there is nothing to measure: {:?}",
        report.message
    );
}

#[tokio::test]
async fn bakeoff_with_no_multi_technique_cells_says_so() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_single_technique_project(&project_dir, &db_path, "bakeoff_single_technique_fixture");
    seed_tables(&db_path, "main");

    let config = Config::load(&project_dir).expect("load config");
    let opts = BakeoffOptions {
        cells: vec![],
        runs: 3,
        target: "dev".to_string(),
        keep: false,
        pin: false,
    };

    let report = run_bakeoff(
        &project_dir,
        std::sync::Arc::new(config),
        "daily_events",
        opts,
    )
    .await
    .expect("bakeoff must succeed (exit-success 'nothing to measure' report, not an error)");

    assert!(
        report.cells.is_empty(),
        "no candidate cells for a single-technique model"
    );
    assert!(
        report
            .message
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains("nothing to measure"),
        "report must clearly say there is nothing to measure: {:?}",
        report.message
    );

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let scratch_schema_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.schemata WHERE schema_name LIKE 'smelt_bakeoff_%'",
            [],
            |row| row.get(0),
        )
        .expect("check for scratch schemas");
    assert_eq!(
        scratch_schema_count, 0,
        "no scratch schema may be created when there is nothing to measure"
    );
}

/// Rewrite (`docs/plans/20260808-membership-sensitivity.md` Phase 3):
/// `stage_multi_technique_project`'s only `UpstreamMutation` cells are now
/// membership-sensitive (`Technique::DeleteInsert`, `admitted_family` maps
/// to `None`), so `run_bakeoff` always takes its "nothing to measure"
/// early-return branch for this fixture — `--keep` is never even consulted
/// on that branch (`run_bakeoff`'s early `return` precedes the `opts.keep`
/// cleanup logic entirely). This test now proves exactly that: `--keep`
/// does not conjure scratch schemas out of nothing when there is nothing to
/// measure.
#[tokio::test]
async fn bakeoff_keep_is_a_no_op_when_nothing_to_measure() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_multi_technique_project(&project_dir, &db_path, "bakeoff_cleanup_fixture");
    seed_tables(&db_path, "main");

    let config = Config::load(&project_dir).expect("load config");

    let keep_opts = BakeoffOptions {
        cells: vec![],
        runs: 2,
        target: "dev".to_string(),
        keep: true,
        pin: false,
    };
    let kept_report = run_bakeoff(
        &project_dir,
        std::sync::Arc::new(config),
        "daily_events_enriched",
        keep_opts,
    )
    .await
    .expect("--keep bakeoff run must succeed (exit-success 'nothing to measure' report)");

    assert!(
        kept_report.cells.is_empty(),
        "nothing to measure for this fixture (membership-sensitive cells only)"
    );
    assert!(
        kept_report.kept_schemas.is_empty(),
        "--keep must report no retained schemas when nothing was measured"
    );

    let scratch_schema_count: i64 = {
        let conn = duckdb::Connection::open(&db_path).expect("reconnect");
        conn.query_row(
            "SELECT count(*) FROM information_schema.schemata WHERE schema_name LIKE \
             'smelt_bakeoff_%'",
            [],
            |row| row.get(0),
        )
        .expect("check for scratch schemas")
    };
    assert_eq!(
        scratch_schema_count, 0,
        "no scratch schema may be created by --keep when there is nothing to measure"
    );
    assert!(
        !project_dir.join(".smelt/targets").exists()
            || std::fs::read_dir(project_dir.join(".smelt/targets"))
                .map(|mut d| d.next().is_none())
                .unwrap_or(true),
        "no scratch state dir may be created by --keep when there is nothing to measure"
    );
}

/// `assert_cmd`-style subprocess smoke test, built with only `std::process`
/// (no new dev-dependency) — the real `smelt` binary via `CARGO_BIN_EXE_smelt`
/// against `examples/timeseries`.
#[test]
fn bakeoff_runs_via_real_binary() {
    if skip_without_duckdb_lib() {
        return;
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crates/smelt-cli -> workspace root");
    let example_dir = workspace_root.join("examples/timeseries");
    assert!(
        example_dir.exists(),
        "examples/timeseries must exist at {}",
        example_dir.display()
    );

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let copy_dir = tmp.path().join("timeseries");
    copy_dir_recursive(&example_dir, &copy_dir).expect("copy examples/timeseries");

    let bin = env!("CARGO_BIN_EXE_smelt");

    // `daily_events_enriched` reads `smelt.sources.raw.events` /
    // `smelt.sources.raw.users` — physical tables `sources_raw_events` /
    // `sources_raw_users` under the default `<schema>.<segs.join("_")>`
    // source-naming convention (`smelt-runtime::compile::make_path_ref_
    // resolver`). Seed them directly (same shape `examples/timeseries`'s
    // own `setup_sources.sql` uses, on the current naming convention)
    // rather than depending on `smelt seed`'s unrelated seed-address space.
    let db_path = copy_dir.join("target").join("dev.duckdb");
    std::fs::create_dir_all(db_path.parent().unwrap()).expect("create target dir");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS main;
            CREATE OR REPLACE TABLE main.sources_raw_events (
                event_id INTEGER, user_id INTEGER, event_type VARCHAR,
                event_timestamp TIMESTAMP, properties VARCHAR
            );
            INSERT INTO main.sources_raw_events VALUES
                (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00', NULL),
                (2, 1, 'page_view', TIMESTAMP '2025-01-11 08:05:00', NULL),
                (3, 2, 'login', TIMESTAMP '2025-01-12 09:00:00', NULL),
                (4, 1, 'purchase', TIMESTAMP '2025-01-13 10:30:00', NULL);
            CREATE OR REPLACE TABLE main.sources_raw_users (
                user_id INTEGER, user_name VARCHAR, signup_date DATE
            );
            INSERT INTO main.sources_raw_users VALUES
                (1, 'Alice', DATE '2025-01-01'),
                (2, 'Bob', DATE '2025-01-02');
            "#,
        )
        .expect("seed source tables");
    }

    let output = Command::new(bin)
        .args([
            "bakeoff",
            "daily_events_enriched",
            "--project-dir",
            copy_dir.to_str().expect("utf8 path"),
            "--runs",
            "2",
        ])
        .output()
        .expect("spawn smelt bakeoff");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "smelt bakeoff must exit 0 (or name a real error to fix); stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("smelt bakeoff report"),
        "expected the report header in stdout; got: {stdout}"
    );
}

/// Rewrite (`docs/plans/20260808-membership-sensitivity.md` Phase 3):
/// `run_bakeoff`'s early "nothing to measure" return precedes its own
/// `opts.pin` check entirely (`src/bakeoff.rs`) — `report.pin` is `None`
/// regardless of `--pin` whenever `candidate_cells` is empty, which it now
/// always is for `stage_multi_technique_project` (its only
/// `UpstreamMutation` cells are membership-sensitive `DeleteInsert`,
/// inadmissible for bakeoff — see `bakeoff_reports_nothing_to_measure_for_
/// membership_sensitive_cells`'s own doc comment). This test now proves
/// `--pin` does not conjure a pin suggestion out of nothing to measure.
#[tokio::test]
async fn pin_is_none_when_nothing_to_measure() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_multi_technique_project(&project_dir, &db_path, "bakeoff_pin_fixture");
    seed_tables(&db_path, "main");

    let config = Config::load(&project_dir).expect("load config");
    let opts = BakeoffOptions {
        cells: vec![],
        runs: 2,
        target: "dev".to_string(),
        keep: false,
        pin: true,
    };

    let report = run_bakeoff(
        &project_dir,
        std::sync::Arc::new(config.clone()),
        "daily_events_enriched",
        opts,
    )
    .await
    .expect("bakeoff must succeed (exit-success 'nothing to measure' report, not an error)");

    assert!(
        report.cells.is_empty(),
        "nothing to measure for this fixture (membership-sensitive cells only)"
    );
    assert!(
        report.pin.is_none(),
        "--pin must not populate a pin suggestion when there is nothing to measure"
    );
}

/// `--pin` is emit-only (B2): the report text is the only output, no file on
/// disk is ever written or modified.
#[tokio::test]
async fn pin_mutates_no_files() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_multi_technique_project(&project_dir, &db_path, "bakeoff_pin_no_mutate_fixture");
    seed_tables(&db_path, "main");

    let before = snapshot_model_files(&project_dir);

    let config = Config::load(&project_dir).expect("load config");
    let opts = BakeoffOptions {
        cells: vec![],
        runs: 2,
        target: "dev".to_string(),
        keep: false,
        pin: true,
    };
    let report = run_bakeoff(
        &project_dir,
        std::sync::Arc::new(config),
        "daily_events_enriched",
        opts,
    )
    .await
    .expect("bakeoff must succeed (exit-success 'nothing to measure' report, not an error)");
    // `report.pin` is `None` here — `stage_multi_technique_project`'s only
    // `UpstreamMutation` cells are membership-sensitive and never a bakeoff
    // candidate (see `pin_is_none_when_nothing_to_measure`'s own doc
    // comment) — but the emit-only invariant this test actually checks
    // (`--pin` never mutates a file on disk) holds trivially either way, so
    // it is still meaningful to assert on this "nothing to measure" path.
    assert!(report.pin.is_none());

    let after = snapshot_model_files(&project_dir);
    assert_eq!(
        before, after,
        "smelt bakeoff --pin must never modify or create model files (emit-only per B2), even \
         when there is nothing to measure"
    );
}

/// Byte contents of every file under `models/` plus `smelt.yml`, sorted by
/// relative path — a diffable snapshot of "the model files" `--pin` must
/// never touch (deliberately excludes `.smelt/`/the DuckDB database file,
/// which bakeoff's own scratch-schema replay legitimately touches).
fn snapshot_model_files(project_dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let models_dir = project_dir.join("models");
    collect_files(&models_dir, &models_dir, &mut files);
    files.push((
        "smelt.yml".to_string(),
        std::fs::read(project_dir.join("smelt.yml")).expect("read smelt.yml"),
    ));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("relative path")
                .to_string_lossy()
                .to_string();
            out.push((rel, std::fs::read(&path).expect("read file")));
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            if entry.file_name() == "target" || entry.file_name() == ".smelt" {
                continue;
            }
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}
