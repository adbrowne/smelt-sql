//! Pure derivation of a [`MaintenancePlan`] from analysis facts — v0.
//!
//! Consumes the derivations that exist (`analysis::source_bounds` for reach,
//! `analysis::discriminants` for combiner algebra, `analysis::model_diff` for
//! the additive-only column-add proof) and takes as *inputs* the two
//! classifiers that do not exist yet (column groups, skeleton roles) — see
//! the module doc in [`super`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{ColumnRef, Expr};
use smelt_types::SqlFunction;

use super::{
    ColumnGroup, Corner, FingerprintProjection, Grain, MaintenancePlan, MutationProfile,
    OutputSpec, PartitionLocal, PlanCell, Refusal, RowIdentity, RowIdentityVerdict, ScanClamp,
    SourceFacts, Technique, Trigger,
};
use crate::analysis::definition_change::{
    classify_definition_change, DefinitionChangeClass, DefinitionChangeCtx,
};
use crate::analysis::discriminants::combiner_discriminants;
use crate::analysis::faithful_fold::{faithful_fold, ConditionVerdict, FaithfulFold};
use crate::analysis::fingerprint::fingerprint_projection;
use crate::analysis::footprint::{reflect_footprint, FootprintResult};
use crate::analysis::input_delta::{
    input_delta_discovery, MutationProfile as DeltaMutationProfile, SourceShape,
};
use crate::analysis::join_shape::{
    self, join_contribution_monotone, ContributionVerdict, JoinContext,
};
use crate::analysis::locality_projection::{locality_verdict, LocalityVerdict};
use crate::analysis::model_diff::ColumnDef;
use crate::analysis::output_delta::OutputDelta;
use crate::analysis::source_bounds::{
    derive_cross_axis_links, derive_model_bounds, resolve_table_ref_source_name, BoundContext,
    BoundResult, CrossAxisLink, Seconds,
};
use crate::analysis::walk::model_property_vector;
use crate::analysis::{item_alias, item_expr, select_stmt_items, SelectItemKind};
use crate::maintenance::repair;
use crate::maintenance::skeleton::skeleton_roles;

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
    // join-context: excluded (the general per-cell row-identity proof, not a
    // model-edge/repair admission route — out of scope for
    // `docs/outcomes/20260904-walk-migration-residue/outcome.md` phase 5,
    // which only reaches `JoinContext`-taking maintenance-cell routes)
    row_identity_with_context(declared_unique_key, sql, &JoinContext::new())
}

