//! Pure derivation of a [`MaintenancePlan`] from analysis facts — v0.
//!
//! Consumes the derivations that exist (`analysis::source_bounds` for reach,
//! `analysis::discriminants` for combiner algebra, `analysis::model_diff` for
//! the additive-only column-add proof) and takes as *inputs* the two
//! classifiers that do not exist yet (column groups, skeleton roles) — see
//! the module doc in [`super`].

use std::collections::BTreeSet;

use smelt_types::SqlFunction;

use super::{
    ColumnGroup, Corner, Grain, MaintenancePlan, MutationProfile, OutputSpec, PartitionLocal,
    PlanCell, Refusal, SourceFacts, Technique, Trigger,
};
use crate::analysis::discriminants::combiner_discriminants;
use crate::analysis::model_diff::ModelDiff;
use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult};

/// Caller-supplied fold admission input for a keyed-grain model: which key
/// the fold addresses and which columns fold additively under `combiner`.
/// Checked against the combiner-algebra classifier — never trusted bare.
#[derive(Debug, Clone)]
pub struct FoldSpec {
    pub add_columns: Vec<String>,
    pub combiner: SqlFunction,
}

/// Everything the v0 derivation reads. `column_groups` and
/// `output.skeleton_columns` are hand-supplied (the deferred classifiers);
/// the rest is derived from `sql` and the source declarations.
#[derive(Debug)]
pub struct ModelInputs<'a> {
    /// Expanded model SQL (functions inlined), used for bound derivation.
    pub sql: &'a str,
    pub output: OutputSpec,
    pub sources: Vec<SourceFacts>,
    pub column_groups: Vec<ColumnGroup>,
    /// Present for keyed-grain models whose new-data cell should fold.
    pub fold: Option<FoldSpec>,
    /// The additive-only proof for a `ColumnAdded` trigger, computed by the
    /// caller via [`crate::analysis::model_diff::additive_only_diff`] over
    /// the old/new column lists. Required to admit an in-place update.
    pub column_add_proof: Option<&'a ModelDiff>,
}

impl ModelInputs<'_> {
    fn source(&self, name: &str) -> Option<&SourceFacts> {
        self.sources.iter().find(|s| s.name == name)
    }

    fn bound_context(&self) -> BoundContext {
        let mut ctx = BoundContext::new();
        for s in &self.sources {
            if let Some(p) = &s.partition_col {
                ctx.add_source(&s.name, p);
            }
        }
        ctx
    }
}

/// Derive the plan cells (and refusals) for `triggers` against `inputs`.
pub fn derive_maintenance_plan(inputs: &ModelInputs, triggers: &[Trigger]) -> MaintenancePlan {
    let mut plan = MaintenancePlan::default();
    let bounds = derive_model_bounds(inputs.sql, &inputs.bound_context());

    for trigger in triggers {
        match trigger {
            Trigger::NewData { source } => derive_new_data(inputs, &bounds, source, &mut plan),
            Trigger::UpstreamMutation { source } => derive_mutation(inputs, source, &mut plan),
            Trigger::ColumnAdded { columns } => derive_column_added(inputs, columns, &mut plan),
            Trigger::Backfill => derive_backfill(inputs, &bounds, &mut plan),
        }
    }
    plan
}

