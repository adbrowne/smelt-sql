//! W10 Phase 3 (`docs/plans/20260720-prod-w10-keyed-mutable-admission.md`):
//! the key grain's `NewData` append-only obligation binds a
//! FOLD-CONTRIBUTING source, not every source the model references
//! (`incremental_shapes.md` §"The key grain (`grain: key`)"). A mutable
//! source consumed only through a covered `UpstreamMutation` enrichment
//! cell is admitted (no `NoAdmissibleTechnique` refusal, and its own
//! `ColumnScopedMerge` cell still stands); a source that is BOTH a fold
//! input and mutable stays refused — the folded contribution is
//! un-retractable and the conservative admission refuses it fail-loud.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts, Technique, Trigger,
};
use smelt_types::SqlFunction;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// `fact` is append-only and drives the fold's own `SUM`; `dim` is an
/// unclocked `mutable_snapshot` enrichment source declared `allow_full_scan`
/// (a `smelt.sources.dim` lookup join). `dim` appears in the model's `FROM`
/// and (in the fold-contributing variant) in `WHERE`/`SELECT`, but never as
/// an argument to the fold's own `SUM`.
fn sources() -> Vec<SourceFacts> {
    vec![
        SourceFacts {
            name: "fact".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "dim".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        },
    ]
}

/// A column group mutation-sensitive to `dim` — the caller-supplied fact
/// `derive_mutation` needs to admit a `ColumnScopedMerge` cell for `dim`'s
/// own `UpstreamMutation` trigger, independent of the `NewData` narrowing
/// this test file pins.
fn dim_column_group() -> ColumnGroup {
    ColumnGroup {
        columns: strings(&["status"]),
        mutation_sensitivity: set(&["dim"]),
        membership_sensitivity: BTreeSet::new(),
    }
}

/// **Red (admit):** `dim` is enrich-only — it is joined and filtered on, but
/// never inside the fold's own `SUM`. The plan carries NO
/// `NoAdmissibleTechnique` refusal for `dim`'s `NewData` trigger (and
/// synthesizes no spurious fold cell for it either — `dim`'s deltas do not
/// feed the fold, so this trigger needs no technique at all), the fact
/// source still gets its ordinary `KeyedFold` creation cell, and `dim`'s own
/// `UpstreamMutation` trigger still admits `ColumnScopedMerge`. Fails before
/// the Phase 3 narrowing (the blanket `MutationProfile != AppendOnly`
/// refusal fires for `dim`'s `NewData` trigger too).
#[test]
fn admits_enrich_only_covered_mutable_source() {
    let sql = "SELECT f.user_id AS user_id, SUM(f.amount) AS lifetime_spend, \
               d.status AS status \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.user_id = d.user_id \
               WHERE d.active = true \
               GROUP BY f.user_id, d.status";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "user_lifetime_status".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: sources(),
        column_groups: vec![dim_column_group()],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
    };

    let triggers = vec![
        Trigger::NewData {
            source: "fact".to_string(),
        },
        Trigger::NewData {
            source: "dim".to_string(),
        },
        Trigger::UpstreamMutation {
            source: "dim".to_string(),
        },
    ];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    assert!(
        plan.refusals.is_empty(),
        "expected no refusals once dim's enrich-only coverage waives the append-only \
         obligation, got {:?}",
        plan.refusals
    );

    let fact_new_data = Trigger::NewData {
        source: "fact".to_string(),
    };
    let fact_cell = plan
        .cell_for(&fact_new_data)
        .unwrap_or_else(|| panic!("expected a KeyedFold cell for fact's NewData: {plan:#?}"));
    assert_eq!(fact_cell.technique, Technique::KeyedFold);

    let dim_new_data = Trigger::NewData {
        source: "dim".to_string(),
    };
    assert!(
        plan.cell_for(&dim_new_data).is_none(),
        "dim does not feed the fold — its NewData trigger should synthesize no cell, got \
         {:?}",
        plan.cell_for(&dim_new_data)
    );

    let dim_mutation = Trigger::UpstreamMutation {
        source: "dim".to_string(),
    };
    let dim_cell = plan
        .cell_for(&dim_mutation)
        .unwrap_or_else(|| panic!("expected dim's ColumnScopedMerge cell: {plan:#?}"));
    assert_eq!(dim_cell.technique, Technique::ColumnScopedMerge);
}

