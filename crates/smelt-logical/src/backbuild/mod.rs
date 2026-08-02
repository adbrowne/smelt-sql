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

pub mod classify;
pub mod diff;
pub mod emit;
pub mod requalify;

pub use classify::{assemble, derive_backbuild_options, Selection};
pub use diff::definition_diff;

use std::collections::BTreeMap;

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

/// The FROM/JOIN-tree + GROUP BY + dedup ("skeleton") diff. Also covers the
/// post-processing clauses that gate row-set, ordering, or row-count
/// semantics on their own — `HAVING`, `QUALIFY`, `WINDOW`, `ORDER BY`, and
/// `LIMIT`/`OFFSET` — since none of those has anywhere else to be
/// represented and a silent pass-through would misreport a real semantic
/// change as unchanged.
#[derive(Debug, Clone)]
pub enum SkeletonDiff {
    Unchanged,
    /// Otherwise-unchanged skeleton (same FROM target, same GROUP BY /
    /// DISTINCT, same HAVING/QUALIFY/WINDOW/ORDER BY/LIMIT, all pre-existing
    /// joins present and unchanged in order) plus one or more newly added
    /// `LEFT JOIN`s.
    AddedLeftJoins(Vec<JoinClause>),
    /// Any other skeleton difference: the FROM target changed, GROUP BY /
    /// DISTINCT changed, HAVING/QUALIFY/WINDOW/ORDER BY/LIMIT changed, an
    /// existing join's condition or type changed, a join was removed, or an
    /// added join is not a `LEFT JOIN`.
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

// ===== The option-set data model (research §2 "Options, not choices"; §3
// "Inputs and outputs"; §6 architecture) =====
//
// `classify.rs` is the only producer of these values (`derive_backbuild_options`)
// and the only place ordered statement scripts get assembled from them
// (`assemble`); this module just carries the pure data shapes.

/// Physical facts about the deployed model and its inputs, supplied
/// alongside the two SQL definitions (research §3 "Inputs and outputs").
/// Type inference over `added_column_types` lives in `smelt-db`, above this
/// crate — tests hand-write it.
#[derive(Debug, Clone)]
pub struct BackbuildInputs {
    /// Physical name of the deployed table.
    pub table: String,
    /// The after-definition's SQL text, verbatim. Needed for the
    /// model-level `FullRefresh` baseline (`CREATE OR REPLACE TABLE t AS
    /// <after>`) — not part of research §3's original field sketch, added
    /// here because [`DefinitionDiff`] deliberately does not retain either
    /// whole definition's raw text (see `diff.rs`'s clause-by-clause
    /// factoring), so classification has no other way to reconstruct it.
    pub after_sql: String,
    /// Declared row identity, or derived from `GROUP BY` — `None` when the
    /// model has neither.
    pub row_identity: Option<Vec<String>>,
    /// Added output column name → SQL type string.
    pub added_column_types: BTreeMap<String, String>,
    /// Upstream name (as referenced in the model's FROM/JOIN tree) → source
    /// facts.
    pub sources: BTreeMap<String, SourceRef>,
}

/// One upstream's physical facts, as declared to [`BackbuildInputs`].
#[derive(Debug, Clone)]
pub struct SourceRef {
    pub physical_name: String,
    pub unique_key: Option<Vec<String>>,
}

/// Every admissible technique for one atomic change, plus every named
/// inadmissibility, and the always-present model-level `FullRefresh`
/// baseline (research §2 "Options, not choices").
#[derive(Debug, Clone)]
pub struct BackbuildOptions {
    /// Per-atom analysis. Empty exactly when the diff is a no-op (research
    /// catalogue case A0) — never empty for a diff that changed something,
    /// even when nothing is classified yet, per fail-loud discipline: a
    /// diff this phase cannot factor into an admissible technique still
    /// yields a named refusal, never a silent "nothing to do".
    pub atoms: Vec<AtomAnalysis>,
    /// The model-level baseline: `CREATE OR REPLACE TABLE t AS <after>`.
    /// Always present, regardless of `atoms`, so the eventual cost model
    /// always has something to compare targeted scripts against.
    pub full_refresh: BackbuildOption,
}

/// One atomic change (research §4's "atoms") and everything classification
/// could prove or refuse about it.
#[derive(Debug, Clone)]
pub struct AtomAnalysis {
    pub change: AtomicChange,
    /// Every independently-proven admissible technique (research §2
    /// "Options, not choices" — never a single "chosen" verdict; a chooser
    /// picking among these is deliberately deferred).
    pub options: Vec<BackbuildOption>,
    /// Every named reason a technique was NOT admitted for this atom
    /// (fail-loud discipline — never a silent omission).
    pub inadmissible: Vec<BackbuildRefusal>,
}

/// This phase's atom granularity explodes the SELECT-list diff into one
/// atom per added or renamed column (research §4's B1/B2); everything else
/// (a changed existing column, a dropped column not paired into a rename, a
/// `WHERE`/set-operation change) still lands in the coarse
/// [`AtomicChange::Unclassified`] catch-all — those catalogue classes
/// (D/C/E/F) arrive in later phases.
#[derive(Debug, Clone)]
pub enum AtomicChange {
    /// The whole model definition, when the pure diff could not factor it
    /// at all ([`DefinitionDiff::Opaque`] — a changed CTE section, or
    /// one/both definitions not a plain `SELECT` this module recognises).
    WholeDefinition { reason: String },
    /// The FROM/JOIN tree, `GROUP BY`, `DISTINCT`, or another
    /// post-processing clause changed in a way the shared grain/join guards
    /// (research §4 intro, G-class) refuse outright — a grain change (G1)
    /// or a join-multiplicity change (G2).
    Skeleton { reason: String },
    /// One added SELECT-list output column (research §4 B1, and B3/B4/B5/B6
    /// in later phases) that was not consumed by rename pairing. `name` is
    /// the after-definition's output column name.
    AddedColumn { name: String },
    /// A dropped column `from` paired with an added column `to` whose
    /// expression is identical (modulo trivia) to `from`'s old expression
    /// (research §4 B2 "rename") — matched *before* add/drop classification
    /// so a genuine rename is never misread as a drop-plus-unrelated-add.
    RenamedColumn { from: String, to: String },
    /// A non-empty, non-skeleton-refused diff this phase does not yet
    /// classify into admissible techniques (research catalogue classes
    /// C/D/E/F, an unpaired dropped column, an ambiguous rename cluster, or
    /// an opaque SELECT-list/WHERE/set-operation diff). A conservative
    /// catch-all: an honest "not yet handled" refusal, never a silently
    /// empty atom list for a definition that did change.
    Unclassified,
}

/// One admissible technique for an atom (or the model-level `FullRefresh`
/// baseline), carrying its own statement script plus the research §2
/// cost/safety metadata a future chooser needs. Every option carries its
/// own proof path — there is no chooser or cost logic anywhere in this
/// module.
#[derive(Debug, Clone)]
pub struct BackbuildOption {
    pub technique: Technique,
    /// This option's place in research §4 "H. Composites"'s ordering
    /// (`renames → ALTER ADD/TYPE → DELETEs → UPDATEs/MERGEs → INSERTs →
    /// ALTER DROPs`), or `None` for a technique that is never composed into
    /// a targeted, per-atom script — today only [`Technique::FullRefresh`].
    pub slot: Option<HSlot>,
    /// The ordered statement strings this option executes, authored once
    /// here and only executed by callers (statement single-ownership,
    /// `docs/specs/architecture.md` §"Constraints & Invariants" item 12).
    /// Statement count (research §2 metadata) is `statements.len()` —
    /// see [`Self::statement_count`].
    pub statements: Vec<String>,
    pub write_scope: WriteScope,
    pub reads_upstream: bool,
    /// Whether re-running this option's statements against a table it has
    /// already applied to reproduces the same result (research §2
    /// "Idempotence").
    pub rerun_safe: bool,
}

impl BackbuildOption {
    /// Research §2 metadata: statement count, derived rather than stored
    /// separately so it can never drift from `statements`.
    pub fn statement_count(&self) -> usize {
        self.statements.len()
    }
}

/// The technique a [`BackbuildOption`] implements. Later phases add one
/// variant per remaining research §4 catalogue case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Technique {
    /// `CREATE OR REPLACE TABLE t AS <after>` — the always-present
    /// model-level baseline (research §2).
    FullRefresh,
    /// B1 — new column = pure function of existing stored columns
    /// (research §4): `ALTER TABLE t ADD COLUMN c <ty>; UPDATE t SET c =
    /// <requalified expr>;`.
    SelfDerivedColumnAdd,
    /// B2 — rename (research §4): `ALTER TABLE t RENAME COLUMN d TO a;`,
    /// zero rows touched.
    Rename,
}

/// The research §2 "write scope" option metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteScope {
    None,
    ColumnScoped,
    RowSubset,
    FullWrite,
}

/// The research §4 "H. Composites" ordering slots:
/// `renames → ALTER ADD/TYPE → DELETEs → UPDATEs/MERGEs → INSERTs → ALTER
/// DROPs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HSlot {
    Rename,
    Alter,
    Delete,
    UpdateMerge,
    Insert,
    Drop,
}

/// A named, fail-closed reason one technique was not admitted for `atom`
/// (research §2 "Refusal posture" — fail-loud discipline: every refusal is
/// named, never a silent fallback; actionable refusals name what would
/// admit the technique where a small edit or declaration would supply the
/// missing fact).
#[derive(Debug, Clone)]
pub struct BackbuildRefusal {
    pub atom: String,
    pub reason: String,
}
