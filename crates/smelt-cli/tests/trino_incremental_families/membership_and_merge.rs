use super::common;
use super::{explain_json, run_smelt};
use common::{
    drop_trino_schema, fetch_trino_rows, trino_backend, trino_env, trino_schema, trino_target_block,
};

use std::fs;

/// Stage a `grain: key` membership-sensitive project
/// (`docs/outcomes/20260913-trino-incremental/phases/05-plan.md` test 8):
/// an append-only, clocked `raw.transactions` fact folded per `user_id` via
/// `COUNT`, inner-joined purely for row admission to `raw.users` (an
/// unclocked `mutable_snapshot` dimension) — the same shape
/// `crates/smelt-runtime/tests/technique_lowering/keyed_membership_recompute_e2e.rs`
/// proves against DuckDB directly. A departed `raw.users` row makes the
/// join drop that user's whole group, which only the staged-candidate
/// conditional recompute's departed-row delete leg (not a column-scoped
/// `MERGE`) can repair — `Technique::DeleteInsert`, dispatched by
/// `execute_staged_membership_recompute` over T3's `TargetSchema`-resident,
/// non-atomic staged relation on Trino, this phase's own subject.
fn stage_membership_project(tmp: &tempfile::TempDir, schema: &str) -> std::path::PathBuf {
    let root = tmp.path().join(format!("membership_trino_{schema}"));
    fs::create_dir_all(root.join("models/sources/raw")).unwrap();

    let yml = format!(
        "name: membership_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/raw/transactions.yml"),
        "description: Transaction events.\n\
         columns:\n\
         \x20\x20- name: transaction_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: user_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: transaction_date\n\
         \x20\x20\x20\x20type: DATE\n\
         timeseries:\n\
         \x20\x20event_time_column: transaction_date\n\
         \x20\x20partition_column: transaction_date\n\
         \x20\x20granularity: day\n\
         mutation_profile:\n\
         \x20\x20kind: append_only\n",
    )
    .unwrap();
    fs::write(
        root.join("models/sources/raw/users.yml"),
        "description: Raw user dimension.\n\
         columns:\n\
         \x20\x20- name: user_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: tier\n\
         \x20\x20\x20\x20type: VARCHAR\n\
         mutation_profile:\n\
         \x20\x20kind: mutable_snapshot\n\
         unique_key: [user_id]\n",
    )
    .unwrap();

    fs::write(
        root.join("models/user_lifetime_status.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: user_id\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20raw.users:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         \x20\x20\x20\x20\x20\x20raw.transactions:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT t.user_id AS user_id, COUNT(t.transaction_id) AS event_count \
         FROM smelt.sources.raw.transactions t \
         JOIN smelt.sources.raw.users u ON t.user_id = u.user_id \
         GROUP BY t.user_id\n",
    )
    .unwrap();

    root
}