/// **Guard (still refuse — the safety case):** same shape, but the fold now
/// also aggregates `dim` (`SUM(d.bonus)`), so `dim` is fold-contributing.
/// `dim`'s `NewData` trigger still refuses with `NoAdmissibleTechnique` even
/// though it is covered by an `UpstreamMutation` cell — coverage alone must
/// never admit a fold-contributing mutable source. This test must stay
/// green through the narrowing; it pins the safety carve-out
/// (`incremental_shapes.md` §"The key grain (`grain: key`)").
#[test]
fn both_fold_and_enrich_stays_refused() {
    let sql = "SELECT f.user_id AS user_id, SUM(f.amount) AS lifetime_spend, \
               SUM(d.bonus) AS bonus \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.user_id = d.user_id \
               GROUP BY f.user_id";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "user_lifetime_bonus".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: sources(),
        column_groups: vec![dim_column_group()],
        fold: Some(FoldSpec {
            add_columns: vec![
                ("lifetime_spend".to_string(), SqlFunction::Sum),
                ("bonus".to_string(), SqlFunction::Sum),
            ],
        }),
        old_columns: Vec::new(),
        old_sql: None,
    };

    let triggers = vec![
        Trigger::NewData {
            source: "fact".to_string(),
        },
        Trigger::NewData {
            source: "dim".to_string(),
        },
        Trigger::UpstreamMutation {
            source: "dim".to_string(),
        },
    ];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    let dim_new_data = Trigger::NewData {
        source: "dim".to_string(),
    };
    assert!(
        plan.cell_for(&dim_new_data).is_none(),
        "a fold-contributing mutable source must never admit a fold cell: {:?}",
        plan.cell_for(&dim_new_data)
    );
    let refusal = plan
        .refusals
        .iter()
        .find(|r| matches!(r, Refusal::NoAdmissibleTechnique { trigger, .. } if trigger.contains("dim")))
        .unwrap_or_else(|| panic!("expected a NoAdmissibleTechnique refusal naming dim's \
             NewData trigger: {:?}", plan.refusals));
    let Refusal::NoAdmissibleTechnique { why, .. } = refusal else {
        unreachable!()
    };
    assert!(
        why.contains("append-only") || why.contains("retract"),
        "refusal should cite the source-posture obligation, got: {why}"
    );
}

/// **Unchanged:** `dim` is mutable and enrich-only (same shape as the admit
/// test), but this model's trigger list carries no `UpstreamMutation{dim}`
/// (e.g. `dim` is not declared `explicitly_mutable` in `sources.md`
/// terms) — coverage is a NECESSARY condition, not merely sufficient. `dim`'s
/// `NewData` trigger still refuses exactly as before the narrowing.
#[test]
fn uncovered_mutable_source_stays_refused() {
    let sql = "SELECT f.user_id AS user_id, SUM(f.amount) AS lifetime_spend, \
               d.status AS status \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.user_id = d.user_id \
               WHERE d.active = true \
               GROUP BY f.user_id, d.status";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "user_lifetime_status".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: sources(),
        column_groups: vec![dim_column_group()],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
    };

    // No `Trigger::UpstreamMutation` for dim: it is not in the covered set.
    let triggers = vec![
        Trigger::NewData {
            source: "fact".to_string(),
        },
        Trigger::NewData {
            source: "dim".to_string(),
        },
    ];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    assert!(
        plan.refusals.iter().any(
            |r| matches!(r, Refusal::NoAdmissibleTechnique { trigger, .. } if trigger.contains("dim"))
        ),
        "an uncovered mutable source must still refuse dim's NewData trigger, got {:?}",
        plan.refusals
    );
}
