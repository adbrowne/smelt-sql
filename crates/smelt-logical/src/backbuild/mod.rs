//! Backbuild synthesis — turning a (before, after) pair of model definitions
//! into targeted migration scripts instead of a full rebuild.
//!
//! See `docs/research/20260802-backbuild-synthesis.md` for the correctness
//! oracle this module implements against. This is the diff-foundation phase
//! (research §6 `diff.rs`): purely syntactic CST-level factoring of two
//! definitions into a [`DefinitionDiff`]. No admission judgements and no
//! classification happen here — that is later phases' `classify.rs`.
//! `plan.rs` re-shapes a [`BackbuildOptions`] into the printable
//! [`plan::MigrationPlan`] `smelt migrate` renders
//! (`docs/specs/definition_deltas.md` §Overview); the ranged-rebuild path
//! (`smelt backbuild`) does not consume this module.

pub mod classify;
pub mod diff;
pub mod emit;
pub mod hash;
pub mod plan;
pub mod requalify;

pub use classify::{assemble, derive_backbuild_options, Selection};
pub use diff::definition_diff;
pub use hash::plan_hash;
pub use plan::{
    derive_migration_plan, ColumnGroupPlan, CostClass, MigrationPlan, TechniqueCandidate, Verdict,
};

use std::collections::{BTreeMap, BTreeSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{Expr, JoinClause, SelectStmt};

use crate::analysis::walk::{self, LeafColumn};

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
        /// The `before` side's own declared output column names, in
        /// declared order (including dropped/changed/unchanged columns —
        /// every column `before` declares, positionally). Consumed by the
        /// multi-branch pure-diff gate's declared-order equality check
        /// (research §4 F1 "Trap … the C4 reorder guard"): a name-keyed
        /// no-op (same names, same expressions) can still hide a pure
        /// column-order swap, which a name-based `INSERT`/`UNION ALL`
        /// positional-binding check must not trust.
        before_order: Vec<String>,
        /// The `after` side's own declared output column names, in
        /// declared order — also this side's true first-branch column
        /// order for F1's positional-`UNION ALL` validation
        /// (`classify.rs`'s `first_branch_order`), correct even when the
        /// SELECT list itself changed (unlike reading it off `unchanged`,
        /// which only ever reflects `before`'s order and silently drops
        /// added/changed columns' positions).
        after_order: Vec<String>,
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
    ///
    /// `before`/`after` retain the raw (un-factored) `WHERE` expression from
    /// each side, when present — the conjunct-set diff could not be
    /// factored, but B4's opaque-WHERE alias sweep
    /// (`classify.rs`'s `admit_added_left_join`) still needs to know whether
    /// *either* side's whole predicate references a newly-added join's
    /// alias, since a top-level `OR` is exactly the shape where "no added
    /// conjuncts" would otherwise be misread as "nothing changed".
    Opaque {
        reason: String,
        before: Option<Expr>,
        after: Option<Expr>,
    },
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
    /// added join is not a `LEFT JOIN`. `cause` is `diff.rs`'s own
    /// structural classification of which comparison produced this —
    /// substrate-unification Phase 5's replacement for `classify.rs`
    /// deriving the same classification by lowercased-`.contains` scanning
    /// `reason`'s English prose after the fact.
    Changed {
        reason: String,
        cause: SkeletonCause,
    },
}

/// `diff.rs`'s own structural classification of a [`SkeletonDiff::Changed`]
/// difference — set at the exact comparison that produced it, so
/// `classify.rs`'s research §4 G1/G2 catalogue labelling reads this
/// directly instead of re-deriving it by scanning `reason`'s free English
/// text for substrings like `"group by"` or `"join"` (substrate-unification
/// Phase 5: that scan could misfire on a `reason` string that happens to
/// *mention* the other class's vocabulary — e.g. a grain change over a
/// column literally named `join_key` — without describing it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkeletonCause {
    /// Research §4 G1: `GROUP BY` keys added/removed/changed, or `DISTINCT`
    /// toggled.
    GrainChanged,
    /// Research §4 G2: an existing join's condition or type changed, a join
    /// was removed or reordered incompatibly, or an added join is not a
    /// `LEFT JOIN`.
    JoinShapeChanged,
    /// Any other skeleton difference this module refuses without a G1/G2
    /// label: the FROM target changed, or a post-processing clause
    /// (`HAVING`/`QUALIFY`/`WINDOW`/`ORDER BY`/`LIMIT`) changed.
    Other,
}

