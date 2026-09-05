//! Phase 3 of `docs/plans/20260719-prod-w7-bakeoff.md` — the request-scoped
//! forcing seam (`ExecuteRequest::technique_overrides`) plus the
//! scratch-as-synthetic-target pattern (decision B1) that `smelt bakeoff`
//! builds on. Reuses the same fact (`events`) + dimension (`users`)
//! enrichment fixture as `maintenance_pins.rs`.
//!
//! **Post-`docs/plans/20260808-membership-sensitivity.md` Phase 3 note:**
//! `users` is read purely in the `JOIN`'s own `ON` predicate — a
//! row-admission read — so the model's `UpstreamMutation(users)` `{user_name}`
//! cell (and its sibling `{event_id, event_type, user_id}` cell for the
//! SAME trigger) is now membership-sensitive (`Technique::DeleteInsert`),
//! never `ColumnScopedMerge`. This model is `grain: partition` with no
//! `unique_key` — `RowIdentity::WholeRow` — and since `docs/outcomes/
//! 20260815-definition-delta-migrate/phases/27c-plan.md` a live runtime
//! DISPATCH exists for exactly this shape: the keyless (whole-row) staged-
//! candidate conditional write (`MembershipRecomputeWrite::StagedKeyless`),
//! reported as `"delete_insert_suppressed"` — an ADMISSIBLE
//! `technique_overrides` entry (or no override at all) now resolves to this
//! live, change-suppressed write rather than the plain unconditional
//! `"deleteinsert"` batch loop.
//!
//! **That does not mean overrides are unconsulted.** `resolve_live_column_
//! scoped_cell`'s pin-consulting loop (called unconditionally by the
//! `grain: partition` batch loop, looking for a live `ColumnScopedMerge`
//! opportunity this shape no longer has) still runs `resolve_cell_choice`
//! over the trigger's cell(s) — an INADMISSIBLE override refuses loudly
//! there, before the "not ColumnScopedMerge, discard" branch is ever
//! reached. A Phase-3-reviewer fix (same date) corrected a real bug in that
//! loop: the trigger derives TWO sibling cells, and the loop used to
//! consult only whichever one `MaintenancePlan::cell_for`'s first-match
//! lookup happened to return, so an override scoped to `columns:
//! [user_name]` was silently never matched whenever the OTHER sibling came
//! first. Fixed via `MaintenancePlan::cells_for`
//! (`crates/smelt-logical/src/maintenance/mod.rs`) — every sibling is now
//! offered the override, matched against its own columns
//! (`crates/smelt-runtime/src/maintenance_driver.rs`).
//!
//! The scratch target is built the same way `smelt bakeoff` builds it
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

    // The `batched:` sub-block is retired everywhere; the MERGE-dedup-only
    // `merge_key:` (this row-shaped join can't become the composed
    // key+clock shape — no `GROUP BY`) is declared via the `smelt.yml`
    // model override instead (`docs/specs/models.md` §"Batched sub-block
    // retirement").
    let smelt_yml = format!(
        "name: bakeoff_seam_fixture\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    \
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
        LinkCProject::load(project.project_dir.clone(), project.db_path.clone())
            .expect("reload LinkCProject for scratch target")
            .with_config(std::sync::Arc::new(cfg)),
        target_name,
    )
}

