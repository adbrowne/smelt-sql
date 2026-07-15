//! Per-cell admission obligations 2 and 3 (`incremental_models.md`
//! §"Per-cell admission") for the keyed-fold family: the faithful-fold
//! obligation composes two INDEPENDENT conditions — source posture (does the
//! delta stream partition the input, i.e. is it retraction-free) and
//! combiner algebra (does a monoid need an inverse to undo a retracted
//! contribution) — and either failing alone refuses the fold family, never
//! only in combination.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts, Trigger,
};
use smelt_types::SqlFunction;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn source(mutation: MutationProfile) -> SourceFacts {
    SourceFacts {
        name: "payments".to_string(),
        mutation,
        partition_col: Some("pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn inputs(combiner: SqlFunction, mutation: MutationProfile) -> (ModelInputs<'static>, Trigger) {
    let inputs = ModelInputs {
        sql: "SELECT user_id, SUM(amount) AS lifetime_spend \
              FROM smelt.sources.payments GROUP BY user_id",
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![source(mutation)],
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["payments"]),
        }],
        fold: Some(FoldSpec {
            add_columns: strings(&["lifetime_spend"]),
            combiner,
        }),
        column_add_proof: None,
    };
    let trigger = Trigger::NewData {
        source: "payments".to_string(),
    };
    (inputs, trigger)
}

/// Obligation 3: a holistic/unrecognised combiner refuses the fold family
/// regardless of source posture — even over an append-only source, `MEDIAN`
/// and an exact `COUNT(DISTINCT x)`-shaped holistic aggregate never admit a
/// `KeyedFold` cell, leaving only the recompute family (today represented by
/// a declared `full` refresh / the `Backfill` trigger's recompute-region
/// cell, not a technique synthesized inside this same cell — v0 admits at
/// most one technique per cell).
#[test]
fn holistic_combiner_leaves_recompute_only() {
    for combiner in [SqlFunction::Median, SqlFunction::Mode] {
        let (inputs, trigger) = inputs(combiner, MutationProfile::AppendOnly);
        let plan = derive_maintenance_plan(&inputs, &[trigger]);

        assert!(
            plan.cells.is_empty(),
            "{combiner:?}: no KeyedFold cell should be admitted, got {:?}",
            plan.cells
        );
        assert!(
            matches!(&plan.refusals[..], [Refusal::NoAdmissibleTechnique { .. }]),
            "{combiner:?}: expected exactly one NoAdmissibleTechnique refusal, got {:?}",
            plan.refusals
        );
        let Refusal::NoAdmissibleTechnique { why, .. } = &plan.refusals[0] else {
            unreachable!()
        };
        assert!(
            why.contains("holistic") || why.contains("not a monoid"),
            "{combiner:?}: refusal should cite the combiner-algebra obligation, got: {why}"
        );
        assert!(
            why.to_lowercase().contains("recompute"),
            "{combiner:?}: refusal should name the recompute family as what remains, got: {why}"
        );
    }
}

/// Obligation 2's independence: a source whose declared mutation profile
/// carries retractions (not append-only) feeding a non-invertible-but-monoid
/// combiner (`MIN`/`MAX`, `needs_inverse == true`) fails the fold family even
/// though the combiner passes obligation 3 (it *is* a monoid) — the
/// source-posture condition is checked on its own, not folded into whether
/// the combiner happens to be a monoid.
#[test]
fn retractions_into_noninvertible_fail_faithful_fold() {
    for combiner in [SqlFunction::Min, SqlFunction::Max] {
        let (inputs, trigger) = inputs(combiner, MutationProfile::MutableSnapshot);
        let plan = derive_maintenance_plan(&inputs, &[trigger]);

        assert!(
            plan.cells.is_empty(),
            "{combiner:?}: no KeyedFold cell should be admitted over a retracting source, \
             got {:?}",
            plan.cells
        );
        assert!(
            matches!(&plan.refusals[..], [Refusal::NoAdmissibleTechnique { .. }]),
            "{combiner:?}: expected exactly one NoAdmissibleTechnique refusal, got {:?}",
            plan.refusals
        );
        let Refusal::NoAdmissibleTechnique { why, .. } = &plan.refusals[0] else {
            unreachable!()
        };
        assert!(
            why.contains("append-only") || why.contains("retract"),
            "{combiner:?}: refusal should cite the source-posture obligation, got: {why}"
        );
        assert!(
            why.contains("independent"),
            "{combiner:?}: refusal should name the two faithful-fold conditions as \
             independent, got: {why}"
        );
    }
}

/// Cross-check: the same MutableSnapshot posture also refuses an invertible
/// monoid (`SUM`) — obligation 2 fails on source posture alone, independent
/// of whether the combiner could in principle undo a contribution. This is
/// the pre-existing tracer floor (`maintenance_tracer.rs::
/// ex24_mutable_source_fails_the_faithful_fold_condition`); asserted again
/// here alongside the two conditions' independence to pin the "either alone
/// refuses" property for both the invertible and non-invertible combiner.
#[test]
fn retractions_also_refuse_an_invertible_monoid() {
    let (inputs, trigger) = inputs(SqlFunction::Sum, MutationProfile::MutableSnapshot);
    let plan = derive_maintenance_plan(&inputs, &[trigger]);
    assert!(plan.cells.is_empty());
    assert!(matches!(
        &plan.refusals[..],
        [Refusal::NoAdmissibleTechnique { .. }]
    ));
}