impl SkeletonDiff {
    pub fn is_noop(&self) -> bool {
        matches!(self, SkeletonDiff::Unchanged)
    }
}

/// The set-operation (`UNION ALL`) branch diff: a multiset comparison over
/// whole branches by exact text, so reordered-but-otherwise-identical
/// branches compare unchanged; leftover (non-exact-text-matched) branches on
/// each side are then paired up positionally among themselves and diffed as
/// an [`EditedBranch`] — a surviving branch that carries its own edit,
/// distinct from a branch that is purely added or purely removed (research
/// §4 F1's edited-survivor generalization).
#[derive(Debug, Clone)]
pub enum SetOpDiff {
    /// Neither version has a top-level set operation.
    NotApplicable,
    Branches {
        added: Vec<SelectStmt>,
        removed: Vec<SelectStmt>,
        unchanged: Vec<SelectStmt>,
        /// Surviving branches paired by position among the leftovers once
        /// exact-text matching is exhausted, each carrying its own
        /// SELECT-list/WHERE/skeleton diff. `classify.rs` admits atoms from
        /// an edited branch only when it is uniquely `index == 0` and every
        /// other pair is exact (`removed` empty) — the top-level
        /// (`definition_diff`'s own) select-list/WHERE/skeleton diff *is*
        /// branch 0's diff by construction only in that case; any other
        /// edited-branch shape refuses by name (research §4 F1 trap,
        /// generalized).
        edited: Vec<EditedBranch>,
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
            SetOpDiff::Branches {
                added,
                removed,
                edited,
                ..
            } => added.is_empty() && removed.is_empty() && edited.is_empty(),
            SetOpDiff::Opaque { .. } => false,
        }
    }
}

/// One surviving `UNION ALL` branch, matched against its old self by
/// position among the leftovers (research §4 F1's edited-survivor
/// generalization — see [`SetOpDiff::Branches`]). `index` is the branch's
/// position within the **before**-definition's own branch chain (0 =
/// declared first); `after_index` is the branch it was *paired* against's
/// position within the **after**-definition's own branch chain. Neither
/// alone determines whether the top-level (whole-definition)
/// SELECT-list/WHERE/skeleton diff actually describes this branch's edit —
/// `definition_diff` computes those diffs from each side's outermost
/// `SelectStmt` node, i.e. **before**'s own branch 0 against **after**'s own
/// branch 0. That top-level diff is trustworthy for this edited branch only
/// when *both* `index == 0` and `after_index == 0`: `set_op_diff` pairs
/// leftovers by their position *among the leftovers*, not by declared
/// chain position, so an edited branch whose before-position is 0 can still
/// be paired against an after-leftover that is not after's own declared
/// branch 0 (e.g. when after's literal branch 0 exact-text-matched a
/// different before-branch and was consumed into `unchanged`) — in that
/// shape the top-level diff describes neither this pairing's before side
/// nor its after side, and must not be trusted.
#[derive(Debug, Clone)]
pub struct EditedBranch {
    pub index: usize,
    pub after_index: usize,
    pub select_list: SelectListDiff,
    pub where_clause: ConjunctDiff,
    pub skeleton: SkeletonDiff,
}

// ===== The option-set data model (research §2 "Options, not choices"; §3
// "Inputs and outputs"; §6 architecture) =====
//
// `classify.rs` is the only producer of these values (`derive_backbuild_options`)
// and the only place ordered statement scripts get assembled from them
// (`assemble`); this module just carries the pure data shapes.

