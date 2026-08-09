//! Classifier for the `refresh: incremental` + `grain: key` shape.
//!
//! See `docs/specs/incremental_models.md` §"The key grain (`grain: key`)" for the normative spec.
//! This module classifies the direct-monoid families (additive fold,
//! extremal/lattice fold), the order-monotone overwrite family
//! (`MAX_BY`/`MIN_BY`, `docs/plans/20260809-keyed-frontier.md` Phase 1),
//! the plain-overwrite family (`ANY_VALUE`, Phase 3), and the once-write
//! family (`COALESCE`, Phase 4) — across both derived run shapes
//! (window-forward / snapshot-reconcile, §"The two run shapes").
//!
//! The classifier is a pure function that reads an inlined SELECT
//! (post function expansion) plus a small source-timeseries lookup
//! and derives:
//!
//! - `unique_key` — the GROUP BY column list.
//! - `aggregator_columns` — per non-key projection, the
//!   `(per_partition_agg, cross_partition_combiner)` pair from a
//!   fixed allowlist.
//! - `driving_source` — the single source the rule iterates over,
//!   clocked (window-forward) or unclocked (snapshot-reconcile) — the run
//!   shape is derived from which.
//!
//! Returns a `CumulativeClassification` on success or a list of
//! `KeyedDiagnostic`s on rejection.

use serde::Serialize;
use smelt_core::config::{FunctionalDependency, TimeseriesConfig};
use std::collections::HashMap;

use crate::analysis::functional_dependency::{
    functional_dependency_verdict, FunctionalDependencyVerdict,
};
use crate::analysis::join_shape::JoinContext;
use crate::analysis::monotonicity::NONDETERMINISTIC_FUNCTIONS;
use crate::analysis::source_bounds::{resolve_single_anchor, AnchorAmbiguity};
use crate::analysis::walk::{model_property_vector, PropertyVector};
use crate::analysis::{analyze_select, SelectItemKind};
use smelt_types::SqlFunction;

/// A per-partition aggregator paired with its cross-partition combiner.
///
/// The combiner is a fixed lookup off the per-partition aggregator —
/// `COUNT → SUM`, everything else folds onto itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AggregatorColumn {
    /// The output column name (the projection's `AS` alias).
    pub output_name: String,
    /// The SQL aggregate function called on the source rows for one partition.
    pub per_partition_agg: String,
    /// The SQL aggregate function that combines the target's value with
    /// the new partition's value. Stored as a name; the rule renders it
    /// as `f(target.col, delta.col)` or the equivalent (e.g. `LEAST`/`GREATEST`).
    pub cross_partition_combiner: CrossPartitionCombiner,
    /// The hidden decomposed state this column folds through instead of
    /// folding its own presented value directly (`docs/specs/
    /// incremental_models.md` §"Decomposed state (rung 2) in keyed
    /// models"). `Some` for the order-monotone overwrite family
    /// (`MAX_BY`/`MIN_BY`) and the once-write family's fallback-bearing or
    /// multi-candidate spellings; `None` for the once-write family's
    /// key-derived and bare-reduction spellings (never need state) and for
    /// every other column family — admission has not yet widened onto the
    /// mechanism for them (`docs/outcomes/20260809-rung2-state-shapes`
    /// row 7).
    pub state: Option<crate::analysis::decomposed_state::DecomposedState>,
}

/// How target and delta values combine for a column on `MERGE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CrossPartitionCombiner {
    /// `target.c + delta.c` — additive.
    Sum,
    /// `LEAST(target.c, delta.c)` — minimum.
    Min,
    /// `GREATEST(target.c, delta.c)` — maximum.
    Max,
    /// `target.c AND delta.c`.
    BoolAnd,
    /// `target.c OR delta.c`.
    BoolOr,
    /// `target.c & delta.c`.
    BitAnd,
    /// `target.c | delta.c`.
    BitOr,
    /// `xor(target.c, delta.c)` — function form is DuckDB-compatible and works
    /// in Postgres as well; the `#` infix operator is Postgres-only.
    BitXor,
    /// The order-monotone overwrite family (`MAX_BY`/`MIN_BY`,
    /// `incremental_models.md` §"The column-family catalogue"): the delta's
    /// value wins iff its ordering value strictly beats the target's stored
    /// ordering value — incumbent wins on a tie (§"Ordering ties"). Storage
    /// decision (`docs/outcomes/20260809-rung2-state-shapes` row 5):
    /// `ordering_column` names the hidden `<alias>__o` state column derived
    /// by `analysis::decomposed_state::decompose_arg_by` — never a
    /// user-visible companion projection — so target/delta values for the
    /// comparison come from that hidden state column's own
    /// `target.<name>` / `delta.<name>` refs.
    OrderMonotone {
        ordering_column: String,
        /// `true` for `MAX_BY`/`ArgMax` (the delta wins iff its ordering
        /// value is strictly greater); `false` for `MIN_BY`/`ArgMin` (the
        /// delta wins iff its ordering value is strictly lesser). Storage
        /// decision (Phase 3, `docs/outcomes/20260809-rung2-state-shapes`):
        /// `render` was unconditionally `>`, which was correct only for
        /// `MAX_BY` — `MIN_BY`'s state-shape fold (`decomposed_state.rs`)
        /// needs the opposite comparison over its `o` state column.
        prefer_greater: bool,
    },
    /// The plain-overwrite family (`ANY_VALUE(...)`,
    /// `incremental_models.md` §"The column-family catalogue"): the delta's
    /// value always wins — the incoming row is the current observation, no
    /// target comparison is made. Admitted only under the snapshot-reconcile
    /// run shape (Phase 3, `docs/plans/20260809-keyed-frontier.md`); refused
    /// window-forward (`KeyedUnknownCombiner`).
    PlainOverwrite,
    /// The once-write family (`COALESCE`, `incremental_models.md` §"The
    /// column-family catalogue"): `COALESCE(target.c, delta.c)` — the
    /// target's value wins once set; the delta only ever fills a `NULL`
    /// target. Admitted only under the once-write provenance proof
    /// (`classify_once_write`): a key-derived value, or a declared
    /// functional dependency consulted via
    /// `analysis::functional_dependency::functional_dependency_verdict_over_vector`
    /// (`docs/plans/20260809-keyed-frontier.md` Phase 4).
    OnceWrite,
    /// The decomposed-fold family (`AVG`/`STDDEV_*`/`VAR_*`,
    /// `incremental_models.md` §"The column-family catalogue"): the
    /// presented column has no target/delta fold of its own — it is always
    /// recomputed as `π(merged state)` from the column's hidden state
    /// columns (`analysis::decomposed_state::DecomposedState::
    /// presentation_expr`), which `expand_aggregator_column_folds`
    /// (`smelt-logical::maintenance::emit`) substitutes in directly rather
    /// than calling `render` (`docs/outcomes/20260809-rung2-state-shapes`
    /// row 7). `render` is unreachable by construction for a `Recomputed`
    /// column: every such column carries `state: Some(..)`, and `refuse()`
    /// (`smelt-runtime::cumulative::WindowedKeyedRule`) rejects a
    /// `Recomputed` column with `state: None` before any statement is
    /// built.
    Recomputed,
}

impl CrossPartitionCombiner {
    /// Render the SQL expression that produces the merged value for one column.
    pub fn render(&self, target_col: &str, delta_col: &str) -> String {
        match self {
            CrossPartitionCombiner::Sum => format!("{} + {}", target_col, delta_col),
            CrossPartitionCombiner::Min => format!("LEAST({}, {})", target_col, delta_col),
            CrossPartitionCombiner::Max => format!("GREATEST({}, {})", target_col, delta_col),
            CrossPartitionCombiner::BoolAnd => format!("{} AND {}", target_col, delta_col),
            CrossPartitionCombiner::BoolOr => format!("{} OR {}", target_col, delta_col),
            CrossPartitionCombiner::BitAnd => format!("{} & {}", target_col, delta_col),
            CrossPartitionCombiner::BitOr => format!("{} | {}", target_col, delta_col),
            CrossPartitionCombiner::BitXor => format!("xor({}, {})", target_col, delta_col),
            CrossPartitionCombiner::OrderMonotone {
                ordering_column,
                prefer_greater,
            } => {
                // `target_col`/`delta_col` are already qualified
                // (`target.<name>`/`delta.<name>`, the caller's own
                // convention, `smelt-runtime::cumulative::build_cumulative_
                // merge_sql`) — the ordering column lives in the same
                // target/delta scope, so the same qualifiers apply.
                let target_qualifier = target_col.rsplit_once('.').map_or("target", |(q, _)| q);
                let delta_qualifier = delta_col.rsplit_once('.').map_or("delta", |(q, _)| q);
                let target_ord = format!("{target_qualifier}.{ordering_column}");
                let delta_ord = format!("{delta_qualifier}.{ordering_column}");
                // Strict comparison — incumbent wins on a tie (§"Ordering
                // ties": "the delta wins iff `delta.ordering > target.ordering`
                // (strict); on equality the incumbent wins"), mirrored for
                // `MIN_BY`/`ArgMin` (`prefer_greater: false`) with `<`.
                let op = if *prefer_greater { ">" } else { "<" };
                format!(
                    "CASE WHEN {delta_ord} {op} {target_ord} THEN {delta_col} ELSE {target_col} END"
                )
            }
            CrossPartitionCombiner::PlainOverwrite => delta_col.to_string(),
            CrossPartitionCombiner::OnceWrite => {
                format!("COALESCE({target_col}, {delta_col})")
            }
            // Unreachable by construction — see the variant's doc comment.
            // Returns the incumbent rather than panicking: a caller that
            // somehow reaches this arm despite `refuse()`'s guard gets an
            // inert fold, not a crash.
            CrossPartitionCombiner::Recomputed => target_col.to_string(),
        }
    }
}

