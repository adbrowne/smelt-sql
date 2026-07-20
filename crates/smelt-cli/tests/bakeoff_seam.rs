//! Phase 3 of `docs/plans/20260719-prod-w7-bakeoff.md` — the request-scoped
//! forcing seam (`ExecuteRequest::technique_overrides`) plus the
//! scratch-as-synthetic-target pattern (decision B1) that `smelt bakeoff`
//! will build on. Reuses the same fact (`events`) + dimension (`users`)
//! enrichment fixture as `maintenance_pins.rs` — its `{user_name}`
//! `UpstreamMutation` cell admits exactly `{ColumnScopedMerge,
//! RegionRecompute}` (`rederive_columns`/`recompute` in `CellTechnique`
//! terms) — so a request override naming either has an observable effect
//! to prove, and naming `fold` (a family this cell never admits) must
//! refuse exactly like a frontmatter `cells[].technique: fold` pin does.
//!
//! The scratch target is built the same way `smelt bakeoff` will build it
//! (decision B1): clone the real `dev` target in an in-memory `Config`
//! under a synthetic name, only `schema` changed — no runtime schema seam,
//! everything still goes through `execute_project`.

use std::path::Path;

use smelt_core::config::CellTechnique;
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_runtime::types::CellTechniqueOverride;

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

/// Same fact+dimension enrichment shape as `maintenance_pins.rs::stage_project`,
/// but with NO `maintenance.cells[]` frontmatter at all — this phase's tests
/// exercise the request-scope override alone, never a frontmatter pin.
fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create models/sources");

    let smelt_yml = format!(
        "name: bakeoff_seam_fixture\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    \
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
batched:
  unique_key: [event_id]
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

/// Seed source tables into `schema` — ref resolution ties source refs to
/// the SAME `config.targets[target].schema` as the model's own output
/// (`resolve_refs_in_sql`, `execute.rs`), so a scratch target with a
/// different schema needs its own copy of the source tables, exactly like
/// a real `smelt bakeoff` run would need real source data reachable in the
/// scratch schema.
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
            (2, TIMESTAMP '2025-01-10 09:00:00', 2, 'login');
        CREATE OR REPLACE TABLE {schema}.sources_users (user_id INTEGER, user_name VARCHAR);
        INSERT INTO {schema}.sources_users VALUES (1, 'Alice'), (2, 'Bob');
        "#
    ))
    .expect("seed source tables");
}

fn rename_user_one(db_path: &Path, schema: &str) {
    let conn = duckdb::Connection::open(db_path).expect("reconnect");
    conn.execute(
        &format!("UPDATE {schema}.sources_users SET user_name = 'Alicia' WHERE user_id = 1"),
        [],
    )
    .expect("mutate dimension");
}

fn day_request() -> smelt_runtime::types::ExecuteRequest {
    let mut request = base_request("dev");
    request.start = Some("2025-01-10".to_string());
    request.end = Some("2025-01-11".to_string());
    request
}

fn user_name_override(technique: CellTechnique) -> CellTechniqueOverride {
    CellTechniqueOverride {
        columns: vec!["user_name".to_string()],
        on: "users".to_string(),
        technique,
    }
}

/// Clone `project`'s `dev` target into a synthetic target named
/// `scratch_name` with `schema: smelt_bakeoff_test_<scratch_name>` — the
/// exact pattern `smelt bakeoff` will use (decision B1). Returns a new
/// `LinkCProject` pointed at the SAME database file, so both scratch runs
/// and the real target share one connection's worth of source tables.
fn scratch_project(project: &LinkCProject, scratch_name: &str) -> (LinkCProject, String) {
    let mut cfg = (*project.config).clone();
    let dev_target = cfg.targets.get("dev").expect("dev target present").clone();
    let mut scratch_target = dev_target;
    scratch_target.schema = format!("smelt_bakeoff_test_{scratch_name}");
    let target_name = format!("scratch_{scratch_name}");
    cfg.targets.insert(target_name.clone(), scratch_target);
    (
        LinkCProject {
            project_dir: project.project_dir.clone(),
            db_path: project.db_path.clone(),
            config: std::sync::Arc::new(cfg),
        },
        target_name,
    )
}

