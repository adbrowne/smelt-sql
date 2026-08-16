//! Phase 2 of `docs/plans/20260719-prod-w7-bakeoff.md` — the choice ladder
//! (`smelt_logical::maintenance::choice::{effective_override,
//! resolve_cell_choice}`) wired into `smelt-runtime`'s real execute path.
//! Before this phase, `maintenance_driver::resolve_live_column_scoped_cell`
//! called the pin-less two-way resolver with a hardcoded `pin: None` —
//! frontmatter `cells[].technique`/`cells[].prefer` overrides were parsed
//! into `ModelMetadata::maintenance` but never actually consulted at
//! execution time. These tests drive the real `execute_project` pipeline
//! (via `smelt_maintenance_testkit::link_c_harness::LinkCProject`, never a
//! hand-injected filter — root `CLAUDE.md` §"Run pipeline parity rule") over
//! a fact+dimension enrichment model.
//!
//! **Post-`docs/plans/20260808-membership-sensitivity.md` Phase 3 note:**
//! `users` is read purely in the `JOIN`'s own `ON` predicate — a
//! row-admission read — so the `{user_name}` `UpstreamMutation` cell (and
//! its sibling `{event_id, event_type, user_id}` cell for the SAME
//! trigger — membership sensitivity is row-scoped, not per-column) is now
//! membership-sensitive (`Technique::DeleteInsert`), never
//! `Technique::ColumnScopedMerge`. This model is `grain: partition`: there
//! is no live runtime DISPATCH for a `grain: partition` `DeleteInsert`
//! membership cell (`resolve_live_membership_recompute_cell`'s own doc
//! comment — that fact is still true and unchanged by this note), so
//! neither pin below ever steers a run onto a different write path.
//!
//! **What this note corrects (Phase 3 reviewer fix, same date):** a hard
//! `cells[].technique:` pin is still validated even though it never steers
//! anything — `resolve_live_column_scoped_cell`'s own pin-consulting loop
//! (called unconditionally by the `grain: partition` batch loop, looking
//! for a live `ColumnScopedMerge` opportunity that this shape no longer
//! has) runs `resolve_cell_choice` over the trigger's admitted cell(s)
//! regardless of whether the result ends up dispatching anything, and an
//! inadmissible pin refuses loudly there via `?`, before the "not
//! ColumnScopedMerge, discard" branch is ever reached. An EARLIER pass of
//! this file (now corrected) mis-attributed the pin's *observed* silence to
//! "no live dispatch exists, so the pin is never consulted" — the real
//! cause was `MaintenancePlan::cell_for`'s first-match lookup: the
//! `UpstreamMutation(users)` trigger derives TWO sibling cells (`{user_name}`
//! and `{event_id, event_type, user_id}`), and the pin-consulting loop only
//! ever evaluated an override against whichever sibling `cell_for` returned
//! first — a `technique: fold` pin scoped to `columns: [user_name]` was
//! silently never matched whenever the OTHER sibling happened to be first.
//! Fixed in `crates/smelt-runtime/src/maintenance_driver.rs`
//! (`MaintenancePlan::cells_for`, `crates/smelt-logical/src/maintenance/
//! mod.rs`): every sibling cell sharing a trigger is now offered the
//! override, matched against its OWN columns — so `inadmissible_pin_fails_
//! loud` below is restored to its original loud-refusal expectation. Every
//! test in this file implicitly exercises the fix's other half too: the
//! `{event_id, event_type, user_id}` sibling — which no test's `columns:
//! [user_name]` pin ever addresses — must still resolve cleanly to its own
//! honest default every run, never refuse and never get silently skipped
//! just because a DIFFERENT sibling carries the pin.

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};

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

