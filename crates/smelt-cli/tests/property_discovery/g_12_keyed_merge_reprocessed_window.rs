//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-12` — the `cumulative_aggregate`/`merge_into` MERGE path, the
//! **only live path where a ledger obligation can actually be violated
//! today** and, until this cell, entirely unprobed
//! (`docs/research/20260705-refresh-as-maintenance-plan/09-spec-readiness.md`
//! §3 item 1 — the one empirical gap named as blocking *trust* in the
//! spec's targeted-write sections).
//!
//! Two arms, both through the REAL `execute_project` run path (not
//! backbuild):
//!
//! 1. **Frontier advance (HOLDS hypothesis):** disjoint windows folded in
//!    temporal order equal a full refresh — the keyed fold's happy path.
//! 2. **Reprocessed window (the ledger obligation):** re-running an
//!    already-merged window must never fold the same delta twice
//!    (`01-framework.md` §4: "never fold a delta already reflected in the
//!    state"; `keyed_models.md` specs the `KeyedReprocessedWindow` refusal).
//!    **Hypothesis: VIOLATED today.** `crates/smelt-runtime/src/cumulative.rs`
//!    step 2 is an admitted placeholder — "For now we do *not* check
//!    existence here" — so the run path re-folds and double-counts. This
//!    cell pins the actual behaviour so the spec work starts from empirical
//!    truth, and fails loudly the day the refusal (or a real ledger check)
//!    lands, at which point its second arm flips to asserting the refusal.

use crate::link_c_harness::{base_request, LinkCProject};

fn stage_keyed_project(project_dir: &std::path::Path, db_path: &std::path::Path) {
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Driving source: self-contained VALUES table model with timeseries
    // frontmatter (same fixture design as backbuild_cumulative_e2e.rs) so
    // the cumulative classifier resolves the driving source via
    // `smelt.events`.
    let events_sql = r#"---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT * FROM (
    VALUES
        (DATE '2026-01-01', 1),
        (DATE '2026-01-01', 1),
        (DATE '2026-01-02', 1),
        (DATE '2026-01-02', 2)
) AS t(event_date, device_id)
"#;
    let device_stats_sql = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT
    device_id,
    COUNT(*) AS event_count
FROM smelt.events
GROUP BY device_id
"#;
    std::fs::write(project_dir.join("models/events.sql"), events_sql).unwrap();
    std::fs::write(
        project_dir.join("models/device_stats.sql"),
        device_stats_sql,
    )
    .unwrap();

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn device_count(db_path: &std::path::Path, device_id: i32) -> i64 {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.query_row(
        "SELECT event_count FROM main.device_stats WHERE device_id = ?",
        [device_id],
        |r| r.get(0),
    )
    .expect("read device count")
}

/// Arm 1 — frontier advance: disjoint windows folded in order equal the
/// full refresh (device 1: Jan-1 contributes 2, Jan-2 contributes 1 → 3).
/// Arm 2 — reprocessed window: re-running Jan-1 double-folds today
/// (device 1: 3 → 5), because the run path has no ledger / no
/// `KeyedReprocessedWindow` refusal. The double-count assertion is the
/// probe: it pins the live violation, and its failure is the signal that a
/// real never-fold-twice check landed (flip this arm to assert refusal or
/// unchanged state then).
#[tokio::test]
async fn keyed_merge_frontier_holds_but_reprocessed_window_double_folds() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");
    stage_keyed_project(&project_dir, &db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Arm 1: fold Jan-1 then Jan-2 through the real run path.
    let mut r1 = base_request("dev");
    r1.start = Some("2026-01-01".to_string());
    r1.end = Some("2026-01-02".to_string());
    project.run_quiet("run-1", r1).await.expect("run 1 (Jan-1)");
    assert_eq!(device_count(&db_path, 1), 2, "Jan-1: two device-1 events");

    let mut r2 = base_request("dev");
    r2.start = Some("2026-01-02".to_string());
    r2.end = Some("2026-01-03".to_string());
    project.run_quiet("run-2", r2).await.expect("run 2 (Jan-2)");
    assert_eq!(
        device_count(&db_path, 1),
        3,
        "frontier advance HOLDS: Jan-1 (2) + Jan-2 (1) = full refresh"
    );

    // Arm 2: re-run the already-merged Jan-1 window. The spec'd behaviour
    // (`keyed_models.md` §Reprocessing) is a refusal; the live path has no
    // ledger and re-folds.
    let mut r3 = base_request("dev");
    r3.start = Some("2026-01-01".to_string());
    r3.end = Some("2026-01-02".to_string());
    let rerun = project.run_quiet("run-3", r3).await;

    match rerun {
        Err(err) => {
            // The refusal landed — record it and flip this arm.
            panic!(
                "reprocessed window was REFUSED — the never-fold-twice check has landed; \
                 update cell G-12's second arm to pin the refusal and close the ledger \
                 entry. Error: {err:#}"
            );
        }
        Ok(_) => {
            let count = device_count(&db_path, 1);
            assert_eq!(
                count, 5,
                "CONFIRMED live ledger violation (pinned): re-folding the already-merged \
                 Jan-1 window double-counts device 1 (3 + 2 = 5). If this assertion fails \
                 with count == 3, an idempotence/ledger check landed silently — update \
                 this cell and the ledger."
            );
        }
    }
}
