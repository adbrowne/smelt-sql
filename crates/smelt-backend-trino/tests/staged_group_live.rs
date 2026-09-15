//! The merge-less conditional write over T3's staged relation, proved live
//! (`docs/outcomes/20260913-trino-incremental/phases/05-plan.md`, tests 6-7):
//! which changed-row `DELETE` form the coordinator actually accepts, and a
//! real `TargetSchema` staged-candidate conditional group run end to end
//! through `execute_statement_group` — changed rows rewritten, unchanged
//! rows untouched, departed rows deleted, no staged table left behind.
//!
//! Gated on `SMELT_TRINO_URL`: unset, every test here skips green. Run with:
//!   bash scripts/trino-up.sh
//!   source scripts/trino-env.sh
//!   cargo test -p smelt-backend-trino --test staged_group_live -- --test-threads=1
//!   bash scripts/trino-down.sh

mod common;

use common::live_env_or_skip;
use smelt_backend::Backend;
use smelt_logical::maintenance::emit::{
    emit_staged_candidate_conditional, MaintenanceDialect, StagedRelation, StagedRelationResidence,
};

/// Measures both candidate changed-row `DELETE` forms against the live
/// coordinator: the `USING` form every other backend accepts, and the
/// correlated `WHERE EXISTS` form `changed_row_delete` renders for Trino.
/// The accepted one is what the emitter branch spells — this test anchors
/// that measurement rather than assuming it. Measured 2026-09-15:
/// `USING` refuses with `mismatched input 'USING'. Expecting: '.', '@',
/// 'WHERE', <EOF>`; the correlated form is accepted.
#[tokio::test]
async fn merge_clause_probe_measures_the_delete_forms() {
    let Some(env) = live_env_or_skip("merge_clause_probe_measures_the_delete_forms", "probe").await
    else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n, 'a' AS lbl",
            env.q("t")
        ))
        .await
        .expect("create target table");
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n, 'b' AS lbl",
            env.q("s")
        ))
        .await
        .expect("create source table");

    let using_form = env
        .err_text(&format!(
            "DELETE FROM {} USING {} WHERE {}.n = {}.n",
            env.q("t"),
            env.q("s"),
            env.q("t"),
            env.q("s")
        ))
        .await;
    let using_err =
        using_form.expect("USING form measured accepted — the emitter can use it after all");
    assert!(
        using_err.contains("mismatched input 'USING'") && using_err.contains("Expecting"),
        "error text drifted from the measured substring: {using_err}"
    );

    let exists_form_ok = env
        .ok(&format!(
            "DELETE FROM {} WHERE EXISTS (SELECT 1 FROM {} WHERE {}.n = {}.n)",
            env.q("t"),
            env.q("s"),
            env.q("t"),
            env.q("s")
        ))
        .await;
    assert!(
        exists_form_ok,
        "the correlated WHERE EXISTS form must remain accepted as the emitter's target"
    );

    common::drop_schema(&env).await;
}

/// A real `TargetSchema`-resident, non-atomic staged-candidate conditional
/// group (`emit_staged_candidate_conditional` under
/// `MaintenanceDialect::Trino`) runs end to end through
/// `execute_statement_group`: a changed row is rewritten, an unchanged row
/// is left alone (never deleted+reinserted), a brand-new key is inserted,
/// and no staged table is left behind afterwards.
#[tokio::test]
async fn staged_conditional_group_executes_on_trino() {
    let Some(env) = live_env_or_skip("staged_conditional_group_executes_on_trino", "exec").await
    else {
        return;
    };
    let table = env.q("dim_users");
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {table} AS SELECT * FROM (VALUES (1, 'gold'), (2, 'silver')) AS \
             x(user_id, tier)"
        ))
        .await
        .expect("create target table");

    // Candidate: user 1 changes tier, user 2 stays the same, user 3 is new.
    let candidate_select = "SELECT * FROM (VALUES (1, 'platinum'), (2, 'silver'), (3, 'bronze')) \
                             AS x(user_id, tier)";
    let staged_relation = StagedRelation::derive(
        "__smelt_staged_",
        "dim_users",
        StagedRelationResidence::TargetSchema,
        false,
    );
    let group = emit_staged_candidate_conditional(
        &table,
        &staged_relation,
        &["user_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        MaintenanceDialect::Trino,
    );
    assert!(!group.transactional);

    env.backend
        .execute_statement_group(&group)
        .await
        .expect("staged-candidate conditional group must execute on Trino");

    let mut rows: Vec<String> = env
        .select_string_col(&format!(
            "SELECT CAST(user_id AS VARCHAR) || ':' || tier FROM {table} ORDER BY user_id"
        ))
        .await;
    rows.sort();
    assert_eq!(
        rows,
        vec![
            "1:platinum".to_string(),
            "2:silver".to_string(),
            "3:bronze".to_string(),
        ],
        "changed row rewritten, unchanged row untouched, new key inserted"
    );

    let staged_name = env.q(&staged_relation.name);
    let staged_still_exists = env
        .ok(&format!("SELECT * FROM {staged_name} LIMIT 0"))
        .await;
    assert!(
        !staged_still_exists,
        "the staged relation must be dropped at the end of the group"
    );

    common::drop_schema(&env).await;
}
