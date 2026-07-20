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
use smelt_core::config::{CellTechnique, MaintenanceCellConfig};
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, ChosenTechnique,
};
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

    // `batched:` is retired in `.sql` frontmatter; the MERGE-dedup-only
    // `unique_key` (this row-shaped join can't become the composed
    // key+clock shape — no `GROUP BY`) stays declared via the `smelt.yml`
    // model override instead (`docs/specs/models.md` §"The Relation
    // Contract").
    let smelt_yml = format!(
        "name: {project_name}\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    \
         type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n\
         models:\n  daily_events_enriched:\n    batched:\n      unique_key: [event_id]\n",
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

fn schema_exists(db_path: &Path, schema: &str) -> bool {
    let conn = duckdb::Connection::open(db_path).expect("reconnect");
    conn.query_row(
        "SELECT count(*) > 0 FROM information_schema.schemata WHERE schema_name = ?",
        [schema],
        |row| row.get(0),
    )
    .expect("check schema existence")
}

#[tokio::test]
async fn bakeoff_reports_measured_cost_per_admissible_technique() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_multi_technique_project(&project_dir, &db_path, "bakeoff_report_fixture");
    seed_tables(&db_path, "main");

    let config = Config::load(&project_dir).expect("load config");
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
    .expect("bakeoff run must succeed");

    assert!(
        report.message.is_none(),
        "a measurable cell must not report 'nothing to measure'"
    );
    assert_eq!(
        report.cells.len(),
        1,
        "exactly one bakeoff-candidate cell in this fixture"
    );
    let cell = &report.cells[0];
    assert_eq!(
        cell.techniques.len(),
        2,
        "the cell's resolvable set has exactly 2 members"
    );
    assert!(
        cell.equivalence_checked,
        "cross-variant EXCEPT ALL must have been checked"
    );

    for measurement in &cell.techniques {
        assert!(
            measurement.total_wall_clock_ms() > 0,
            "technique '{:?}' must report a nonzero measured wall-clock cost",
            measurement.technique
        );
        assert_eq!(
            measurement.run_wall_clock_ms.len(),
            2,
            "one wall-clock measurement per replayed window (--runs 2)"
        );
    }
    let row_counts: Vec<i64> = cell.techniques.iter().map(|m| m.row_count).collect();
    assert_eq!(
        row_counts[0], row_counts[1],
        "every admissible technique must materialize the same row count"
    );
    assert!(row_counts[0] > 0, "the measured table must be non-empty");
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

#[tokio::test]
async fn bakeoff_drops_scratch_unless_keep() {
    if skip_without_duckdb_lib() {
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_multi_technique_project(&project_dir, &db_path, "bakeoff_cleanup_fixture");
    seed_tables(&db_path, "main");

    let config = Config::load(&project_dir).expect("load config");

    // Default run: scratch schemas and state dirs must be gone afterward.
    let dropped_opts = BakeoffOptions {
        cells: vec![],
        runs: 2,
        target: "dev".to_string(),
        keep: false,
        pin: false,
    };
    let dropped_report = run_bakeoff(
        &project_dir,
        std::sync::Arc::new(config.clone()),
        "daily_events_enriched",
        dropped_opts,
    )
    .await
    .expect("default bakeoff run must succeed");
    assert!(dropped_report.kept_schemas.is_empty());
    for cell in &dropped_report.cells {
        for m in &cell.techniques {
            assert!(
                !schema_exists(&db_path, &m.scratch_schema),
                "scratch schema '{}' must be dropped without --keep",
                m.scratch_schema
            );
            assert!(
                !project_dir
                    .join(".smelt/targets")
                    .join(format!("__bakeoff_{}", m.scratch_schema))
                    .exists(),
                "scratch state dir for '{}' must be removed without --keep",
                m.scratch_schema
            );
        }
    }

    // `--keep` run: scratch schemas and state dirs persist, and are named
    // in the report.
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
    .expect("--keep bakeoff run must succeed");
    assert!(
        !kept_report.kept_schemas.is_empty(),
        "--keep must report the retained schemas"
    );
    for cell in &kept_report.cells {
        for m in &cell.techniques {
            assert!(
                schema_exists(&db_path, &m.scratch_schema),
                "scratch schema '{}' must persist with --keep",
                m.scratch_schema
            );
            assert!(kept_report.kept_schemas.contains(&m.scratch_schema));
        }
    }
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

/// Phase 5: `--pin` must emit a `cells[]` entry that round-trips through the
/// SAME real ladder (`effective_override` + `resolve_cell_choice`) a live run
/// consults, and it must resolve to exactly the technique bakeoff measured as
/// the winner — not a re-derived or hand-picked value.
/// `stage_multi_technique_project`'s model already declares a `maintenance:`
/// block (`scan_bounds` only, no `cells[]`), so this exercises the "model
/// already has a `maintenance:` block" branch: the pin is a bare `cells[]`
/// entry, not a full `maintenance:` block.
#[tokio::test]
async fn pin_emits_parseable_cells_entry() {
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
    .expect("bakeoff run must succeed");

    let pin = report
        .pin
        .as_ref()
        .expect("--pin must populate a pin suggestion on the report");
    assert!(
        pin.header.contains("daily_events_enriched.sql"),
        "pin header must name the model's .sql file: {}",
        pin.header
    );

    let parsed: Vec<MaintenanceCellConfig> = serde_yaml::from_str(&pin.yaml)
        .expect("emitted pin YAML must parse as a cells[] entry list");
    assert_eq!(
        parsed.len(),
        1,
        "exactly one bakeoff-candidate cell in this fixture"
    );
    let parsed_cell = &parsed[0];
    assert_eq!(parsed_cell.on, "users");
    assert_eq!(parsed_cell.columns, vec!["user_name".to_string()]);
    let winning_technique = parsed_cell
        .technique
        .expect("the pin must be a hard technique pin");

    // Round-trip: feed the parsed cell config through the SAME real ladder
    // (`effective_override` + `resolve_cell_choice`) a live run consults,
    // and confirm it resolves to exactly the winning technique.
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
    let result = smelt_db::maintenance_plan_report(&db, ws, file).expect("maintenance plan report");

    let trigger = Trigger::UpstreamMutation {
        source: parsed_cell.on.clone(),
    };
    let cell = result
        .plan
        .cell_for(&trigger)
        .expect("cell exists in the derived plan");
    let group_columns = result
        .column_groups
        .iter()
        .find(|g| g.name() == cell.group)
        .map(|g| g.columns.clone())
        .unwrap_or_default();

    let overrides = effective_override(
        None,
        std::slice::from_ref(parsed_cell),
        &parsed_cell.on,
        &group_columns,
    );
    let chosen = resolve_cell_choice(&result.plan, &trigger, &overrides, None, true)
        .expect("the pinned technique must be admissible for this cell");

    let expected = match winning_technique {
        CellTechnique::Recompute => ChosenTechnique::RegionRecompute,
        _ => ChosenTechnique::Admitted(cell.technique.clone()),
    };
    assert_eq!(
        chosen, expected,
        "the pinned YAML must round-trip to exactly the technique bakeoff measured as the winner"
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
    .expect("bakeoff run must succeed");
    assert!(
        report.pin.is_some(),
        "sanity: this fixture must actually produce a pin"
    );

    let after = snapshot_model_files(&project_dir);
    assert_eq!(
        before, after,
        "smelt bakeoff --pin must never modify or create model files (emit-only per B2)"
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