/// Test 8 (`docs/outcomes/20260913-trino-incremental/phases/05-plan.md`): a
/// membership-sensitive model through real `execute_project` (the CLI's
/// `smelt run`), oracle-equal to `--full-refresh`. `user_id` 2's row is
/// deleted from `raw.users` between the two windows — a genuine departure
/// the join can only repair by removing user 2's group entirely, which the
/// staged-candidate conditional recompute's departed-row delete leg
/// (`emit_staged_candidate_conditional_recompute`) does, over Trino's
/// `TargetSchema`-resident staged relation and its `WHERE EXISTS`
/// changed-row delete (`USING` is not Trino grammar).
#[test]
fn membership_recompute_conditional_write_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             membership_recompute_conditional_write_matches_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("membership");
    let oracle_schema = trino_schema("membership_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_membership_project(&tmp, &schema);

    // Seed the two source tables directly (mirrors `seed_trino_events`).
    {
        use smelt_backend::Backend;
        let backend = trino_backend(&schema);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            backend
                .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
                .await
                .expect("create schema");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_raw_transactions (transaction_id INTEGER, \
                     user_id INTEGER, transaction_date DATE)"
                ))
                .await
                .expect("create transactions source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_raw_transactions VALUES \
                     (1, 1, DATE '2026-01-01'), (2, 2, DATE '2026-01-01')"
                ))
                .await
                .expect("seed transactions");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_raw_users (user_id INTEGER, tier VARCHAR)"
                ))
                .await
                .expect("create users source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_raw_users VALUES (1, 'gold'), (2, 'silver')"
                ))
                .await
                .expect("seed users");
        });
    }

    // First window: creation.
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
        "first window (creation) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    // Second window: user 1 gets a new transaction, user 2 DEPARTS the
    // dimension entirely.
    {
        use smelt_backend::Backend;
        let backend = trino_backend(&schema);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_raw_transactions VALUES (3, 1, DATE \
                     '2026-01-02')"
                ))
                .await
                .expect("insert additional transaction");
            backend
                .execute_sql(&format!(
                    "DELETE FROM {schema}.sources_raw_users WHERE user_id = 2"
                ))
                .await
                .expect("user 2 departs the dimension");
        });
    }

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
        "second window (membership recompute) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    // Full-refresh oracle: user 2 never in the dimension, all transactions
    // present (user 2's transaction is unreachable through the inner join
    // regardless of the dimension row's departure timing).
    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_membership_project(&oracle_tmp, &oracle_schema);
    {
        use smelt_backend::Backend;
        let backend = trino_backend(&oracle_schema);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            backend
                .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {oracle_schema}"))
                .await
                .expect("create oracle schema");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {oracle_schema}.sources_raw_transactions (transaction_id \
                     INTEGER, user_id INTEGER, transaction_date DATE)"
                ))
                .await
                .expect("create oracle transactions source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {oracle_schema}.sources_raw_transactions VALUES \
                     (1, 1, DATE '2026-01-01'), (2, 2, DATE '2026-01-01'), \
                     (3, 1, DATE '2026-01-02')"
                ))
                .await
                .expect("seed oracle transactions");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {oracle_schema}.sources_raw_users (user_id INTEGER, tier \
                     VARCHAR)"
                ))
                .await
                .expect("create oracle users source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {oracle_schema}.sources_raw_users VALUES (1, 'gold')"
                ))
                .await
                .expect("seed oracle users (user 2 never present)");
        });
    }
    let oracle_run = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run.status.success(),
        "full-refresh oracle run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run.stdout),
        String::from_utf8_lossy(&oracle_run.stderr),
    );

    let mut actual = fetch_trino_rows(&schema, "user_lifetime_status");
    actual.sort();
    let mut expected = fetch_trino_rows(&oracle_schema, "user_lifetime_status");
    expected.sort();
    assert_eq!(
        actual, expected,
        "membership-sensitive staged-candidate recompute must be multiset-equal to a \
         full-refresh rebuild: {actual:?} vs {expected:?}"
    );
    assert_eq!(
        actual.len(),
        1,
        "user 2 must be gone entirely after departing the dimension: {actual:?}"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}

/// Stage a `grain: partition` column-scoped-merge project
/// (`docs/outcomes/20260913-trino-incremental/phases/05-plan.md` test 9):
/// an append-only fact `LEFT JOIN`-enriched by a `mutable_snapshot`
/// dimension's payload column — the same shape
/// `crates/smelt-cli/tests/trino_explain_downgrade.rs`'s `cs_merged` fixture
/// uses offline. `Technique::ColumnScopedMerge` needs a transactional merge
/// ledger no backend realising no correctness structure has (Trino, exactly
/// like Spark) — `resolve_availability` downgrades the cell to its
/// recompute-family equivalent and records `MaintenanceStateDowngraded`.
fn stage_column_scoped_merge_project(tmp: &tempfile::TempDir, schema: &str) -> std::path::PathBuf {
    let root = tmp.path().join(format!("cs_merge_trino_{schema}"));
    fs::create_dir_all(root.join("models/sources")).unwrap();

    let yml = format!(
        "name: cs_merge_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/sources/cs_fact.yml"),
        "description: column-scoped-merge fact source.\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .unwrap();
    fs::write(
        root.join("models/sources/cs_dim.yml"),
        "description: column-scoped-merge mutable dimension.\n\
         mutation_profile: mutable_snapshot\nunique_key: [id]\n\
         columns:\n\
         - name: id\n  type: INTEGER\n\
         - name: attr\n  type: INTEGER\n",
    )
    .unwrap();
    fs::write(
        root.join("models/cs_merged.sql"),
        "---\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         refresh: incremental\ngrain: partition\n\
         maintenance:\n  scan_bounds:\n    per_source:\n      cs_dim:\n        \
         allow_full_scan: true\n---\n\
         SELECT f.d AS d, f.id AS id, f.val AS val, dim.attr AS attr\n\
         FROM smelt.sources.cs_fact f LEFT JOIN smelt.sources.cs_dim dim ON f.id = dim.id\n",
    )
    .unwrap();

    root
}