/// Creation: new rows in the driving source. Partition grain recomputes the
/// new region (today's mechanism — for a pure append the RMW corner
/// degenerates to the same insert); key grain folds the delta into stored
/// key state, admitted only for a faithful additive combiner over an
/// append-only source (`01-framework.md` §4).
fn derive_new_data(
    inputs: &ModelInputs,
    bounds: &std::collections::HashMap<String, BoundResult>,
    source: &str,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::NewData {
        source: source.to_string(),
    };
    match &inputs.output.grain {
        Grain::Partition { .. } => {
            plan.cells.push(PlanCell {
                group: "{*}".to_string(),
                trigger,
                corner: Corner::RecomputeRegion,
                technique: Technique::DeleteInsert,
                partition_local: read_locality(inputs, bounds),
                ledger_catch_up: false,
            });
        }
        Grain::Key { .. } => {
            let Some(fold) = &inputs.fold else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: "keyed grain with no fold specification".to_string(),
                });
                return;
            };
            let Some(facts) = inputs.source(source) else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!("unknown source '{source}'"),
                });
                return;
            };
            // Faithful-fold admission: the delta stream must partition the
            // input (append-only) and the combiner must be a monoid whose
            // fold equals the batch aggregate. Fail closed on either.
            let disc = combiner_discriminants(fold.combiner, false);
            if facts.mutation != MutationProfile::AppendOnly {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!(
                        "fold over '{source}' is not faithful: the source is not append-only"
                    ),
                });
                return;
            }
            if !disc.is_monoid {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!(
                        "combiner {:?} is not a monoid — no delta+state read exists",
                        fold.combiner
                    ),
                });
                return;
            }
            plan.cells.push(PlanCell {
                group: format!("{{{}}}", fold.add_columns.join(", ")),
                trigger,
                corner: Corner::FoldDelta,
                technique: Technique::KeyedFold,
                // Keyed end-state: the write is key-addressed, not
                // partition-addressed; there is no partition axis to bound.
                partition_local: PartitionLocal::Yes,
                ledger_catch_up: false,
            });
        }
    }
}

/// Mutation: a post-creation change in `source` touches exactly the column
/// groups mutation-sensitive to it — the bottom-left column-scoped
/// re-derivation, partition-local only when the source is clocked (K8's
/// ratified `require: partition_local` refuses the unclocked case unless the
/// full scan is declared).
fn derive_mutation(inputs: &ModelInputs, source: &str, plan: &mut MaintenancePlan) {
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };
    let Some(facts) = inputs.source(source) else {
        plan.refusals.push(Refusal::NoAdmissibleTechnique {
            trigger: format!("{trigger:?}"),
            why: format!("unknown source '{source}'"),
        });
        return;
    };

    for group in inputs
        .column_groups
        .iter()
        .filter(|g| g.mutation_sensitivity.contains(source))
    {
        let locality = if facts.partition_col.is_some() {
            PartitionLocal::Yes
        } else {
            PartitionLocal::No {
                source: source.to_string(),
                why: "unclocked source: a change's footprint projects onto no bounded \
                      partition interval of the output"
                    .to_string(),
            }
        };
        if matches!(locality, PartitionLocal::No { .. }) && !facts.allow_full_scan {
            plan.refusals.push(Refusal::ScanUnbounded {
                source: source.to_string(),
                why: format!(
                    "maintenance of {} driven by '{source}' scatters across all output \
                     partitions; declare allow_full_scan to accept the full-table write",
                    group.name()
                ),
            });
            continue;
        }
        plan.cells.push(PlanCell {
            group: group.name(),
            trigger: trigger.clone(),
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: locality,
            ledger_catch_up: false,
        });
    }
}

