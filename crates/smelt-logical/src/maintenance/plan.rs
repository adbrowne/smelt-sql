use super::*;

use std::collections::BTreeMap;

use crate::analysis::walk::{ColumnComparability, Comparability};

/// The admitted key-temporal-locality verdict for a `grain: key` model that
/// also declares a `timeseries:` block
/// (`locality::establish_locality`'s admitted result, plus the derived
/// settle bound). Carried on [`MaintenancePlan`] so `smelt-db` and `smelt
/// explain` can fold the already-admitted verdict into `Grain::Key`'s plan
/// shape and the explain surface without re-deriving admission — the
/// single derivation in `locality.rs` is the only place that decides both
/// (`CLAUDE.md` §"Maintenance-plan purity": "derived once by pure
/// functions … consumers never re-derive it").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLocality {
    /// The admitted slice a `merge_into` target scan may be pruned to.
    pub slice: locality::LocalitySlice,
    /// How long a written slice may still change before it is safe to
    /// treat as final (route 2 is honestly [`locality::SettleBound::Never`],
    /// never a large sentinel duration).
    pub settle_bound: locality::SettleBound,
}

/// The derived maintenance plan: admitted cells plus fail-loud refusals.
#[derive(Debug, Clone, Default)]
pub struct MaintenancePlan {
    pub cells: Vec<PlanCell>,
    pub refusals: Vec<Refusal>,
    /// The admitted key-temporal-locality verdict, for a `grain: key` model
    /// whose `timeseries:` block cleared the locality gate. `None` for a
    /// `grain: partition` model, a `grain: key` model with no `timeseries:`
    /// block, or a locality refusal (in which case the plan is
    /// [`locality_refused_plan`]'s no-cells shape instead).
    pub key_locality: Option<KeyLocality>,
}

impl MaintenancePlan {
    /// The FIRST admitted cell for `trigger`, if any (v0 plans hold at most
    /// one cell per trigger × group, but a trigger commonly has MULTIPLE
    /// sibling cells — one per membership-sensitive group a shared join
    /// admits, `docs/plans/20260808-membership-sensitivity.md` Phase 1).
    ///
    /// **First-match, not "the" cell.** A caller resolving a per-cell
    /// override (`maintenance.cells[].technique`/`prefer`/`write`) must
    /// never use this alone to decide admissibility — the override's own
    /// `columns` may address a DIFFERENT sibling cell than whichever one
    /// this happens to return first. Use [`Self::cells_for`] and match each
    /// sibling's own derived column group instead (`docs/plans/
    /// 20260808-membership-sensitivity.md` Phase 3's own fix — the bug this
    /// doc comment now flags: `smelt-runtime`'s pin-resolution loops used to
    /// call this and evaluate overrides against only the first sibling,
    /// silently never consulting a pin scoped to a later sibling's
    /// columns). Safe call sites for `cell_for` alone are ones that only
    /// ever derive a single cell per trigger for their own shape (e.g. a
    /// `NewData`/`Backfill` whole-row `{*}` trigger, or a keyed model's
    /// single-group recipe) — read the call site's own shape before adding
    /// a new one.
    pub fn cell_for(&self, trigger: &Trigger) -> Option<&PlanCell> {
        self.cells.iter().find(|c| &c.trigger == trigger)
    }

    /// Every admitted cell sharing `trigger` — one per column group the
    /// trigger's mutation source contributes sensitivity to. Iteration
    /// order matches `self.cells`' own derivation order (not
    /// group-name-sorted); a caller that needs to pick ONE specific sibling
    /// must match on the sibling's own derived group/columns, never rely on
    /// this order to mean anything.
    pub fn cells_for<'a, 'b>(
        &'a self,
        trigger: &'b Trigger,
    ) -> impl Iterator<Item = &'a PlanCell> + use<'a, 'b> {
        self.cells.iter().filter(move |c| &c.trigger == trigger)
    }
}

/// The plan tracking `grain:` shapes maintenance-plan derivation does not yet
/// support (`Refusal::UnsupportedGrain`'s `tracking_plan`).
pub const UNSUPPORTED_GRAIN_TRACKING_PLAN: &str =
    "docs/plans/20260715-composed-axes-conditional-maintenance.md";

