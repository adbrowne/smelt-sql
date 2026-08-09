//! `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs`) must
//! stay in lockstep with the runtime classifier
//! (`smelt_logical::rules::cumulative::classify_order_monotone_column`) on
//! whether an order-monotone (`MAX_BY`/`MIN_BY`, `ArgMax`/`ArgMin`) column
//! is admitted into a fold — otherwise `smelt explain`/LSP diagnostics
//! report a `KeyedFold` cell the runtime then refuses with
//! `KeyedUnknownCombiner` (a fail-loud discipline violation: a compile-time
//! false positive).
//!
//! Spec: `docs/specs/incremental_models.md` §"The column-family catalogue",
//! §"Statement emission (single owner)".

use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// A `MAX_BY(value, ordering)` projection with NO companion `MAX(ordering)`
/// projection in the same SELECT list must not be admitted into the derived
/// `FoldSpec` — the plan layer must refuse exactly where the runtime
/// classifier refuses (`KeyedUnknownCombiner`).
#[test]
fn max_by_without_companion_is_not_admitted() {
    let sql = "SELECT user_id, MAX_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    assert!(
        derive_fold_spec(sql).is_none(),
        "a companion-less MAX_BY must not be admitted into a FoldSpec — the runtime \
         classifier refuses this exact SQL with KeyedUnknownCombiner"
    );
}

/// The mirror-image positive case: a companion `MAX(updated_at)` projection
/// is present, so the `MAX_BY` column is admitted.
#[test]
fn max_by_with_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(status, updated_at) AS status, \
               MAX(updated_at) AS updated_at \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql).expect("companioned MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "status" && *f == SqlFunction::ArgMax));
}

/// `MIN_BY` without a companion `MIN(ordering)` is refused the same way.
#[test]
fn min_by_without_companion_is_not_admitted() {
    let sql = "SELECT user_id, MIN_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    assert!(derive_fold_spec(sql).is_none());
}

/// Degenerate self-companion: `MAX_BY(x, x)` — the value expression IS the
/// ordering expression, so the projected value is trivially the running
/// max of the ordering value already. No separate companion column is
/// required, and this must be admitted identically to the runtime
/// classifier.
#[test]
fn max_by_self_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(event_ts, event_ts) AS first_seen \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql).expect("self-companion MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_seen" && *f == SqlFunction::ArgMax));
}

/// Plan/classifier agreement for the snapshot-reconcile run shape
/// (`docs/specs/incremental_models.md` §"The two run shapes"): a keyed
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
