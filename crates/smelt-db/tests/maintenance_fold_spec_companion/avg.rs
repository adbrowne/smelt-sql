use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// `derive_fold_spec` must stay in lockstep with the runtime classifier on
/// the decomposed-fold family (`AVG`/`STDDEV_*`/`VAR_*`,
/// `docs/outcomes/20260809-rung2-state-shapes` row 7) exactly as it already
/// does for `ArgMax`/`ArgMin` above: an `AVG` column is admitted into the
/// `FoldSpec`, a wrong-arity or `DISTINCT` `AVG` refuses the whole
/// derivation, and the derived plan carries a real `Technique::KeyedFold`
/// cell (no `NoAdmissibleTechnique` refusal) for it.
#[test]
fn avg_model_derives_fold_spec_and_keyed_fold_cell() {
    use smelt_core::config::{Grain, Granularity, RefreshStrategy};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique};

    let sql = "SELECT device_id, AVG(amount) AS avg_amount \
               FROM smelt.sources.events GROUP BY device_id";
    let spec = derive_fold_spec(sql, &[]).expect("AVG must be admitted into the FoldSpec");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "avg_amount" && *f == SqlFunction::Avg));

    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.device_avg_amount",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        Some(Granularity::Day),
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("refresh: incremental model must derive a plan");
    assert!(
        result
            .plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the AVG column; got cells {:?} and refusals {:?}",
        result.plan.cells,
        result.plan.refusals
    );
}

/// A wrong-arity `AVG` call refuses the whole `FoldSpec` derivation — fail-
/// closed, mirroring [`super::companion::max_by_wrong_arity_is_not_admitted`].
#[test]
fn avg_wrong_arity_is_not_admitted_into_fold_spec() {
    let sql = "SELECT device_id, AVG(amount, weight) AS avg_amount \
               FROM smelt.sources.events GROUP BY device_id";
    assert!(derive_fold_spec(sql, &[]).is_none());
}

/// `AVG(DISTINCT ...)` is holistic — refuses the whole derivation, the same
/// as the runtime classifier's `decompose_to_state` refusal.
#[test]
fn avg_distinct_is_not_admitted_into_fold_spec() {
    let sql = "SELECT device_id, AVG(DISTINCT amount) AS avg_amount \
               FROM smelt.sources.events GROUP BY device_id";
    assert!(derive_fold_spec(sql, &[]).is_none());
}
