//! Per-cell admission obligations 2 and 3 (`incremental_models.md`
//! §"Per-cell admission") for the keyed-fold family: the faithful-fold
//! obligation composes two INDEPENDENT conditions — source posture (does the
//! delta stream partition the input, i.e. is it retraction-free) and
//! combiner algebra (does a monoid need an inverse to undo a retracted
//! contribution) — and either failing alone refuses the fold family, never
//! only in combination.

use std::collections::{BTreeSet, HashSet};

use smelt_logical::maintenance::derive::{
    derive_maintenance_plan, derive_triggers, FoldSpec, ModelInputs,
};
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
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), combiner)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
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
        // The repair narrowing also attempts a per-group recompute over the
        // posture failure; `payments` declares no `unique_key`, so its own
        // affected-key discovery fails closed too, pushing an additive
        // `RepairKeysNotDiscoverable` refusal alongside the pre-existing one
        // (`incremental_models.md` §"The repair family" — fail-closed
        // refusal is additive, never a replacement).
        let no_admissible: Vec<_> = plan
            .refusals
            .iter()
            .filter(|r| matches!(r, Refusal::NoAdmissibleTechnique { .. }))
            .collect();
        assert!(
            matches!(&no_admissible[..], [Refusal::NoAdmissibleTechnique { .. }]),
            "{combiner:?}: expected exactly one NoAdmissibleTechnique refusal, got {:?}",
            plan.refusals
        );
        assert!(plan
            .refusals
            .iter()
            .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { .. })));
        let Refusal::NoAdmissibleTechnique { why, .. } = no_admissible[0] else {
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
    // Additive repair refusal, same rationale as the test above: `payments`
    // declares no `unique_key`, so the repair narrowing's own affected-key
    // discovery fails closed too.
    assert!(plan
        .refusals
        .iter()
        .any(|r| matches!(r, Refusal::NoAdmissibleTechnique { .. })));
    assert!(plan
        .refusals
        .iter()
        .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { .. })));
}

