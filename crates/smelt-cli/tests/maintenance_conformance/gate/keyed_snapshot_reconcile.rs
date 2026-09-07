//! End-to-end snapshot-reconcile run-shape cases: the plain-overwrite settle/delete-departed drive, and the loud refusal of an event-time window on a snapshot-reconcile model.

use super::keyed_support::{
    delete_row_keyed_snapshot, insert_row_keyed_snapshot, stage_keyed_recipe,
    update_row_keyed_snapshot,
};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{KeyedCombiner, KeyedRecipe};

/// Phase 3 (`docs/plans/20260809-keyed-frontier.md`), delete leg extended
/// phase 32b (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 32b-plan.md`): drive the ONE family the admission matrix actually admits
/// under snapshot-reconcile (plain-overwrite, `ANY_VALUE`) end to end
/// through the real `execute_project` pipeline and the snapshot-reconcile
/// executor: seed rows, run (creation), mutate/delete/insert source rows,
/// run again (reconcile), and assert the maintained table equals the
/// current snapshot's own aggregation exactly — the default `retain_
/// departed` point deletes a key absent from the incoming scan
/// (`incremental_shapes.md` §"Departed keys and deletion"), so no retained-
/// departed-keys adjustment survives into the oracle comparison.
#[tokio::test]
async fn snapshot_reconcile_plain_overwrite_settles_and_deletes_departed_keys() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::PlainOverwrite);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage snapshot-reconcile recipe");

    // Seed three keys.
    insert_row_keyed_snapshot(&project, &recipe, 1, 100).expect("seed id=1");
    insert_row_keyed_snapshot(&project, &recipe, 2, 200).expect("seed id=2");
    insert_row_keyed_snapshot(&project, &recipe, 3, 300).expect("seed id=3");

    // First run: no event-time window (snapshot-reconcile has no clock) —
    // creates the target table.
    project
        .run_quiet("snapshot-reconcile-1", base_request("dev"))
        .await
        .expect("first (creation) run must succeed");

    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let full_scan_oracle_sql = format!(
        "SELECT {key}, ANY_VALUE({attr}) AS current_val FROM main.sources_{name} GROUP BY {key}",
        key = recipe.source.key_column,
        attr = recipe.source.payload_column,
        name = recipe.source.name,
    );
    {
        let backend = project.backend().await.expect("backend");
        let equal =
            multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &full_scan_oracle_sql)
                .await
                .expect("comparison must run");
        assert!(equal, "creation run must equal the full-scan oracle");
    }

    // Mutate: update id=1's value, delete id=2 (genuine departure), insert
    // a fresh id=4.
    update_row_keyed_snapshot(&project, &recipe, 1, 999).expect("update id=1");
    delete_row_keyed_snapshot(&project, &recipe, 2).expect("delete id=2");
    insert_row_keyed_snapshot(&project, &recipe, 4, 400).expect("insert id=4");

    // Second run: still no window — reconciles via the whole-source MERGE +
    // the default point's anti-join delete leg.
    project
        .run_quiet("snapshot-reconcile-2", base_request("dev"))
        .await
        .expect("second (reconcile) run must succeed");

    {
        let backend = project.backend().await.expect("backend");
        let equal =
            multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &full_scan_oracle_sql)
                .await
                .expect("comparison must run");
        assert!(
            equal,
            "reconcile run must equal the full-scan oracle exactly — the default point deletes \
             departed keys"
        );
    }

    // Explicit assertion, not just the multiset comparison: the departed
    // key (id=2) is gone.
    let conn = project.connect().expect("connect");
    let departed_count: i64 = conn
        .query_row(
            &format!(
                "SELECT count(*) FROM main.{} WHERE id = 2",
                recipe.model_name
            ),
            [],
            |row| row.get(0),
        )
        .expect("count query");
    assert_eq!(
        departed_count, 0,
        "the departed key must be DELETED under the default retain_departed point"
    );
}

/// Phase 3: `--event-time-start`/`--event-time-end` on a snapshot-reconcile
/// model (no clocked driving source) is rejected loudly, naming the run
/// shape — rather than silently ignored or dispatched through the
/// window-forward executor.
#[tokio::test]
async fn snapshot_reconcile_rejects_event_time_window() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::PlainOverwrite);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage snapshot-reconcile recipe");
    insert_row_keyed_snapshot(&project, &recipe, 1, 100).expect("seed id=1");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());

    let err = project
        .run_quiet("snapshot-reconcile-windowed", request)
        .await
        .expect_err("an event-time window on a snapshot-reconcile model must be refused");
    let message = format!("{err:#}");
    assert!(
        message.contains("snapshot-reconcile"),
        "expected the refusal to name the snapshot-reconcile run shape: {message}"
    );
}