/// Two runs of the same model, forced to different admissible techniques
/// via `ExecuteRequest::technique_overrides`, land in two scratch schemas,
/// agree exactly on the mutated dimension value, and never touch the real
/// `main` schema — decision B1's scratch-as-synthetic-target pattern, and
/// the point of the seam: an operator can force EITHER member of the
/// cell's resolvable set `{rederive_columns (ColumnScopedMerge), recompute}`
/// and get the same answer.
#[tokio::test]
async fn request_override_forces_each_admissible_technique() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let (recompute_project, recompute_target) = scratch_project(&project, "recompute");
    let (rederive_project, rederive_target) = scratch_project(&project, "rederive");
    seed_tables(&db_path, "smelt_bakeoff_test_recompute");
    seed_tables(&db_path, "smelt_bakeoff_test_rederive");

    // Creation runs (both scratch schemas, BEFORE the mutation) — the
    // technique override has no observable effect yet (creation always
    // takes the plain build path, see `maintenance_pins.rs`), but both
    // scratch tables start from the SAME unmutated source snapshot.
    let mut recompute_request = day_request();
    recompute_request.target = recompute_target.clone();
    recompute_request.technique_overrides = vec![user_name_override(CellTechnique::Recompute)];
    recompute_project
        .run_quiet("recompute-create", recompute_request.clone())
        .await
        .expect("recompute-target creation run must succeed");

    let mut rederive_request = day_request();
    rederive_request.target = rederive_target.clone();
    rederive_request.technique_overrides = vec![user_name_override(CellTechnique::RederiveColumns)];
    rederive_project
        .run_quiet("rederive-create", rederive_request.clone())
        .await
        .expect("rederive-target creation run must succeed");

    // The SAME mutation applied identically to both scratch schemas' own
    // source copies — `users` is `mutation_profile: mutable_snapshot`, so
    // this makes the `{user_name}` UpstreamMutation cell live for the next
    // run on BOTH scratch targets over the SAME window.
    rename_user_one(&db_path, "smelt_bakeoff_test_recompute");
    rename_user_one(&db_path, "smelt_bakeoff_test_rederive");

    let recompute_outcome = recompute_project
        .run_quiet("recompute-second", recompute_request)
        .await
        .expect("request override forcing recompute must succeed");
    assert_ne!(
        recompute_outcome
            .models
            .get("daily_events_enriched")
            .expect("model ran")
            .strategy,
        "column_scoped_merge",
        "a request override naming `recompute` must force the whole-region recompute path"
    );

    let rederive_outcome = rederive_project
        .run_quiet("rederive-second", rederive_request)
        .await
        .expect("request override forcing rederive_columns must succeed");
    assert_eq!(
        rederive_outcome
            .models
            .get("daily_events_enriched")
            .expect("model ran")
            .strategy,
        "column_scoped_merge",
        "a request override naming `rederive_columns` must force the column-scoped MERGE path"
    );

    // Both scratch outputs are non-empty and agree exactly (`EXCEPT ALL`
    // empty both directions) — same forced-technique-agrees-with-each-other
    // proof the maintenance-plan equivalence invariant makes for
    // frontmatter pins, now over the request-scope seam.
    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let recompute_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM smelt_bakeoff_test_recompute.daily_events_enriched",
            [],
            |row| row.get(0),
        )
        .expect("recompute scratch table non-empty");
    assert!(
        recompute_count > 0,
        "recompute scratch output must be non-empty"
    );
    let rederive_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM smelt_bakeoff_test_rederive.daily_events_enriched",
            [],
            |row| row.get(0),
        )
        .expect("rederive scratch table non-empty");
    assert!(
        rederive_count > 0,
        "rederive scratch output must be non-empty"
    );

    let forward_diff: i64 = conn
        .query_row(
            "SELECT count(*) FROM (
                SELECT * FROM smelt_bakeoff_test_recompute.daily_events_enriched
                EXCEPT ALL
                SELECT * FROM smelt_bakeoff_test_rederive.daily_events_enriched
            )",
            [],
            |row| row.get(0),
        )
        .expect("forward EXCEPT ALL");
    assert_eq!(
        forward_diff, 0,
        "recompute-forced output must match rederive-forced output"
    );
    let backward_diff: i64 = conn
        .query_row(
            "SELECT count(*) FROM (
                SELECT * FROM smelt_bakeoff_test_rederive.daily_events_enriched
                EXCEPT ALL
                SELECT * FROM smelt_bakeoff_test_recompute.daily_events_enriched
            )",
            [],
            |row| row.get(0),
        )
        .expect("backward EXCEPT ALL");
    assert_eq!(
        backward_diff, 0,
        "rederive-forced output must match recompute-forced output"
    );

    // Neither scratch run ever touched the real `main` schema — the model
    // table was never created there.
    let real_schema_has_table: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_name = 'daily_events_enriched'",
            [],
            |row| row.get(0),
        )
        .expect("check real schema");
    assert!(
        !real_schema_has_table,
        "scratch-target runs must leave the real target's schema untouched"
    );
}

/// A request override naming `fold` for the `{user_name}` cell — a family
/// this cell never admits (`{ColumnScopedMerge, RegionRecompute}` is the
/// full resolvable set, same as `maintenance_pins.rs::inadmissible_pin_
/// fails_loud`'s frontmatter-pin counterpart) — must refuse loudly with the
/// SAME `ChoiceRefusal` wording as a frontmatter pin, never execute.
#[tokio::test]
async fn request_override_subject_to_admission() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");
    let (scratch, target_name) = scratch_project(&project, "fold");
    seed_tables(&db_path, "smelt_bakeoff_test_fold");

    let mut request = day_request();
    request.target = target_name;
    request.technique_overrides = vec![user_name_override(CellTechnique::Fold)];

    let err = scratch.run_quiet("fold-create", request).await.expect_err(
        "a request override naming `fold` for a cell that only ever admits \
             ColumnScopedMerge must refuse the run loudly, never silently execute",
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("MaintenanceUnboundedFootprint"),
        "expected the same ChoiceRefusal diagnostic family a frontmatter pin refusal uses: \
         {message}"
    );
}

/// An `ExecuteRequest` with `technique_overrides` left at its default
/// (empty `Vec`) resolves byte-identically to a request built before this
/// phase added the field — guards `execute_parity`: this seam must never
/// perturb the unpinned, no-override default path.
#[tokio::test]
async fn empty_overrides_change_nothing() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_tables(&db_path, "main");

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let request = day_request();
    assert!(
        request.technique_overrides.is_empty(),
        "base_request's default must leave technique_overrides empty"
    );
    project
        .run_quiet("create", request.clone())
        .await
        .expect("creation run must succeed");

    rename_user_one(&db_path, "main");

    let outcome = project
        .run_quiet("second", request)
        .await
        .expect("empty technique_overrides must never perturb the default resolution path");
    assert_eq!(
        outcome
            .models
            .get("daily_events_enriched")
            .expect("model ran")
            .strategy,
        "column_scoped_merge",
        "with no override at all, the cell's own live default (ColumnScopedMerge) must still \
         resolve — unchanged from before this phase"
    );
}
