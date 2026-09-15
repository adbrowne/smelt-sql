use super::common;
use super::{explain_json, run_smelt};
use common::{
    drop_trino_schema, fetch_trino_rows, trino_backend, trino_env, trino_schema, trino_target_block,
};

use std::fs;

// =============================================================================
// Phase 3h: the whole-row `MERGE` upsert (keyed-fold) family, live on Trino
// (gaps 4 and 5 closed in phases 3f/3g — see this file's module doc).
// =============================================================================

/// A `refresh: incremental` / `grain: key` model over a clocked, append-only
/// `events` source (`event_date` a real `DATE` `timeseries.partition_column`).
/// `combiner` parameterises the fold: `MIN`/`MAX` grades `Grade::Idempotent`
/// (stays on the `MERGE` route); `SUM` grades `Grade::Additive` (downgrades
/// on Trino's fully-degraded dialect, per gap 5's fix). `allow_full_scan` is
/// declared unconditionally — the additive downgrade's whole-target rebuild
/// reads the source unwindowed regardless of which combiner this particular
/// project uses.
fn stage_keyed_fold_project(
    tmp: &tempfile::TempDir,
    schema: &str,
    combiner: &str,
) -> std::path::PathBuf {
    let root = tmp.path().join(format!("keyed_fold_trino_{schema}"));
    fs::create_dir_all(root.join("models/sources")).unwrap();

    let yml = format!(
        "name: keyed_fold_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/events.yml"),
        "description: Clocked per-device events.\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: event_date\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         mutation_profile:\n\
         \x20\x20kind: append_only\n",
    )
    .unwrap();

    fs::write(
        root.join("models/device_agg.sql"),
        format!(
            "---\n\
             materialization: table\n\
             refresh: incremental\n\
             grain: key\n\
             maintenance:\n\
             \x20\x20scan_bounds:\n\
             \x20\x20\x20\x20per_source:\n\
             \x20\x20\x20\x20\x20\x20events:\n\
             \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
             ---\n\
             SELECT device_id, {combiner}(amount) AS agg_amount \
             FROM smelt.sources.events GROUP BY 1\n"
        ),
    )
    .unwrap();

    root
}

/// Seed `schema.sources_events` (create schema, create table, insert `rows`)
/// on the live Trino tier.
fn seed_trino_events(schema: &str, rows: &[(i64, &str, f64)]) {
    use smelt_backend::Backend;
    let backend = trino_backend(schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let values: Vec<String> = rows
        .iter()
        .map(|(id, date, amount)| format!("({id}, DATE '{date}', {amount})"))
        .collect();
    rt.block_on(async {
        backend
            .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .await
            .expect("create schema");
        backend
            .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_events (device_id INTEGER, event_date DATE, \
                 amount DOUBLE)"
            ))
            .await
            .expect("create source table");
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_events VALUES {}",
                values.join(", ")
            ))
            .await
            .expect("seed source table");
    });
}

/// Insert additional rows into an already-seeded `schema.sources_events`.
fn insert_trino_events(schema: &str, rows: &[(i64, &str, f64)]) {
    use smelt_backend::Backend;
    let backend = trino_backend(schema);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let values: Vec<String> = rows
        .iter()
        .map(|(id, date, amount)| format!("({id}, DATE '{date}', {amount})"))
        .collect();
    rt.block_on(async {
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_events VALUES {}",
                values.join(", ")
            ))
            .await
            .expect("insert additional rows");
    });
}