/// [`row_identity`], but folding an explicit [`JoinContext`] into the walk's
/// fan-out check instead of an always-empty one. Used by
/// [`append_model_edge_cells`] (T3, `docs/plans/20260715-composed-axes-
/// conditional-maintenance.md` Phase E3) so a model-edge cell's row-identity
/// proof can trust a proven grain key across an enrichment join whose
/// partner's row-uniqueness is already an established fact — the SAME
/// per-edge declared `unique_key` fact [`model_edge_enrichment_closure`]'s
/// P1 proof already folds into its own `ctx` for the identical join, never a
/// second, independent guess at the partner's uniqueness. Every other caller
/// (via [`row_identity`]) is unaffected — an empty `ctx` reproduces exactly
/// the pre-existing fail-closed behaviour (any join is untrusted absent an
/// external fact).
pub fn row_identity_with_context(
    declared_unique_key: &[String],
    sql: &str,
    ctx: &JoinContext,
) -> RowIdentityVerdict {
    let proven_key = model_property_vector(sql, ctx).and_then(|vector| {
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

/// Derive a model's full [`Trigger`] set from pure facts — the single
/// derivation `docs/specs/incremental_models.md` §"Per-cell admission"'s
/// "Which changed inputs get a mutation cell" paragraph describes. One
/// `Trigger::NewData` per distinct source (declaration order,
/// first-occurrence deduplicated — a source repeated in `sources`, e.g. once
/// per alias, contributes exactly one creation trigger), one
/// `Trigger::UpstreamMutation` per source the rule below admits, always
/// `Trigger::Backfill`, and `Trigger::ColumnAdded` iff `added_columns` is
/// non-empty.
///
/// A source gets an `UpstreamMutation` cell iff it **explicitly** declares
/// `mutation_profile: mutable_snapshot` (named in `explicitly_mutable` — the
/// fail-closed admission default alone never synthesises one; an undeclared
/// source is not silently treated as mutable) **or** it is `AppendOnly` and
/// named in some [`ColumnGroup::mutation_sensitivity`] (a late append into an
/// already-written region changes stored values, so that region is
/// maintained, not left stale).
///
/// The source's clock (`SourceFacts::partition_col`) is deliberately not
/// part of this rule: whether the resulting cell's scan can be clamped to
/// the output partition axis is a downstream *admission* question
/// (`project_source_link`'s locality proof), not a derivation-time gate. A
/// clocked mutable source whose scan the locality proof cannot clamp
/// surfaces the ordinary `MaintenanceScanUnbounded` refusal — escapable by
/// `allow_full_scan` / `scan_bounds.on_violation: warn` — the same loud path
/// an unclocked one already takes, never a silently-dropped cell.
pub fn derive_triggers(
    sources: &[SourceFacts],
    column_groups: &[ColumnGroup],
    explicitly_mutable: &HashSet<String>,
    added_columns: &[String],
) -> Vec<Trigger> {
    let mut triggers = Vec::new();
    let mut seen = BTreeSet::new();
    for s in sources {
        if !seen.insert(s.name.clone()) {
            continue;
        }
        triggers.push(Trigger::NewData {
            source: s.name.clone(),
        });
        let gets_mutation_cell = match s.mutation {
            MutationProfile::MutableSnapshot => explicitly_mutable.contains(&s.name),
            // A change feed can only arise from an explicit declaration —
            // there is no fail-closed default it could be silently
            // conflated with — so the declaration alone suffices, unlike
            // `MutableSnapshot`'s `explicitly_mutable` gate.
            MutationProfile::ChangeFeed => true,
            MutationProfile::AppendOnly => column_groups
                .iter()
                .any(|g| g.mutation_sensitivity.contains(&s.name)),
        };
        if gets_mutation_cell {
            triggers.push(Trigger::UpstreamMutation {
                source: s.name.clone(),
            });
        }
    }
    triggers.push(Trigger::Backfill);
    if !added_columns.is_empty() {
        triggers.push(Trigger::ColumnAdded {
            columns: added_columns.to_vec(),
        });
    }
    triggers
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
/// [`MutationProfile`] maps onto the analysis-layer one 1:1.
fn source_shape(facts: &SourceFacts) -> SourceShape {
    SourceShape {
        has_clock: facts.partition_col.is_some(),
        mutation_profile: Some(match facts.mutation {
            MutationProfile::AppendOnly => DeltaMutationProfile::AppendOnly,
            MutationProfile::MutableSnapshot => DeltaMutationProfile::Mutable,
            MutationProfile::ChangeFeed => DeltaMutationProfile::ChangeFeed,
        }),
    }
}

mod backfill;
mod column_added;
mod fold;
mod inputs;
mod model_edge;
mod mutation;
mod new_data;
#[cfg(test)]
mod partition_column_changed_tests;
mod plan;
#[cfg(test)]
mod source_shape_tests;

pub use backfill::group_columns;
pub use column_added::{column_def_from_sql, diff_deployed_columns};
pub use fold::{source_contributes_to_fold, FoldSpec};
pub use inputs::{project_source_link, LocalityInputs, ModelInputs, SourceLink};
pub use model_edge::{append_model_edge_cells, ModelEdge, SourceReferentialIntegrity};
pub use plan::{derive_maintenance_plan, derive_maintenance_plan_with_referential_integrity};

use backfill::{derive_backfill, read_locality};
use column_added::{derive_column_added, partition_column_changed, skeleton_clause_changed};
use model_edge::{mutation_enrichment_closure, source_facts_join_context};
use mutation::derive_mutation;
use new_data::derive_new_data;
