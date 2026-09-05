//! The property profile (`docs/specs/property_diff.md` §"The property
//! profile"): the pure, serialisable record of every composition-relevant
//! verdict the maintenance report prints for one model. Single-owned here
//! (`CLAUDE.md` §Architectural invariants — "Profile single ownership",
//! `docs/specs/property_diff.md` §Constraints item 1) so the single-version
//! report (`smelt_runtime::diagnostics::ModelDiagnostics`, `smelt explain`)
//! and the property diff (`property_diff.md`, a later phase) render from the
//! identical value rather than each deriving their own.
//!
//! [`PropertySet`] and its `derive` constructor moved here, verbatim, from
//! `smelt-runtime::diagnostics` — every input they read
//! ([`crate::analysis::walk::model_property_vector`],
//! [`crate::maintenance::derive::row_identity`],
//! [`crate::analysis::source_bounds::derive_model_bounds`]) already lived in
//! `smelt-logical`, so this is a refactor, not a new derivation
//! (`docs/outcomes/20260905-property-diff/phases/02-plan.md` §"Design
//! decisions").

use std::collections::BTreeMap;

use serde::Serialize;

use crate::analysis::join_shape::JoinContext;
use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult};
use crate::analysis::walk::{
    model_property_vector, ColumnComparability, ColumnDeterminism, ColumnDiscriminant, DerivedFd,
    Grain,
};
use crate::contract::ContractPointView;
use crate::maintenance::derive::row_identity;
use crate::maintenance::{refusal_code, PlanCell, Refusal, RowIdentityVerdict};

/// Errors the property-profile derivation can surface. Fail-loud
/// (`CLAUDE.md` §"Fail-loud discipline"): a model whose SQL cannot be
/// classified into a property vector is reported as an error, never
/// silently defaulted to an empty/optimistic property set.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("could not derive the property set for model {model}: SQL did not parse into an analyzable query tree")]
    PropertyDerivation { model: String },
}

/// The model's full derived-property set (`docs/specs/model_properties.md`
/// §Surface), serialized from the existing single-owner walk output
/// ([`model_property_vector`]) plus the two other already-derived,
/// single-call facts reachable at whole-model scope: region row identity
/// ([`row_identity`]) and per-source bound/reach ([`derive_model_bounds`]).
/// This struct never re-derives any of these facts — it is an adapter over
/// the existing walk/derive outputs, adding `Serialize` and giving the
/// composed shape one name.
///
/// Scope note: `model_properties.md` §Surface catalogues properties beyond
/// what is folded into a single top-level `PropertyVector`/bound-map call —
/// several (event-time monotonicity trace, partition alignment, fan-out/
/// cardinality, skeleton-role extraction, …) are scope-, join-, or
/// column-position facts the walk computes internally per node but does not
/// yet expose as a single whole-model derivation, and several catalogue rows
/// are themselves `not-yet`/`partial` maturity in the spec. `PropertySet`
/// covers every `built`-maturity property reachable from one already-derived
/// per-model call; extending it to the remaining catalogue rows needs new
/// plumbing to locate their inputs (e.g. the event-time expression's AST
/// node) and is left to a follow-up phase rather than invented here.
#[derive(Debug, Clone, Serialize)]
pub struct PropertySet {
    /// Output columns of the model, in projection order.
    pub columns: Vec<String>,
    /// The proven grain (keys). Empty ⇒ unkeyed
    /// (`model_properties.md` §"Fan-out / cardinality" — the grain a
    /// conditional write's row identity is built from, see `row_identity`
    /// below).
    pub grain: Grain,
    /// Query-derived functional dependencies (`model_properties.md` — the
    /// FD set implied by grain + literal columns).
    pub functional_dependencies: Vec<DerivedFd>,
    /// Per-column determinism (`model_properties.md` §"Determinism (run vs
    /// row) and the nondeterminism predicate").
    pub determinism: Vec<ColumnDeterminism>,
    /// Per-column change-comparability (`model_properties.md` §"Change
    /// comparability").
    pub comparability: Vec<ColumnComparability>,
    /// Per-column aggregate discriminants (`model_properties.md` §"Algebraic
    /// discriminants").
    pub discriminants: Vec<ColumnDiscriminant>,
    /// Output columns that are constant literals here, name → literal text.
    pub literal_columns: Vec<(String, String)>,
    /// Whether an output column crosses a set operation whose branches are
    /// not proven key-disjoint — a structural barrier for FD survival.
    pub has_set_op_barrier: bool,
    /// Whether an input join proves `OneToMany` (row-multiplying).
    pub has_fan_out_join: bool,
    /// The model's own region row identity (`model_properties.md` §"Region
    /// row identity"): declared `unique_key` → proven grain key → the
    /// identity-free `WholeRow` fallback.
    pub row_identity: RowIdentityVerdict,
    /// Per-upstream-source bound/reach (`model_properties.md` §"Unified
    /// bound / reach derivation"), keyed by source name.
    pub source_bounds: BTreeMap<String, BoundResult>,
}