/// The once-write family's per-projection admission verdict
/// ([`classify_once_write`]) — shared by the runtime classifier
/// ([`classify_cumulative`]) and the plan-layer fold derivation
/// (`smelt_db::queries::maintenance::derive_fold_spec`) so both admit/refuse
/// a `COALESCE`-shaped once-write column identically
/// (`docs/specs/incremental_models.md` §"The column-family catalogue").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnceWriteAdmission {
    /// The projection is not a direct `COALESCE(...)` call at all — not this
    /// family's concern.
    NotOnceWrite,
    /// Admitted: the coalesced value is a key-derived expression (a bare
    /// reference to one of the model's own `unique_key` columns), or a
    /// declared functional dependency over every candidate's SOURCE column
    /// (not positively disproven by a fan-out join or a set-operation
    /// barrier) proves each a per-key constant. `state` is the decomposed
    /// `(value, written)` state (one pair per candidate) this column folds
    /// through, `None` for the two spellings that stay stateless — the
    /// key-derived route and a single bare reduction with no fallback
    /// (`docs/outcomes/20260809-rung2-state-shapes` row 6).
    Admitted {
        state: Option<crate::analysis::decomposed_state::DecomposedState>,
    },
    /// A `COALESCE`-shaped once-write projection with no once-write
    /// provenance proof. `column` names the coalesced value's source column;
    /// `reason` names why.
    Unproven { column: String, reason: String },
}

/// Classify one projection as the once-write family (`incremental_models.md`
/// §"The column-family catalogue" row "once-write"): a direct
/// `COALESCE(<candidate>+, <fallback>?)` call, where `<candidate>` is
/// either
///
/// 1. a bare reference to one of `unique_key`'s own columns — a per-key
///    constant by construction (the key never changes across merges), no
///    declaration or proof needed, and its fallback (if any) is applied
///    unconditionally — a `unique_key` column is non-null within its own
///    group by construction, so its fallback can never mask a later
///    observation; or
/// 2. the leading maximal run of direct `MAX(<col>)`/`MIN(<col>)`
///    reductions of a single non-key column each, optionally followed by
///    exactly one further trailing argument (the *fallback*). Any other
///    shape after the leading run — a second candidate after the fallback,
///    more than one trailing non-candidate argument — refuses `Unproven`.
///
///    Each candidate is admitted only when a declared functional dependency
///    names that INNER SOURCE column (`<col>`, the payload whose provenance
///    is actually in question) as its `determines`, over a `key` the
///    model's own `unique_key` covers. The declaration must never be
///    matched against the projection's output alias: `unique_key → <alias>`
///    holds by construction for ANY aggregate over the model's own
///    `GROUP BY` key, so an alias-matched declaration would assert nothing
///    and the proof would be vacuous. The world-fact the once-write
///    equivalence needs is `key → <col>` on the source payload — the same
///    `determines` column `analysis::functional_dependency` documents. The
///    first candidate whose provenance cannot be proven refuses the whole
///    column, naming that candidate.
///
///    A declared `key` that is a SUBSET of `unique_key` is accepted: a
///    smaller key is a strictly stronger statement that implies the
///    full-key dependency (matching
///    [`functional_dependency_verdict_over_vector`]'s own `has_subset_key`
///    treatment).
///
///    The verdict is composed from the raw (non-vector)
///    [`functional_dependency_verdict`] plus this function's own explicit
///    structural disproofs — deliberately NOT the grain-composed
///    [`functional_dependency_verdict_over_vector`] variant wholesale:
///    that helper's "grain is a subset of the declared key ⇒ `Constant`"
///    shortcut only proves determinism *within one fixed computation*
///    (trivially true for ANY aggregate over the model's own `GROUP BY`
///    key, extremal folds included), never invariance *across merges* —
///    exactly the pitfall `maintenance::locality`'s route 2 documents on
///    its own once-write route. Its two STRUCTURAL disproofs, however, do
///    apply verbatim and are both enforced here, for every candidate
///    (`model_properties.md` §Constraints "Declared escape hatches may
///    only widen"): `vector.has_fan_out_join` (the walk's whole-scope
///    fan-out fact, a conservative stand-in for "is the determines column
///    sourced from a proven-fan-out join" — coarser than a per-column join
///    trace, but never optimistic: any fan-out anywhere in scope refuses)
///    and `vector.has_set_op_barrier` (SC-6: an FD holding in each branch
///    of an undiscriminated `UNION ALL` need not hold in the union). A
///    declaration widens only past neither of them.
///
///    Once every candidate is proven, a bare single reduction with no
///    fallback (`COALESCE(MAX(col))`) stays the direct `COALESCE(target,
///    delta)` fold it always was ([`OnceWriteAdmission::Admitted`] with
///    `state: None`) — nothing about it changed shape. A fallback-bearing
///    or multi-candidate spelling instead decomposes to hidden `(value,
///    written)` state per candidate
///    (`crate::analysis::decomposed_state::decompose_once_write`): the
///    state's `value` column stays the bare (possibly-NULL) reduction, so
///    it is never fallback-tainted and a later window's real value can
///    still displace it, and the fallback (or the next candidate) is
///    applied fresh in the presentation expression on every read instead
///    of being folded into the merge. The fallback must itself be
///    presentable from the stored row alone — a literal or a `unique_key`
///    column passes the presentation-map purity proof (F7); anything else
///    refuses `Unproven` naming the fallback.
///
/// Any other shape (a bare non-key column with no reducing aggregate, a
/// multi-argument aggregate, a non-MAX/MIN aggregate, …) is
/// [`OnceWriteAdmission::Unproven`], naming the best-effort offending column
/// or expression text.
///
/// A projection whose expression text appears in `group_by_exprs` is a KEY
/// column, not a once-write column — a null-safe composite key
/// (`COALESCE(device_id, 'n/a')` grouped by the same expression) is routine,
/// and this family never claims it ([`OnceWriteAdmission::NotOnceWrite`]).
///
/// `vector` is the model's whole-model [`PropertyVector`]
/// (`analysis::walk::model_property_vector`) — `None` when the model's SQL
/// could not be classified for the walk (an unrelated parse shape the outer
/// classifier will separately refuse); the FD-backed route then fails closed
/// to [`OnceWriteAdmission::Unproven`] rather than guessing.
///
/// `output_name` names the column for the hidden state shape (the
/// projection's own `AS` alias) — only consulted when a decomposed state is
/// actually derived.
#[allow(clippy::too_many_arguments)]
pub fn classify_once_write(
    text: &str,
    expr: &smelt_parser::Expr,
    unique_key: &[String],
    group_by_exprs: &[String],
    declared_fds: &[FunctionalDependency],
    vector: Option<&PropertyVector>,
    output_name: &str,
) -> OnceWriteAdmission {
    if !is_direct_function_call(text, "COALESCE") {
        return OnceWriteAdmission::NotOnceWrite;
    }
    // A `COALESCE(...)` expression that IS the model's GROUP BY key (a
    // null-safe composite key) is a key column, never a once-write column.
    if group_by_exprs.iter().any(|g| g.trim() == text.trim()) {
        return OnceWriteAdmission::NotOnceWrite;
    }
    let Some(fc) = expr.as_function_call() else {
        return OnceWriteAdmission::NotOnceWrite;
    };
    let args = fc.arguments();
    let Some(first) = args.first() else {
        return OnceWriteAdmission::NotOnceWrite;
    };

    // Route 1: key-derived. A bare column reference to one of the model's
    // own `unique_key` columns is a per-key constant by construction.
    if let Some(col_ref) = first.as_column_ref() {
        let name = col_ref.name().to_string();
        if unique_key.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
            return OnceWriteAdmission::Admitted { state: None };
        }
        return OnceWriteAdmission::Unproven {
            column: name,
            reason: "the coalesced value is a bare column reference outside the model's \
                     unique_key — reduce it per key with MAX(...)/MIN(...) first"
                .to_string(),
        };
    }

    // Route 2: FD-backed. The leading maximal run of direct
    // MAX(<col>)/MIN(<col>) reductions of a single column each are the
    // candidates; at most one further trailing argument is the fallback.
    let mut candidates: Vec<(&smelt_parser::Expr, String)> = Vec::new();
    for arg in &args {
        let Some(inner_fc) = arg.as_function_call() else {
            break;
        };
        let inner_name = inner_fc.name().unwrap_or_default().to_ascii_uppercase();
        if inner_name != "MAX" && inner_name != "MIN" {
            break;
        }
        let inner_args = inner_fc.arguments();
        let Some(inner_ref) = inner_args
            .first()
            .filter(|_| inner_args.len() == 1)
            .and_then(|a| a.as_column_ref())
        else {
            break;
        };
        candidates.push((arg, inner_ref.name().to_string()));
    }

    if candidates.is_empty() {
        return OnceWriteAdmission::Unproven {
            column: text.trim().to_string(),
            reason: "the coalesced value is neither a key-derived expression nor a direct \
                     MAX(...)/MIN(...) reduction of a single column"
                .to_string(),
        };
    }

    let remaining = args.len() - candidates.len();
    if remaining > 1 {
        return OnceWriteAdmission::Unproven {
            column: candidates[0].1.clone(),
            reason: "the coalesced value carries more than one trailing argument after its \
                     candidate MAX(...)/MIN(...) reductions — only a single fallback (a \
                     literal or a unique_key column) is permitted after them"
                .to_string(),
        };
    }
    let fallback_expr = (remaining == 1).then(|| &args[candidates.len()]);

    for (_, column) in &candidates {
        // The `determines` column is the coalesced value's SOURCE payload
        // column — never the projection's output alias (see this
        // function's doc comment: `unique_key -> alias` is true by
        // construction and proves nothing).
        let Some(vector) = vector else {
            return OnceWriteAdmission::Unproven {
                column: column.clone(),
                reason: "the model SQL could not be classified for the once-write \
                         provenance proof"
                    .to_string(),
            };
        };
        // The declared key must name the model's actual GROUP BY SOURCE
        // columns, never the projections' output aliases: `SELECT user_id
        // AS device_id ... GROUP BY user_id` groups on `user_id`, so a
        // declaration `key: [device_id]` asserts a dependency on a
        // different column and proves nothing. Where the alias and the
        // source coincide (the common `SELECT device_id ... GROUP BY
        // device_id` shape) the source set contains that name anyway. A
        // GROUP BY expression that is not a (possibly qualified) plain
        // identifier contributes only its raw text, which no column-name
        // declaration can match — failing closed rather than guessing.
        let source_key_set = group_by_source_columns(group_by_exprs);
        let declared = declared_fds.iter().any(|fd| {
            let declared_key: std::collections::BTreeSet<String> =
                fd.key.iter().map(|k| k.to_ascii_lowercase()).collect();
            fd.determines.eq_ignore_ascii_case(column)
                && !declared_key.is_empty()
                // A subset key is a STRONGER statement that implies the
                // unique_key dependency — accepted, mirroring
                // `functional_dependency_verdict_over_vector`'s own
                // `has_subset_key` treatment.
                && declared_key.is_subset(&source_key_set)
        });
        // A conservative, whole-scope stand-in for "the determines column
        // is sourced from a join F6 proves fans out" — see this function's
        // own doc comment for why the grain-composed helper's shortcut is
        // unsound here.
        let determines_fan_out = if vector.has_fan_out_join {
            Some(crate::analysis::join_shape::Cardinality::OneToMany)
        } else {
            None
        };
        let verdict = functional_dependency_verdict(determines_fan_out, declared);
        if let FunctionalDependencyVerdict::Refused(reason) = verdict {
            return OnceWriteAdmission::Unproven {
                column: column.clone(),
                reason,
            };
        }
        // The second structural disproof a declaration may not widen past
        // (`model_properties.md` SC-6), enforced here exactly as
        // `functional_dependency_verdict_over_vector` enforces it.
        if vector.has_set_op_barrier {
            return OnceWriteAdmission::Unproven {
                column: column.clone(),
                reason: "the coalesced column crosses a UNION ALL / set operation whose \
                         branches are not proven key-disjoint (no literal discriminator \
                         covering the declared key); an FD holding in each branch does \
                         not hold in the union, so a declared functional dependency \
                         cannot be assumed to survive it"
                    .to_string(),
            };
        }
        match verdict {
            FunctionalDependencyVerdict::Constant => {}
            FunctionalDependencyVerdict::Refused(reason) => {
                return OnceWriteAdmission::Unproven {
                    column: column.clone(),
                    reason,
                };
            }
            FunctionalDependencyVerdict::NotProven => {
                return OnceWriteAdmission::Unproven {
                    column: column.clone(),
                    reason: "no declared functional dependency names this source column as \
                             its `determines` over a key the model's unique_key covers, and \
                             it is not a key-derived expression"
                        .to_string(),
                };
            }
        }
    }

    // Every candidate proven a per-key constant. A bare single reduction
    // with no fallback is the direct `COALESCE(target, delta)` fold it
    // always was — nothing about it needs hidden state.
    if candidates.len() == 1 && fallback_expr.is_none() {
        return OnceWriteAdmission::Admitted { state: None };
    }

    let state_candidates: Vec<crate::analysis::decomposed_state::OnceWriteCandidate> = candidates
        .iter()
        .map(
            |(candidate_expr, _)| crate::analysis::decomposed_state::OnceWriteCandidate {
                reduction_expr: candidate_expr.text().trim().to_string(),
            },
        )
        .collect();
    let fallback_text = fallback_expr.map(|e| e.text().trim().to_string());
    match crate::analysis::decomposed_state::decompose_once_write(
        &state_candidates,
        fallback_text.as_deref(),
        unique_key,
        output_name,
    ) {
        Ok(state) => OnceWriteAdmission::Admitted { state: Some(state) },
        Err(refusal) => OnceWriteAdmission::Unproven {
            column: fallback_text.unwrap_or_else(|| candidates[0].1.clone()),
            reason: format!(
                "the fallback/candidate presentation could not be decomposed to hidden \
                 state: {refusal:?}"
            ),
        },
    }
}