fn multi_column_inputs(
    add_columns: Vec<(&str, SqlFunction)>,
    mutation: MutationProfile,
) -> (ModelInputs<'static>, Trigger) {
    let inputs = ModelInputs {
        sql: "SELECT user_id, COUNT(*) AS n, MIN(event_ts) AS first_seen, \
              MAX(event_ts) AS last_seen FROM smelt.sources.payments GROUP BY user_id",
        output: OutputSpec {
            table: "user_activity".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![source(mutation)],
        column_groups: vec![ColumnGroup {
            columns: add_columns.iter().map(|(c, _)| c.to_string()).collect(),
            mutation_sensitivity: set(&["payments"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: add_columns
                .into_iter()
                .map(|(c, combiner)| (c.to_string(), combiner))
                .collect(),
        }),
        old_columns: Vec::new(),
        old_sql: None,
    };
    let trigger = Trigger::NewData {
        source: "payments".to_string(),
    };
    (inputs, trigger)
}

/// A multi-column fold, each column carrying its **own** combiner
/// (`COUNT`→`SUM`, `MIN`→`MIN`, `MAX`→`MAX`), admits a single `KeyedFold`
/// cell over an append-only source — the multi-column shape this phase
/// (W0b) unblocks, mirroring `device_user_edges.sql`'s hand-written
/// composition.
#[test]
fn multi_column_mixed_combiners_admit_one_keyed_fold_cell() {
    let (inputs, trigger) = multi_column_inputs(
        vec![
            ("n", SqlFunction::Sum),
            ("first_seen", SqlFunction::Min),
            ("last_seen", SqlFunction::Max),
        ],
        MutationProfile::AppendOnly,
    );
    let plan = derive_maintenance_plan(&inputs, &[trigger]);
    assert!(
        plan.refusals.is_empty(),
        "expected no refusals, got {:?}",
        plan.refusals
    );
    assert_eq!(
        plan.cells.len(),
        1,
        "expected exactly one cell, got {:?}",
        plan.cells
    );
    assert_eq!(plan.cells[0].group, "{n, first_seen, last_seen}");
}

/// Fail-closed: a mixed-combiner fold where ONE column's combiner is
/// holistic/non-monoid refuses the **whole** cell — a monoid combiner on
/// the other columns does not admit a partial fold.
#[test]
fn multi_column_one_non_monoid_combiner_refuses_the_whole_cell() {
    let (inputs, trigger) = multi_column_inputs(
        vec![
            ("n", SqlFunction::Sum),
            ("typical", SqlFunction::Median),
            ("last_seen", SqlFunction::Max),
        ],
        MutationProfile::AppendOnly,
    );
    let plan = derive_maintenance_plan(&inputs, &[trigger]);
    assert!(
        plan.cells.is_empty(),
        "expected no KeyedFold cell — a non-monoid combiner on any one column must refuse the \
         whole cell, got {:?}",
        plan.cells
    );
    assert!(matches!(
        &plan.refusals[..],
        [Refusal::NoAdmissibleTechnique { .. }]
    ));
    let Refusal::NoAdmissibleTechnique { why, .. } = &plan.refusals[0] else {
        unreachable!()
    };
    assert!(
        why.contains("typical") && why.contains("Median"),
        "refusal should name the offending column and combiner, got: {why}"
    );
}

/// The once-write family's waiver is scoped to the ALGEBRA leg only. A
/// `Coalesce` fold column is exempt from the combiner-algebra condition
/// (its admission rests on the independent once-write provenance proof,
/// `rules::cumulative::classify_once_write`), but it is NOT exempt from the
/// source-posture / delta-discovery condition: over a retracting
/// (`MutableSnapshot`) source the cell still refuses.
#[test]
fn once_write_waives_algebra_only_not_source_posture() {
    // Append-only: the algebra leg alone would refuse a non-monoid,
    // non-order-monotone combiner — the once-write waiver admits it.
    let (append_only, trigger) = inputs(SqlFunction::Coalesce, MutationProfile::AppendOnly);
    let plan = derive_maintenance_plan(&append_only, &[trigger]);
    assert!(
        plan.refusals.is_empty(),
        "once-write is exempt from the algebra leg, got {:?}",
        plan.refusals
    );
    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );

    // Retracting source: the posture leg is NOT waived.
    let (mutable, trigger) = inputs(SqlFunction::Coalesce, MutationProfile::MutableSnapshot);
    let plan = derive_maintenance_plan(&mutable, &[trigger]);
    assert!(
        plan.cells.is_empty(),
        "the source-posture leg must still bind for a once-write column, got {:?}",
        plan.cells
    );
    let Refusal::NoAdmissibleTechnique { why, .. } = &plan.refusals[0] else {
        unreachable!()
    };
    assert!(
        why.contains("append-only") || why.contains("retract"),
        "refusal should cite the source-posture obligation, got: {why}"
    );
}

// ---------------------------------------------------------------------------
// `derive_triggers` — the pure "which changed inputs get a mutation cell"
// derivation (`incremental_models.md` §"Per-cell admission").

fn mutable_source(name: &str, clocked: bool) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: clocked.then(|| "updated_at".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn append_only_source(name: &str) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: false,
    }
}

/// A **clocked** explicitly-mutable source still gets an `UpstreamMutation`
/// trigger — the clock is not part of the derivation rule (only the
/// downstream locality/admission proof consults it).
#[test]
fn clocked_mutable_source_gets_a_mutation_trigger() {
    let sources = vec![mutable_source("raw.user_status", true)];
    let explicitly_mutable: HashSet<String> = ["raw.user_status".to_string()].into_iter().collect();
    let triggers = derive_triggers(&sources, &[], &explicitly_mutable, &[]);
    assert!(
        triggers.contains(&Trigger::UpstreamMutation {
            source: "raw.user_status".to_string()
        }),
        "expected an UpstreamMutation trigger for a clocked explicitly-mutable source, got {triggers:?}"
    );
}

/// The fail-closed default: `MutableSnapshot` facts alone, absent from the
/// explicitly-mutable set, yield only `NewData` — declaring
/// `mutation_profile: mutable_snapshot` is opt-in, never inferred.
#[test]
fn undeclared_mutable_source_gets_no_mutation_trigger() {
    let sources = vec![mutable_source("raw.dim_customer", false)];
    let triggers = derive_triggers(&sources, &[], &HashSet::new(), &[]);
    assert_eq!(
        triggers,
        vec![
            Trigger::NewData {
                source: "raw.dim_customer".to_string()
            },
            Trigger::Backfill,
        ]
    );
}

/// An `AppendOnly` source named in some column group's `mutation_sensitivity`
/// (an aggregate that is value-sensitive to late-arriving rows) gets a
/// mutation trigger of its own.
#[test]
fn append_only_source_in_a_value_sensitive_group_gets_a_mutation_trigger() {
    let sources = vec![append_only_source("events")];
    let column_groups = vec![ColumnGroup {
        columns: vec!["event_count".to_string()],
        mutation_sensitivity: ["events".to_string()].into_iter().collect(),
        membership_sensitivity: BTreeSet::new(),
    }];
    let triggers = derive_triggers(&sources, &column_groups, &HashSet::new(), &[]);
    assert!(
        triggers.contains(&Trigger::UpstreamMutation {
            source: "events".to_string()
        }),
        "expected an UpstreamMutation trigger for an aggregate-sensitive append-only source, got \
         {triggers:?}"
    );
}

/// A pass-through append-only read with no value-sensitivity gets no
/// mutation trigger.
#[test]
fn append_only_source_with_no_value_sensitivity_gets_no_mutation_trigger() {
    let sources = vec![append_only_source("events")];
    let column_groups = vec![ColumnGroup {
        columns: vec!["event_id".to_string()],
        mutation_sensitivity: BTreeSet::new(),
        membership_sensitivity: BTreeSet::new(),
    }];
    let triggers = derive_triggers(&sources, &column_groups, &HashSet::new(), &[]);
    assert_eq!(
        triggers,
        vec![
            Trigger::NewData {
                source: "events".to_string()
            },
            Trigger::Backfill,
        ]
    );
}

/// One trigger per source, deterministic order — repeats of the same source
/// name (e.g. read under more than one alias) are deduplicated, not
/// double-counted.
#[test]
fn trigger_derivation_is_order_stable_and_deduplicated() {
    let sources = vec![
        append_only_source("a"),
        mutable_source("b", false),
        append_only_source("a"),
    ];
    let explicitly_mutable: HashSet<String> = ["b".to_string()].into_iter().collect();
    let triggers = derive_triggers(&sources, &[], &explicitly_mutable, &["new_col".to_string()]);
    assert_eq!(
        triggers,
        vec![
            Trigger::NewData {
                source: "a".to_string()
            },
            Trigger::NewData {
                source: "b".to_string()
            },
            Trigger::UpstreamMutation {
                source: "b".to_string()
            },
            Trigger::Backfill,
            Trigger::ColumnAdded {
                columns: vec!["new_col".to_string()]
            },
        ]
    );
}