/// Test 9 (`docs/outcomes/20260913-trino-incremental/phases/05-plan.md`): a
/// `ColumnScopedMerge`-electing model on Trino downgrades
/// (`smelt explain --json` shows `original: ColumnScopedMerge`, a `missing`
/// reason naming the absent ledger), and the run's target still equals a
/// `--full-refresh` rebuild after the dimension mutates.
#[test]
fn column_scoped_merge_cell_downgrades_and_matches_full_refresh_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping \
             column_scoped_merge_cell_downgrades_and_matches_full_refresh_on_trino"
        );
        return;
    };
    let schema = trino_schema("cs_merge");
    let oracle_schema = trino_schema("cs_merge_oracle");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_column_scoped_merge_project(&tmp, &schema);

    let json = explain_json(&root, "cs_merged");
    let cells = json["cells"].as_array().expect("cells array");
    let downgraded = cells
        .iter()
        .find(|c| c.get("state_downgrade").is_some())
        .unwrap_or_else(|| panic!("expected a cell carrying state_downgrade: {json}"));
    assert_eq!(
        downgraded["state_downgrade"]["original"],
        "ColumnScopedMerge"
    );
    assert!(
        downgraded["state_downgrade"]["missing"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "state_downgrade must name the missing structure: {json}"
    );

    {
        use smelt_backend::Backend;
        let backend = trino_backend(&schema);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            backend
                .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
                .await
                .expect("create schema");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_cs_fact (d DATE, id INTEGER, val INTEGER)"
                ))
                .await
                .expect("create fact source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_cs_fact VALUES (DATE '2026-01-01', 1, 10), \
                     (DATE '2026-01-01', 2, 20)"
                ))
                .await
                .expect("seed fact rows");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {schema}.sources_cs_dim (id INTEGER, attr INTEGER)"
                ))
                .await
                .expect("create dimension source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {schema}.sources_cs_dim VALUES (1, 100), (2, 200)"
                ))
                .await
                .expect("seed dimension rows");
        });
    }

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
        "first window (creation) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    // Mutate the dimension: id 1's attr changes.
    {
        use smelt_backend::Backend;
        let backend = trino_backend(&schema);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            backend
                .execute_sql(&format!(
                    "UPDATE {schema}.sources_cs_dim SET attr = 999 WHERE id = 1"
                ))
                .await
                .expect("mutate dimension");
        });
    }

    let second = run_smelt(
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
        second.status.success(),
        "second window (downgraded rebuild) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = stage_column_scoped_merge_project(&oracle_tmp, &oracle_schema);
    {
        use smelt_backend::Backend;
        let backend = trino_backend(&oracle_schema);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            backend
                .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {oracle_schema}"))
                .await
                .expect("create oracle schema");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {oracle_schema}.sources_cs_fact (d DATE, id INTEGER, val \
                     INTEGER)"
                ))
                .await
                .expect("create oracle fact source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {oracle_schema}.sources_cs_fact VALUES (DATE '2026-01-01', 1, \
                     10), (DATE '2026-01-01', 2, 20)"
                ))
                .await
                .expect("seed oracle fact rows");
            backend
                .execute_sql(&format!(
                    "CREATE TABLE {oracle_schema}.sources_cs_dim (id INTEGER, attr INTEGER)"
                ))
                .await
                .expect("create oracle dimension source table");
            backend
                .execute_sql(&format!(
                    "INSERT INTO {oracle_schema}.sources_cs_dim VALUES (1, 999), (2, 200)"
                ))
                .await
                .expect("seed oracle dimension rows (post-mutation)");
        });
    }
    let oracle_run = run_smelt(&oracle_root, &["--target", "trino", "--full-refresh"]);
    assert!(
        oracle_run.status.success(),
        "full-refresh oracle run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_run.stdout),
        String::from_utf8_lossy(&oracle_run.stderr),
    );

    let mut actual = fetch_trino_rows(&schema, "cs_merged");
    actual.sort();
    let mut expected = fetch_trino_rows(&oracle_schema, "cs_merged");
    expected.sort();
    assert_eq!(
        actual, expected,
        "the downgraded column-scoped-merge cell must be multiset-equal to a full-refresh \
         rebuild: {actual:?} vs {expected:?}"
    );

    drop_trino_schema(&schema);
    drop_trino_schema(&oracle_schema);
}
