//! Pure derivation of a [`MaintenancePlan`] from analysis facts — v0.
//!
//! Consumes the derivations that exist (`analysis::source_bounds` for reach,
//! `analysis::discriminants` for combiner algebra, `analysis::model_diff` for
//! the additive-only column-add proof) and takes as *inputs* the two
//! classifiers that do not exist yet (column groups, skeleton roles) — see
//! the module doc in [`super`].

use std::collections::{BTreeSet, HashMap};

use smelt_types::SqlFunction;

use super::{
    ColumnGroup, Corner, Grain, MaintenancePlan, MutationProfile, OutputSpec, PartitionLocal,
    PlanCell, Refusal, ScanClamp, SourceFacts, Technique, Trigger,
};
use crate::analysis::discriminants::combiner_discriminants;
use crate::analysis::model_diff::ModelDiff;
use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};

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

    fn output_partition_col(&self) -> Option<&str> {
        match &self.output.grain {
            Grain::Partition { partition_col } => Some(partition_col),
            Grain::Key { .. } => None,
        }
    }
}

/// How one read source relates to the output's partition axis for a
/// region-anchored maintenance op.
enum SourceLink {
    /// Bounded: the derived scan clamp, anchored to the output region.
    Clamp(ScanClamp),
    /// Clocked but with no derivable link to the output partition axis (or
    /// an unbounded one) — the op cannot be partition-pruned.
    Unlinked { why: String },
    /// Not clocked at all: a lookup read in full.
    Unclocked,
}

/// Link `facts` to the output partition axis via the derived bounds.
///
/// The load-bearing v0 rule: a **cross-axis** source (its partition column is
/// not the output's) is linked only by an *explicit, derivable* predicate on
/// its partition column — the zero-margin `Bounded{0,0}` fallback means "no
/// predicate found at all", which for a cross-axis source is the absence of a
/// link, not a zero-cost one. Neither smelt nor the engine can know how an
/// undeclared timestamp relates to the partition column, so this fails
/// closed. (A same-axis source is linked by identity; zero margin is real
/// there.)
fn link_source(
    output_partition_col: Option<&str>,
    bounds: &HashMap<String, BoundResult>,
    facts: &SourceFacts,
) -> SourceLink {
    let Some(col) = &facts.partition_col else {
        return SourceLink::Unclocked;
    };
    let same_axis = output_partition_col == Some(col.as_str());
    match bounds.get(&facts.name) {
        Some(BoundResult::Bounded { before, after, .. }) => {
            if same_axis || *before > Seconds::ZERO || *after > Seconds::ZERO {
                SourceLink::Clamp(ScanClamp {
                    source: facts.name.clone(),
                    column: col.clone(),
                    before: *before,
                    after: *after,
                })
            } else {
                SourceLink::Unlinked {
                    why: format!(
                        "no predicate links '{col}' to the output partition axis — the \
                         scan cannot be partition-pruned"
                    ),
                }
            }
        }
        Some(BoundResult::Unbounded) => SourceLink::Unlinked {
            why: "derived scan is unbounded".to_string(),
        },
        Some(BoundResult::NotDerivable) | None => SourceLink::Unlinked {
            why: "scan bound not derivable".to_string(),
        },
    }
}

/// Derive the plan cells (and refusals) for `triggers` against `inputs`.
pub fn derive_maintenance_plan(inputs: &ModelInputs, triggers: &[Trigger]) -> MaintenancePlan {
    let mut plan = MaintenancePlan::default();
    let bounds = derive_model_bounds(inputs.sql, &inputs.bound_context());

    for trigger in triggers {
        match trigger {
            Trigger::NewData { source } => derive_new_data(inputs, &bounds, source, &mut plan),
            Trigger::UpstreamMutation { source } => {
                derive_mutation(inputs, &bounds, source, &mut plan)
            }
            Trigger::ColumnAdded { columns } => {
                derive_column_added(inputs, &bounds, columns, &mut plan)
            }
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
    bounds: &HashMap<String, BoundResult>,
    source: &str,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::NewData {
        source: source.to_string(),
    };
    match &inputs.output.grain {
        Grain::Partition { .. } => {
            let (partition_local, scans) = read_locality(inputs, bounds);
            plan.cells.push(PlanCell {
                group: "{*}".to_string(),
                trigger,
                corner: Corner::RecomputeRegion,
                technique: Technique::DeleteInsert,
                partition_local,
                scans,
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
                scans: vec![],
                ledger_catch_up: false,
            });
        }
    }
}

/// Mutation: a post-creation delta in `source` touches exactly the column
/// groups mutation-sensitive to it — the bottom-left column-scoped
/// re-derivation. Partition-local only when the source's partition column is
/// explicitly linked to the output axis (K8's ratified
/// `require: partition_local` refuses the unlinked/unclocked case unless the
/// full scan is declared).
fn derive_mutation(
    inputs: &ModelInputs,
    bounds: &HashMap<String, BoundResult>,
    source: &str,
    plan: &mut MaintenancePlan,
) {
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
        let (locality, scans) = match link_source(inputs.output_partition_col(), bounds, facts) {
            SourceLink::Clamp(clamp) => (PartitionLocal::Yes, vec![clamp]),
            SourceLink::Unclocked => (
                PartitionLocal::No {
                    source: source.to_string(),
                    why: "unclocked source: a change's footprint projects onto no bounded \
                          partition interval of the output"
                        .to_string(),
                },
                vec![],
            ),
            SourceLink::Unlinked { why } => (
                PartitionLocal::No {
                    source: source.to_string(),
                    why,
                },
                vec![],
            ),
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
            scans,
            ledger_catch_up: false,
        });
    }
}

