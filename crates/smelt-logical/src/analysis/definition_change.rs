//! Definition-change column classification
//! (`docs/specs/model_properties.md` §"Definition-change column
//! classification"): the verdict `definition_deltas.md` §"The verdict per column group" needs when a model gains one output column.
//!
//! [`classify_definition_change`] composes three independently-owned proofs
//! rather than re-implementing any of them:
//!
//! 1. **Skeleton-role extraction** (`maintenance::skeleton::skeleton_roles`)
//!    — does the added column land in a row-membership/identity position?
//!    If so the change is a grain change ([`DefinitionChangeClass::
//!    SkeletonAdd`]), never a column backfill, regardless of what the
//!    remaining two legs would otherwise say.
//! 2. **The additive-only model-diff**
//!    (`analysis::model_diff::additive_only_diff`) — is this actually a
//!    pure addition, or does the "added" column's name collide with an
//!    existing stored column whose expression differs (a redefinition
//!    dressed up as an addition)? Only this structural (collision) half of
//!    its verdict is authoritative here; the diff's own per-column
//!    dependency loop cannot distinguish "reads only stored columns" from
//!    "aggregates the whole source with zero named dependencies"
//!    (`COUNT(*)`), which is leg 3's job.
//! 3. **Per-column provenance**
//!    (`analysis::model_diff::collect_dependencies`, plus an aggregate/
//!    window check over the same expression) — is the added column's
//!    expression a pure function of already-stored columns
//!    ([`DefinitionChangeClass::PureBackfill`]), or does it reach upstream
//!    ([`DefinitionChangeClass::UpstreamRederive`])?
//!
//! Fail-closed throughout (`model_properties.md` §Constraints): an
//! unclassifiable shape, a non-additive edit, or an unresolvable dependency
//! (a subquery, a window, an opaque/unregistered/non-deterministic
//! function) never guesses a verdict.

use std::collections::{BTreeSet, HashSet};

use smelt_parser::Expr;
use smelt_types::signatures::{BuiltinRegistry, ExprKind};

use crate::analysis::model_diff::{additive_only_diff, collect_dependencies, ColumnDef, ModelDiff};
use crate::maintenance::skeleton::skeleton_roles;

/// The verdict for one added output column
/// (`model_properties.md` §"Definition-change column classification").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionChangeClass {
    /// The added column occupies a row-membership/identity position — a
    /// grain change, never a column backfill (EX-39).
    SkeletonAdd { reason: String },
    /// A pure function of already-stored columns: no upstream read, an
    /// in-place `UPDATE` is admissible.
    PureBackfill,
    /// Re-derives from upstream (reads a column not yet stored, or
    /// aggregates over the source): a column-scoped `MERGE` is admissible,
    /// keyed where the source is keyed.
    UpstreamRederive,
}

/// Why [`classify_definition_change`] refused rather than guessing a
/// verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifyRefusal {
    /// `sql` has no classifiable top-level `SELECT` scope for skeleton-role
    /// extraction (`maintenance::skeleton::skeleton_roles` returned
    /// `None`).
    UnclassifiableShape,
    /// The edit is not a pure column addition — the "added" column's name
    /// collides with an existing stored column whose expression differs. A
    /// rebuild or declared migration is required, never a guessed
    /// backfill.
    NotAdditive { reason: String },
    /// The added column's own expression has an unresolvable dependency (a
    /// subquery, a window, an opaque/unregistered/non-deterministic
    /// function) — no technique can be proven safe.
    UnresolvableDependency { reason: String },
}

/// Everything [`classify_definition_change`] needs beyond the added column
/// and the model's current SQL.
#[derive(Debug, Clone)]
pub struct DefinitionChangeCtx<'a> {
    /// The model's existing (pre-change) output columns — leg 2's
    /// structural-collision check and leg 3's stored-column set.
    pub old_columns: &'a [ColumnDef],
    /// The output's declared unique key (world-fact, never derived).
    pub declared_unique_key: &'a [String],
    /// The output's partition column, for a partition-addressed grain.
    pub partition_col: Option<&'a str>,
    /// Skeleton columns already known by declaration — a caller's
    /// hand-supplied fallback for a model shape `skeleton_roles` cannot
    /// itself classify (`maintenance::skeleton` module doc: "or still
    /// hand-supplied by a caller"), unioned with the freshly-derived roles
    /// rather than replacing them.
    pub declared_skeleton_columns: &'a BTreeSet<String>,
    /// Declared monotone dimensions the additive-only diff also treats as
    /// backfill-safe reads (`analysis::model_diff::additive_only_diff`).
    pub monotone_dims: &'a [String],
}

