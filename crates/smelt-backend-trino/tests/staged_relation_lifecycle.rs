//! The staged relation group without temp tables, proved live
//! (`docs/outcomes/20260913-trino-ledger/phases/07-plan.md` criterion 8):
//! Trino has no session temp namespace and no transactional write
//! capability (phase 1), so its staged relation is a real, explicitly-named,
//! explicitly-dropped table in the target's own schema, and a run
//! interrupted between the stage and the apply must leave no
//! partially-applied data and no orphan relation a later run mistakes for
//! its own.
//!
//! This is a **bookkeeping** proof, not a maintenance-statement proof: it
//! runs the staged relation's own lifecycle statements
//! ([`StagedRelation::create_prefix`]/[`reclaim_statement`]/
//! [`drop_statement`]) directly through [`TrinoBackend`], the same shape
//! `capability_probes.rs` uses — no `MaintenanceDialect::Trino` variant
//! exists yet (phase 4's standing decision) and any real incremental write
//! on Trino hard-errors downstream today (phase 6's finding), so a group
//! built by `smelt_logical::maintenance::emit`'s emitters cannot complete a
//! live run on this backend. The statement spellings those emitters would
//! use for the DELETE/INSERT "apply" step are T4's
//! (`20260913-trino-incremental`); this test stands in for "apply" with a
//! single INSERT so it can prove the lifecycle without depending on that
//! unbuilt statement layer.
//!
//! Gated on `SMELT_TRINO_URL`: unset, this test skips green. Run with:
//!   bash scripts/trino-up.sh
//!   source scripts/trino-env.sh
//!   cargo test -p smelt-backend-trino --test staged_relation_lifecycle
//!   bash scripts/trino-down.sh

mod common;

use smelt_backend::Backend;
use smelt_logical::maintenance::emit::{StagedRelation, StagedRelationResidence};

async fn row_count(env: &common::LiveEnv, table: &str) -> i64 {
    *env.select_i64_col(&format!("SELECT count(*) AS n FROM {}", env.q(table)))
        .await
        .first()
        .expect("row count query must return exactly one row")
}

async fn table_exists(env: &common::LiveEnv, table: &str) -> bool {
    env.ok(&format!("SELECT * FROM {} LIMIT 0", env.q(table)))
        .await
}

#[tokio::test]
async fn staged_relation_lifecycle_survives_interruption_between_stage_and_apply() {
    let Some(env) = common::live_env_or_skip(
        "staged_relation_lifecycle_survives_interruption_between_stage_and_apply",
        "lifecycle",
    )
    .await
    else {
        return;
    };

    // The target table this staged relation feeds.
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} (n INTEGER)",
            env.q("lifecycle_target")
        ))
        .await
        .expect("create target table");

    let staged = StagedRelation::derive(
        "__smelt_staged_",
        "lifecycle_target",
        StagedRelationResidence::TargetSchema,
        false,
    );
    assert_eq!(staged.create_prefix(), "CREATE TABLE");
    assert!(!staged.atomic);

    // --- Run 1: stage, then get interrupted before the apply. ---
    // Reclaim (no-op — nothing exists yet), CREATE, INSERT (populate). No
    // apply statement runs, simulating a crash between stage and apply.
    if let Some(reclaim) = staged.reclaim_statement() {
        env.backend
            .execute_sql(&reclaim)
            .await
            .expect("reclaim (no-op) must succeed");
    }
    env.backend
        .execute_sql(&format!(
            "{} {} AS SELECT n FROM (VALUES (1)) AS t(n) LIMIT 0",
            staged.create_prefix(),
            env.q(&staged.name)
        ))
        .await
        .expect("stage CREATE must succeed");
    env.backend
        .execute_sql(&format!(
            "INSERT INTO {} SELECT n FROM (VALUES (1), (2)) AS t(n)",
            env.q(&staged.name)
        ))
        .await
        .expect("stage INSERT must succeed");

    // The target is untouched, and exactly one orphan staged relation exists.
    assert_eq!(row_count(&env, "lifecycle_target").await, 0);
    assert!(table_exists(&env, &staged.name).await);
    assert_eq!(row_count(&env, &staged.name).await, 2);

    // --- Run 2: re-run the whole group from the top, despite the orphan. ---
    if let Some(reclaim) = staged.reclaim_statement() {
        env.backend
            .execute_sql(&reclaim)
            .await
            .expect("reclaim must drop the orphan from run 1");
    }
    env.backend
        .execute_sql(&format!(
            "{} {} AS SELECT n FROM (VALUES (1)) AS t(n) LIMIT 0",
            staged.create_prefix(),
            env.q(&staged.name)
        ))
        .await
        .expect("stage CREATE (run 2) must succeed");
    env.backend
        .execute_sql(&format!(
            "INSERT INTO {} SELECT n FROM (VALUES (3), (4), (5)) AS t(n)",
            env.q(&staged.name)
        ))
        .await
        .expect("stage INSERT (run 2) must succeed");
    // Apply: this run's own candidates only — the orphan's rows from run 1
    // are gone, reclaimed before this run's own CREATE ever ran.
    env.backend
        .execute_sql(&format!(
            "INSERT INTO {} SELECT n FROM {}",
            env.q("lifecycle_target"),
            env.q(&staged.name)
        ))
        .await
        .expect("apply INSERT must succeed");
    env.backend
        .execute_sql(&format!("DROP TABLE {}", env.q(&staged.name)))
        .await
        .expect("trailing DROP must succeed");

    // The target is correct (only run 2's candidates), and no staged
    // relation remains.
    assert_eq!(row_count(&env, "lifecycle_target").await, 3);
    assert!(!table_exists(&env, &staged.name).await);

    env.backend
        .execute_sql(&format!("DROP TABLE {}", env.q("lifecycle_target")))
        .await
        .expect("cleanup target table");
    common::drop_schema(&env).await;
}