/// Test 1 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// the idempotent (`MIN`-combiner) whole-row `MERGE` upsert family, run over
/// two disjoint windows where the second both revises an existing key
/// (device 1's amount drops from 50 to 5, a matched-arm update under `MIN`)
/// and introduces a new key (device 3, a not-matched-arm insert), asserted
/// multiset-equal to a `--full-refresh` rebuild of the same, fully-seeded
/// source into an independent schema.
#[test]
fn whole_row_merge_upsert_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping whole_row_merge_upsert_matches_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("keyed_merge");
    let oracle_schema = trino_schema("keyed_merge_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_keyed_fold_project(&tmp, &schema, "MIN");
    seed_trino_events(&schema, &[(1, "2026-01-01", 50.0), (2, "2026-01-01", 20.0)]);

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-02",
        ],
    );
    assert!(
        first.status.success(),
        "first window (create) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    insert_trino_events(&schema, &[(1, "2026-01-02", 5.0), (3, "2026-01-02", 30.0)]);

    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-02",
            "--event-time-end",
            "2026-01-03",
        ],
    );
    assert!(
        second.status.success(),
        "second window (merge) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_keyed_fold_project(&oracle_tmp, &oracle_schema, "MIN");
    seed_trino_events(
        &oracle_schema,
        &[
            (1, "2026-01-01", 50.0),
            (2, "2026-01-01", 20.0),
            (1, "2026-01-02", 5.0),
            (3, "2026-01-02", 30.0),
        ],
    );
    let oracle_run = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run.status.success(),
        "full-refresh oracle run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run.stdout),
        String::from_utf8_lossy(&oracle_run.stderr),
    );

    let mut actual = fetch_trino_rows(&schema, "device_agg");
    actual.sort();
    let mut expected = fetch_trino_rows(&oracle_schema, "device_agg");
    expected.sort();
    assert_eq!(
        actual, expected,
        "two disjoint windowed keyed-fold merge runs must be multiset-equal to a full-refresh \
         rebuild"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Test 2 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// the same idempotent shape's `smelt explain --json` plan report shows the
/// cell resolving to `KeyedFold` with no `state_downgrade` — proving the
/// oracle-equality test above cannot pass vacuously via the degraded
/// whole-target rebuild path. Offline: `smelt explain` never opens a live
/// connection.
#[test]
fn whole_row_merge_upsert_writes_through_the_merge_route_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             whole_row_merge_upsert_writes_through_the_merge_route_on_trino"
        );
        return;
    };
    let schema = trino_schema("keyed_merge_explain");
    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_keyed_fold_project(&tmp, &schema, "MIN");

    let json = explain_json(&root, "device_agg");
    let cells = json["cells"].as_array().expect("cells array");
    let cell = cells
        .iter()
        .find(|c| c["technique"] == "KeyedFold")
        .unwrap_or_else(|| panic!("expected a KeyedFold cell: {json}"));
    assert!(
        cell.get("state_downgrade").is_none(),
        "an idempotent (MIN) combiner's keyed fold must not be downgraded on Trino: {json}"
    );
}

/// Test 3 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// the same fixture with a `SUM` combiner — `Grade::Additive`, no
/// realisable reconciliation ledger on Trino's fully-degraded dialect. The
/// cell must be recorded downgraded (`state_downgrade.original ==
/// "KeyedFold"`, `technique == "PerGroupRecompute"`), and the maintained
/// table must still be multiset-equal to the full-refresh oracle after
/// EACH of two windows — a downgraded cell is asserted oracle-equal, not
/// exempted.
#[test]
fn additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("keyed_fold_downgrade");
    let oracle_schema = trino_schema("keyed_fold_downgrade_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_keyed_fold_project(&tmp, &schema, "SUM");
    seed_trino_events(&schema, &[(1, "2026-01-01", 50.0), (2, "2026-01-01", 20.0)]);

    let first = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-02",
        ],
    );
    assert!(
        first.status.success(),
        "first window (downgraded whole-target rebuild) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let json = explain_json(&root, "device_agg");
    let cells = json["cells"].as_array().expect("cells array");
    let downgraded = cells
        .iter()
        .find(|c| c.get("state_downgrade").is_some())
        .unwrap_or_else(|| panic!("expected a cell carrying state_downgrade: {json}"));
    assert_eq!(downgraded["state_downgrade"]["original"], "KeyedFold");
    assert_eq!(downgraded["technique"], "PerGroupRecompute");

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_keyed_fold_project(&oracle_tmp, &oracle_schema, "SUM");
    seed_trino_events(
        &oracle_schema,
        &[(1, "2026-01-01", 50.0), (2, "2026-01-01", 20.0)],
    );
    let oracle_run_1 = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run_1.status.success(),
        "oracle full-refresh (window 1) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run_1.stdout),
        String::from_utf8_lossy(&oracle_run_1.stderr),
    );

    let mut actual_1 = fetch_trino_rows(&schema, "device_agg");
    actual_1.sort();
    let mut expected_1 = fetch_trino_rows(&oracle_schema, "device_agg");
    expected_1.sort();
    assert_eq!(
        actual_1, expected_1,
        "after window 1, the downgraded cell must already match the full-refresh oracle"
    );

    insert_trino_events(&schema, &[(1, "2026-01-02", 5.0), (3, "2026-01-02", 30.0)]);
    let second = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "2026-01-02",
            "--event-time-end",
            "2026-01-03",
        ],
    );
    assert!(
        second.status.success(),
        "second window (downgraded whole-target rebuild) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    insert_trino_events(
        &oracle_schema,
        &[(1, "2026-01-02", 5.0), (3, "2026-01-02", 30.0)],
    );
    let oracle_run_2 = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run_2.status.success(),
        "oracle full-refresh (window 2) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run_2.stdout),
        String::from_utf8_lossy(&oracle_run_2.stderr),
    );

    let mut actual_2 = fetch_trino_rows(&schema, "device_agg");
    actual_2.sort();
    let mut expected_2 = fetch_trino_rows(&oracle_schema, "device_agg");
    expected_2.sort();
    assert_eq!(
        actual_2, expected_2,
        "after window 2, the downgraded cell must still match the full-refresh oracle"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}
