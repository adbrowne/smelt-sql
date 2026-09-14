//! Characterises Trino's Iceberg `MERGE` support **by execution**: which
//! clause forms the coordinator accepts, and — for the accepted ones —
//! whether the accepted form computes the right rows, not merely parses.
//! `docs/outcomes/20260913-trino-incremental/outcome.md` phase 1: fixes the
//! emitter shape phases 3 and 5 target, and anchors later by-name refusals
//! to measured error text rather than a guess.
//!
//! Gated on `SMELT_TRINO_URL`: unset, every test here skips green. Run with:
//!   bash scripts/trino-up.sh
//!   source scripts/trino-env.sh
//!   cargo test -p smelt-backend-trino --test merge_clause_forms
//!   bash scripts/trino-down.sh

mod common;

use common::{drop_schema, live_env_or_skip};
use smelt_backend::Backend;

/// `WHEN MATCHED THEN UPDATE SET *` — DuckDB/Spark's whole-row shorthand.
/// Measured 2026-09-14: refused. Trino's `MERGE` grammar has no `*` target
/// on the left of `SET`; every column must be named.
#[tokio::test]
async fn merge_whole_row_update_set_star() {
    let Some(env) = live_env_or_skip("merge_whole_row_update_set_star", "merge").await else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n, 'a' AS lbl",
            env.q("t")
        ))
        .await
        .expect("create target table");
    let measured = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 1 AS n, 'b' AS lbl) s ON t.n = s.n \
             WHEN MATCHED THEN UPDATE SET *",
            env.q("t")
        ))
        .await;
    assert!(
        !measured,
        "UPDATE SET * measured accepted — the emitter can use the star shorthand after all"
    );
    drop_schema(&env).await;
}

/// `WHEN NOT MATCHED THEN INSERT *` and `INSERT ROW` — the two whole-row
/// insert shorthands DuckDB/Spark and standard SQL respectively offer.
/// Measured 2026-09-14: both refused; Trino wants an explicit column list
/// (`INSERT (n, lbl) VALUES (s.n, s.lbl)`), which `probe_supports_merge`
/// already exercises and measures accepted.
#[tokio::test]
async fn merge_whole_row_insert_star() {
    let Some(env) = live_env_or_skip("merge_whole_row_insert_star", "merge").await else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n, 'a' AS lbl",
            env.q("t")
        ))
        .await
        .expect("create target table");
    let insert_star = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 2 AS n, 'c' AS lbl) s ON t.n = s.n \
             WHEN NOT MATCHED THEN INSERT *",
            env.q("t")
        ))
        .await;
    assert!(!insert_star, "INSERT * measured accepted");

    let insert_row = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 2 AS n, 'c' AS lbl) s ON t.n = s.n \
             WHEN NOT MATCHED THEN INSERT ROW",
            env.q("t")
        ))
        .await;
    assert!(!insert_row, "INSERT ROW measured accepted");

    let explicit_columns = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 2 AS n, 'c' AS lbl) s ON t.n = s.n \
             WHEN NOT MATCHED THEN INSERT (n, lbl) VALUES (s.n, s.lbl)",
            env.q("t")
        ))
        .await;
    assert!(
        explicit_columns,
        "the explicit column-list INSERT form must remain accepted as the emitter's target"
    );
    drop_schema(&env).await;
}

/// Value leg for the column-by-column fallback: the named-column `UPDATE
/// SET` form, applied over every column the row has, leaves the same table
/// contents a whole-row form would have. Since the whole-row shorthand is
/// refused (measured above), this is the only form the emitter can target,
/// and this test is its correctness proof rather than a comparison against
/// a star form that does not exist on this backend.
#[tokio::test]
async fn merge_named_column_update_set_computes_the_same_rows_as_the_star_form() {
    let Some(env) = live_env_or_skip(
        "merge_named_column_update_set_computes_the_same_rows_as_the_star_form",
        "merge",
    )
    .await
    else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT * FROM (VALUES (1, 'a'), (2, 'a')) AS x(n, lbl)",
            env.q("t")
        ))
        .await
        .expect("create target table");
    env.backend
        .execute_sql(&format!(
            "MERGE INTO {} t USING (SELECT * FROM (VALUES (1, 'b'), (3, 'c')) AS x(n, lbl)) s \
             ON t.n = s.n \
             WHEN MATCHED THEN UPDATE SET n = s.n, lbl = s.lbl \
             WHEN NOT MATCHED THEN INSERT (n, lbl) VALUES (s.n, s.lbl)",
            env.q("t")
        ))
        .await
        .expect("named-column whole-row MERGE must succeed");

    let mut rows: Vec<String> = env
        .select_string_col(&format!(
            "SELECT CAST(n AS VARCHAR) || ':' || lbl FROM {} ORDER BY n",
            env.q("t")
        ))
        .await;
    rows.sort();
    assert_eq!(
        rows,
        vec!["1:b".to_string(), "2:a".to_string(), "3:c".to_string()],
        "matched row 1 updated, unmatched row 2 untouched, new row 3 inserted"
    );
    drop_schema(&env).await;
}

