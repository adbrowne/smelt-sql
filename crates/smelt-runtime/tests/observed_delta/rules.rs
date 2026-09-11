//! The shared [`WindowedKeyedRule`] fixtures the observed-delta tests drive
//! the real windowed-keyed driver with — extracted from `main.rs` so that
//! file stays under its large-file ratchet, the same split
//! `degradation.rs` already is.

use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::emit::{
    emit_keyed_fold, emit_keyed_fold_suppressed, MaintenanceDialect, TargetSlicePredicate,
};
use smelt_runtime::maintenance_driver::{keyed_fold_changed_keys_select, WindowedKeyedRule};

// ── Phase 16: write-side recording for the keyed fold and staged-candidate
// families (`docs/outcomes/20260815-definition-delta-migrate/phases/
// 16-plan.md`) ──

/// A minimal [`WindowedKeyedRule`] for these tests: a single `MAX`-combiner
/// aggregator column (`GREATEST(target.score, delta.score)`), mirroring
/// `keyed`'s own shape (`crate::cumulative::CumulativeClassification`)
/// closely enough to exercise `run_windowed_keyed_maintenance`'s
/// observed-delta recording without pulling in the full `smelt-planner`
/// classification machinery.
pub struct TestKeyedRule {
    pub unique_key: Vec<String>,
    pub folds: Vec<(String, String)>,
}

#[async_trait::async_trait]
impl WindowedKeyedRule for TestKeyedRule {
    fn refuse(&self) -> Option<String> {
        None
    }

    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        suppression: &WriteSuppression,
        dialect: MaintenanceDialect,
    ) -> String {
        let schema_table = format!("{schema}.{table}");
        let group = match suppression {
            WriteSuppression::Suppressed { compared_columns } => emit_keyed_fold_suppressed(
                &schema_table,
                &self.unique_key,
                &self.folds,
                delta_sql,
                slice,
                compared_columns,
                dialect,
            ),
            WriteSuppression::Unconditional { .. } => emit_keyed_fold(
                &schema_table,
                &self.unique_key,
                &self.folds,
                delta_sql,
                slice,
                dialect,
            ),
        };
        group.statements[0].sql.clone()
    }

    fn observed_delta_changed_keys_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        compared_columns: &[String],
        partition_column: Option<&str>,
        dialect: MaintenanceDialect,
    ) -> Option<String> {
        let schema_table = format!("{schema}.{table}");
        Some(keyed_fold_changed_keys_select(
            &schema_table,
            &self.unique_key,
            delta_sql,
            compared_columns,
            &self.folds,
            partition_column,
            dialect,
        ))
    }
}

pub fn max_score_rule() -> TestKeyedRule {
    TestKeyedRule {
        unique_key: vec!["user_id".to_string()],
        folds: vec![(
            "score".to_string(),
            "GREATEST(target.score, delta.score)".to_string(),
        )],
    }
}

/// Same shape as [`TestKeyedRule`], but `merge_sql` returns intentionally
/// broken SQL (a `MERGE` referencing a column the target table does not
/// have) — used to prove the recorded delta and the write share one
/// commit point (test 8: a failed write leaves no delta row behind).
pub struct FailingMergeKeyedRule {
    pub inner: TestKeyedRule,
}

#[async_trait::async_trait]
impl WindowedKeyedRule for FailingMergeKeyedRule {
    fn refuse(&self) -> Option<String> {
        None
    }

    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        _delta_sql: &str,
        _slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
        _dialect: MaintenanceDialect,
    ) -> String {
        format!(
            "MERGE INTO {schema}.{table} AS target USING (SELECT 1 AS user_id) AS delta ON \
             target.user_id = delta.user_id WHEN MATCHED THEN UPDATE SET \
             does_not_exist_column = 1"
        )
    }

    fn observed_delta_changed_keys_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        compared_columns: &[String],
        partition_column: Option<&str>,
        dialect: MaintenanceDialect,
    ) -> Option<String> {
        self.inner.observed_delta_changed_keys_sql(
            schema,
            table,
            delta_sql,
            compared_columns,
            partition_column,
            dialect,
        )
    }
}