/// Definition change: the model gained fields. Skeleton adds are grain
/// changes and refuse (EX-39); payload adds land in the 2×2's left column by
/// what they read (EX-36/37/40), instantiating their ledger entries at
/// `S = ∅` (the catch-up flag).
fn derive_column_added(inputs: &ModelInputs, columns: &[String], plan: &mut MaintenancePlan) {
    let trigger = Trigger::ColumnAdded {
        columns: columns.to_vec(),
    };
    // Boundary first: a skeleton-position add changes which rows exist.
    for col in columns {
        if inputs.output.skeleton_columns.contains(col) {
            plan.refusals.push(Refusal::SkeletonColumnAdded {
                column: col.clone(),
            });
            return;
        }
    }

    // The added fields factor by shared mutation-sensitivity exactly as the
    // base plan does; each added group gets its own catch-up op.
    for group in inputs
        .column_groups
        .iter()
        .filter(|g| g.columns.iter().any(|c| columns.contains(c)))
    {
        if group.mutation_sensitivity.is_empty() {
            // Pure function of stored columns — admissible in place only if
            // the additive-only proof holds (fail closed without it).
            match inputs.column_add_proof {
                Some(ModelDiff::AdditiveOnly) => plan.cells.push(PlanCell {
                    group: group.name(),
                    trigger: trigger.clone(),
                    corner: Corner::FoldDelta,
                    technique: Technique::InPlaceUpdate,
                    partition_local: PartitionLocal::Yes,
                    ledger_catch_up: true,
                }),
                Some(ModelDiff::NotAdditive { reason }) => {
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!("in-place update not proven additive-only: {reason}"),
                    });
                }
                None => {
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: "in-place update requires the additive-only model-diff proof"
                            .to_string(),
                    });
                }
            }
            continue;
        }

        // Re-derives from upstream: column-scoped MERGE, inheriting each
        // read source's partition-locality verdict unchanged (EX-38).
        let unclocked: Option<&SourceFacts> = group
            .mutation_sensitivity
            .iter()
            .filter_map(|s| inputs.source(s))
            .find(|f| f.partition_col.is_none());
        match unclocked {
            Some(f) if !f.allow_full_scan => {
                plan.refusals.push(Refusal::ScanUnbounded {
                    source: f.name.clone(),
                    why: format!(
                        "backfill of {} reads unclocked '{}' with no partition bound",
                        group.name(),
                        f.name
                    ),
                });
            }
            _ => plan.cells.push(PlanCell {
                group: group.name(),
                trigger: trigger.clone(),
                corner: Corner::ColumnMerge,
                technique: Technique::ColumnScopedMerge,
                partition_local: match unclocked {
                    Some(f) => PartitionLocal::No {
                        source: f.name.clone(),
                        why: "unclocked source read in full (declared)".to_string(),
                    },
                    None => PartitionLocal::Yes,
                },
                ledger_catch_up: true,
            }),
        }
    }
}

/// Backfill: the universal ground-truth reset — recompute the region from
/// replayable input, unconditionally correct (`01-framework.md` §3).
fn derive_backfill(
    inputs: &ModelInputs,
    bounds: &std::collections::HashMap<String, BoundResult>,
    plan: &mut MaintenancePlan,
) {
    plan.cells.push(PlanCell {
        group: "{*}".to_string(),
        trigger: Trigger::Backfill,
        corner: Corner::RecomputeRegion,
        technique: Technique::DeleteInsert,
        partition_local: read_locality(inputs, bounds),
        ledger_catch_up: false,
    });
}

/// Partition-locality of a whole-row recompute's *reads*: every clocked
/// source must have a bounded derived scan; an unclocked source is a
/// full-read lookup (partition-local only as a declared acceptance).
fn read_locality(
    inputs: &ModelInputs,
    bounds: &std::collections::HashMap<String, BoundResult>,
) -> PartitionLocal {
    for s in &inputs.sources {
        if s.partition_col.is_none() {
            return PartitionLocal::No {
                source: s.name.clone(),
                why: "unclocked source is read in full on every recompute".to_string(),
            };
        }
        match bounds.get(&s.name) {
            Some(BoundResult::Bounded { .. }) => {}
            Some(BoundResult::Unbounded) => {
                return PartitionLocal::No {
                    source: s.name.clone(),
                    why: "derived scan is unbounded".to_string(),
                }
            }
            Some(BoundResult::NotDerivable) | None => {
                // The driving source's own partition column bounds its scan
                // even when no explicit predicate names it (the window
                // clamp); only a *non-driving* clocked source with no
                // derivable bound is an unbounded read. v0 treats the
                // absence of a derived bound on a clocked source as
                // driving-source behaviour (clamped by the run window).
            }
        }
    }
    PartitionLocal::Yes
}

/// Convenience used by tests: the set of column names across `groups`.
pub fn group_columns(groups: &[ColumnGroup]) -> BTreeSet<String> {
    groups
        .iter()
        .flat_map(|g| g.columns.iter().cloned())
        .collect()
}