/// Definition change: the model gained fields. Skeleton adds are grain
/// changes and refuse (EX-39); payload adds land in the 2×2's left column by
/// what they read (EX-36/37/40), instantiating their ledger entries at
/// `S = ∅` (the catch-up flag).
fn derive_column_added(
    inputs: &ModelInputs,
    bounds: &HashMap<String, BoundResult>,
    columns: &[String],
    plan: &mut MaintenancePlan,
) {
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
                    scans: vec![],
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

        // Re-derives from upstream: column-scoped MERGE. Every read source
        // must be linked to the output partition axis or explicitly accepted
        // as a full read (EX-38: the field-add inherits its source's
        // partition-locality verdict unchanged).
        let mut scans = Vec::new();
        let mut locality = PartitionLocal::Yes;
        let mut refused = false;
        for source_name in &group.mutation_sensitivity {
            let Some(facts) = inputs.source(source_name) else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!("unknown source '{source_name}'"),
                });
                refused = true;
                break;
            };
            match link_source(inputs.output_partition_col(), bounds, facts) {
                SourceLink::Clamp(clamp) => scans.push(clamp),
                SourceLink::Unclocked | SourceLink::Unlinked { .. } if !facts.allow_full_scan => {
                    plan.refusals.push(Refusal::ScanUnbounded {
                        source: facts.name.clone(),
                        why: format!(
                            "backfill of {} reads '{}' with no partition bound",
                            group.name(),
                            facts.name
                        ),
                    });
                    refused = true;
                    break;
                }
                SourceLink::Unclocked => {
                    locality = PartitionLocal::No {
                        source: facts.name.clone(),
                        why: "unclocked source read in full (declared)".to_string(),
                    };
                }
                SourceLink::Unlinked { why } => {
                    locality = PartitionLocal::No {
                        source: facts.name.clone(),
                        why: format!("{why} (declared full scan)"),
                    };
                }
            }
        }
        if refused {
            continue;
        }
        plan.cells.push(PlanCell {
            group: group.name(),
            trigger: trigger.clone(),
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: locality,
            scans,
            ledger_catch_up: true,
        });
    }
}

/// Backfill: the universal ground-truth reset — recompute the region from
/// replayable input, unconditionally correct (`01-framework.md` §3).
fn derive_backfill(
    inputs: &ModelInputs,
    bounds: &HashMap<String, BoundResult>,
    plan: &mut MaintenancePlan,
) {
    let (partition_local, scans) = read_locality(inputs, bounds);
    plan.cells.push(PlanCell {
        group: "{*}".to_string(),
        trigger: Trigger::Backfill,
        corner: Corner::RecomputeRegion,
        technique: Technique::DeleteInsert,
        partition_local,
        scans,
        ledger_catch_up: false,
    });
}

/// Partition-locality of a whole-row recompute's *reads*, plus the derived
/// scan clamps for the sources that are linked. The first unlinked or
/// unclocked source decides the `No` verdict (backfill stays admitted — a
/// recompute is the universal ground-truth reset — but the full read is
/// named, never silent).
fn read_locality(
    inputs: &ModelInputs,
    bounds: &HashMap<String, BoundResult>,
) -> (PartitionLocal, Vec<ScanClamp>) {
    // Keyed grain: a backfill is a whole-table rebuild; there is no output
    // partition axis to be local to.
    if inputs.output_partition_col().is_none() {
        return (PartitionLocal::Yes, vec![]);
    }
    let mut scans = Vec::new();
    let mut verdict = PartitionLocal::Yes;
    for s in &inputs.sources {
        match link_source(inputs.output_partition_col(), bounds, s) {
            SourceLink::Clamp(clamp) => scans.push(clamp),
            SourceLink::Unclocked => {
                if matches!(verdict, PartitionLocal::Yes) {
                    verdict = PartitionLocal::No {
                        source: s.name.clone(),
                        why: "unclocked source is read in full on every recompute".to_string(),
                    };
                }
            }
            SourceLink::Unlinked { why } => {
                if matches!(verdict, PartitionLocal::Yes) {
                    verdict = PartitionLocal::No {
                        source: s.name.clone(),
                        why,
                    };
                }
            }
        }
    }
    (verdict, scans)
}

/// Convenience used by tests: the set of column names across `groups`.
pub fn group_columns(groups: &[ColumnGroup]) -> BTreeSet<String> {
    groups
        .iter()
        .flat_map(|g| g.columns.iter().cloned())
        .collect()
}
