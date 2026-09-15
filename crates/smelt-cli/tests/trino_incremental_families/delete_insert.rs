use super::common;
use super::{run_smelt, stage_int_partition_project};
use common::{
    drop_trino_schema, fetch_trino_rows, trino_backend, trino_env, trino_schema, trino_target_block,
};

use std::fs;

/// Runs one raw SQL statement against `schema` on the live Trino tier —
/// used below to mutate a declared source's underlying table directly
/// between `smelt run` invocations, the same bridging pattern as
/// `seed_trino_events`/`insert_trino_events`.
fn run_trino_sql(schema: &str, sql: &str) {
    use smelt_backend::Backend;
    let backend = trino_backend(schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        backend
            .execute_sql(sql)
            .await
            .unwrap_or_else(|e| panic!("run_trino_sql({sql:?}) failed: {e}"));
    });
}

/// A `grain: partition`/`DeleteInsert` mart over a **declared external**
/// source (`models/sources/events.yml`) rather than a first-class inline
/// model: unlike `stage_int_partition_project`'s `seed_events` (a plain
/// `materialization: table` model smelt itself rebuilds from its static
/// `VALUES` body on every run, silently undoing any raw mutation between
/// `smelt run` invocations), a declared source's backing table is never
/// written by smelt — only read — so a raw `DELETE`+`INSERT` against it
/// between runs stays in place for the next run to observe.
fn stage_delete_insert_mutation_project(
    tmp: &tempfile::TempDir,
    schema: &str,
) -> std::path::PathBuf {
    let root = tmp.path().join("delete_insert_mutation_trino");
    fs::create_dir_all(root.join("models/sources")).unwrap();

    let yml = format!(
        "name: delete_insert_mutation_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/events.yml"),
        "description: Batched events source.\n\
         columns:\n\
         \x20\x20- name: id\n\
         \x20\x20\x20\x20type: BIGINT\n\
         \x20\x20- name: batch_id\n\
         \x20\x20\x20\x20type: BIGINT\n\
         \x20\x20- name: event_ts\n\
         \x20\x20\x20\x20type: TIMESTAMP\n\
         timeseries:\n\
         \x20\x20event_time_column: event_ts\n\
         \x20\x20partition_column: batch_id\n\
         \x20\x20granularity: day\n\
         mutation_profile:\n\
         \x20\x20kind: append_only\n",
    )
    .unwrap();

    fs::write(
        root.join("models/int_partition_mart.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: batch_id\n  granularity: day\n\
         maintenance:\n\
         \x20 scan_bounds:\n    per_source:\n      events:\n        allow_full_scan: true\n\
         ---\n\
         SELECT id, batch_id, event_ts FROM smelt.sources.events\n",
    )
    .unwrap();

    root
}

/// Seeds the declared `events` source's backing table directly on the live
/// tier — smelt never writes this table itself, so this is the only writer.
fn seed_delete_insert_source(schema: &str, rows: &[(i64, i64, &str)]) {
    let values: Vec<String> = rows
        .iter()
        .map(|(id, batch_id, ts)| {
            format!("(CAST({id} AS BIGINT), CAST({batch_id} AS BIGINT), TIMESTAMP '{ts}')")
        })
        .collect();
    run_trino_sql(schema, &format!("CREATE SCHEMA IF NOT EXISTS {schema}"));
    run_trino_sql(
        schema,
        &format!(
            "CREATE TABLE {schema}.sources_events (id BIGINT, batch_id BIGINT, event_ts TIMESTAMP)"
        ),
    );
    run_trino_sql(
        schema,
        &format!(
            "INSERT INTO {schema}.sources_events VALUES {}",
            values.join(", ")
        ),
    );
}