/// A request-scope `technique_overrides` entry is validated by the SAME
/// `resolve_cell_choice` ladder a frontmatter `cells[].technique` pin goes
/// through (`maintenance_pins.rs`'s own tests) — `recompute` (mapping to
/// `ChosenTechnique::RegionRecompute`) is always a resolvable-set member,
/// so it is honored (never refuses); `rederive_columns` (mapping to
/// `Technique::ColumnScopedMerge`) is NOT a member of this membership-
/// sensitive cell's resolvable set `{recompute, DeleteInsert}` — the cell's
/// own admitted technique is `DeleteInsert`, never `ColumnScopedMerge` —
/// so it refuses loudly, on the creation run already (the choice ladder is
/// consulted every run, `maintenance_pins.rs`'s own established
/// convention). This is the genuinely-changed half of the original claim
/// ("force EITHER member of the resolvable set"): `ColumnScopedMerge` is
/// unreachable from ANY currently-shipped shape post-Phase-1
/// (`docs/TODO.md`), so `rederive_columns` is no longer a member to force
/// at all — the honest test proves `recompute` still works exactly as
/// before, and `rederive_columns` now fails exactly the way an unadmitted
/// pin should.
///
/// **Post-27c note:** for this `RowIdentity::WholeRow` cell, `DeleteInsert`
/// IS the family's own "recompute" option (`resolve_live_membership_
/// recompute_cell`'s own doc comment) — `technique: recompute` resolves to
/// `ChosenTechnique::Admitted(Technique::DeleteInsert)`, the SAME arm the
/// unpinned default resolves to, which now dispatches the live keyless
/// staged-candidate conditional write (`"delete_insert_suppressed"`) rather
/// than the plain unconditional batch loop.
#[tokio::test]
async fn request_override_forces_each_admissible_technique() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let (recompute_project, recompute_target) = scratch_project(&project, "recompute");
    seed_tables(&db_path, "smelt_bakeoff_test_recompute");

    // `recompute` is always resolvable: the creation run succeeds, and so
    // does a later run over a mutated dimension.
    let mut recompute_request = day_request();
    recompute_request.target = recompute_target.clone();
    recompute_request.technique_overrides = vec![user_name_override(CellTechnique::Recompute)];
    recompute_project
        .run_quiet("recompute-create", recompute_request.clone())
        .await
        .expect("recompute-target creation run must succeed");

    rename_user_one(&db_path, "smelt_bakeoff_test_recompute");

    let recompute_outcome = recompute_project
        .run_quiet("recompute-second", recompute_request)
        .await
        .expect("run with a `recompute` override must succeed — it is always resolvable");
    assert_eq!(
        recompute_outcome
            .models
            .get("daily_events_enriched")
            .expect("model ran")
            .strategy,
        "delete_insert_suppressed",
        "the `recompute` override resolves to the SAME live keyless staged-candidate write path \
         this shape already dispatches by default — the point of this half of the test is that \
         the override is CONSULTED and ADMITTED, not that it changes the write path"
    );
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

    // `rederive_columns` (ColumnScopedMerge) is NOT a member of this
    // membership-sensitive cell's resolvable set — refuses loudly, already
    // on the creation run.
    let (rederive_project, rederive_target) = scratch_project(&project, "rederive");
    seed_tables(&db_path, "smelt_bakeoff_test_rederive");
    let mut rederive_request = day_request();
    rederive_request.target = rederive_target;
    rederive_request.technique_overrides = vec![user_name_override(CellTechnique::RederiveColumns)];
    let err = rederive_project
        .run_quiet("rederive-create", rederive_request)
        .await
        .expect_err(
            "a `rederive_columns` override for a cell that only ever admits DeleteInsert \
             (membership-sensitive) must refuse the run loudly, never silently fall back",
        );
    assert!(
        format!("{err:#}").contains("MaintenanceUnboundedFootprint"),
        "expected the ChoiceRefusal diagnostic family in the error: {err:#}"
    );

    // Neither scratch run ever touched the real `main` schema.
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

/// A request override naming `fold` for the `{user_name}` cell refuses
/// loudly (`ChoiceRefusal`/`MaintenanceUnboundedFootprint`) — `Fold` maps
/// to `KeyedFold`/`InPlaceUpdate`, neither of which this membership-
/// sensitive cell's resolvable set `{recompute, DeleteInsert}` contains.
///
/// **Restored to its original loud-refusal expectation** (module doc
/// comment above): a Phase-3-reviewer fix corrected
/// `resolve_live_column_scoped_cell`'s pin-consulting loop to offer the
/// override to EVERY sibling cell sharing the trigger (`MaintenancePlan::
/// cells_for`), matched against each sibling's own columns — an override
/// scoped to `columns: [user_name]` now genuinely reaches the `{user_name}`
/// cell it names and refuses, on the creation run already.
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
        "a request override naming `fold` for a cell that only ever admits DeleteInsert \
         (membership-sensitive) must refuse the run loudly, never silently fall back",
    );
    assert!(
        format!("{err:#}").contains("MaintenanceUnboundedFootprint"),
        "expected the ChoiceRefusal diagnostic family in the error: {err:#}"
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
        "delete_insert_suppressed",
        "with no override at all, the cell's own honest default (the live keyless \
         staged-candidate write for this membership-sensitive, grain: partition, \
         RowIdentity::WholeRow cell) must still resolve, unperturbed by this seam"
    );
}