/// Classify one added output column per `model_properties.md`
/// §"Definition-change column classification". See the module doc for the
/// three composed legs.
pub fn classify_definition_change(
    added_column: &ColumnDef,
    sql: &str,
    ctx: &DefinitionChangeCtx<'_>,
) -> Result<DefinitionChangeClass, ClassifyRefusal> {
    // Leg 1 — skeleton-role extraction.
    if ctx.declared_skeleton_columns.contains(&added_column.name) {
        return Ok(DefinitionChangeClass::SkeletonAdd {
            reason: format!(
                "'{}' is a declared skeleton column — a grain change, not a column backfill",
                added_column.name
            ),
        });
    }
    let roles = skeleton_roles(sql, ctx.declared_unique_key, ctx.partition_col)
        .ok_or(ClassifyRefusal::UnclassifiableShape)?;
    if let Some((_, role)) = roles.iter().find(|(name, _)| name == &added_column.name) {
        if role.is_skeleton() {
            return Ok(DefinitionChangeClass::SkeletonAdd {
                reason: format!(
                    "'{}' occupies a {role:?} skeleton position — a grain change, not a column \
                     backfill",
                    added_column.name
                ),
            });
        }
    }

    // Leg 2 — additive-only model-diff: only the structural (collision)
    // half of its verdict is authoritative here (see module doc).
    let mut new_columns: Vec<ColumnDef> = ctx.old_columns.to_vec();
    new_columns.push(added_column.clone());
    if let ModelDiff::NotAdditive { reason } =
        additive_only_diff(ctx.old_columns, &new_columns, ctx.monotone_dims)
    {
        let collides_with_existing = ctx.old_columns.iter().any(|c| c.name == added_column.name);
        if collides_with_existing {
            return Err(ClassifyRefusal::NotAdditive { reason });
        }
        // Otherwise the diff's only other failure mode, given `new_columns
        // = old_columns + [added_column]`, is the added column's own
        // dependency-derivability — leg 3 below resolves that more
        // precisely (an aggregate has zero named dependencies but is never
        // pure), so fall through rather than refuse here.
    }

    // Leg 3 — per-column provenance.
    match collect_dependencies(&added_column.expr) {
        Err(reason) => Err(ClassifyRefusal::UnresolvableDependency { reason }),
        Ok(deps) => {
            if contains_aggregate_call(&added_column.expr) {
                return Ok(DefinitionChangeClass::UpstreamRederive);
            }
            let stored: HashSet<&str> = ctx.old_columns.iter().map(|c| c.name.as_str()).collect();
            let monotone: HashSet<&str> = ctx.monotone_dims.iter().map(|s| s.as_str()).collect();
            if deps
                .iter()
                .all(|d| stored.contains(d.as_str()) || monotone.contains(d.as_str()))
            {
                Ok(DefinitionChangeClass::PureBackfill)
            } else {
                Ok(DefinitionChangeClass::UpstreamRederive)
            }
        }
    }
}

/// Whether `expr` contains a call to a registry-classified aggregate or
/// window function (`smelt_types::signatures::ExprKind::Agg` /
/// `Window`) anywhere in its subtree.
///
/// **Leaf classifier** (`docs/specs/architecture.md` §"Property composition
/// walk rule"): inspects only `expr`'s own already-bounded syntax subtree,
/// resolving each embedded function-call name against the shared
/// `BuiltinRegistry` — it does not scan free text.
fn contains_aggregate_call(expr: &Expr) -> bool {
    expr.syntax()
        .descendants()
        .filter_map(smelt_parser::FunctionCall::cast)
        .filter_map(|f| f.name())
        .any(|name| {
            BuiltinRegistry::resolve(&name)
                .map(|sig| matches!(sig.kind, ExprKind::Agg | ExprKind::Window))
                .unwrap_or(false)
        })
}