/// The set of names a declared functional-dependency `key:` may legitimately
/// use to refer to this model's grouping columns — the GROUP BY **source**
/// expressions, never the SELECT list's output aliases
/// (`model_properties.md` §"Algebraic discriminants", the
/// functional-dependency declaration: the declared key is a world fact about
/// the model's *inputs*).
///
/// Each GROUP BY expression contributes its own lowercased text. A
/// qualified plain identifier (`e.device_id`) additionally contributes its
/// bare column name (`device_id`) — the same column, differently spelled —
/// but only when that bare name is unambiguous across the whole GROUP BY
/// list; two differently-qualified columns sharing a bare name
/// (`e.id`, `o.id`) contribute neither, so a declaration naming `id` cannot
/// match. Anything that is not a plain (optionally qualified) identifier —
/// `date_trunc('day', ts)`, a `COALESCE(...)` composite key — contributes
/// only its raw text, which no column-name declaration matches: the
/// undecidable case fails closed.
fn group_by_source_columns(group_by_exprs: &[String]) -> std::collections::BTreeSet<String> {
    fn bare_identifier(text: &str) -> Option<String> {
        let mut segments = text.split('.');
        let last = segments.next_back()?.trim();
        if text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            && !last.is_empty()
            && !last.starts_with(|c: char| c.is_ascii_digit())
        {
            Some(last.to_ascii_lowercase())
        } else {
            None
        }
    }

    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut bare_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for expr in group_by_exprs {
        let text = expr.trim();
        names.insert(text.to_ascii_lowercase());
        if let Some(bare) = bare_identifier(text) {
            *bare_counts.entry(bare).or_insert(0) += 1;
        }
    }
    for (bare, count) in bare_counts {
        if count == 1 {
            names.insert(bare);
        }
    }
    names
}

/// Lookup a per-partition aggregator name and return its cross-partition
/// combiner. The gate is the shared algebraic-discriminants classifier
/// (`analysis::discriminants::combiner_discriminants`) — only a monoid
/// combiner is admitted; `None` means the aggregator is not a monoid (either
/// holistic, e.g. `AVG`/`STRING_AGG`, or unrecognised).
pub fn combiner_for(agg_name: &str) -> Option<CrossPartitionCombiner> {
    let function = SqlFunction::from_name(agg_name)?;
    let discriminants = crate::analysis::discriminants::combiner_discriminants(function, false);
    if !discriminants.is_monoid {
        return None;
    }
    match function {
        SqlFunction::Count | SqlFunction::Sum => Some(CrossPartitionCombiner::Sum),
        SqlFunction::Min => Some(CrossPartitionCombiner::Min),
        SqlFunction::Max => Some(CrossPartitionCombiner::Max),
        SqlFunction::BoolAnd => Some(CrossPartitionCombiner::BoolAnd),
        SqlFunction::BoolOr => Some(CrossPartitionCombiner::BoolOr),
        SqlFunction::BitAnd => Some(CrossPartitionCombiner::BitAnd),
        SqlFunction::BitOr => Some(CrossPartitionCombiner::BitOr),
        SqlFunction::BitXor => Some(CrossPartitionCombiner::BitXor),
        _ => None,
    }
}

/// A diagnostic code emitted by the keyed classifier.
///
/// Mirrors `incremental_models.md` §"Key-grain diagnostic codes".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum KeyedDiagnostic {
    KeyedRequiresGroupBy,
    KeyedUnknownCombiner {
        projection: String,
        offending: String,
    },
    KeyedGroupByContainsPartitionColumn {
        partition_column: String,
    },
    KeyedForbidsWindowFunctions,
    KeyedForbidsNondeterministic {
        offending: String,
    },
    /// No clocked driving source was found, AND no single unambiguous
    /// source could be resolved to derive the snapshot-reconcile run shape
    /// either — e.g. the FROM clause joins more than one candidate source
    /// with none of them clocked. Genuinely unsupportable, not a "not yet"
    /// refusal (`docs/specs/incremental_models.md` §"The two run shapes").
    KeyedSnapshotPostureUnsupported,
    /// A fold-family projection (additive, extremal/lattice, or
    /// order-monotone overwrite) under the snapshot-reconcile run shape
    /// (`docs/specs/incremental_models.md` §"Admission matrix"): these
    /// families consume events, not observations — re-folding a mutable
    /// snapshot double-counts (additive) or computes a history observation
    /// instead of the current value (observer semantics, the other
    /// families).
    KeyedSnapshotSourceUnsupportedColumn {
        projection: String,
        family: String,
        reason: String,
    },
    KeyedMultipleDrivingSources {
        candidates: Vec<String>,
    },
    KeyedSqlNotParseable,
    /// A `COALESCE`-shaped once-write column (`incremental_models.md` §"The
    /// column-family catalogue") has no once-write provenance proof: the
    /// coalesced value is neither key-derived nor backed by a declared
    /// functional dependency the fan-out proof does not positively
    /// disprove. `column` names the coalesced value's source column;
    /// `reason` names why the proof did not close (unproven, or a
    /// structural disproof — `analysis::functional_dependency`).
    KeyedOnceWriteUnproven {
        projection: String,
        column: String,
        reason: String,
    },
    /// A hidden decomposed-state column (`docs/specs/incremental_models.md`
    /// §"Decomposed state (rung 2) in keyed models") collides with a
    /// user-declared or projected output column of the same name.
    /// `state_column` is the generated state column's name (always carrying
    /// the reserved `__` suffix, e.g. `spend__sum`); `user_column` is the
    /// colliding user-facing column. Never silently renamed — smelt does not
    /// guess which of the two a consumer meant.
    KeyedStateColumnCollision {
        state_column: String,
        user_column: String,
    },
}

