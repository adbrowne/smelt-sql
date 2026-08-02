//! Backbuild synthesis — turning a (before, after) pair of model definitions
//! into targeted migration scripts instead of a full rebuild.
//!
//! See `docs/research/20260802-backbuild-synthesis.md` for the correctness
//! oracle this module implements against. This is the diff-foundation phase
//! (research §6 `diff.rs`): purely syntactic CST-level factoring of two
//! definitions into a [`DefinitionDiff`]. No admission judgements and no
//! classification happen here — that is later phases' `classify.rs`. This
//! module is deliberately unwired: nothing outside `smelt-logical` calls it
//! yet.

pub mod diff;

pub use diff::definition_diff;

use smelt_parser::{Expr, JoinClause, SelectStmt};

/// The CST-level factoring of a (before, after) pair of model definitions.
///
/// Fail-closed by construction: any shape the pure diff cannot confidently
/// factor (a changed CTE, an unrecognised top-level query form) lands in
/// [`DefinitionDiff::Opaque`] rather than being silently treated as
/// unchanged. Classification (a later phase) refuses on `Opaque`.
#[derive(Debug, Clone)]
pub enum DefinitionDiff {
    /// Both versions parsed as a plain `SELECT` whose `WITH`-prefix (if any)
    /// is unchanged modulo trivia; the final `SELECT` is factored
    /// clause-by-clause below.
    Comparable(Box<ComparableDiff>),
    /// The diff could not be factored beyond a whole-definition comparison —
    /// either the `WITH`-prefix (CTE section) differs between versions (the
    /// conservative CTE posture: an unchanged `WITH` prefix diffs the final
    /// `SELECT` normally, a changed one refuses), or one/both definitions
    /// are not a plain `SELECT` statement this module recognises.
    Opaque { reason: String },
}

impl DefinitionDiff {
    /// Whole-definition no-op verdict (research catalogue case A0):
    /// whitespace, comment, and case-preserving-reformat-only changes
    /// between versions. `false` for any `Opaque` diff — a diff this module
    /// could not factor is never asserted to be a no-op.
    pub fn is_noop(&self) -> bool {
        match self {
            DefinitionDiff::Comparable(c) => c.is_noop(),
            DefinitionDiff::Opaque { .. } => false,
        }
    }
}

/// The clause-by-clause factoring of a comparable (non-opaque-CTE) pair of
/// definitions.
#[derive(Debug, Clone)]
pub struct ComparableDiff {
    pub select_list: SelectListDiff,
    pub where_clause: ConjunctDiff,
    pub skeleton: SkeletonDiff,
    pub set_ops: SetOpDiff,
}

impl ComparableDiff {
    pub fn is_noop(&self) -> bool {
        self.select_list.is_noop()
            && self.where_clause.is_noop()
            && self.skeleton.is_noop()
            && self.set_ops.is_noop()
    }
}

/// One SELECT-list output column: its name and the expression that computes
/// it. Carried by `select_list.added`/`dropped`/`unchanged`.
#[derive(Debug, Clone)]
pub struct SelectColumn {
    pub name: String,
    pub expr: Expr,
}

/// A SELECT-list output column present in both versions under the same name,
/// whose computing expression differs (modulo trivia).
#[derive(Debug, Clone)]
pub struct ChangedColumn {
    pub name: String,
    pub before: Expr,
    pub after: Expr,
}

/// SELECT-list diff, keyed on output column name (research §6 `diff.rs`:
/// "added / dropped / changed / unchanged (per column, Expr pairs)").
#[derive(Debug, Clone)]
pub enum SelectListDiff {
    Diffed {
        added: Vec<SelectColumn>,
        dropped: Vec<SelectColumn>,
        changed: Vec<ChangedColumn>,
        unchanged: Vec<SelectColumn>,
    },
    /// A SELECT list this module cannot key by output column name — a
    /// wildcard (`*`/`qualifier.*`), a spread (`...expr`), a missing
    /// expression, or duplicate output names on either side.
    Opaque { reason: String },
}

impl SelectListDiff {
    pub fn is_noop(&self) -> bool {
        matches!(
            self,
            SelectListDiff::Diffed { added, dropped, changed, .. }
                if added.is_empty() && dropped.is_empty() && changed.is_empty()
        )
    }
}

/// The `WHERE`-clause conjunct-set diff (top-level `AND`s only).
#[derive(Debug, Clone)]
pub enum ConjunctDiff {
    Diffed {
        added: Vec<Expr>,
        removed: Vec<Expr>,
        unchanged: Vec<Expr>,
    },
    /// One or both sides is not a top-level conjunction — e.g. a top-level
    /// `OR` in place of what used to be (or becomes) an `AND`-list. A
    /// conjunct-set add/remove framing is unsound for a non-conjunctive
    /// rewrite (removing a conjunct always relaxes an `AND`-predicate;
    /// removing a disjunct does the opposite), so this refuses rather than
    /// reporting a member swap.
    Opaque { reason: String },
}

impl ConjunctDiff {
    pub fn is_noop(&self) -> bool {
        matches!(
            self,
            ConjunctDiff::Diffed { added, removed, .. }
                if added.is_empty() && removed.is_empty()
        )
    }
}

/// The FROM/JOIN-tree + GROUP BY + dedup ("skeleton") diff.
#[derive(Debug, Clone)]
pub enum SkeletonDiff {
    Unchanged,
    /// Otherwise-unchanged skeleton (same FROM target, same GROUP BY /
    /// DISTINCT, all pre-existing joins present and unchanged in order) plus
    /// one or more newly added `LEFT JOIN`s.
    AddedLeftJoins(Vec<JoinClause>),
    /// Any other skeleton difference: the FROM target changed, GROUP BY /
    /// DISTINCT changed, an existing join's condition or type changed, a
    /// join was removed, or an added join is not a `LEFT JOIN`.
    Changed {
        reason: String,
    },
}

impl SkeletonDiff {
    pub fn is_noop(&self) -> bool {
        matches!(self, SkeletonDiff::Unchanged)
    }
}

/// The set-operation (`UNION ALL`) branch diff: a multiset comparison over
/// whole branches, so reordered-but-otherwise-identical branches compare
/// unchanged.
#[derive(Debug, Clone)]
pub enum SetOpDiff {
    /// Neither version has a top-level set operation.
    NotApplicable,
    Branches {
        added: Vec<SelectStmt>,
        removed: Vec<SelectStmt>,
        unchanged: Vec<SelectStmt>,
    },
    /// A set-operation shape this module does not diff: plain `UNION`
    /// (dedup) or `INTERSECT`/`EXCEPT` rather than `UNION ALL`, a `BY NAME`
    /// modifier, or set-operation presence differs in a way that isn't a
    /// pure add/remove of `UNION ALL` branches.
    Opaque { reason: String },
}

impl SetOpDiff {
    pub fn is_noop(&self) -> bool {
        match self {
            SetOpDiff::NotApplicable => true,
            SetOpDiff::Branches { added, removed, .. } => added.is_empty() && removed.is_empty(),
            SetOpDiff::Opaque { .. } => false,
        }
    }
}