/// One stored 1:1 representative's own upstream provenance (research §4
/// intro "Derivability representatives — one uniform rule"): the FROM-tree
/// qualifier its bare pull-through expression reads from (`None` for an
/// unqualified reference), the raw column name at that qualifier, and the
/// representative's own output name — the name actually stored in the
/// deployed table. `classify.rs` builds these from the diff's `unchanged`
/// SELECT items; `requalify.rs` and `classify.rs`'s B1/D1 derivability
/// proofs both resolve a dependency against them via
/// [`resolve_representative`] — the single provenance-matching machinery
/// every derivability/grain-link proof in this module shares (B1/D1/E1/E2/E4
/// via [`resolve_representative`]; B3/B4/B7/D2 via a direct
/// qualifier-and-raw-name match against this same struct, since those proofs
/// already know which single alias they are binding to).
#[derive(Debug, Clone)]
pub struct RepresentativeSource {
    pub output_name: String,
    pub qualifier: Option<String>,
    pub raw_name: String,
    /// This representative's own base-relation leaf, chased through CTE /
    /// derived-table renames from the definition that produced it
    /// (`analysis::walk::resolve_reference_leaf`, substrate-unification
    /// Phase 5) — `None` when no anchor syntax tree was available to chase
    /// from (e.g. a B7 representative synthesized for a newly-added join
    /// alias, which has no defining SELECT scope of its own to walk) or the
    /// walk could not resolve it. [`resolve_representative`]'s lineage
    /// fallback matches against this when the flat qualifier/raw-name
    /// comparison misses.
    pub leaf: Option<LeafColumn>,
}

/// Resolve one column-reference dependency — `(qualifier, raw_name)`, read
/// straight off the CST — against the stored 1:1 representative set, per the
/// uniform rule (research §4 intro) and the fix for final-review finding C2.
///
/// A **qualified** dependency `q.n` is satisfied only by a representative
/// whose own provenance is bound to exactly that qualifier and raw column
/// name (`qualifier == Some(q) && raw_name == n`) — never by any
/// representative that merely happens to share output name `n` under a
/// *different* qualifier, or none at all. A **bare** (unqualified)
/// dependency `n` is satisfied only by a name-preserving representative
/// (`raw_name == output_name == n`) — the representative's own source column
/// is *also* named `n`, so an unqualified reference to `n` cannot be
/// confused with a same-named-but-differently-sourced qualified
/// representative.
///
/// This is the fix for the reproduced C2 unsoundness: matching by bare
/// output name alone (ignoring the dependency's own qualifier entirely) let
/// a qualified reference to a brand-new join alias (`d.region_u`, whose
/// dependency walk previously discarded the qualifier down to bare `region`)
/// silently resolve against an unrelated pre-existing representative that
/// also happened to be named `region` (`o.rc AS region`) — emitting a
/// self-read `UPDATE` that read the wrong stored column instead of refusing
/// (or routing to an upstream-read technique).
///
/// substrate-unification Phase 5 widening: when the flat comparison above
/// finds nothing, `dependency_node` — the syntax node the `(qualifier,
/// raw_name)` pair was itself read off, when the caller has one at hand —
/// licenses a lineage fallback. The flat rule only ever matches a dependency
/// written with the exact same qualifier and raw column name as the
/// representative's own recorded text; it cannot see that two differently
/// -named references (e.g. a bare reference to a CTE's already-renamed
/// column, versus that same column re-renamed *again* at this scope's own
/// SELECT list) name the very same stored data. The fallback chases both
/// sides to their base-relation leaf (`analysis::walk`'s `ColumnLineage`,
/// through CTE/derived-table projections — [`chase_leaf`]) and matches by
/// leaf identity instead — sound because a shared leaf *proves* the same
/// physical column, never a same-named-but-differently-sourced collision
/// (the C2 hazard the flat rule above was written to avoid: this is an
/// independent, structural proof of identity, not a relaxation of it).
pub(crate) fn resolve_representative<'a>(
    qualifier: Option<&str>,
    raw_name: &str,
    representative_sources: &'a [RepresentativeSource],
    dependency_node: Option<&SyntaxNode>,
) -> Option<&'a RepresentativeSource> {
    let flat = match qualifier {
        Some(q) => representative_sources
            .iter()
            .find(|r| r.qualifier.as_deref() == Some(q) && r.raw_name == raw_name),
        None => representative_sources
            .iter()
            .find(|r| r.raw_name == raw_name && r.output_name == raw_name),
    };
    if flat.is_some() {
        return flat;
    }

    let dep_leaf = chase_leaf(dependency_node?, qualifier, raw_name)?;
    representative_sources
        .iter()
        .find(|r| r.leaf.as_ref() == Some(&dep_leaf))
}

