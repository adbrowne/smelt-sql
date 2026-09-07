/// Plan/classifier agreement for the snapshot-reconcile run shape
/// (`docs/specs/incremental_shapes.md` §"The two run shapes"): a keyed
/// model with a `SUM` (additive-fold) column, over a source declared
/// `mutation_profile: append_only` but with NO clocked source anywhere in
/// the model (`SourceFacts::partition_col` is `None` for every declared
/// source) — the snapshot-reconcile shape — must be refused at the plan
/// layer exactly as `rules::cumulative::classify_cumulative` refuses it
/// (`KeyedSnapshotSourceUnsupportedColumn`: "re-folding state
/// double-counts — a mutable snapshot is not a replayable,
/// retraction-free event feed"). Before the fix, `derive_new_data`'s
/// `Grain::Key` arm consulted only the triggering source's declared
/// `MutationProfile` (append-only passes the faithful-fold source-posture
/// condition on posture alone, clock-independent) and admitted a
/// `Technique::KeyedFold` cell here — `smelt explain` showed an admitted
/// cell the runtime classifier refuses outright, a plan/runtime-agreement
/// violation (execution stayed safe only because both dispatch paths
/// re-classify independently).
#[test]
fn sum_over_unclocked_append_only_source_is_refused_at_plan_layer() {
    use smelt_core::config::{Grain, RefreshStrategy};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, Refusal, SourceFacts, Technique};

    let sql = "SELECT user_id, SUM(amount) AS total FROM smelt.sources.dim_table \
               GROUP BY user_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "dim_table".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: false,
    }];

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.dim_table_totals",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("refresh: incremental model must derive a plan");

    assert!(
        !result
            .plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "a SUM fold over a wholly-unclocked (snapshot-reconcile) model must never admit \
         a KeyedFold cell, regardless of the triggering source's declared posture; got \
         cells {:?}",
        result.plan.cells
    );

    let refusal_names_snapshot_reconcile = result.plan.refusals.iter().any(|r| match r {
        Refusal::NoAdmissibleTechnique { why, .. } => {
            let why_lower = why.to_lowercase();
            why_lower.contains("snapshot-reconcile")
                && (why_lower.contains("double-count") || why_lower.contains("double count"))
        }
        #[allow(unreachable_patterns)]
        _ => false,
    });
    assert!(
        refusal_names_snapshot_reconcile,
        "expected a refusal naming the snapshot-reconcile double-count reason \
         (mirroring KeyedSnapshotSourceUnsupportedColumn); got refusals {:?}",
        result.plan.refusals
    );
}