/// The plan derived for a `grain:` this phase of derivation does not yet
/// support: no cells, a single [`Refusal::UnsupportedGrain`] naming `grain`
/// and [`UNSUPPORTED_GRAIN_TRACKING_PLAN`]. There is nothing meaningful to
/// derive for an unsupported grain, so this bypasses
/// [`derive::derive_maintenance_plan`] entirely rather than feeding it inputs
/// built from a grain shape it was never taught to admit.
pub fn unsupported_grain_plan(grain: &str) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::UnsupportedGrain {
            grain: grain.to_string(),
            tracking_plan: UNSUPPORTED_GRAIN_TRACKING_PLAN.to_string(),
        }],
        key_locality: None,
    }
}

/// The plan derived when the locality gate
/// ([`locality::establish_locality`]) refuses a keyed model's `timeseries:`
/// block: no cells, a single [`Refusal::LocalityNotEstablished`] carrying
/// the rendered `KeyedForbidsTimeseries` message. Bypasses
/// [`derive::derive_maintenance_plan`] entirely — there is nothing
/// meaningful to derive for a keyed output whose partitioning was never
/// admitted.
pub fn locality_refused_plan(message: String) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::LocalityNotEstablished { message }],
        key_locality: None,
    }
}

/// The plan derived when the keyed-succession leaf classifier
/// (`analysis::succession::classify_keyed_succession`) returns
/// `NotSuccession`: no cells, a single [`Refusal::SuccessionNotRecognized`]
/// carrying the classifier's reason verbatim.
pub fn succession_refused_plan(
    reason: crate::analysis::succession::NotSuccessionReason,
) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::SuccessionNotRecognized { reason }],
        key_locality: None,
    }
}

/// The plan derived when a declared `key_recurrence` disagrees with route
/// 3's statically-derived recurrence bound
/// ([`locality::LocalityRefusal::RecurrenceDeclarationMismatch`],
/// key-grain rule 16): no cells, a single
/// [`Refusal::KeyedRecurrenceDeclarationMismatch`] carrying the rendered
/// message. Bypasses [`derive::derive_maintenance_plan`] entirely — the
/// derived bound is authoritative, so a disagreeing declaration blocks the
/// plan the same way any other locality refusal does.
pub fn recurrence_mismatch_plan(message: String) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::KeyedRecurrenceDeclarationMismatch { message }],
        key_locality: None,
    }
}

/// The plan derived when a `grain: key` model has no derivable identity at
/// all — no declared top-level `unique_key:` and an empty GROUP-BY-derived
/// key. Bypasses [`derive::derive_maintenance_plan`] entirely — there is no
/// identity to fold cells around.
pub fn identity_not_derivable_plan(message: String) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::IdentityNotDerivable { message }],
        key_locality: None,
    }
}

/// Fold a model's declared per-column equivalence contract
/// (`columns.<c>.contract:`, `smelt_core::metadata::Contract`) over the
/// walk's derived change-comparability verdict
/// (`analysis::walk::PropertyVector::comparability`,
/// `model_properties.md` §"Change comparability"): a `contract: plausible`
/// column is `Incomparable` regardless of what the walk proved — the walk
/// only sees the query's own SQL and cannot know that a payload column's
/// non-determinism has been accepted by the modeller as an equivalence
/// contract, so the override is applied here, where the derived vector meets
/// the model's declared metadata, not inside the walk itself. Widen-never:
/// a column the walk already proved `Comparable` and that carries no
/// `plausible` declaration passes through unchanged.
///
/// This is plain plumbing — carrying the verdict on a plan-facing type — no
/// admission or emitter reads the result yet.
pub fn column_comparability_with_contract(
    walk_comparability: &[ColumnComparability],
    plausible_columns: &BTreeMap<String, smelt_core::metadata::Contract>,
) -> Vec<ColumnComparability> {
    walk_comparability
        .iter()
        .map(|c| {
            let is_plausible = plausible_columns
                .get(&c.output.to_ascii_lowercase())
                .is_some_and(|contract| *contract == smelt_core::metadata::Contract::Plausible);
            let comparability = if is_plausible {
                Comparability::Incomparable
            } else {
                c.comparability
            };
            ColumnComparability {
                output: c.output.clone(),
                comparability,
            }
        })
        .collect()
}
