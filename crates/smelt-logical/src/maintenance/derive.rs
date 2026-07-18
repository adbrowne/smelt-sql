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
    PlanCell, Refusal, RowIdentity, RowIdentityVerdict, ScanClamp, SourceFacts, Technique, Trigger,
};
use crate::analysis::discriminants::combiner_discriminants;
use crate::analysis::input_delta::{
    input_delta_discovery, InputDeltaKind, MutationProfile as DeltaMutationProfile, SourceShape,
};
use crate::analysis::join_shape::JoinContext;
use crate::analysis::model_diff::ModelDiff;
use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};
use crate::analysis::walk::model_property_vector;

/// Derive the region row identity (P2, `model_properties.md` §"Region row
/// identity") for a model: the declared `unique_key` off the output's own
/// `Grain::Key` when present, else the proven grain key the composition walk
/// establishes over `sql` (`analysis::walk::PropertyVector::grain`), else the
/// identity-free `WholeRow` fallback.
///
/// Fail-closed: a proven key is only trusted when the walk also proves no
/// input join fans the output out (`PropertyVector::has_fan_out_join`) — a
/// key that does not cover every output row is never used, not even as a
/// partial key. `declared_unique_key` and a differing proven key may both be
/// present at once; declared wins the precedence, but the disagreement is
/// carried in [`RowIdentityVerdict::proven_mismatch`] rather than silently
/// dropped.
pub fn row_identity(declared_unique_key: &[String], sql: &str) -> RowIdentityVerdict {
    let proven_key = model_property_vector(sql, &JoinContext::new()).and_then(|vector| {
        if vector.has_fan_out_join {
            None
        } else {
            vector.grain.keys.into_iter().next()
        }
    });

    if !declared_unique_key.is_empty() {
        let declared = declared_unique_key.to_vec();
        let proven_mismatch = proven_key.filter(|proven| !same_key_set(proven, &declared));
        return RowIdentityVerdict {
            identity: RowIdentity::Key(declared),
            proven_mismatch,
        };
    }

    match proven_key {
        Some(key) => RowIdentityVerdict {
            identity: RowIdentity::Key(key),
            proven_mismatch: None,
        },
        None => RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
    }
}

/// Order-independent, case-insensitive key-set equality — the same
/// convention `Grain::has_subset_key` and the key-temporal-locality route's
/// `unique_key` comparison use.
fn same_key_set(a: &[String], b: &[String]) -> bool {
    let a: BTreeSet<String> = a.iter().map(|c| c.to_ascii_lowercase()).collect();
    let b: BTreeSet<String> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
    a == b
}

/// The [`SourceShape`] [`input_delta_discovery`] reads for `facts`: a
/// clocked source's own partition column stands in for
/// `SourceShape::has_clock` (`SourceFacts::partition_col`'s doc comment: "the
/// source's partition column, when it is clocked"), and the plan-layer
/// [`MutationProfile`] maps onto the analysis-layer one 1:1 (v0 has no
/// `ChangeFeed` source in the plan layer yet — `sources.md`'s structured
/// `mutation_profile` kind is consumed at the `MutationProfile::AppendOnly`/
/// `MutableSnapshot` granularity here; a `change_feed` source is out of scope
/// for this phase, per `incremental_models.md` §Known Divergences).
fn source_shape(facts: &SourceFacts) -> SourceShape {
    SourceShape {
        has_clock: facts.partition_col.is_some(),
        mutation_profile: Some(match facts.mutation {
            MutationProfile::AppendOnly => DeltaMutationProfile::AppendOnly,
            MutationProfile::MutableSnapshot => DeltaMutationProfile::Mutable,
        }),
    }
}

/// Caller-supplied fold admission input for a keyed-grain model: which
/// columns fold additively, each under its **own** combiner (a mixed fold —
/// e.g. `COUNT`→`SUM`, `MIN`→`MIN`, `MAX`→`MAX` composed over the same
/// key — is the common shape, not a single shared combiner across every
/// column). Checked against the combiner-algebra classifier per column —
/// never trusted bare.
#[derive(Debug, Clone)]
pub struct FoldSpec {
    pub add_columns: Vec<(String, SqlFunction)>,
}