impl PropertySet {
    /// Derive a model's [`PropertySet`] from its (frontmatter-stripped) SQL,
    /// its declared `unique_key`, and a caller-built [`BoundContext`]
    /// (mirroring `smelt-cli::explain::compute_source_bounds`'s own
    /// construction: one `BoundContext::add_source` per upstream source with
    /// a declared timeseries clock).
    pub fn derive(
        model_name: &str,
        sql: &str,
        declared_unique_key: &[String],
        bound_ctx: &BoundContext,
    ) -> Result<Self, ProfileError> {
        let vector = model_property_vector(sql, &JoinContext::new()).ok_or_else(|| {
            ProfileError::PropertyDerivation {
                model: model_name.to_string(),
            }
        })?;
        let identity = row_identity(declared_unique_key, sql);
        let source_bounds: BTreeMap<String, BoundResult> =
            derive_model_bounds(sql, bound_ctx).into_iter().collect();

        Ok(PropertySet {
            columns: vector.columns,
            grain: vector.grain,
            functional_dependencies: vector.fds,
            determinism: vector.determinism,
            comparability: vector.comparability,
            discriminants: vector.discriminants,
            literal_columns: vector.literal_columns,
            has_set_op_barrier: vector.has_set_op_barrier,
            has_fan_out_join: vector.has_fan_out_join,
            row_identity: identity,
            source_bounds,
        })
    }
}

/// One maintenance cell's composition-relevant verdict
/// (`docs/specs/property_diff.md` §"The property profile" item 2): the same
/// scalar fields `smelt explain --json` renders per cell, read once here so
/// the report and the profile can never disagree about them
/// (`docs/specs/property_diff.md` §Constraints item 4, "Report/profile
/// parity"). Deliberately narrower than
/// `smelt_runtime::diagnostics::PlanCellDiagnostics` (which also carries the
/// full technique-preview matrix, out of scope for a property verdict).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CellVerdict {
    /// Display name of the cell's column group (`smelt_logical::maintenance::PlanCell::group`).
    pub group: String,
    /// `{:?}`-rendered `smelt_logical::maintenance::Trigger`.
    pub trigger: String,
    /// `{:?}`-rendered `smelt_logical::maintenance::Corner`.
    pub corner: String,
    /// `{:?}`-rendered `smelt_logical::maintenance::Technique` — the
    /// technique the derived plan actually admitted for this cell.
    pub technique: String,
    /// The cell's own region row identity (P2, `model_properties.md`
    /// §"Region row identity").
    pub row_identity: RowIdentityVerdict,
    /// The cell's effective contract lattice point
    /// (`docs/specs/incremental_models.md` §"The contract lattice"),
    /// resolved through the single-owner
    /// [`crate::contract::effective_contract`], never re-resolved by a
    /// renderer.
    pub contract_point: ContractPointView,
}

/// Render one [`PlanCell`]'s [`CellVerdict`] — the single place a cell's
/// trigger/corner/technique are turned into the `{:?}` strings both
/// `smelt explain`'s report and its `--json` form show
/// (`docs/specs/property_diff.md` §"The property profile"). A renderer
/// consumes this value rather than re-deriving the strings itself.
pub fn render_cell_verdict(cell: &PlanCell, contract_point: ContractPointView) -> CellVerdict {
    CellVerdict {
        group: cell.group.clone(),
        trigger: format!("{:?}", cell.trigger),
        corner: format!("{:?}", cell.corner),
        technique: format!("{:?}", cell.technique),
        row_identity: cell.row_identity.clone(),
        contract_point,
    }
}