/// Stage a fact (`events`) + dimension (`users`) enrichment project whose
/// `{user_name}` `UpstreamMutation` cell admits `Technique::ColumnScopedMerge`
/// (identical shape to `examples/timeseries/models/daily_events_enriched.sql`),
/// with `cells_yaml` spliced into the model's `maintenance:` frontmatter block
/// so each test can exercise a different override on that exact cell.
fn stage_project(project_dir: &Path, db_path: &Path, cells_yaml: &str) {
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create models/sources");

    // The `batched:` sub-block is retired everywhere
    // (`docs/specs/models.md` §"Batched sub-block retirement") — this
    // model's own row-shaped join can't become the composed key+clock shape
    // a top-level `unique_key:` would derive (no `GROUP BY` to satisfy
    // `KeyedRequiresGroupBy`), so the MERGE-dedup-only `merge_key:` is
    // declared via the `smelt.yml` model override instead.
    let smelt_yml = format!(
        "name: maintenance_pins_fixture\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    \
         type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n\
         models:\n  daily_events_enriched:\n    merge_key: [event_id]\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).expect("write smelt.yml");

    let model_sql = format!(
        r#"---
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
{cells_yaml}---
SELECT
    e.event_id,
    CAST(e.event_timestamp AS DATE) AS event_date,
    e.user_id,
    e.event_type,
    u.user_name
FROM smelt.sources.events e
JOIN smelt.sources.users u ON e.user_id = u.user_id
"#
    );
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

fn seed_tables(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events (
            event_id INTEGER, event_timestamp TIMESTAMP, user_id INTEGER, event_type VARCHAR
        );
        INSERT INTO main.sources_events VALUES
            (1, TIMESTAMP '2025-01-10 08:00:00', 1, 'login'),
            (2, TIMESTAMP '2025-01-10 09:00:00', 2, 'login');
        CREATE OR REPLACE TABLE main.sources_users (user_id INTEGER, user_name VARCHAR);
        INSERT INTO main.sources_users VALUES (1, 'Alice'), (2, 'Bob');
        "#,
    )
    .expect("seed source tables");
}

fn rename_user_one(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("reconnect");
    conn.execute(
        "UPDATE main.sources_users SET user_name = 'Alicia' WHERE user_id = 1",
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

/// A `cells[].technique: recompute` hard pin forces the `{user_name}` cell's
/// mutation trigger away from its own admitted `ColumnScopedMerge` default
/// and onto the always-available whole-region recompute — proving the pin
/// is actually threaded into `resolve_cell_choice`, not merely parsed and
/// discarded (the bug this phase fixes: the driver used to hardcode
/// `pin: None`).
#[tokio::test]
async fn technique_pin_forces_region_recompute_at_runtime() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(
        &project_dir,
        &db_path,
        "  cells:\n    - columns: [user_name]\n      on: users\n      technique: recompute\n",
    );
    seed_tables(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Run 1: creation — the target doesn't exist yet, so this always takes
    // the plain build path regardless of any pin.
    project
        .run_quiet("run-1", day_request())
        .await
        .expect("creation run must succeed");

    // Between runs: rename user 1 — `users` is `mutation_profile:
    // mutable_snapshot`, so this makes the `{user_name}` UpstreamMutation
    // cell live for the next run over the SAME window.
    rename_user_one(&db_path);

    let outcome = project
        .run_quiet("run-2", day_request())
        .await
        .expect("pinned recompute run must succeed, not refuse");
    let record = outcome
        .models
        .get("daily_events_enriched")
        .expect("model ran");
    assert_ne!(
        record.strategy, "column_scoped_merge",
        "a `technique: recompute` pin must force the whole-region recompute path, bypassing \
         the cell's own admitted ColumnScopedMerge default — got strategy {:?}",
        record.strategy
    );

    // The recomputed table must still reflect the mutation (a region
    // recompute reads current source contents, same as the cost-model
    // default would) — matching a hand-written full-refresh oracle.
    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let recomputed_name: String = conn
        .query_row(
            "SELECT user_name FROM main.daily_events_enriched WHERE user_id = 1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read recomputed user_name");
    assert_eq!(
        recomputed_name, "Alicia",
        "the pinned region recompute must still pick up the mutated dimension value"
    );
    let oracle_name: String = conn
        .query_row(
            "SELECT u.user_name FROM main.sources_events e JOIN main.sources_users u \
             ON e.user_id = u.user_id WHERE e.user_id = 1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("full-refresh oracle read");
    assert_eq!(
        recomputed_name, oracle_name,
        "the pinned recompute path must match a full-refresh oracle over current source state"
    );
}

/// A `cells[].technique: fold` hard pin names a technique outside the
/// `{user_name}` cell's resolvable set (`{recompute, DeleteInsert}` —
/// `fold` maps to `KeyedFold`/`InPlaceUpdate`, neither of which this cell,
/// now membership-sensitive, ever admits) — `resolve_cell_choice` must
/// refuse loudly (`ChoiceRefusal`/`MaintenanceUnboundedFootprint`), never
/// silently fall back to region recompute or the cell's own default.
///
/// **Restored to its original loud-refusal expectation** (module doc
/// comment above): the choice ladder is consulted on every run, including
/// the very first (creation) run, and — once
/// `resolve_live_column_scoped_cell` correctly offers the pin to EVERY
/// sibling cell sharing the trigger, not only whichever one `cell_for`
/// happened to return first — this pin (scoped to `[user_name]`, the
/// FIRST-derived sibling here) is genuinely consulted and refuses
/// immediately. No `run-2`/mutation is needed to observe the refusal.
#[tokio::test]
async fn inadmissible_pin_fails_loud() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(
        &project_dir,
        &db_path,
        "  cells:\n    - columns: [user_name]\n      on: users\n      technique: fold\n",
    );
    seed_tables(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let err = project.run_quiet("run-1", day_request()).await.expect_err(
        "a `technique: fold` pin on a cell that only ever admits DeleteInsert (membership- \
         sensitive) must refuse the run loudly, never silently fall back",
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("MaintenanceUnboundedFootprint"),
        "expected the ChoiceRefusal diagnostic family in the error: {message}"
    );
}

/// A `cells[].prefer: fold` SOFT preference names the same non-member
/// family as the hard pin above, but per `resolve_cell_choice`'s contract a
/// `prefer` never refuses — "the cost model may still choose a different
/// admissible technique". Since `fold` is not a member of this cell's
/// resolvable set at all, the preference has no admissible target to steer
/// toward and the cell keeps resolving to its own live default
/// (`"deleteinsert"` — `Technique::DeleteInsert`, this shape's honest
/// membership-sensitive default post-Phase-1, never `"column_scoped_merge"`,
/// which is unreachable for this shape) — the run succeeds, unlike the hard
/// pin above.
#[tokio::test]
async fn prefer_is_soft_and_never_refuses() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(
        &project_dir,
        &db_path,
        "  cells:\n    - columns: [user_name]\n      on: users\n      prefer: fold\n",
    );
    seed_tables(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    project
        .run_quiet("run-1", day_request())
        .await
        .expect("creation run must succeed");

    rename_user_one(&db_path);

    let outcome = project
        .run_quiet("run-2", day_request())
        .await
        .expect("a soft `prefer: fold` must never refuse the run, even naming a non-member family");
    let record = outcome
        .models
        .get("daily_events_enriched")
        .expect("model ran");
    assert_eq!(
        record.strategy, "deleteinsert",
        "an unresolvable soft preference must not perturb the cell's own live default \
         technique — got strategy {:?}",
        record.strategy
    );

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let maintained_name: String = conn
        .query_row(
            "SELECT user_name FROM main.daily_events_enriched WHERE user_id = 1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained user_name");
    assert_eq!(
        maintained_name, "Alicia",
        "the column-scoped MERGE must still pick up the mutated dimension value"
    );
}
