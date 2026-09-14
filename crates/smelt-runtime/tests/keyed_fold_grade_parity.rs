//! Gap 5 test 5 (`docs/outcomes/20260913-trino-incremental/phases/
//! 03g-plan.md`): the plan layer's derived `FoldGrade` and the runtime
//! classifier's `WindowedKeyedRule::ledger_grade()` must agree — one owner
//! (`smelt_logical::rules::cumulative::is_additive_combiner`), not two
//! independent classifications of the same combiner.

use std::collections::HashMap;

use smelt_core::config::{Grain as ConfigGrain, Granularity, RefreshStrategy, TimeseriesConfig};
use smelt_core::ModelMetadata;
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{FoldGrade, MutationProfile, SourceFacts, Trigger};
use smelt_runtime::maintenance_driver::WindowedKeyedRule;
use smelt_state::reconciliation::Grade;

fn metadata() -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        ..Default::default()
    }
}

fn sources() -> Vec<SourceFacts> {
    vec![SourceFacts {
        name: "payments".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }]
}

fn plan_fold_grade(sql: &str) -> FoldGrade {
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "lifetime_spend",
        &metadata(),
        &sources(),
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("a keyed-fold model must derive a plan");
    result
        .plan
        .cells
        .iter()
        .find(|c| matches!(c.trigger, Trigger::NewData { .. }))
        .and_then(|c| c.fold_grade)
        .expect("the creation cell must carry a fold grade")
}

fn runtime_ledger_grade(sql: &str) -> Grade {
    let mut source_timeseries = HashMap::new();
    source_timeseries.insert(
        "smelt.sources.payments".to_string(),
        TimeseriesConfig {
            event_time_column: "pay_date".to_string(),
            partition_column: "pay_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        },
    );
    let classification = smelt_runtime::classify_cumulative_sql(
        "model.under.test",
        sql,
        &source_timeseries,
        false,
        &[],
    )
    .expect("classification must succeed");
    classification.ledger_grade()
}

#[test]
fn sum_agrees_additive() {
    let sql = "SELECT user_id, SUM(amount) AS lifetime_spend \
               FROM smelt.sources.payments GROUP BY user_id";
    assert_eq!(plan_fold_grade(sql), FoldGrade::Additive);
    assert_eq!(runtime_ledger_grade(sql), Grade::Additive);
}

#[test]
fn max_agrees_idempotent() {
    let sql = "SELECT user_id, MAX(amount) AS lifetime_spend \
               FROM smelt.sources.payments GROUP BY user_id";
    assert_eq!(plan_fold_grade(sql), FoldGrade::Idempotent);
    assert_eq!(runtime_ledger_grade(sql), Grade::Idempotent);
}