/// Chase `(qualifier, raw_name)` — a column reference read straight off
/// `node`'s own enclosing `SELECT` statement — to its base-relation leaf, via
/// [`walk::QueryTree::from_select`] + [`walk::resolve_reference_leaf`]. Used
/// both by [`resolve_representative`]'s lineage fallback and by
/// `classify.rs`'s `representative_sources` (to record each representative's
/// own leaf up front). `None` when `node` has no enclosing `SELECT`
/// (unreachable for any node backbuild constructs from a parsed definition)
/// or the walk cannot resolve the reference.
pub(crate) fn chase_leaf(
    node: &SyntaxNode,
    qualifier: Option<&str>,
    raw_name: &str,
) -> Option<LeafColumn> {
    let stmt = node.ancestors().find_map(SelectStmt::cast)?;
    let tree = walk::QueryTree::from_select(&stmt);
    walk::resolve_reference_leaf(&tree, qualifier, raw_name)
}

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
    /// Declared NOT NULL columns of the model's own deployed table (a schema
    /// fact, not derived here — mirrors [`SourceRef::not_null_columns`], but
    /// for `table` itself rather than an upstream). Consumed by the shared
    /// key-addressability obligation (research §4 intro "Key
    /// addressability") for E4's identity anti-join guard: `t.<id> =
    /// __backbuild_diff.<id>` never matches a NULL-keyed row on either side,
    /// so a NULL `row_identity` column would let a rerun silently re-insert
    /// an already-inserted row — the guard (and `rerun_safe: true`) is only
    /// admitted when every `row_identity` column is present here. Missing
    /// here means "not proven" — fail-closed, never assumed, same posture
    /// as `SourceRef::not_null_columns`.
    pub not_null_columns: BTreeSet<String>,
    /// Added output column name → SQL type string.
    pub added_column_types: BTreeMap<String, String>,
    /// Upstream name (as referenced in the model's FROM/JOIN tree) → source
    /// facts.
    pub sources: BTreeMap<String, SourceRef>,
}