impl std::fmt::Display for KeyedDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyedDiagnostic::KeyedRequiresGroupBy => write!(
                f,
                "KeyedRequiresGroupBy: a `grain: key` SELECT must have a GROUP BY \
                 clause — the GROUP BY columns are the unique key"
            ),
            KeyedDiagnostic::KeyedUnknownCombiner {
                projection,
                offending,
            } => write!(
                f,
                "KeyedUnknownCombiner: projection `{}` uses `{}`, which is not a catalogued \
                 column-family aggregator: the fold families (COUNT, SUM, MIN, MAX, \
                 BOOL_AND, BOOL_OR, BIT_AND, BIT_OR, BIT_XOR), the order-monotone overwrite \
                 family (MAX_BY/MIN_BY over an ordering column, window-forward), the \
                 once-write family (COALESCE, given its provenance proof), or the \
                 plain-overwrite family (ANY_VALUE, snapshot-reconcile only). Composite \
                 expressions over aggregates are not allowed — split into separate \
                 projections.",
                projection, offending
            ),
            KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { partition_column } => {
                write!(
                    f,
                    "KeyedGroupByContainsPartitionColumn: the GROUP BY contains the driving \
                     source's partition_column `{}`, which produces a per-partition output shape, \
                     not the keyed one — switch to `grain: partition` + `timeseries:` instead, or \
                     declare `timeseries:` on this model to stay `grain: key`",
                    partition_column
                )
            }
            KeyedDiagnostic::KeyedForbidsWindowFunctions => write!(
                f,
                "KeyedForbidsWindowFunctions: window functions (OVER (...)) are not allowed \
                 in a `grain: key` SELECT — the keyed state is the window"
            ),
            KeyedDiagnostic::KeyedForbidsNondeterministic { offending } => write!(
                f,
                "KeyedForbidsNondeterministic: non-deterministic function `{}` is not \
                 allowed in a `grain: key` SELECT — cross-window combine requires \
                 deterministic per-window output",
                offending
            ),
            KeyedDiagnostic::KeyedSnapshotPostureUnsupported => write!(
                f,
                "KeyedSnapshotPostureUnsupported: this model has no clocked driving source \
                 (no timeseries-tagged source in the FROM clause), and no single unambiguous \
                 source could be resolved to derive the snapshot-reconcile run shape either \
                 — the FROM clause must join exactly one candidate source when none is \
                 clocked. Declare `timeseries:` on a driving source to use the window-forward \
                 run shape instead, or reduce the FROM clause to a single source."
            ),
            KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                projection,
                family,
                reason,
            } => write!(
                f,
                "KeyedSnapshotSourceUnsupportedColumn: projection `{projection}` is a \
                 {family} column, which is refused under the snapshot-reconcile run shape \
                 (no clocked driving source) — {reason}. Wrap it as `ANY_VALUE(...)` for the \
                 plain-overwrite family instead, or declare `timeseries:` on a driving source \
                 to use the window-forward run shape."
            ),
            KeyedDiagnostic::KeyedMultipleDrivingSources { candidates } => write!(
                f,
                "KeyedMultipleDrivingSources: multiple timeseries-tagged sources in the \
                 FROM clause ({}). v1 supports exactly one driving source.",
                candidates.join(", ")
            ),
            KeyedDiagnostic::KeyedSqlNotParseable => {
                write!(f, "SQL body could not be parsed for keyed classification")
            }
            KeyedDiagnostic::KeyedOnceWriteUnproven {
                projection,
                column,
                reason,
            } => write!(
                f,
                "KeyedOnceWriteUnproven: projection `{projection}`'s once-write column `{column}` \
                 has no once-write provenance proof — {reason}. Fix by: (1) making it a \
                 key-derived expression (a pure function of the model's unique_key columns), (2) \
                 declaring a functional dependency (`functional_dependencies: [{{key: [...], \
                 determines: {column}}}]`), or (3) remodelling `{column}` out into its own \
                 separate model."
            ),
            KeyedDiagnostic::KeyedStateColumnCollision {
                state_column,
                user_column,
            } => write!(
                f,
                "KeyedStateColumnCollision: the hidden decomposed-state column `{state_column}` \
                 collides with the user column `{user_column}` — column names ending in the \
                 reserved `__` suffix (e.g. `__sum`, `__count`, `__v`, `__o`, `__value`, \
                 `__written`) are reserved for decomposed state; rename `{user_column}`."
            ),
        }
    }
}

/// Pure detector for `KeyedStateColumnCollision`: which of `aggregator_columns`'
/// own hidden decomposed-state columns collide with another projection's
/// output name in the same classification. Reachable via
/// [`classify_cumulative`] for the order-monotone overwrite family
/// (`MAX_BY`/`MIN_BY`, `docs/outcomes/20260809-rung2-state-shapes` row 5);
/// every other column family still classifies with `state: None` until row
/// 6 widens admission onto them too.
pub fn diagnose_state_column_collisions(
    aggregator_columns: &[AggregatorColumn],
) -> Vec<KeyedDiagnostic> {
    let user_columns: Vec<String> = aggregator_columns
        .iter()
        .map(|c| c.output_name.clone())
        .collect();
    aggregator_columns
        .iter()
        .filter_map(|c| c.state.as_ref())
        .flat_map(|state| {
            crate::analysis::decomposed_state::state_column_collisions(
                &state.state_columns,
                &user_columns,
            )
        })
        .map(
            |(state_column, user_column)| KeyedDiagnostic::KeyedStateColumnCollision {
                state_column,
                user_column,
            },
        )
        .collect()
}

/// One presented column's hidden decomposed state, summarized for reporting
/// (`smelt explain`, `docs/outcomes/20260809-rung2-state-shapes` row 9). A
/// pure read of `AggregatorColumn::state` — it never re-decides which
/// spellings are state-bearing (single owner: `classify_cumulative` /
/// `analysis::decomposed_state::decompose_to_state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateColumnSummary {
    /// The output column name this state backs.
    pub presented_column: String,
    /// The hidden state columns' names, in declaration order.
    pub state_columns: Vec<String>,
    /// The presentation expression `π(state)` that recomputes
    /// `presented_column`'s value from the state columns above.
    pub presentation_expr: String,
}

/// One entry per [`CumulativeClassification::aggregator_columns`] entry
/// whose `state` is `Some` — empty when no column folds through decomposed
/// state (a rung-1 model reports no state section at all).
pub fn state_column_summary(classification: &CumulativeClassification) -> Vec<StateColumnSummary> {
    classification
        .aggregator_columns
        .iter()
        .filter_map(|c| {
            let state = c.state.as_ref()?;
            Some(StateColumnSummary {
                presented_column: c.output_name.clone(),
                state_columns: state
                    .state_columns
                    .iter()
                    .map(|sc| sc.name.clone())
                    .collect(),
                presentation_expr: state.presentation_expr.clone(),
            })
        })
        .collect()
}

/// The result of classifying a `cumulative_aggregate` model.
#[derive(Debug, Clone, Serialize)]
pub struct CumulativeClassification {
    /// Columns from the GROUP BY list. The order matches the SELECT's
    /// GROUP BY ordering.
    pub unique_key: Vec<String>,
    /// Non-key projections with their derived combiners.
    pub aggregator_columns: Vec<AggregatorColumn>,
    /// The single source the rule iterates over. `name` is the model/source
    /// name as it appears in `smelt.<path>` references; `timeseries` is
    /// `Some` (window-forward) or `None` (snapshot-reconcile) — see
    /// [`DrivingSource::timeseries`].
    pub driving_source: DrivingSource,
}