/// Phase 4 (`docs/outcomes/20260913-trino-incremental/phases/04-plan.md`),
/// criterion 4 — the emulated delete-and-insert window's `DELETE` covers
/// exactly the range the `INSERT` writes: no wider (data loss) and no
/// narrower (duplicates). The declared `events` source is seeded across
/// three batches (`batch_id` 1, 2, 3); run 1 materialises all three via
/// window `[1, 4)`, the source row driving batch 2 (`id = 3`) is then
/// mutated directly on the live tier, and run 2 re-runs only window
/// `[2, 3)`. Batch 1 (`id` 1, 2) and batch 3 (`id` 4) must be byte-identical
/// to before (proving the `DELETE` did not widen into either neighbour),
/// and batch 2's row must reflect the mutation exactly once (proving the
/// `INSERT` did not narrow and silently drop it).
#[test]
fn delete_insert_window_replaces_only_its_own_rows_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             delete_insert_window_replaces_only_its_own_rows_on_trino"
        );
        return;
    };
    let schema = trino_schema("delete_insert_window");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_delete_insert_mutation_project(&tmp, &schema);
    seed_delete_insert_source(
        &schema,
        &[
            (1, 1, "2026-01-01 00:00:00"),
            (2, 1, "2026-01-01 06:00:00"),
            (3, 2, "2026-01-02 00:00:00"),
            (4, 3, "2026-01-03 00:00:00"),
        ],
    );

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "4",
        ],
    );
    assert!(
        first.status.success(),
        "first run (materialise all batches) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let before = fetch_trino_rows(&schema, "int_partition_mart");
    let batch1_before: Vec<_> = before.iter().filter(|r| r[1] == "1").cloned().collect();
    let batch3_before: Vec<_> = before.iter().filter(|r| r[1] == "3").cloned().collect();
    assert_eq!(batch1_before.len(), 2, "batch 1 seeds two rows: {before:?}");
    assert_eq!(batch3_before.len(), 1, "batch 3 seeds one row: {before:?}");

    // Mutate only the source row driving batch 2's single row (`id = 3`):
    // move its event timestamp within the same day, so the mutation is
    // visible in the mart without changing which batch it belongs to.
    run_trino_sql(
        &schema,
        &format!("DELETE FROM {schema}.sources_events WHERE id = CAST(3 AS BIGINT)"),
    );
    run_trino_sql(
        &schema,
        &format!(
            "INSERT INTO {schema}.sources_events VALUES \
             (CAST(3 AS BIGINT), CAST(2 AS BIGINT), TIMESTAMP '2026-01-02 12:00:00')"
        ),
    );

    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2",
            "--event-time-end",
            "3",
        ],
    );
    assert!(
        second.status.success(),
        "second run (narrow window over batch 2 only) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let after = fetch_trino_rows(&schema, "int_partition_mart");
    let batch1_after: Vec<_> = after.iter().filter(|r| r[1] == "1").cloned().collect();
    let batch2_after: Vec<_> = after.iter().filter(|r| r[1] == "2").cloned().collect();
    let batch3_after: Vec<_> = after.iter().filter(|r| r[1] == "3").cloned().collect();

    assert_eq!(
        batch1_after, batch1_before,
        "batch 1 must be untouched — the narrower [2, 3) DELETE must not widen into it"
    );
    assert_eq!(
        batch3_after, batch3_before,
        "batch 3 must be untouched — the narrower [2, 3) DELETE must not widen into it either"
    );
    assert_eq!(
        batch2_after.len(),
        1,
        "batch 2 must have exactly one row after the re-run — no duplication: {after:?}"
    );
    assert!(
        batch2_after[0][2].contains("12:00:00"),
        "batch 2's row must reflect the mutated event_ts exactly once: {after:?}"
    );

    drop_trino_schema(&schema);
}

/// Phase 4, criterion 4 — applied out of order. `int_partition_mart` is run
/// with its later window `[3, 5)` first (the first-run `CREATE TABLE … AS`
/// arm, materialising only batch 3), then its earlier window `[1, 3)`
/// (materialising batches 1 and 2 through the `DELETE`+`INSERT` recompute
/// arm). The result must be multiset-equal to a `--full-refresh` oracle of
/// the identical project in a separate schema — exact coverage holds
/// regardless of which order the windows arrive in.
#[test]
fn delete_insert_windows_applied_out_of_order_match_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             delete_insert_windows_applied_out_of_order_match_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("delete_insert_out_of_order");
    let oracle_schema = trino_schema("delete_insert_out_of_order_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let later_window = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "3",
            "--event-time-end",
            "5",
        ],
    );
    assert!(
        later_window.status.success(),
        "later window [3, 5) (applied first) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&later_window.stdout),
        String::from_utf8_lossy(&later_window.stderr),
    );

    let earlier_window = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "3",
        ],
    );
    assert!(
        earlier_window.status.success(),
        "earlier window [1, 3) (applied second) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&earlier_window.stdout),
        String::from_utf8_lossy(&earlier_window.stderr),
    );

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_int_partition_project(&oracle_tmp, &oracle_schema);
    let oracle_run = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run.status.success(),
        "full-refresh oracle run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run.stdout),
        String::from_utf8_lossy(&oracle_run.stderr),
    );

    let mut actual = fetch_trino_rows(&schema, "int_partition_mart");
    actual.sort();
    let mut expected = fetch_trino_rows(&oracle_schema, "int_partition_mart");
    expected.sort();
    assert_eq!(
        actual, expected,
        "windows applied out of order must still match a full-refresh oracle"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Phase 4, criterion 4 — repeated application is idempotent. `int_partition_
/// mart`'s window `[1, 3)` is applied twice with no source change in
/// between; the second application's `DELETE`+`INSERT` must reproduce
/// exactly the same rows as the first, with no duplicate rows and no loss.
#[test]
fn delete_insert_repeated_window_is_idempotent_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             delete_insert_repeated_window_is_idempotent_on_trino"
        );
        return;
    };
    let schema = trino_schema("delete_insert_repeated");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let args = [
        "--target",
        "trino",
        "--event-time-start",
        "1",
        "--event-time-end",
        "3",
    ];

    let first = run_smelt(&root, &args);
    assert!(
        first.status.success(),
        "first application of [1, 3) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );
    let mut once = fetch_trino_rows(&schema, "int_partition_mart");
    once.sort();

    let second = run_smelt(&root, &args);
    assert!(
        second.status.success(),
        "repeated application of [1, 3) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );
    let mut twice = fetch_trino_rows(&schema, "int_partition_mart");
    twice.sort();

    assert_eq!(
        once, twice,
        "re-running the identical window must be idempotent: no duplicate rows, no loss"
    );
    assert_eq!(
        twice.len(),
        3,
        "window [1, 3) covers batches 1 and 2: two rows plus one row: {twice:?}"
    );

    drop_trino_schema(&schema);
}