/// One upstream's physical facts, as declared to [`BackbuildInputs`].
///
/// Keyed by the FROM-tree reference name the model's own SQL uses to reach
/// this upstream — its alias when the model aliases it, otherwise its bare
/// table identifier. A self-join (`orders o1 JOIN orders o2`) declares one
/// entry *per alias* (`"o1"`, `"o2"`), even though both share the same
/// `physical_name` — the B3/D2 grain-link proof binds per FROM-tree alias,
/// not per physical table (research
/// `docs/research/20260802-backbuild-synthesis.md` §4 B3).
#[derive(Debug, Clone)]
pub struct SourceRef {
    pub physical_name: String,
    pub unique_key: Option<Vec<String>>,
    /// Declared NOT NULL columns of this source (a schema fact, not derived
    /// here) — the shared key-addressability obligation (research §4 intro
    /// "Key addressability") consumes this to prove `unique_key` columns are
    /// NOT NULL before admitting an equality backfill. `analysis::not_null`
    /// does not apply here: `partition_column_provably_not_null` is a narrow
    /// proof scoped to a *timeseries model's own* partition column derived
    /// from its driving source's clock column, not a general "is this
    /// upstream's column NOT NULL" fact — so this mirrors the
    /// `ModelInputs`/`SourceFacts` plain-data pattern
    /// (`maintenance/derive.rs`) instead. Missing here means "not proven" —
    /// fail-closed, never assumed.
    pub not_null_columns: BTreeSet<String>,
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
    /// One dropped SELECT-list output column left over after rename pairing
    /// (research §4 C1). `name` is the before-definition's output column
    /// name. Classification here only *sequences* the drop (research §4 "H.
    /// Composites": last, after every statement that might read the
    /// column); whether drops are permitted at all
    /// (`--allow-column-removal`) is a policy decision this module never
    /// makes — it stays owned by `docs/specs/schema_evolution.md` and is
    /// applied at wiring time.
    DroppedColumn { name: String },
    /// A SELECT-list output column present in both versions under the same
    /// name whose computing expression differs (research §4 D-class; D1 in
    /// this phase, D2 in a later one). `name` is the column's output name.
    ChangedColumn { name: String },
    /// One added `WHERE`-clause conjunct (research §4 E1 "filter
    /// tightened") — classified and scripted independently per conjunct
    /// (unlike E4's single paired atom), each its own `DELETE ... WHERE
    /// (<q>) IS NOT TRUE`. `index` is this conjunct's position within the
    /// diff's `added` conjunct set.
    AddedConjunct { index: usize },
    /// A removed+added `WHERE`-clause conjunct pair recognised as a
    /// provably-widened range predicate on one column (research §4 E4
    /// "time-horizon extension"). `column` is the range column's raw name.
    /// The range view classifies only — the emitted script is always the
    /// complement-form difference `INSERT` (research §4 E4).
    RangePredicateChange { column: String },
    /// One removed `WHERE`-clause conjunct (research §4 E2 "filter
    /// loosened"), admitted when it is the diff's *only* predicate change —
    /// no added conjunct alongside it. `index` is this conjunct's position
    /// within the diff's `removed` conjunct set (today always `0`: this
    /// phase only admits a single removed conjunct at a time).
    RemovedConjunct { index: usize },
    /// One added `UNION ALL` branch (research §4 F1), admitted when it is
    /// the diff's *only* set-operation change — no removed branch, no other
    /// added branch. `index` is this branch's position within the diff's
    /// `added` branch set (today always `0`: this phase only admits a
    /// single added branch at a time).
    AddedSetOpBranch { index: usize },
    /// One removed `UNION ALL` branch (research §4 F2), admitted when it is
    /// the diff's *only* set-operation change — no added branch, and the
    /// branch is distinguishable in the stored table by a discriminator
    /// (a column that is a distinct literal constant in every branch of the
    /// before-definition). `index` is this branch's position within the
    /// diff's `removed` branch set (today always `0`: this phase only
    /// admits a single removed branch at a time).
    RemovedSetOpBranch { index: usize },
    /// A non-empty, non-skeleton-refused diff this phase does not yet
    /// classify into admissible techniques (research catalogue class F, an
    /// ambiguous rename cluster, a D-class change this phase cannot admit as
    /// D1, a `WHERE`-clause change this phase's E-class (E1/E4 only) cannot
    /// admit, or an opaque SELECT-list/WHERE/set-operation diff). A
    /// conservative catch-all: an honest "not yet handled" refusal, never a
    /// silently empty atom list for a definition that did change.
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
    /// D1 — changed existing-column expression, derivable from stored
    /// columns (research §4): `UPDATE t SET c = <requalified expr>;` —
    /// siblings untouched, no `ALTER`, no upstream read.
    SelfDerivedColumnRewrite,
    /// B3 (added column) / D2 (changed column) — one admission path, two
    /// triggers (research §4 B3/D2): the output carries a 1:1 bare
    /// pull-through of an upstream's declared `unique_key`, bound per
    /// FROM-tree alias. `ALTER TABLE t ADD COLUMN c <ty>;` precedes the
    /// script for B3 only (D2's column already exists);
    /// `UPDATE t SET c = <requalified expr over u> FROM <upstream> u WHERE
    /// t.<k> = u.<k>` either way.
    UpstreamPullthrough,
    /// B4 (research §4 B4), bare-pull-through shape: an added column that is
    /// a bare pull-through (`d.x`) of a newly-added `LEFT JOIN`'s alias,
    /// admitted once the join's row-set-preservation proof holds (LEFT +
    /// at-most-one-match on the dimension side + the join key's NOT NULL
    /// obligation). `ALTER TABLE t ADD COLUMN c <ty>;` then
    /// `UPDATE t SET c = d.x FROM dim d WHERE t.<k> = d.<k>` — unmatched
    /// rows stay at the post-`ALTER` NULL, exactly LEFT-JOIN semantics.
    /// Only ever offered alongside [`Technique::JoinEnrichmentScalarSubquery`]
    /// for the *same* atom (research §2 "Options, not choices"), never
    /// alone — a bare pull-through always admits both shapes. Safety
    /// asymmetry worth recording (research §2): under a wrongly-declared
    /// `unique_key`, this shape silently picks one of several duplicate
    /// dimension matches, whereas the scalar-subquery sibling errors loudly
    /// at run time — a chooser reason to prefer the loud shape when
    /// uniqueness is FD-derived rather than declared.
    JoinEnrichmentUpdateFrom,
    /// B4 (research §4 B4), per-reference substituted scalar-subquery shape:
    /// each dimension-column reference inside the added expression is
    /// individually replaced by its own scalar subquery
    /// (`(SELECT d.x FROM dim d WHERE t.<k> = d.<k>)`), the surrounding
    /// expression preserved around them — a subquery over zero matching
    /// rows yields NULL for exactly that one reference, so a general
    /// expression like `COALESCE(d.x, 'none')` still NULL-extends correctly
    /// (unlike wrapping the *whole* expression in one subquery, which would
    /// null the entire result before `COALESCE` ever runs — research §4
    /// B4). The only option offered for a general expression; offered
    /// alongside [`Technique::JoinEnrichmentUpdateFrom`] for a bare
    /// pull-through. Each subquery also errors loudly on a duplicate
    /// dimension match — the safety-asymmetry counterpart to
    /// `JoinEnrichmentUpdateFrom`'s doc comment (research §2).
    JoinEnrichmentScalarSubquery,
    /// E1 — filter tightened (research §4): `DELETE FROM t WHERE (<q'>) IS
    /// NOT TRUE;`, `q'` the added conjunct requalified to stored columns.
    /// Three-valued logic pinned at the single authoring site
    /// (`emit::emit_predicate_delete`): a NULL-evaluating conjunct deletes
    /// the row — a bare `NOT q'` would wrongly keep it (research §4 E1's
    /// named regression trap). No upstream read.
    PredicateTightenDelete,
    /// E4 — time-horizon extension (research §4): a region-scoped
    /// difference `INSERT` built from the after-definition's own SELECT
    /// body, scoped by the always-complement-form predicate (the *added*
    /// conjunct is already implied by reading the after-definition; the
    /// script adds `AND (<removed conjunct>) IS NOT TRUE`, requalified to
    /// stored columns) — this gets every boundary and NULL case right by
    /// construction (research §4 E4). Carries an identity anti-join guard,
    /// making reruns safe, only when `BackbuildInputs::row_identity` is
    /// declared *and* every identity column is present in
    /// `BackbuildInputs::not_null_columns` — an equality anti-join never
    /// matches a NULL-keyed row on either side, so a nullable identity
    /// column would let a rerun silently re-insert (research §4 intro "Key
    /// addressability"). Otherwise the option omits the guard and records
    /// `rerun_safe: false` (research §2 "Idempotence").
    HorizonExtensionInsert,
    /// E2 — filter loosened (research §4): the after-definition's own
    /// difference `INSERT`, scoped by the always-complement-form predicate
    /// (`AND (<removed conjunct>) IS NOT TRUE`, requalified to stored
    /// columns) — exactly the shape `HorizonExtensionInsert` uses, minus the
    /// range-widening proof (research §4 E4: "mechanically an E2 whose
    /// removed/added conjuncts are range predicates"). Carries the same
    /// identity anti-join guard posture as `HorizonExtensionInsert`
    /// (`rerun_safe` follows the same NOT-NULL-proven-identity
    /// conditionality).
    FilterLoosenInsert,
    /// F1 — new `UNION ALL` branch (research §4): `INSERT INTO t (<cols>)
    /// SELECT <cols> FROM (<branch>) …` — `UNION ALL` is additive, so the
    /// added branch is exactly the delta; no difference predicate is needed
    /// (unlike `FilterLoosenInsert`/`HorizonExtensionInsert`). Carries the
    /// same identity anti-join guard posture as those two techniques.
    UnionBranchInsert,
    /// F2 — removed `UNION ALL` branch (research §4): `DELETE FROM t WHERE
    /// <discriminator column> = <discriminator literal>;` — the discriminator
    /// is a column proven (`classify.rs`'s `find_branch_discriminator`) to be
    /// a distinct literal constant in *every* branch of the before-definition
    /// (not re-derived from `analysis/discriminants.rs`, which classifies
    /// aggregate-combiner algebra and has no branch-constant-column concept
    /// to consume — see this technique's admission site for the note),
    /// including the removed one; the removed branch's own literal, read
    /// straight off its declared expression, builds the predicate. An
    /// **equality** predicate, not `IS NOT TRUE` like
    /// [`Technique::PredicateTightenDelete`]'s three-valued-logic form: the
    /// discriminator is proven non-NULL (a bare literal, never NULL) and
    /// proven to land on exactly the removed branch's rows (distinct from
    /// every surviving branch's own constant), so there is no NULL-evaluation
    /// case to guard against the way an arbitrary predicate would need. Naturally
    /// idempotent (`rerun_safe: true`) — rows a prior run already deleted can
    /// never match the predicate again.
    DiscriminatedBranchDelete,
    /// B5 — new aggregate column at unchanged `GROUP BY` grain (research
    /// §4): `ALTER TABLE t ADD COLUMN c <ty>;` then a matched-only column
    /// backfill from the after-definition's own re-aggregation — full
    /// `FROM`/`WHERE` tree, `GROUP BY` keys — via
    /// `emit::emit_column_backfill_update_from_subquery`, never the plain
    /// upstream-table shape `UpstreamPullthrough` uses (a bare
    /// `SELECT <keys>, <agg> FROM <upstream> GROUP BY <keys>` over-counts
    /// rows the model's own `WHERE` filters out). No insert arm: a group
    /// absent from `t` is a group the row-set-unchanged proof already
    /// guarantees cannot exist.
    AggregateColumnBackfill,
    /// B6 — new window-function column over stored columns (research §4):
    /// `ALTER TABLE t ADD COLUMN c <ty>;` then a self-read column backfill —
    /// `UPDATE t SET c = s.c FROM (SELECT <id...>, <window> AS c FROM t) s
    /// WHERE t.<id> = s.<id>` — via
    /// `emit::emit_column_backfill_update_from_subquery`, the source
    /// subquery reading the deployed table `t` itself rather than any
    /// upstream (`reads_upstream: false`): computing the window over the
    /// *stored* rows is exactly what makes its draw match a rebuild's
    /// (research §2 "self-read scripts"), so no additional row-set proof is
    /// needed beyond the window's own dependencies resolving to stored bare
    /// representatives. Requires a declared, NOT-NULL-proven
    /// `BackbuildInputs::row_identity` (key addressability — the join back
    /// to `t` needs an addressable identity) and an explicit `ORDER BY`
    /// inside the `OVER` clause: without one, the window's draw within each
    /// partition is underdetermined and can never be proven equal to a
    /// rebuild's own draw (research §2 "Determinism caveat" narrowed to
    /// this technique's own obligation).
    WindowColumnBackfill,
    /// C1 — dropped column (research §4): `ALTER TABLE t DROP COLUMN d;`.
    /// This technique only *sequences* the drop into the H "ALTER DROPs"
    /// slot, last, after every statement that might still read the column
    /// (research §4 "H. Composites"); it carries
    /// [`WriteScope::Destructive`] metadata so a wiring-time chooser can
    /// gate it behind the `--allow-column-removal` opt-in doctrine
    /// (`docs/specs/schema_evolution.md`) without re-classifying — that
    /// policy decision is never made in this pure module.
    ColumnDrop,
}

/// The research §2 "write scope" option metadata — coarse cost/safety
/// classification of what an option's statements touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteScope {
    /// No row or schema mutation (e.g. [`Technique::FullRefresh`]'s sibling
    /// techniques that touch zero rows, such as a pure rename).
    None,
    /// One or more columns of every (or a matched subset of) existing rows.
    ColumnScoped,
    /// A row-level insert or delete over a subset of rows.
    RowSubset,
    /// A full-table rewrite (`CREATE OR REPLACE TABLE …`,
    /// [`Technique::FullRefresh`]).
    FullWrite,
    /// An irreversible schema change that discards data
    /// ([`Technique::ColumnDrop`]) — distinct from [`Self::ColumnScoped`]
    /// (which only ever *updates* column values, never removes the column
    /// itself) so a wiring-time chooser can require the
    /// `--allow-column-removal` opt-in without inspecting the technique.
    Destructive,
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