impl CumulativeClassification {
    /// Whether this classification derived the snapshot-reconcile run shape
    /// (`docs/specs/incremental_models.md` §"The two run shapes") — zero
    /// clocked sources in the FROM clause. Derived from the classifier's own
    /// resolved [`DrivingSource`], never a second independent check.
    pub fn is_snapshot_reconcile(&self) -> bool {
        self.driving_source.timeseries.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DrivingSource {
    pub name: String,
    /// `Some` for the window-forward run shape (the source's own declared
    /// `timeseries:` block, the driving-fact/anchor proof); `None` for the
    /// snapshot-reconcile run shape — zero clocked sources in the FROM
    /// clause (`docs/specs/incremental_models.md` §"The two run shapes").
    /// The run shape is derived from this field, never declared.
    pub timeseries: Option<TimeseriesConfig>,
}

/// Lookup table for a source's `timeseries:` declaration. The classifier
/// uses this to identify the driving source and to enforce
/// `KeyedGroupByContainsPartitionColumn`.
pub type SourceTimeseriesMap = HashMap<String, TimeseriesConfig>;

/// Derive a `grain: key` model's `unique_key` from its own GROUP BY: the
/// SELECT aliases corresponding to each GROUP BY expression, falling back
/// to the raw expression text for a GROUP BY expression with no matching
/// non-aggregate projection (e.g. an expression-based GROUP BY).
fn group_by_unique_key_from_analysis(analysis: &crate::analysis::SelectAnalysis) -> Vec<String> {
    let mut unique_key: Vec<String> = Vec::new();
    for group_expr in &analysis.group_by_exprs {
        let matched = analysis.items.iter().find_map(|item| match item {
            SelectItemKind::GroupByKey { text, alias, .. } if text == group_expr => {
                Some(alias.clone())
            }
            _ => None,
        });
        if let Some(alias) = matched {
            unique_key.push(alias);
        } else {
            unique_key.push(group_expr.clone());
        }
    }
    unique_key
}

/// Derive a `grain: key` model's `unique_key` from its own GROUP BY clause,
/// independent of the full [`classify_cumulative`] admission (aggregator
/// allowlisting, driving-source resolution). Plan derivation
/// (`smelt-db::queries::maintenance::derive_model_maintenance_plan`) needs
/// only the key itself — admitted or not — to thread the model's real
/// `unique_key` into `PlanGrain::Key`/`SourceFacts` instead of a hardcoded
/// empty vec.
///
/// Returns an empty vec when the SQL doesn't parse or declares no GROUP BY.
pub fn group_by_unique_key(sql: &str) -> Vec<String> {
    match analyze_select(sql) {
        Some(analysis) => group_by_unique_key_from_analysis(&analysis),
        None => Vec::new(),
    }
}

/// Compare a **declared** top-level `unique_key:` (`docs/specs/models.md`
/// §"Refresh axis") against the GROUP-BY-derived key [`group_by_unique_key`]
/// computes for the same SQL — a leaf classifier check over one
/// already-bounded model's own text (`architecture.md` §"Property
/// composition walk rule"), order-independent (declaring the same columns in
/// a different order is agreement, not a mismatch).
///
/// `Ok(())` on agreement (including when `declared` is empty and so is the
/// derived key). `Err((declared, derived))` on disagreement — both lists, in
/// their original declared/derived order, for the caller to name in a
/// diagnostic (`models.md` §"Constraint violations": "For aggregated key
/// bodies: `unique_key` ≠ the `GROUP BY` column set → hard error (checked
/// restatement)"). This function only compares; it does not decide which
/// list wins — `smelt-db::queries::maintenance` is the sole caller that
/// turns a mismatch into a refused plan.
pub fn declared_unique_key_matches(
    declared: &[String],
    sql: &str,
) -> Result<(), (Vec<String>, Vec<String>)> {
    let derived = group_by_unique_key(sql);
    let mut declared_sorted = declared.to_vec();
    declared_sorted.sort();
    let mut derived_sorted = derived.clone();
    derived_sorted.sort();
    if declared_sorted == derived_sorted {
        Ok(())
    } else {
        Err((declared.to_vec(), derived))
    }
}

/// Classify a `cumulative_aggregate` model.
///
/// `sql` is the inlined model SQL (post function expansion). `refs` is the
/// list of `smelt.<path>` references discovered in the FROM clause. Source
/// timeseries declarations are looked up via `source_timeseries`.
/// `model_has_timeseries` is whether the model's own frontmatter declares a
/// `timeseries:` block — it narrows `KeyedGroupByContainsPartitionColumn`
/// to the no-`timeseries:` case (a model that declares its own
/// `timeseries:` is instead decided by the key-temporal-locality gate,
/// `maintenance::locality::establish_locality`).
///
/// `declared_functional_dependencies` is the model's own declared
/// `functional_dependencies:` block — the once-write family's provenance
/// proof (`classify_once_write`) consults it, widening a `COALESCE`
/// projection whose coalesced value isn't itself key-derived
/// (`docs/plans/20260809-keyed-frontier.md` Phase 4).
///
/// Returns the classification on success, or a vector of diagnostics
/// describing every classifier rejection (the function does not short-circuit
/// on the first error — it surfaces every problem it can detect).
pub fn classify_cumulative(
    sql: &str,
    refs: &[String],
    source_timeseries: &SourceTimeseriesMap,
    model_has_timeseries: bool,
    declared_functional_dependencies: &[FunctionalDependency],
) -> Result<CumulativeClassification, Vec<KeyedDiagnostic>> {
    let mut diagnostics = Vec::new();

    let analysis = match analyze_select(sql) {
        Some(a) => a,
        None => {
            return Err(vec![KeyedDiagnostic::KeyedSqlNotParseable]);
        }
    };

    // The whole-model property vector — the once-write family's FD-backed
    // route consults it (`classify_once_write`). `None` when the walk
    // itself can't classify this SQL shape; the once-write route then fails
    // closed rather than guess (`analysis::functional_dependency`'s
    // documented consumer seam).
    let property_vector = model_property_vector(sql, &JoinContext::new());

    // Rule: GROUP BY required.
    if analysis.group_by_exprs.is_empty() {
        diagnostics.push(KeyedDiagnostic::KeyedRequiresGroupBy);
    }

    // Build the unique_key as the SELECT aliases corresponding to GROUP BY
    // expressions. Each GROUP BY expression is matched to a projection by
    // textual identity (the analyser already resolves ordinals).
    let unique_key = group_by_unique_key_from_analysis(&analysis);

    // Find the driving source and derive the run shape (`docs/specs/
    // incremental_models.md` §"The two run shapes") BEFORE walking the
    // projection list — every column family's admission depends on which
    // run shape the model derives (the admission matrix is a `(column
    // family × run shape)` table), so the run shape must be known first.
    //
    // Exactly one clocked (timeseries-tagged) source in the FROM clause is
    // the window-forward driving source — the shared anchor resolver
    // (`resolve_single_anchor`) also used by `resolve_join_driving_fact`'s
    // alias-scoped monotonicity trace. Zero clocked sources derives
    // snapshot-reconcile instead of refusing outright: a single
    // unambiguous joined source (of ANY posture — the FROM clause need not
    // register a `timeseries:` block for this resolution) stands in as
    // "the source" the whole-scan reconciliation re-scans. Two or more
    // clocked sources stays `KeyedMultipleDrivingSources`.
    let alias_sources: Vec<(String, String)> =
        smelt_parser::File::cast(smelt_parser::parse(sql).syntax())
            .and_then(|file| file.select_stmt())
            .and_then(|select| select.from_clause())
            .map(|from_clause| {
                crate::analysis::source_bounds::from_clause_alias_sources(&from_clause)
            })
            .unwrap_or_default();

    let clocked_resolution = resolve_single_anchor(&alias_sources, |source_name| {
        let key = format!("smelt.{source_name}");
        if !refs.iter().any(|r| r == &key) {
            return None;
        }
        source_timeseries.get(&key).map(|ts| DrivingSource {
            name: key.clone(),
            timeseries: Some(ts.clone()),
        })
    });

    let (driving_source, is_snapshot_reconcile) = match clocked_resolution {
        Ok(ds) => (Some(ds), false),
        Err(AnchorAmbiguity::Multiple(candidates)) => {
            diagnostics.push(KeyedDiagnostic::KeyedMultipleDrivingSources {
                candidates: candidates
                    .into_iter()
                    .map(|n| format!("smelt.{n}"))
                    .collect(),
            });
            (None, false)
        }
        Err(AnchorAmbiguity::NoCandidate) => {
            let snapshot_source = resolve_single_anchor(&alias_sources, |source_name| {
                let key = format!("smelt.{source_name}");
                refs.iter().any(|r| r == &key).then_some(key)
            });
            match snapshot_source {
                Ok(name) => (
                    Some(DrivingSource {
                        name,
                        timeseries: None,
                    }),
                    true,
                ),
                Err(_) => {
                    diagnostics.push(KeyedDiagnostic::KeyedSnapshotPostureUnsupported);
                    (None, false)
                }
            }
        }
    };

    // Walk the projection list. Non-key projections must be allowlisted
    // aggregator calls (per the admission matrix, gated on `is_snapshot_
    // reconcile` above). GroupByKey items are the key columns.
    let mut aggregator_columns: Vec<AggregatorColumn> = Vec::new();
    for item in &analysis.items {
        match item {
            SelectItemKind::GroupByKey { text, alias, expr } => {
                // The once-write family (`COALESCE`, `incremental_models.md`
                // §"The column-family catalogue") is checked first — a
                // `COALESCE(...)` call is a non-aggregate scalar (so it
                // lands here, not in `OtherAggregate`), and its own
                // admission has nothing to do with the generic composite-
                // expression refusal below. A `COALESCE` expression that is
                // itself the GROUP BY key stays a key column: the helper
                // consults `group_by_exprs` and declines it.
                match classify_once_write(
                    text,
                    expr,
                    &unique_key,
                    &analysis.group_by_exprs,
                    declared_functional_dependencies,
                    property_vector.as_ref(),
                    alias,
                ) {
                    OnceWriteAdmission::Admitted { state } => {
                        if is_snapshot_reconcile {
                            diagnostics.push(
                                KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                                    projection: alias.clone(),
                                    family: "once-write".to_string(),
                                    reason: "once-write assumes a value is fixed the first \
                                             time it is observed across window-forward event \
                                             history — re-scanning a mutable snapshot every \
                                             run cannot preserve first-write-wins semantics, \
                                             since the merge would only ever see the current \
                                             snapshot's value"
                                        .to_string(),
                                },
                            );
                        } else {
                            aggregator_columns.push(AggregatorColumn {
                                output_name: alias.clone(),
                                per_partition_agg: "COALESCE".to_string(),
                                cross_partition_combiner: CrossPartitionCombiner::OnceWrite,
                                state,
                            });
                        }
                        continue;
                    }
                    OnceWriteAdmission::Unproven { column, reason } => {
                        diagnostics.push(KeyedDiagnostic::KeyedOnceWriteUnproven {
                            projection: alias.clone(),
                            column,
                            reason,
                        });
                        continue;
                    }
                    OnceWriteAdmission::NotOnceWrite => {}
                }

                // A "GroupByKey" item is the analyser's classification for
                // any non-aggregate expression. If the projection's text
                // appears in the GROUP BY, it is genuinely a key column.
                // Otherwise — whether a composite expression (`SUM(x) + 1`)
                // or a bare column reference — it is not a valid keyed
                // projection either way: the catalogue's overwrite families
                // are only ever expressed as aggregate calls (`MAX_BY(...)`,
                // `ANY_VALUE(...)`), never a bare passthrough column
                // (`docs/specs/incremental_models.md` §"The column-family
                // catalogue").
                let in_group_by = analysis.group_by_exprs.iter().any(|g| g == text);
                if !in_group_by {
                    // The once-write suggestion names the family's reduction
                    // spelling — offered ONLY for a bare column reference:
                    // `classify_once_write`'s FD-backed route requires each
                    // candidate's `MAX(...)`/`MIN(...)` argument to be a
                    // single bare column, so `COALESCE(MAX(a || b))` over a
                    // composite projection would itself refuse
                    // `KeyedOnceWriteUnproven` (`docs/specs/
                    // incremental_models.md` §"The column-family
                    // catalogue"). The `MAX_BY`/`ANY_VALUE` spellings take
                    // an arbitrary expression and stay offered either way.
                    let projection_text = text.trim();
                    let once_write_fix = if expr.as_column_ref().is_some() {
                        format!(
                            ", or `COALESCE(MAX({projection_text}))` — an optional trailing \
                             fallback (a literal or a unique_key column) is admitted too — \
                             under a declared functional dependency for the once-write family"
                        )
                    } else {
                        String::new()
                    };
                    diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                        projection: alias.clone(),
                        offending: format!(
                            "composite expression `{projection_text}` — wrap it as \
                             `MAX_BY({projection_text}, <ordering>)` for the order-monotone \
                             overwrite family (window-forward), or `ANY_VALUE({projection_text})` \
                             for the plain-overwrite family (snapshot-reconcile only)\
                             {once_write_fix}"
                        ),
                    });
                }
            }
            SelectItemKind::CountDistinct { alias, .. } => {
                // COUNT(DISTINCT x) is not commutative under merge (the union
                // of distinct values across partitions cannot be reconstructed
                // from per-partition counts). Refuse.
                diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                    projection: alias.clone(),
                    offending: "COUNT(DISTINCT)".to_string(),
                });
            }
            SelectItemKind::OtherAggregate {
                text, alias, expr, ..
            } => {
                // Extract the outer function name from the already-parsed
                // expression (no string re-parse) and confirm it against the
                // typed aggregate classifier — the same
                // `SqlFunction::is_aggregate` predicate `analysis::mod` used
                // to classify this item as `OtherAggregate` in the first
                // place.
                let agg_name = expr
                    .as_function_call()
                    .and_then(|f| f.name())
                    .map(|n| n.to_ascii_uppercase())
                    .filter(|n| SqlFunction::from_name(n).is_some_and(|f| f.is_aggregate()));

                // The plain-overwrite family (`ANY_VALUE`,
                // `incremental_models.md` §"The column-family catalogue"):
                // admitted only under snapshot-reconcile; refused
                // window-forward naming the `MAX_BY` fix.
                if agg_name.as_deref() == Some("ANY_VALUE") {
                    if is_snapshot_reconcile {
                        if is_direct_function_call(text, "ANY_VALUE") {
                            aggregator_columns.push(AggregatorColumn {
                                output_name: alias.clone(),
                                per_partition_agg: "ANY_VALUE".to_string(),
                                cross_partition_combiner: CrossPartitionCombiner::PlainOverwrite,
                                state: None,
                            });
                        } else {
                            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                                projection: alias.clone(),
                                offending: format!("composite expression `{}`", text.trim()),
                            });
                        }
                    } else {
                        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                            projection: alias.clone(),
                            offending: format!(
                                "ANY_VALUE (the plain-overwrite family is snapshot-reconcile \
                                 only) — wrap the value as `MAX_BY({}, <ordering>)` for the \
                                 order-monotone overwrite family instead",
                                text.trim()
                            ),
                        });
                    }
                    continue;
                }

                // The order-monotone overwrite family (`MAX_BY`/`MIN_BY` —
                // `ArgMax`/`ArgMin`, `Monotone::Order`) is not gated through
                // `combiner_for`'s `is_monoid` allowlist (it is a semilattice
                // fold, not a commutative monoid) — handle it up front and
                // move to the next projection.
                let order_monotone = agg_name.as_deref().and_then(|n| {
                    let sql_fn = SqlFunction::from_name(n)?;
                    let is_order =
                        crate::analysis::discriminants::combiner_discriminants(sql_fn, false)
                            .monotone
                            == crate::analysis::discriminants::Monotone::Order;
                    is_order.then_some((n.to_string(), sql_fn))
                });
                if let Some((agg_upper, sql_fn)) = order_monotone {
                    if is_snapshot_reconcile {
                        diagnostics.push(KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                            projection: alias.clone(),
                            family: "order-monotone overwrite".to_string(),
                            reason: "observer semantics — MAX_BY/MIN_BY over successive \
                                     snapshots retains a stale incumbent forever if a mutation \
                                     regresses the ordering value"
                                .to_string(),
                        });
                    } else {
                        classify_order_monotone_column(
                            text,
                            alias,
                            expr,
                            sql_fn,
                            &agg_upper,
                            &mut aggregator_columns,
                            &mut diagnostics,
                        );
                    }
                    continue;
                }

                let combiner = agg_name.as_deref().and_then(combiner_for);
                match (agg_name, combiner) {
                    (Some(agg_upper), Some(combiner)) => {
                        if is_snapshot_reconcile {
                            let (family, reason) = snapshot_refusal_reason(&agg_upper);
                            diagnostics.push(
                                KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                                    projection: alias.clone(),
                                    family: family.to_string(),
                                    reason: reason.to_string(),
                                },
                            );
                            continue;
                        }
                        // Verify the projection is a *direct* call — no
                        // composition like `SUM(x) + 1`. We detect this by
                        // checking that the projection text starts with the
                        // function name and ends with `)`, ignoring whitespace.
                        if is_direct_function_call(text, &agg_upper) {
                            aggregator_columns.push(AggregatorColumn {
                                output_name: alias.clone(),
                                per_partition_agg: agg_upper,
                                cross_partition_combiner: combiner,
                                state: None,
                            });
                        } else {
                            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                                projection: alias.clone(),
                                offending: format!("composite expression `{}`", text.trim()),
                            });
                        }
                    }
                    (Some(agg_upper), None) => {
                        // The decomposed-fold family (`AVG`/`STDDEV_*`/
                        // `VAR_*`, `docs/outcomes/20260809-rung2-state-shapes`
                        // row 7): not in `combiner_for`'s monoid-over-the-
                        // presented-value allowlist (there is no single
                        // target/delta fold for the presented value), but
                        // its algebra decomposes into hidden additive state
                        // — attempt that before falling back to
                        // `KeyedUnknownCombiner`. `decompose_to_state`
                        // itself fails closed for every other holistic-or-
                        // unknown-shape aggregate (`MEDIAN`,
                        // `APPROX_COUNT_DISTINCT`, ...), so this widening
                        // cannot admit anything the mechanism doesn't
                        // actually encode.
                        let decomposed_fold_fn = SqlFunction::from_name(&agg_upper).filter(|f| {
                            matches!(
                                f,
                                SqlFunction::Avg
                                    | SqlFunction::Variance
                                    | SqlFunction::Stddev
                                    | SqlFunction::StddevPop
                                    | SqlFunction::StddevSamp
                                    | SqlFunction::VarPop
                                    | SqlFunction::VarSamp
                            )
                        });
                        match decomposed_fold_fn {
                            Some(_) if is_snapshot_reconcile => {
                                let (family, reason) = snapshot_refusal_reason(&agg_upper);
                                diagnostics.push(
                                    KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn {
                                        projection: alias.clone(),
                                        family: family.to_string(),
                                        reason: reason.to_string(),
                                    },
                                );
                            }
                            Some(sql_fn) => {
                                classify_decomposed_fold_column(
                                    text,
                                    alias,
                                    expr,
                                    sql_fn,
                                    &agg_upper,
                                    &mut aggregator_columns,
                                    &mut diagnostics,
                                );
                            }
                            None => {
                                diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                                    projection: alias.clone(),
                                    offending: agg_upper,
                                });
                            }
                        }
                    }
                    (None, _) => {
                        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                            projection: alias.clone(),
                            offending: text.clone(),
                        });
                    }
                }
            }
        }
    }

    // Rule: no window functions in the outer body.
    //
    // Known walk-invariant violation: this is a raw whole-SQL text scan in an
    // admission path, not a leaf classifier invoked by the shared
    // composition walk (`docs/specs/architecture.md` §"Property composition
    // walk rule"). It predates that walk and is excluded from the
    // `walk_coverage` structural gate (`crates/smelt-logical/tests/walk_coverage.rs`'s
    // `KNOWN_NONCOMPLIANT` skip-list) rather than mislabeled with a
    // classification tag it doesn't actually carry. Migrating this admission
    // check onto the walk is tracked as deferred work in
    // `docs/plans/20260707-property-composition-walk.md` (see "Deferred
    // during implementation") and `docs/specs/model_properties.md` §Known
    // Divergences ("Heuristic text-scanning layer").
    let upper_sql = sql.to_uppercase();
    if upper_sql.contains("OVER(") || upper_sql.contains("OVER (") {
        diagnostics.push(KeyedDiagnostic::KeyedForbidsWindowFunctions);
    }

    // Rule: no non-deterministic functions in the outer body.
    for nd in NONDETERMINISTIC_FUNCTIONS {
        let pattern = format!("{}(", nd);
        if upper_sql.contains(&pattern) {
            diagnostics.push(KeyedDiagnostic::KeyedForbidsNondeterministic {
                offending: nd.to_string(),
            });
            break;
        }
    }

    // Rule: GROUP BY must not contain the driving source's partition column
    // — narrowed to models with no `timeseries:` block of their own
    // (`docs/specs/incremental_models.md` §"Key temporal locality"). A
    // keyed model whose GROUP BY includes the partition column AND that
    // declares its own `timeseries:` is not this rule's concern: it is a
    // candidate for the key-embedded locality route (route 1 —
    // `partition_column` is a `unique_key` column), decided by the locality
    // gate (`maintenance::locality::establish_locality`), not refused here.
    if !model_has_timeseries {
        if let Some(ts) = driving_source
            .as_ref()
            .and_then(|ds| ds.timeseries.as_ref())
        {
            let partition_col = &ts.partition_column;
            let partition_col_lower = partition_col.to_ascii_lowercase();
            let contains_partition = unique_key
                .iter()
                .any(|k| k.to_ascii_lowercase() == partition_col_lower)
                || analysis
                    .group_by_exprs
                    .iter()
                    .any(|e| e.to_ascii_lowercase() == partition_col_lower);
            if contains_partition {
                diagnostics.push(KeyedDiagnostic::KeyedGroupByContainsPartitionColumn {
                    partition_column: partition_col.clone(),
                });
            }
        }
    }

    // Rule: no hidden decomposed-state column may collide with a user
    // column of the same name — reachable for every state-bearing family
    // this classifier admits (the order-monotone overwrite family, the
    // once-write family's fallback/multi-candidate spellings, and the
    // decomposed-fold family, `docs/outcomes/20260809-rung2-state-shapes`
    // rows 5-7).
    diagnostics.extend(diagnose_state_column_collisions(&aggregator_columns));

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(CumulativeClassification {
        unique_key,
        aggregator_columns,
        driving_source: driving_source.expect("driving_source must be set when diagnostics empty"),
    })
}