/// One maintenance admission refusal, as the property profile carries it
/// (`docs/specs/property_diff.md` §"The property profile" item 3): the
/// diagnostic code's **name** (`DiagnosticCode` lives in `smelt-db`, above
/// `smelt-logical` — layered single-ownership, ruling R1.3) plus the
/// report's own `{:?}` rendering of the refusal, verbatim
/// (`crates/smelt-cli/src/explain.rs`'s existing "Refusals" section).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProfileRefusal {
    pub code: String,
    pub text: String,
}

impl ProfileRefusal {
    /// Build a [`ProfileRefusal`] from a derived-plan [`Refusal`] — `code`
    /// from [`refusal_code`], `text` the same `{:?}` string the text report
    /// prints, so the two can never drift.
    pub fn from_refusal(refusal: &Refusal) -> Self {
        ProfileRefusal {
            code: refusal_code(refusal).to_string(),
            text: format!("{refusal:?}"),
        }
    }
}

/// One declared-fact probe entry, as the property profile carries it
/// (`docs/specs/property_diff.md` §"The property profile" item 4) —
/// byte-identical fields to
/// [`crate::analysis::profile::ProbePlanEntry`] minus the offline `cost`
/// rendering, which stays a `smelt-cli`/`smelt-ui` presentation concern.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProfileProbe {
    pub fact: String,
    pub probe: String,
    pub cell: String,
}

/// One declared-fact probe descriptor this model would dispatch on a
/// consuming run — the fact, its named diagnostic, the maintenance cell it
/// licenses, and its static per-run cost. Never carries executable SQL:
/// `smelt explain` stays offline (`docs/specs/cli.md` §"`smelt explain
/// <model>` maintenance-plan report").
///
/// Moved here, verbatim, from `smelt-runtime::probe_plan` — the *struct*
/// only; its builder (`probe_plan_for_model`) stays in `smelt-runtime`
/// because it needs `smelt-backend`/`smelt-state`, both above
/// `smelt-logical` (`docs/outcomes/20260905-property-diff/phases/02-plan.md`
/// task 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbePlanEntry {
    pub fact: String,
    pub probe: String,
    pub cell: String,
    pub cost: String,
}

/// A model's full property profile (`docs/specs/property_diff.md` §"The
/// property profile"): one pure value per model per project version, the
/// single-owned record every renderer (the single-version report, and,
/// later, the property diff) reads rather than re-derives
/// (`docs/specs/property_diff.md` §Constraints item 1). Deliberately carries
/// **no model name** — a diff keys a `BTreeMap<String, PropertyProfile>` by
/// name at the caller.
#[derive(Debug, Clone, Serialize)]
pub struct PropertyProfile {
    pub properties: PropertySet,
    pub cell_verdicts: Vec<CellVerdict>,
    pub refusals: Vec<ProfileRefusal>,
    pub probes: Vec<ProfileProbe>,
}

