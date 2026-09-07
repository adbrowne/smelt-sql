//! T1 (change-suppressed keyed-fold `MERGE`) vs T2 (staged-candidate conditional DELETE+INSERT) vs the full-refresh oracle, at a fixed processed-input set.

use super::composed_routes::assert_backend_multiset_equal;
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;

// =============================================================================
// Phase C5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
// — T1 (change-suppressed keyed-fold MERGE) vs T2 (staged-candidate
// conditional DELETE+INSERT) vs the full-refresh oracle, over the keyed
// pool's own shape (a `unique_key`-addressed region), at a fixed processed-
// input set `S`. The three techniques must be interchangeable
// (`docs/specs/model_transforms.md` §"Change-suppressed MERGE and the
// staged-candidate conditional DELETE+INSERT" — "the fixed-`S` bit-equality
// obligation"): given the identical seed state and the identical candidate
// delta, all three end states agree.
// =============================================================================

/// Seed three independently-named tables (T1's MERGE target, T2's staged-
/// candidate target, and the full-refresh oracle) with identical state, then
/// drive each to the same fixed `S` via its own technique, asserting all
/// three end states agree as multisets. `run_marker` proves suppression
/// actually happened (not merely that the bits match): a row whose fold
/// result reproduces the stored value keeps its prior marker under both T1
/// and T2, while a changed or brand-new row picks up the new run's marker.
#[tokio::test]
async fn keyed_pool_t1_t2_and_full_refresh_agree_at_fixed_s() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("db.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    for table in ["t1_target", "t2_target", "oracle"] {
        backend
            .execute_sql(&format!(
                "CREATE TABLE main.{table} (device_id BIGINT, event_count BIGINT, run_marker \
                 VARCHAR)"
            ))
            .await
            .expect("create table");
        backend
            .execute_sql(&format!(
                "INSERT INTO main.{table} VALUES (1, 5, 'run1'), (2, 3, 'run1'), (3, 8, 'run1')"
            ))
            .await
            .expect("seed table");
    }

    // Fixed `S`: device 1 gets no new events (unchanged-effect re-run);
    // device 2's delta genuinely changes the combined result; device 3 is
    // absent from this run's delta entirely (out of the touched region);
    // device 4 is brand new.
    let delta_values = "(1, 0, 'run2'), (2, 4, 'run2'), (4, 6, 'run2')";
    let key = vec!["device_id".to_string()];
    let compared_columns = vec!["event_count".to_string()];

    // T1: change-suppressed keyed-fold MERGE.
    let folds = vec![
        (
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        ),
        ("run_marker".to_string(), "delta.run_marker".to_string()),
    ];
    let t1_group = smelt_logical::maintenance::emit::emit_keyed_fold_suppressed(
        "main.t1_target",
        &key,
        &folds,
        &format!("SELECT * FROM (VALUES {delta_values}) AS t(device_id, event_count, run_marker)"),
        None,
        &compared_columns,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&t1_group)
        .await
        .expect("T1 change-suppressed keyed-fold merge must succeed");

    // T2: staged-candidate conditional DELETE+INSERT. Its candidate select
    // must carry the fully-combined row (the same effect the MERGE's fold
    // expression computes), since T2 has no combiner of its own — it
    // re-derives full candidate rows and diffs them against stored state.
    let t2_candidate_select = "SELECT t.device_id, t.event_count + d.delta_count AS event_count, \
                                d.new_marker AS run_marker FROM main.t2_target t JOIN (SELECT * \
                                FROM (VALUES (1, 0, 'run2'), (2, 4, 'run2')) AS \
                                x(device_id, delta_count, new_marker)) AS d ON t.device_id = \
                                d.device_id UNION ALL SELECT 4, 6, 'run2'";
    let t2_group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.t2_target",
        "__smelt_staged_t2_target",
        &key,
        t2_candidate_select,
        &compared_columns,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&t2_group)
        .await
        .expect("T2 staged-candidate conditional write must succeed");

    // Full-refresh oracle: recompute the whole region directly.
    backend
        .execute_sql(
            "UPDATE main.oracle SET event_count = 5, run_marker = 'run1' WHERE device_id = 1",
        )
        .await
        .expect("oracle: device 1 unchanged");
    backend
        .execute_sql(
            "UPDATE main.oracle SET event_count = 7, run_marker = 'run2' WHERE device_id = 2",
        )
        .await
        .expect("oracle: device 2 changed");
    backend
        .execute_sql("INSERT INTO main.oracle VALUES (4, 6, 'run2')")
        .await
        .expect("oracle: device 4 new");

    // All three end states are multiset-equal over the addressed columns.
    assert_backend_multiset_equal(
        &backend,
        "SELECT device_id, event_count FROM main.t1_target",
        "SELECT device_id, event_count FROM main.oracle",
        "T1 (change-suppressed keyed-fold MERGE) vs full-refresh oracle",
    )
    .await
    .expect("T1 must equal the full-refresh oracle at fixed S");
    assert_backend_multiset_equal(
        &backend,
        "SELECT device_id, event_count FROM main.t2_target",
        "SELECT device_id, event_count FROM main.oracle",
        "T2 (staged-candidate conditional DELETE+INSERT) vs full-refresh oracle",
    )
    .await
    .expect("T2 must equal the full-refresh oracle at fixed S");
    assert_backend_multiset_equal(
        &backend,
        "SELECT device_id, event_count FROM main.t1_target",
        "SELECT device_id, event_count FROM main.t2_target",
        "T1 vs T2 (the two conditional-write realisations must be interchangeable)",
    )
    .await
    .expect("T1 and T2 must agree with each other, not just with the oracle");

    // Suppression proof: device 1 (unchanged effect) and device 3 (absent
    // from the delta) must keep their prior run's marker under BOTH
    // conditional techniques — proving the write never happened, not merely
    // that it reproduced the same bits.
    for table in ["t1_target", "t2_target"] {
        let rows = backend
            .execute_sql(&format!(
                "SELECT device_id, run_marker FROM main.{table} ORDER BY device_id"
            ))
            .await
            .expect("read back marker column");
        let batch = &rows[0];
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("device_id is Int64");
        let markers = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        let by_id: std::collections::HashMap<i64, String> = (0..ids.len())
            .map(|i| (ids.value(i), markers.value(i).to_string()))
            .collect();
        assert_eq!(
            by_id.get(&1).map(String::as_str),
            Some("run1"),
            "{table}: device 1's unchanged-effect row must never be written"
        );
        assert_eq!(
            by_id.get(&2).map(String::as_str),
            Some("run2"),
            "{table}: device 2's changed row must be written"
        );
        assert_eq!(
            by_id.get(&3).map(String::as_str),
            Some("run1"),
            "{table}: device 3 (absent from the delta) must never be touched"
        );
        assert_eq!(
            by_id.get(&4).map(String::as_str),
            Some("run2"),
            "{table}: device 4 (brand new) must be inserted with the new run's marker"
        );
    }
}