/// Classify one `MAX_BY(value, ordering)`/`MIN_BY(value, ordering)`
/// projection (`ArgMax`/`ArgMin`, `Monotone::Order`) — the order-monotone
/// overwrite family (`incremental_models.md` §"The column-family
/// catalogue").
///
/// **Storage decision** (`docs/outcomes/20260809-rung2-state-shapes` row 5).
/// The cross-window combiner needs the *stored* ordering value to compare a
/// new delta's ordering against. Rather than require the SELECT to already
/// project a `MAX(<ordering>)`/`MIN(<ordering>)` companion column, this
/// classifier decomposes the column to hidden `(v, o)` state
/// (`analysis::decomposed_state::decompose_to_state`) — the ordering value
/// lives in a state column invisible to consumers, never a user-facing
/// companion projection. Every `MAX_BY`/`MIN_BY` call of the right arity
/// admits this way; there is no stateless fast path even when the SELECT
/// happens to already project a matching companion (that companion, if
/// present, is classified separately as an ordinary extremal-fold output
/// column — it just no longer participates in this column's proof).
fn classify_order_monotone_column(
    text: &str,
    alias: &str,
    expr: &smelt_parser::Expr,
    sql_fn: SqlFunction,
    agg_upper: &str,
    aggregator_columns: &mut Vec<AggregatorColumn>,
    diagnostics: &mut Vec<KeyedDiagnostic>,
) {
    if !is_direct_function_call(text, agg_upper) {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: format!("composite expression `{}`", text.trim()),
        });
        return;
    }

    let Some(fc) = expr.as_function_call() else {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: text.trim().to_string(),
        });
        return;
    };
    let args = fc.arguments();
    if args.len() != 2 {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: format!(
                "{agg_upper} requires exactly 2 arguments (value, ordering), got {}",
                args.len()
            ),
        });
        return;
    }
    let value_text = args[0].text().trim().to_string();
    let ordering_text = args[1].text().trim().to_string();

    match crate::analysis::decomposed_state::decompose_to_state(
        sql_fn,
        false,
        &[&value_text, &ordering_text],
        alias,
    ) {
        Ok(state) => {
            let ordering_column = format!("{alias}__o");
            aggregator_columns.push(AggregatorColumn {
                output_name: alias.to_string(),
                per_partition_agg: agg_upper.to_string(),
                cross_partition_combiner: CrossPartitionCombiner::OrderMonotone {
                    ordering_column,
                    prefer_greater: sql_fn == SqlFunction::ArgMax,
                },
                state: Some(state),
            });
        }
        Err(refusal) => {
            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                projection: alias.to_string(),
                offending: format!(
                    "{agg_upper}({value_text}, {ordering_text}) could not be decomposed to \
                     hidden state: {refusal:?}"
                ),
            });
        }
    }
}