impl PropertyProfile {
    /// Assemble a [`PropertyProfile`] from already-derived inputs — a pure
    /// composition, no walk call beyond [`PropertySet::derive`] and no SQL
    /// scan (`docs/outcomes/20260905-property-diff/phases/02-plan.md`
    /// §"Design decisions" — "Refactor, not new derivation"). `plan_cells`
    /// and `contract_points` are the model's already-derived
    /// `MaintenancePlan::cells` and each cell's already-resolved
    /// `ContractPoint` (`crate::contract::effective_contract`), in the same
    /// order — the caller resolves the contract point per cell because that
    /// needs the cell's own trigger address and column-group membership,
    /// which live in a shape below `smelt-logical`
    /// (`smelt_core::config::ContractConfig`) but are looked up by the
    /// caller, not derived twice here.
    #[allow(clippy::too_many_arguments)]
    pub fn assemble(
        properties: PropertySet,
        plan_cells: &[PlanCell],
        contract_points: &[ContractPointView],
        refusals: &[Refusal],
        probe_entries: &[ProbePlanEntry],
    ) -> Self {
        let cell_verdicts = plan_cells
            .iter()
            .zip(contract_points.iter())
            .map(|(cell, contract_point)| render_cell_verdict(cell, contract_point.clone()))
            .collect();
        let refusals = refusals.iter().map(ProfileRefusal::from_refusal).collect();
        let probes = probe_entries
            .iter()
            .map(|p| ProfileProbe {
                fact: p.fact.clone(),
                probe: p.probe.clone(),
                cell: p.cell.clone(),
            })
            .collect();
        PropertyProfile {
            properties,
            cell_verdicts,
            refusals,
            probes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance::{Corner, PartitionLocal, RowIdentity, Technique, Trigger};

    fn sample_cell() -> PlanCell {
        PlanCell {
            group: "{amount}".to_string(),
            trigger: Trigger::NewData {
                source: "raw.orders".to_string(),
            },
            corner: Corner::RecomputeRegion,
            technique: Technique::KeyedFold,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::Key(vec!["id".to_string()]),
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: Default::default(),
            key_scope: None,
        }
    }

    /// The moved `derive` over a `GROUP BY` model yields the grain/columns
    /// the pre-move `smelt-runtime` units asserted
    /// (`docs/outcomes/20260905-property-diff/phases/02-plan.md` test 1).
    #[test]
    fn property_set_moves_intact() {
        let sql = "SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id";
        let bound_ctx = BoundContext::default();
        let set = PropertySet::derive("orders_by_customer", sql, &[], &bound_ctx)
            .expect("a simple GROUP BY must derive a property set");
        assert_eq!(set.columns, vec!["customer_id", "total"]);
        assert!(
            set.grain
                .keys
                .iter()
                .any(|k| k.iter().any(|c| c == "customer_id")),
            "GROUP BY customer_id must prove a grain key over customer_id; got {:?}",
            set.grain
        );
    }

    #[test]
    fn property_set_derive_fails_loud_on_unparsable_sql() {
        let bound_ctx = BoundContext::default();
        let err = PropertySet::derive("broken", "not valid sql at all (((", &[], &bound_ctx)
            .expect_err("unparsable SQL must fail loud, never default to an empty property set");
        assert!(matches!(err, ProfileError::PropertyDerivation { .. }));
    }

    /// Two cells (one admitted `KeyedFold`), one `ScanUnbounded` refusal, one
    /// probe → the profile assembles the expected verdicts, and a
    /// no-`contract:` cell's `contract_point` renders the default point
    /// (`docs/outcomes/20260905-property-diff/phases/02-plan.md` test 4).
    #[test]
    fn profile_assembles_cells_refusals_probes() {
        let sql = "SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id";
        let bound_ctx = BoundContext::default();
        let properties = PropertySet::derive("orders_by_customer", sql, &[], &bound_ctx).unwrap();

        let cell = sample_cell();
        let contract_point =
            crate::contract::effective_contract(None, "raw.orders", &["amount".to_string()]).into();
        let refusal = Refusal::ScanUnbounded {
            source: "raw.orders".to_string(),
            why: "no partition_column declared".to_string(),
        };
        let probe = ProbePlanEntry {
            fact: "assert_monotonic".to_string(),
            probe: "MonotonicityViolated".to_string(),
            cell: "main.orders_by_customer (declared)".to_string(),
            cost: "+1 query per consuming run".to_string(),
        };

        let profile = PropertyProfile::assemble(
            properties,
            std::slice::from_ref(&cell),
            &[contract_point],
            &[refusal],
            &[probe],
        );

        assert_eq!(profile.cell_verdicts.len(), 1);
        let verdict = &profile.cell_verdicts[0];
        assert_eq!(verdict.group, "{amount}");
        assert_eq!(verdict.technique, "KeyedFold");
        assert!(verdict.contract_point.is_default());

        assert_eq!(profile.refusals.len(), 1);
        assert_eq!(profile.refusals[0].code, "MaintenanceScanUnbounded");
        assert!(profile.refusals[0].text.contains("ScanUnbounded"));

        assert_eq!(profile.probes.len(), 1);
        assert_eq!(profile.probes[0].fact, "assert_monotonic");
    }
}