/// `WHEN MATCHED AND <pred> THEN UPDATE ...` — accepted, and the guard
/// actually selects which matched rows update (value leg, not acceptance
/// only).
#[tokio::test]
async fn merge_when_matched_conditional_and_guard() {
    let Some(env) = live_env_or_skip("merge_when_matched_conditional_and_guard", "merge").await
    else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT * FROM (VALUES (1, 'a'), (2, 'a')) AS x(n, lbl)",
            env.q("t")
        ))
        .await
        .expect("create target table");
    let accepted = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT * FROM (VALUES (1, 'b'), (2, 'b')) AS x(n, lbl)) s \
             ON t.n = s.n \
             WHEN MATCHED AND t.n = 1 THEN UPDATE SET lbl = s.lbl",
            env.q("t")
        ))
        .await;
    assert!(accepted, "WHEN MATCHED AND <pred> must be accepted");

    let mut rows = env
        .select_string_col(&format!(
            "SELECT CAST(n AS VARCHAR) || ':' || lbl FROM {} ORDER BY n",
            env.q("t")
        ))
        .await;
    rows.sort();
    assert_eq!(
        rows,
        vec!["1:b".to_string(), "2:a".to_string()],
        "only the guarded row (n = 1) updates; row 2 is left as 'a'"
    );
    drop_schema(&env).await;
}

/// `WHEN MATCHED THEN DELETE` — the delete arm exists, needed by the
/// merge-less conditional-write and delete-and-insert routes later.
#[tokio::test]
async fn merge_when_matched_then_delete() {
    let Some(env) = live_env_or_skip("merge_when_matched_then_delete", "merge").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .expect("create target table");
    let accepted = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 1 AS n) s ON t.n = s.n \
             WHEN MATCHED THEN DELETE",
            env.q("t")
        ))
        .await;
    assert!(accepted, "WHEN MATCHED THEN DELETE must be accepted");

    let remaining = env
        .select_i64_col(&format!("SELECT count(*) FROM {}", env.q("t")))
        .await;
    assert_eq!(remaining, vec![0], "the matched row must be gone");
    drop_schema(&env).await;
}

/// Two ordered `WHEN MATCHED` arms: the resulting row shows first-match-wins
/// ordering, not last-match-wins or both applied.
#[tokio::test]
async fn merge_multiple_when_clauses_are_first_match_wins() {
    let Some(env) =
        live_env_or_skip("merge_multiple_when_clauses_are_first_match_wins", "merge").await
    else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n, 'orig' AS lbl",
            env.q("t")
        ))
        .await
        .expect("create target table");
    env.backend
        .execute_sql(&format!(
            "MERGE INTO {} t USING (SELECT 1 AS n) s ON t.n = s.n \
             WHEN MATCHED AND t.lbl = 'orig' THEN UPDATE SET lbl = 'first' \
             WHEN MATCHED THEN UPDATE SET lbl = 'second'",
            env.q("t")
        ))
        .await
        .expect("multi-arm MERGE must succeed");

    let rows = env
        .select_string_col(&format!("SELECT lbl FROM {}", env.q("t")))
        .await;
    assert_eq!(
        rows,
        vec!["first".to_string()],
        "the first matching arm wins, not the second"
    );
    drop_schema(&env).await;
}

/// `USING (SELECT ... FROM <staged>) s` — the source shape T3's staged
/// relation actually presents, not a `VALUES` list.
#[tokio::test]
async fn merge_source_may_be_a_subquery_over_a_staged_relation() {
    let Some(env) = live_env_or_skip(
        "merge_source_may_be_a_subquery_over_a_staged_relation",
        "merge",
    )
    .await
    else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .expect("create target table");
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n",
            env.q("staged")
        ))
        .await
        .expect("create staged relation");
    let accepted = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT n FROM {}) s ON t.n = s.n \
             WHEN MATCHED THEN UPDATE SET n = s.n",
            env.q("t"),
            env.q("staged")
        ))
        .await;
    assert!(
        accepted,
        "MERGE's USING clause must accept a subquery over a staged relation"
    );
    drop_schema(&env).await;
}

/// `WHEN NOT MATCHED BY SOURCE THEN DELETE` — refused, and the coordinator's
/// message is stable enough to anchor a later by-name refusal.
#[tokio::test]
async fn merge_not_matched_by_source_error_text_is_stable() {
    let Some(env) =
        live_env_or_skip("merge_not_matched_by_source_error_text_is_stable", "merge").await
    else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .expect("create target table");
    let err = env
        .err_text(&format!(
            "MERGE INTO {} t USING (SELECT 999 AS n) s ON t.n = s.n \
             WHEN NOT MATCHED BY SOURCE THEN DELETE",
            env.q("t")
        ))
        .await;
    let err = err.expect("WHEN NOT MATCHED BY SOURCE must be refused, not accepted");
    assert!(
        err.contains("mismatched input 'BY'") && err.contains("Expecting"),
        "error text drifted from the measured substring: {err}"
    );
    drop_schema(&env).await;
}

/// Reads this file's own decision-log entry and asserts every clause form
/// the tests above measure appears there — a measurement cannot land
/// undocumented.
#[test]
fn every_measured_clause_form_is_recorded_in_the_decision_log() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let outcome = std::fs::read_to_string(
        repo_root.join("docs/outcomes/20260913-trino-incremental/outcome.md"),
    )
    .unwrap();
    let log_start = outcome
        .find("## Decision log")
        .expect("outcome.md has no ## Decision log section");
    let log = &outcome[log_start..];
    let entry_start = log
        .find("phase 1")
        .expect("no phase 1 entry in the decision log");
    let entry = &log[entry_start..];

    let measured_forms = [
        "UPDATE SET *",
        "INSERT *",
        "INSERT ROW",
        "WHEN MATCHED AND",
        "WHEN MATCHED THEN DELETE",
        "first-match-wins",
        "subquery",
        "NOT MATCHED BY SOURCE",
    ];
    for form in measured_forms {
        assert!(
            entry.contains(form),
            "decision log's phase 1 entry does not mention measured clause form `{form}`"
        );
    }
}