/// Classify one `AVG`/`STDDEV_*`/`VAR_*` projection — the decomposed-fold
/// family (`incremental_models.md` §"The column-family catalogue"). Mirrors
/// [`classify_order_monotone_column`]'s shape: verify the projection is a
/// *direct* call, then hand its argument(s) to
/// `analysis::decomposed_state::decompose_to_state` and admit on `Ok` with
/// `CrossPartitionCombiner::Recomputed` + the derived state, or refuse
/// `KeyedUnknownCombiner` on `Err` (`docs/outcomes/20260809-rung2-state-shapes`
/// row 7).
fn classify_decomposed_fold_column(
    text: &str,
    alias: &str,
    expr: &smelt_parser::Expr,
    sql_fn: SqlFunction,
    agg_upper: &str,
    aggregator_columns: &mut Vec<AggregatorColumn>,
    diagnostics: &mut Vec<KeyedDiagnostic>,
) {
    if !is_direct_function_call(text, agg_upper) {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: format!("composite expression `{}`", text.trim()),
        });
        return;
    }

    let Some(fc) = expr.as_function_call() else {
        diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
            projection: alias.to_string(),
            offending: text.trim().to_string(),
        });
        return;
    };
    let distinct = crate::analysis::has_distinct_keyword(&fc);
    let args = fc.arguments();
    let arg_texts: Vec<String> = args.iter().map(|a| a.text().trim().to_string()).collect();
    let arg_refs: Vec<&str> = arg_texts.iter().map(String::as_str).collect();

    match crate::analysis::decomposed_state::decompose_to_state(sql_fn, distinct, &arg_refs, alias)
    {
        Ok(state) => {
            aggregator_columns.push(AggregatorColumn {
                output_name: alias.to_string(),
                per_partition_agg: agg_upper.to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Recomputed,
                state: Some(state),
            });
        }
        Err(refusal) => {
            diagnostics.push(KeyedDiagnostic::KeyedUnknownCombiner {
                projection: alias.to_string(),
                offending: format!(
                    "{agg_upper}({}) could not be decomposed to hidden state: {refusal:?}",
                    arg_texts.join(", ")
                ),
            });
        }
    }
}

/// The `(family name, refusal reason)` the admission matrix
/// (`docs/specs/incremental_models.md` §"Admission matrix") names for a
/// fold-family aggregator refused under the snapshot-reconcile run shape.
/// `agg_upper` is one of `combiner_for`'s allowlisted names — the additive
/// family (`SUM`/`COUNT`/`BIT_XOR`) double-counts on a re-fold; the
/// remaining extremal/lattice family computes a history observation instead
/// of the current value.
fn snapshot_refusal_reason(agg_upper: &str) -> (&'static str, &'static str) {
    match agg_upper {
        "SUM" | "COUNT" | "BIT_XOR" => (
            "additive fold",
            "re-folding state double-counts — a mutable snapshot is not a replayable, \
             retraction-free event feed",
        ),
        "AVG" | "STDDEV" | "STDDEV_POP" | "STDDEV_SAMP" | "VARIANCE" | "VAR_POP" | "VAR_SAMP" => (
            "decomposed fold",
            "the hidden state (sum/count or n/Σx/Σx²) folds additively — re-folding a window \
             already reflected double-counts it, the same reason the additive fold family \
             refuses a mutable snapshot",
        ),
        _ => (
            "extremal/lattice fold",
            "observer semantics — folding successive snapshots computes the extremal value \
             ever observed, not the current one (e.g. `MIN(price)` folded over snapshots is \
             the min ever seen, not the current min)",
        ),
    }
}