/// One upstream **maintained-model** edge (`incremental_models.md` §"Upstream
/// model edges"): a downstream maintained model's ref to another maintained
/// model in the same project. Built by the caller from the upstream's own
/// already-validated metadata — the derivation never re-resolves the ref.
#[derive(Debug, Clone)]
pub struct ModelEdge {
    /// The upstream model's address as it appears in the downstream ref, with
    /// the leading `smelt.` stripped (e.g. `silver.events_parsed`). Used as
    /// the edge's `Trigger::NewData` source name and the clamp's source.
    pub name: String,
    /// The upstream's own validated `timeseries.partition_column`, when it
    /// declares (or infers) one. `None` ⇒ the clock is not derivable ⇒ a
    /// recorded [`Refusal::ReachNotDerivable`] naming the edge, never a
    /// silent drop.
    pub clock_col: Option<String>,
}

/// Append the creation-trigger cells (and refusals) for `model_edges` to an
/// already-derived `plan` (`incremental_models.md` §"Upstream model edges").
///
/// Kept separate from [`derive_maintenance_plan`] so every existing
/// source-only caller is unaffected: the assembler calls both and merges the
/// results into one plan (still one derivation, purely data-in/data-out).
///
/// A clocked upstream contributes a `{*}` creation cell whose scan clamp is
/// anchored to the downstream's output partition axis via the same
/// [`link_source`] rule sources use; an upstream with no derivable clock is a
/// [`Refusal::ReachNotDerivable`] naming the edge. Model edges only
/// contribute to a **partition-addressed** downstream (`output_partition_col`
/// is `Some`); a key-addressed downstream's model-edge creation is a keyed
/// fold, out of scope here.
///
/// `declared_unique_key` is the downstream's own declared `unique_key:`
/// (`docs/specs/models.md` §"Refresh axis"), threaded into the same
/// [`row_identity`] derivation `derive_maintenance_plan` uses for this
/// model's other cells, so a model-edge creation cell carries the identical
/// row-identity verdict as every other cell of the same output.
pub fn append_model_edge_cells(
    plan: &mut MaintenancePlan,
    sql: &str,
    output_partition_col: Option<&str>,
    model_edges: &[ModelEdge],
    declared_unique_key: &[String],
) {
    if model_edges.is_empty() {
        return;
    }
    let identity = row_identity(declared_unique_key, sql);
    // A key-addressed downstream has no partition axis to clamp a creation
    // cell to; its model-edge creation would be a keyed fold, deferred.
    let Some(output_partition_col) = output_partition_col else {
        return;
    };

    // Derive per-edge bounds over the downstream SQL, keyed by each clocked
    // edge's clock column — the same Form A/B extraction sources use.
    let mut ctx = BoundContext::new();
    for edge in model_edges {
        if let Some(clock) = &edge.clock_col {
            ctx.add_source(&edge.name, clock);
        }
    }
    let bounds = derive_model_bounds(sql, &ctx);

    for edge in model_edges {
        let Some(clock) = &edge.clock_col else {
            plan.refusals.push(Refusal::ReachNotDerivable {
                edge: edge.name.clone(),
                why: format!(
                    "upstream maintained model '{}' declares no timeseries clock and none is \
                     inferable — its creation-trigger edge cannot be clamped to the output \
                     partition axis",
                    edge.name
                ),
            });
            continue;
        };
        let facts = SourceFacts {
            name: edge.name.clone(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some(clock.clone()),
            unique_key: vec![],
            allow_full_scan: false,
        };
        // A creation-trigger recompute is unconditionally valid (like the
        // `NewData`/`Backfill` region recompute), so an unlinked edge records
        // a non-local verdict but is never refused under the K8 guardrail —
        // only the *underivable-clock* case above refuses.
        let (partition_local, scans) =
            match link_source(Some(output_partition_col), &bounds, &facts) {
                SourceLink::Clamp(clamp) => (PartitionLocal::Yes, vec![clamp]),
                SourceLink::Unlinked { why } => (
                    PartitionLocal::No {
                        source: edge.name.clone(),
                        why,
                    },
                    vec![],
                ),
                // Unreachable: `facts.partition_col` is `Some` by construction.
                SourceLink::Unclocked => (
                    PartitionLocal::No {
                        source: edge.name.clone(),
                        why: "model edge lost its clock column".to_string(),
                    },
                    vec![],
                ),
            };
        plan.cells.push(PlanCell {
            group: "{*}".to_string(),
            trigger: Trigger::NewData {
                source: edge.name.clone(),
            },
            corner: Corner::RecomputeRegion,
            technique: Technique::DeleteInsert,
            partition_local,
            scans,
            ledger_catch_up: false,
            row_identity: identity.clone(),
            skeleton_source_closure: None,
        });
    }
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

    /// The declared identity off the output's own grain (P2, `row_identity`):
    /// `Grain::Key`'s `unique_key`, or nothing for `Grain::Partition` — a
    /// partition-grain output declares no row-level identity through
    /// `Grain` itself.
    fn declared_unique_key(&self) -> &[String] {
        match &self.output.grain {
            Grain::Key { unique_key } => unique_key,
            Grain::Partition { .. } => &[],
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
    let identity = row_identity(inputs.declared_unique_key(), inputs.sql);

    for trigger in triggers {
        match trigger {
            Trigger::NewData { source } => {
                derive_new_data(inputs, &bounds, source, &identity, &mut plan)
            }
            Trigger::UpstreamMutation { source } => {
                derive_mutation(inputs, &bounds, source, &identity, &mut plan)
            }
            Trigger::ColumnAdded { columns } => {
                derive_column_added(inputs, &bounds, columns, &identity, &mut plan)
            }
            Trigger::Backfill => derive_backfill(inputs, &bounds, &identity, &mut plan),
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
    identity: &RowIdentityVerdict,
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
                row_identity: identity.clone(),
                skeleton_source_closure: None,
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
            // Per-cell admission obligation 2 (`incremental_models.md`
            // §"Per-cell admission"): the faithful fold's two INDEPENDENT
            // conditions — source posture (does the delta stream partition
            // the input, i.e. is it retraction-free) and combiner algebra
            // (can a retracted contribution be undone) — either failing
            // alone refuses the fold family for this cell
            // (`model_properties.md` §"Faithful-fold conditions"). Obligation
            // 3 (combiner algebra class) is checked independently of source
            // posture: a holistic/unrecognised combiner refuses regardless of
            // how clean the source is, and leaves only the recompute family
            // admissible for this cell (no fold cell is synthesized in v0 —
            // `derive_backfill`/a declared `full` refresh is that family's
            // representative today; wiring the fallback as an alternate
            // technique inside the same cell is deferred, since v0 admits at
            // most one technique per cell). Checked per column below —
            // obligation 3 is independent per combiner, so a mixed fold
            // (e.g. `SUM` alongside `MIN`/`MAX`) refuses as a whole the
            // moment any one column's combiner fails it.

            // Obligation 2, source-posture half: `input_delta_discovery` is
            // the SC-2 tripwire's (`docs/research/property-discovery/
            // ledger.md`) production consumer. A clocked `Mutable` source's
            // `WindowForward` discovery only proves *how new rows are found*
            // — it has no branch for an in-place update to an
            // already-processed partition, so it can never by itself widen a
            // source to "retraction-free". The declared `MutationProfile`
            // remains the sole source of that fact (never derived from
            // discovery kind alone) — this is the explicit
            // `MutationProfile::Mutable` guard the (now-deleted) dead-code
            // tripwire required of its first production caller.
            let discovery = input_delta_discovery(source_shape(facts));
            let carries_retractions = facts.mutation != MutationProfile::AppendOnly;
            if carries_retractions {
                if discovery == InputDeltaKind::WindowForward {
                    // The blind spot the (now-deleted) dead-code tripwire
                    // required a human sign-off before wiring: a clocked
                    // Mutable source's discovery kind is WindowForward, but
                    // that kind only proves how *new* rows are found — it has
                    // no branch for an in-place update to an already-scanned
                    // partition. A window-forward incremental read would
                    // never re-visit that partition at all, so the retracted
                    // contribution is not merely un-undoable, it is silently
                    // invisible to the next run. Name this specific blind
                    // spot distinctly from the unclocked case below, where a
                    // full re-scan at least *sees* the change (SC-2,
                    // `docs/research/property-discovery/ledger.md`).
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!(
                            "fold over '{source}' fails the faithful-fold source-posture \
                             condition: the source is not append-only, and input-delta \
                             discovery classifies it as window-forward (clocked) — a \
                             window-forward incremental read only visits new partitions, \
                             so an in-place update to an already-processed partition would \
                             go entirely unseen by the next run, not merely un-undoable; no \
                             un-fold mechanism exists to undo an already-folded contribution \
                             either, so this refuses the fold family whether or not any of the \
                             fold's combiners ({:?}) are themselves monoids — the two \
                             faithful-fold conditions are independent and either alone refuses",
                            fold.add_columns.iter().map(|(_, c)| *c).collect::<Vec<_>>()
                        ),
                    });
                } else {
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!(
                            "fold over '{source}' fails the faithful-fold source-posture \
                             condition: the source is not append-only and may carry \
                             retractions (input-delta discovery = {discovery:?}); no un-fold \
                             mechanism exists to undo an already-folded contribution, so this \
                             refuses the fold family whether or not any of the fold's combiners \
                             ({:?}) are themselves monoids — the two faithful-fold conditions \
                             are independent and either alone refuses",
                            fold.add_columns.iter().map(|(_, c)| *c).collect::<Vec<_>>()
                        ),
                    });
                }
                return;
            }

            // Obligation 3: combiner algebra class, checked independently of
            // the (already-passed) source-posture condition above, per
            // column — a mixed-combiner fold refuses as a whole (fail-closed,
            // not a partial fold) the moment any one column's combiner is
            // not a monoid.
            if let Some((column, combiner)) = fold.add_columns.iter().find_map(|(name, c)| {
                (!combiner_discriminants(*c, false).is_monoid).then_some((name.clone(), *c))
            }) {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!(
                        "combiner {combiner:?} for column '{column}' is holistic or \
                         unrecognised (not a monoid) — no delta+state read exists; only the \
                         recompute family (a full rebuild) can serve this cell",
                    ),
                });
                return;
            }
            plan.cells.push(PlanCell {
                group: format!(
                    "{{{}}}",
                    fold.add_columns
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                trigger,
                corner: Corner::FoldDelta,
                technique: Technique::KeyedFold,
                // Keyed end-state: the write is key-addressed, not
                // partition-addressed; there is no partition axis to bound.
                partition_local: PartitionLocal::Yes,
                scans: vec![],
                ledger_catch_up: false,
                row_identity: identity.clone(),
                skeleton_source_closure: None,
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
    identity: &RowIdentityVerdict,
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
            row_identity: identity.clone(),
            skeleton_source_closure: None,
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
    identity: &RowIdentityVerdict,
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
                    row_identity: identity.clone(),
                    skeleton_source_closure: None,
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
            row_identity: identity.clone(),
            skeleton_source_closure: None,
        });
    }
}

/// Backfill: the universal ground-truth reset — recompute the region from
/// replayable input, unconditionally correct (`01-framework.md` §3).
fn derive_backfill(
    inputs: &ModelInputs,
    bounds: &HashMap<String, BoundResult>,
    identity: &RowIdentityVerdict,
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
        row_identity: identity.clone(),
        skeleton_source_closure: None,
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