/// Verify the projection is a direct call to `expected_fn` and nothing else —
/// i.e. it starts with `<fn>(` (after trimming) and the closing `)` is the
/// last non-whitespace character.
fn is_direct_function_call(text: &str, expected_fn: &str) -> bool {
    let trimmed = text.trim();
    let upper = trimmed.to_ascii_uppercase();
    let prefix = format!("{}(", expected_fn);
    // Allow qualified prefixes like `PG_CATALOG.SUM(`.
    let starts_ok = upper.starts_with(&prefix)
        || upper
            .rsplit('.')
            .next()
            .map(|s| s.starts_with(&prefix))
            .unwrap_or(false);
    if !starts_ok {
        return false;
    }
    // The expression must terminate at the matching close-paren.
    if !trimmed.ends_with(')') {
        return false;
    }
    // Confirm the close-paren matches the open-paren of the outer call —
    // i.e. the parenthesis depth returns to zero exactly at the end.
    let mut depth: i32 = 0;
    let mut closed_at_end = false;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    closed_at_end = i + ch.len_utf8() == trimmed.len();
                    break;
                }
            }
            _ => {}
        }
    }
    closed_at_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::Granularity;

    fn ts(partition_col: &str) -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: partition_col.to_string(),
            partition_column: partition_col.to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }
    }

    fn events_source_map() -> SourceTimeseriesMap {
        let mut m = HashMap::new();
        m.insert("smelt.silver.events_parsed".to_string(), ts("event_date"));
        m
    }

    /// The motivating device_user_edges SELECT classifies cleanly.
    #[test]
    fn test_classify_simple() {
        let sql = r#"SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id"#;

        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
            .expect("must classify");

        assert_eq!(classification.unique_key, vec!["device_id", "user_id"]);
        assert_eq!(classification.aggregator_columns.len(), 3);
        let event_count = &classification.aggregator_columns[0];
        assert_eq!(event_count.output_name, "event_count");
        assert_eq!(event_count.per_partition_agg, "COUNT");
        assert_eq!(
            event_count.cross_partition_combiner,
            CrossPartitionCombiner::Sum
        );
        let first_seen = &classification.aggregator_columns[1];
        assert_eq!(first_seen.output_name, "first_seen");
        assert_eq!(first_seen.per_partition_agg, "MIN");
        assert_eq!(
            first_seen.cross_partition_combiner,
            CrossPartitionCombiner::Min
        );
        let last_seen = &classification.aggregator_columns[2];
        assert_eq!(last_seen.output_name, "last_seen");
        assert_eq!(last_seen.per_partition_agg, "MAX");
        assert_eq!(
            last_seen.cross_partition_combiner,
            CrossPartitionCombiner::Max
        );
        assert_eq!(
            classification.driving_source.name,
            "smelt.silver.events_parsed"
        );
    }

    /// A SELECT with no GROUP BY produces KeyedRequiresGroupBy.
    #[test]
    fn test_no_group_by_refused() {
        let sql = "SELECT COUNT(*) AS n FROM smelt.silver.events_parsed";
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedRequiresGroupBy)),
            "diagnostics: {:?}",
            err
        );
    }

    /// STRING_AGG on a non-key projection produces KeyedUnknownCombiner.
    #[test]
    fn test_unknown_aggregator_refused() {
        let sql = r#"SELECT
    device_id,
    STRING_AGG(name, ',') AS names
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                    if offending.to_uppercase() == "STRING_AGG"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// A composite aggregate expression `SUM(x) + 1` is refused.
    #[test]
    fn test_composite_aggregate_expression_refused() {
        let sql = r#"SELECT
    device_id,
    SUM(amount) + 1 AS shifted_sum
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                    if offending.contains("composite") || offending.contains("expression")
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// COUNT(DISTINCT x) is refused — not commutative under merge.
    #[test]
    fn test_count_distinct_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(DISTINCT user_id) AS unique_users
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedUnknownCombiner { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// GROUP BY containing the driving source's partition_column is refused.
    #[test]
    fn test_group_by_contains_partition_column_refused() {
        let sql = r#"SELECT
    device_id,
    user_id,
    event_date,
    COUNT(*) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id, user_id, event_date"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { partition_column }
                    if partition_column == "event_date"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// The same GROUP BY-contains-partition-column shape, but the model
    /// declares its own `timeseries:` block: `KeyedGroupByContainsPartitionColumn`
    /// must NOT fire. The model is instead a candidate for the key-embedded
    /// locality route (`partition_column` is a `unique_key` column) and is
    /// decided by the locality gate
    /// (`maintenance::locality::establish_locality`), not refused here
    /// (`docs/specs/incremental_models.md` §"Key temporal locality").
    #[test]
    fn test_group_by_contains_partition_column_not_refused_when_model_has_timeseries() {
        let sql = r#"SELECT
    device_id,
    user_id,
    event_date,
    COUNT(*) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id, user_id, event_date"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let result = classify_cumulative(sql, &refs, &events_source_map(), true, &[]);
        match result {
            // With the check narrowed off, this shape has no other
            // rejection — GROUP BY over three plain columns plus a bare
            // COUNT(*) classifies cleanly, and `event_date` lands in the
            // derived unique_key exactly as it would for any other key
            // column.
            Ok(classification) => {
                assert!(
                    classification.unique_key.iter().any(|k| k == "event_date"),
                    "expected event_date in unique_key: {:?}",
                    classification.unique_key
                );
            }
            Err(diagnostics) => {
                assert!(
                    !diagnostics.iter().any(|d| matches!(
                        d,
                        KeyedDiagnostic::KeyedGroupByContainsPartitionColumn { .. }
                    )),
                    "KeyedGroupByContainsPartitionColumn must not fire when the model \
                     declares its own timeseries: block; diagnostics: {:?}",
                    diagnostics
                );
            }
        }
    }

    /// An OVER (...) projection produces KeyedForbidsWindowFunctions.
    #[test]
    fn test_window_function_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) OVER (PARTITION BY device_id) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedForbidsWindowFunctions)),
            "diagnostics: {:?}",
            err
        );
    }

    /// NOW() in the outer body is refused.
    #[test]
    fn test_nondeterministic_refused() {
        let sql = r#"SELECT
    device_id,
    MAX(event_ts) - NOW() AS stale_for
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedForbidsNondeterministic { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// A SELECT from a single unclocked source now derives the
    /// snapshot-reconcile run shape (Phase 3, `docs/plans/20260809-keyed-
    /// frontier.md`) instead of refusing the whole model outright — but a
    /// `COUNT(*)` additive-fold column is still refused, per column, with
    /// `KeyedSnapshotSourceUnsupportedColumn` naming the double-count
    /// reason.
    #[test]
    fn test_zero_clocked_sources_derives_snapshot_reconcile_but_refuses_fold_column() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.lookup_table".to_string()];
        let err = classify_cumulative(sql, &refs, &HashMap::new(), false, &[]).unwrap_err();
        assert!(
            !err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedSnapshotPostureUnsupported)),
            "the posture itself (a single unclocked source) is supportable now: {:?}",
            err
        );
        assert!(
            err.iter().any(|d| matches!(
                d,
                KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn { family, .. }
                    if family == "additive fold"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// Two timeseries-tagged sources produces KeyedMultipleDrivingSources.
    #[test]
    fn test_multiple_driving_sources_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.events_a
JOIN smelt.silver.events_b USING (device_id)
GROUP BY device_id"#;
        let refs = vec![
            "smelt.silver.events_a".to_string(),
            "smelt.silver.events_b".to_string(),
        ];
        let mut map = HashMap::new();
        map.insert("smelt.silver.events_a".to_string(), ts("event_date"));
        map.insert("smelt.silver.events_b".to_string(), ts("event_date"));
        let err = classify_cumulative(sql, &refs, &map, false, &[]).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, KeyedDiagnostic::KeyedMultipleDrivingSources { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// A timeseries-tagged ref that is only reached through a subquery (not
    /// one of the top-level FROM/JOIN inputs) is not a driving-fact
    /// candidate — the alias-scoped resolver only considers the joined
    /// inputs of the outer scope, unlike the former ref-count selection
    /// (which would have flat-counted both refs and refused as ambiguous).
    #[test]
    fn test_driving_source_resolved_via_alias_scope_ignores_non_joined_ref() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.events_a
WHERE device_id IN (SELECT device_id FROM smelt.silver.events_b)
GROUP BY device_id"#;
        let refs = vec![
            "smelt.silver.events_a".to_string(),
            "smelt.silver.events_b".to_string(),
        ];
        let mut map = HashMap::new();
        map.insert("smelt.silver.events_a".to_string(), ts("event_date"));
        map.insert("smelt.silver.events_b".to_string(), ts("event_date"));
        let classification = classify_cumulative(sql, &refs, &map, false, &[])
            .expect("must classify: only events_a is joined");
        assert_eq!(classification.driving_source.name, "smelt.silver.events_a");
    }

    /// A SELECT from one timeseries source and one lookup classifies cleanly.
    #[test]
    fn test_lookup_source_admitted() {
        let sql = r#"SELECT
    e.device_id,
    e.user_id,
    COUNT(*) AS event_count
FROM smelt.silver.events_parsed e
JOIN smelt.silver.user_lookup l USING (user_id)
WHERE l.is_active
GROUP BY e.device_id, e.user_id"#;
        let refs = vec![
            "smelt.silver.events_parsed".to_string(),
            "smelt.silver.user_lookup".to_string(),
        ];
        let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
            .expect("must classify");
        assert_eq!(
            classification.driving_source.name,
            "smelt.silver.events_parsed"
        );
    }

    /// Cross-partition combiners render correctly.
    #[test]
    fn test_combiner_rendering() {
        assert_eq!(
            CrossPartitionCombiner::Sum.render("target.x", "delta.x"),
            "target.x + delta.x"
        );
        assert_eq!(
            CrossPartitionCombiner::Min.render("target.x", "delta.x"),
            "LEAST(target.x, delta.x)"
        );
        assert_eq!(
            CrossPartitionCombiner::Max.render("target.x", "delta.x"),
            "GREATEST(target.x, delta.x)"
        );
        assert_eq!(
            CrossPartitionCombiner::BoolOr.render("target.x", "delta.x"),
            "target.x OR delta.x"
        );
    }

    /// combiner_for handles case-insensitive lookups.
    #[test]
    fn test_combiner_lookup_case_insensitive() {
        assert_eq!(combiner_for("count"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("COUNT"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("Sum"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("avg"), None);
        assert_eq!(combiner_for("string_agg"), None);
    }

    /// A projection aliased `spend__sum` alongside a state-bearing `spend`
    /// (whose derived state carries a `spend__sum` column) collides —
    /// `KeyedStateColumnCollision` names both the state column and the user
    /// column, and the reserved `__` suffix.
    #[test]
    fn state_column_collision_is_diagnosed() {
        use crate::analysis::decomposed_state::{DecomposedState, StateColumn};

        let aggregator_columns = vec![
            AggregatorColumn {
                output_name: "spend".to_string(),
                per_partition_agg: "AVG".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::PlainOverwrite,
                state: Some(DecomposedState {
                    state_columns: vec![
                        StateColumn {
                            name: "spend__sum".to_string(),
                            per_partition_expr: "SUM(amount)".to_string(),
                            combiner: CrossPartitionCombiner::Sum,
                        },
                        StateColumn {
                            name: "spend__count".to_string(),
                            per_partition_expr: "COUNT(amount)".to_string(),
                            combiner: CrossPartitionCombiner::Sum,
                        },
                    ],
                    presentation_expr: "spend__sum / spend__count".to_string(),
                }),
            },
            AggregatorColumn {
                output_name: "spend__sum".to_string(),
                per_partition_agg: "SUM".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
                state: None,
            },
        ];

        let diagnostics = diagnose_state_column_collisions(&aggregator_columns);
        assert_eq!(
            diagnostics,
            vec![KeyedDiagnostic::KeyedStateColumnCollision {
                state_column: "spend__sum".to_string(),
                user_column: "spend__sum".to_string(),
            }]
        );
        let message = diagnostics[0].to_string();
        assert!(message.contains("spend__sum"), "{message}");
        assert!(message.contains("__"), "{message}");
    }

    /// `state_column_summary` reports one entry for an `AVG` column's hidden
    /// `(sum, count)` state, naming both state columns and the presentation
    /// expression (`docs/outcomes/20260809-rung2-state-shapes` row 9).
    #[test]
    fn state_summary_reports_hidden_columns_for_avg() {
        let sql = r#"SELECT
    device_id,
    AVG(amount) AS avg_amount
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
            .expect("must classify");

        let summary = state_column_summary(&classification);
        assert_eq!(
            summary,
            vec![StateColumnSummary {
                presented_column: "avg_amount".to_string(),
                state_columns: vec![
                    "avg_amount__sum".to_string(),
                    "avg_amount__count".to_string()
                ],
                presentation_expr: "avg_amount__sum / avg_amount__count".to_string(),
            }]
        );
    }

    /// A `SUM`/`MAX`-only classification carries no state, so the summary
    /// section must not appear for rung-1 models.
    #[test]
    fn state_summary_is_empty_for_stateless_columns() {
        let sql = r#"SELECT
    device_id,
    SUM(amount) AS total_amount,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
            .expect("must classify");

        assert!(state_column_summary(&classification).is_empty());
    }

    /// `MAX_BY`'s `(v, o)` state and a fallback-bearing once-write column's
    /// `(value, written)` state both report — one entry per state-bearing
    /// column, regardless of family.
    #[test]
    fn state_summary_covers_order_monotone_and_once_write() {
        let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status,
    COALESCE(MAX(signup_referrer), 'unknown') AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let fds = vec![smelt_core::config::FunctionalDependency {
            key: vec!["device_id".to_string()],
            determines: "signup_referrer".to_string(),
        }];
        let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &fds)
            .expect("must classify");

        let summary = state_column_summary(&classification);
        let names: Vec<&str> = summary
            .iter()
            .map(|s| s.presented_column.as_str())
            .collect();
        assert_eq!(names, vec!["status", "first_referrer"]);
        assert_eq!(summary[0].state_columns, vec!["status__v", "status__o"]);
        assert_eq!(
            summary[1].state_columns,
            vec!["first_referrer__value", "first_referrer__written"]
        );
    }

    /// Every column family admitted today still classifies with `state:
    /// None` — the no-admission-widening guard for this phase
    /// (`docs/outcomes/20260809-rung2-state-shapes` phase 3).
    #[test]
    fn existing_keyed_classifications_carry_no_state() {
        let sql = r#"SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
            .expect("must classify");
        assert!(
            classification
                .aggregator_columns
                .iter()
                .all(|c| c.state.is_none()),
            "admission is not yet widened onto decomposed state: {:?}",
            classification.aggregator_columns
        );
    }
}
