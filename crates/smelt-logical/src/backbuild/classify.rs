//! `derive_backbuild_options`: turn a [`super::DefinitionDiff`] plus
//! [`super::BackbuildInputs`] into a [`super::BackbuildOptions`] value, and
//! `assemble`: turn a [`super::BackbuildOptions`] plus a [`Selection`] into
//! an ordered statement script. See the module doc comment in
//! `backbuild/mod.rs` and `docs/research/20260802-backbuild-synthesis.md`
//! §2 ("The contract"), §4 ("The catalogue" — G-class "Honest refusals",
//! the B1/B2 cases, and the CTE posture note), and §6 ("Architecture").
//!
//! This phase implements the refusal paths research §4's G-class names
//! outright — G1 (grain change), G2 (join-multiplicity change), and a
//! changed CTE (or any other whole-definition opacity) — the A0 no-op
//! short-circuit, the always-present model-level `FullRefresh` baseline,
//! and three admissible targeted techniques: B1 (a new column that is a
//! pure function of existing stored columns), B2 (a rename, paired *before*
//! B1/add-drop classification), and D1 (a changed existing-column
//! expression derivable from stored columns — the "fix one column of a huge
//! table" case, admitted through the same uniform-representative
//! derivability check as B1, plus the D-class `SELECT DISTINCT`/`LIMIT`
//! grain guards). Every other diff shape — a dropped column not paired
//! into a rename (C1), a changed expression needing an upstream read (D2),
//! a `WHERE`/set-operation change (E/F-class), an ambiguous rename cluster
//! — still yields a named, fail-closed refusal rather than a silently empty
//! atom list — fail-loud discipline (`docs/specs/architecture.md`
//! §"Fail-loud discipline").

use std::collections::{BTreeMap, BTreeSet, HashSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{Expr, JoinClause, JoinType, SelectEntry, SelectStmt, SyntaxKind, TextRange};
use smelt_types::SqlFunction;

use crate::analysis::model_diff;
use crate::analysis::walk;

use super::{
    chase_leaf, resolve_representative, AtomAnalysis, AtomicChange, BackbuildInputs,
    BackbuildOption, BackbuildOptions, BackbuildRefusal, ChangedColumn, ComparableDiff,
    ConjunctDiff, DefinitionDiff, HSlot, RepresentativeSource, SelectColumn, SelectListDiff,
    SetOpDiff, SkeletonCause, SkeletonDiff, Technique, WriteScope,
};

use super::emit;
use super::requalify;

/// Derive the option set for a `(before, after)` diff (research §2, §4).
pub fn derive_backbuild_options(
    diff: &DefinitionDiff,
    inputs: &BackbuildInputs,
) -> BackbuildOptions {
    let full_refresh = full_refresh_option(inputs);

    let mut atoms = match diff {
        DefinitionDiff::Opaque { reason } => vec![whole_definition_refusal(reason)],
        DefinitionDiff::Comparable(comparable) => {
            if diff.is_noop() {
                // A0 — no-op: nothing changed, so there is nothing to
                // refuse or classify. The targeted script is trivially
                // empty (`assemble` over zero atoms); `FullRefresh` remains
                // available regardless.
                Vec::new()
            } else if let Some(refusal) = multi_branch_pure_diff_gate(comparable) {
                // C3 fix (final-review finding), generalized for an edited
                // surviving branch (research §4 F1): whenever more than one
                // UNION ALL branch is in play, the select-list/WHERE/skeleton
                // diffs below are trusted only when they are all
                // independently no-ops, or when the diff's only edited
                // branch is uniquely branch 0 with every other pair exact
                // — see the gate's own doc comment.
                vec![refusal]
            } else {
                match &comparable.skeleton {
                    SkeletonDiff::Changed { reason, cause } => {
                        vec![skeleton_refusal(reason, *cause)]
                    }
                    SkeletonDiff::Unchanged => classify_comparable(comparable, inputs),
                    // B4 (a single added LEFT JOIN feeding only added
                    // columns, research §4 B4) for exactly one added join;
                    // B7 (research §4 B7, "sequential multi-join
                    // enrichment") for two or more — each join re-runs the
                    // same B4 proof, in reference-dependency order.
                    SkeletonDiff::AddedLeftJoins(joins) => {
                        classify_added_left_join_diff(joins, comparable, inputs)
                    }
                }
            }
        }
    };

    apply_b6_row_set_change_guard(&mut atoms);

    BackbuildOptions {
        atoms,
        full_refresh,
    }
}

/// Cross-atom guard (research §4 "H. Composites" "B6 composability"): B6
/// (`Technique::WindowColumnBackfill`) is a self-read `UPDATE ... FROM
/// (SELECT <id...>, <window> FROM t) s` that reads `t`'s *entire current
/// row-set* — not just unchanged columns the way B1/B3/B4/B5's row-local
/// updates do — so it is unsound whenever the same script also runs a
/// sibling atom whose technique changes that row-set. `assemble`'s fixed
/// H-ordering (`renames → ALTER ADD/TYPE → DELETEs → UPDATEs/MERGEs →
/// INSERTs → ALTER DROPs`) always schedules B6's self-read `UPDATE` in the
/// `Alter`/`UpdateMerge` slots, strictly before the `Delete`/`Insert` slots a
/// row-set-changing sibling lands in, so B6's window is computed over the
/// row population *before* the sibling's delete/insert has run — diverging
/// from a full rebuild (research §4H). Reordering alone cannot fix this
/// either: an inserted row's own window value is drawn from the
/// after-definition SELECT over upstream slices, not from a second pass over
/// the final table, so even a post-INSERT re-run of B6's update would not by
/// itself reproduce a rebuild's single coherent partition — refusal is the
/// right posture here, not reordering.
///
/// Triggered by *atomic-change class* alone (`is_row_set_changing_atom`),
/// not by whether that sibling atom itself ended up admissible: the
/// composability hazard is about what the *diff* contains, independent of
/// whether classification later also refused the sibling atom for an
/// unrelated reason. When triggered, every B6 option is stripped from every
/// atom that carries one and replaced with a named, actionable refusal —
/// `FullRefresh` remains the model-level baseline throughout (it never
/// touches per-atom options). B6 composed only with row-set-*preserving*
/// siblings (B1/B2/B3/B4/B5/D1/D2, or a C1 drop of some *other* column — a
/// dropped column never changes the row set) is left untouched.
fn apply_b6_row_set_change_guard(atoms: &mut [AtomAnalysis]) {
    let row_set_change_present = atoms
        .iter()
        .any(|atom| is_row_set_changing_atom(&atom.change));
    if !row_set_change_present {
        return;
    }

    for atom in atoms.iter_mut() {
        let Some(pos) = atom
            .options
            .iter()
            .position(|option| option.technique == Technique::WindowColumnBackfill)
        else {
            continue;
        };
        atom.options.remove(pos);
        atom.inadmissible.push(BackbuildRefusal {
            atom: atom_change_label(&atom.change),
            reason: "B6 (window column add) refused: window backfill is not composable with \
                     row-set-changing changes in the same diff — B6's self-read UPDATE reads \
                     t's entire current row-set, which a row-set-changing sibling atom (a \
                     tightened/loosened filter, a widened time horizon, or an added/removed \
                     UNION ALL branch) changes later in the script's fixed H-ordering, so the \
                     window would be computed over the wrong partition population; apply the \
                     row-set change first, then add the window column in a second edit"
                .to_string(),
        });
    }
}

/// Whether `change`'s admitted technique(s) — by atomic-change *class*,
/// independent of whether that atom ended up admissible — write a row
/// subset or otherwise change the deployed table's row set: the E-class
/// conjunct atoms (E1 `AddedConjunct`/`PredicateTightenDelete`, E2
/// `RemovedConjunct`/`FilterLoosenInsert`, E4 `RangePredicateChange`/
/// `HorizonExtensionInsert`) and the F-class set-operation atoms (F1
/// `AddedSetOpBranch`/`UnionBranchInsert`, F2 `RemovedSetOpBranch`/
/// `DiscriminatedBranchDelete`) — see [`apply_b6_row_set_change_guard`].
fn is_row_set_changing_atom(change: &AtomicChange) -> bool {
    matches!(
        change,
        AtomicChange::AddedConjunct { .. }
            | AtomicChange::RemovedConjunct { .. }
            | AtomicChange::RangePredicateChange { .. }
            | AtomicChange::AddedSetOpBranch { .. }
            | AtomicChange::RemovedSetOpBranch { .. }
    )
}

/// A short, human-readable label for an [`AtomicChange`], used by
/// [`apply_b6_row_set_change_guard`]'s cross-atom refusal (mirrors the
/// per-site `atom: format!(...)` naming convention used throughout this
/// module's own single-atom refusals).
fn atom_change_label(change: &AtomicChange) -> String {
    match change {
        AtomicChange::WholeDefinition { .. } => "whole-definition".to_string(),
        AtomicChange::Skeleton { .. } => "skeleton".to_string(),
        AtomicChange::AddedColumn { name } => format!("added column '{name}'"),
        AtomicChange::RenamedColumn { from, to } => format!("renamed column '{from}' -> '{to}'"),
        AtomicChange::DroppedColumn { name } => format!("dropped column '{name}'"),
        AtomicChange::ChangedColumn { name } => format!("changed column '{name}'"),
        AtomicChange::AddedConjunct { index } => format!("added conjunct #{index}"),
        AtomicChange::RangePredicateChange { column } => format!("range predicate on '{column}'"),
        AtomicChange::RemovedConjunct { index } => format!("removed conjunct #{index}"),
        AtomicChange::AddedSetOpBranch { index } => format!("added set-operation branch #{index}"),
        AtomicChange::RemovedSetOpBranch { index } => {
            format!("removed set-operation branch #{index}")
        }
        AtomicChange::Unclassified => "unclassified".to_string(),
    }
}

fn full_refresh_option(inputs: &BackbuildInputs) -> BackbuildOption {
    BackbuildOption {
        technique: Technique::FullRefresh,
        slot: None,
        // Statement authored in `emit::emit_full_refresh`, not inline here
        // (final-review batched finding — every backbuild statement,
        // including this baseline, has exactly one authoring site).
        statements: vec![emit::emit_full_refresh(&inputs.table, &inputs.after_sql)],
        write_scope: WriteScope::FullWrite,
        reads_upstream: true,
        // CREATE OR REPLACE TABLE ... AS <after> re-evaluated against the
        // same inputs reproduces the same result every time.
        rerun_safe: true,
    }
}

fn whole_definition_refusal(reason: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::WholeDefinition {
            reason: reason.to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "whole-definition".to_string(),
            reason: format!(
                "the definition diff could not be factored, so no targeted technique can be \
                 proven: {reason}"
            ),
        }],
    }
}

fn skeleton_refusal(reason: &str, cause: SkeletonCause) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Skeleton {
            reason: reason.to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "skeleton".to_string(),
            reason: skeleton_cause_label(reason, cause),
        }],
    }
}

/// Label a `SkeletonDiff::Changed` reason with its research §4 catalogue
/// case, read directly off `diff.rs`'s own structural [`SkeletonCause`]
/// classification (substrate-unification Phase 5) — G1 (grain change) or G2
/// (join-multiplicity change) — rather than re-deriving the same
/// classification by lowercased-`.contains` scanning `reason`'s free English
/// text, which could misfire on a `reason` string that happens to *mention*
/// the other class's vocabulary without describing it (e.g. a grain change
/// over a column literally named `join_key`). Any other skeleton change
/// (FROM target, `HAVING`/`QUALIFY`/`WINDOW`/`ORDER BY`/`LIMIT`) is still
/// refused, just without a G1/G2 label — the catalogue's G-class explicitly
/// refuses those too ("Opaque expressions ... LIMIT/ORDER BY changes:
/// refuse with named reasons").
fn skeleton_cause_label(reason: &str, cause: SkeletonCause) -> String {
    match cause {
        SkeletonCause::GrainChanged => format!("G1 (grain change) — {reason}"),
        SkeletonCause::JoinShapeChanged => format!("G2 (join-multiplicity change) — {reason}"),
        SkeletonCause::Other => format!("skeleton changed, not yet admissible — {reason}"),
    }
}

/// C3 fix (final-review finding): `diff.rs`'s clause-by-clause diffs
/// (`select_list`, `where_clause`, `skeleton`) are computed positionally
/// from each definition's *first* `UNION ALL` branch only (`definition_diff`
/// calls `select_list_diff`/`where_diff`/`skeleton_diff` over
/// `before_stmt`/`after_stmt`, which — for a multi-branch chain — are just
/// the outer, first branch of each side), while `set_ops` diffs branches as
/// a *multiset* (`diff.rs`'s `set_op_diff`/`branch_token_seq`). A pure
/// branch *reorder* is a semantic no-op under the multiset view, but can
/// make the first-branch pair diverge even though no branch's own content
/// actually changed — producing phantom added/removed/changed atoms from
/// the positional diffs (a phantom removed `WHERE` conjunct wrongly admits
/// E2's difference INSERT, duplicating rows the branch reorder never
/// touched; analogously for a phantom `select_list`/`skeleton` diff).
///
/// Whenever either side of the diff has more than one `UNION ALL` branch (or
/// the branch diff itself could not be factored at all — `SetOpDiff::Opaque`
/// may still hide more than one branch, so this stays conservative there
/// too), the `select_list`/`where_clause`/`skeleton` diffs are trusted only
/// in two shapes:
///
/// - **No branch carries its own edit** (`SetOpDiff::Branches::edited` is
///   empty — every surviving branch text-matched exactly, so any change is a
///   pure add/remove/reorder of whole branches): trusted when
///   `select_list`/`where_clause`/`skeleton` are all independently a no-op
///   **and** the SELECT list's declared column *order* agrees between
///   versions (research §4 F1's C4 reorder guard, below) — F1's pure append
///   case (every branch unchanged, one new branch at the tail) passes this,
///   since the first branch genuinely is unchanged between versions.
/// - **Exactly one branch carries its own edit, it is paired before-branch 0
///   against after-branch 0, and every other pair is exact**
///   (`edited.len() == 1 && edited[0].index == 0 && edited[0].after_index ==
///   0 && removed.is_empty()`): the top-level (whole-definition) diff *is*
///   this pairing's own diff in this shape — `definition_diff` computes
///   `select_list`/`where_clause`/`skeleton` from each side's outermost
///   `SelectStmt`, i.e. before's own branch 0 against after's own branch 0
///   — so it is safe to classify atoms from it exactly as the
///   single-branch case would (research §4 F1's edited-survivor
///   generalization). Both indices must be checked: `set_op_diff` pairs
///   leftovers positionally *among the leftovers*, not by declared chain
///   position, so a before-position-0 edit can still be paired against an
///   after-leftover that is not after's own declared branch 0 — e.g. when
///   after's literal branch 0 exact-text-matched a *different*
///   before-branch and was consumed into `unchanged`, leaving the true
///   edit paired with a later after-position. In that shape the top-level
///   diff (which always compares before's literal branch 0 to after's
///   literal branch 0) describes neither side of the actual pairing and
///   must not be trusted, even though `index == 0` alone would suggest it
///   is safe.
///
/// Any other edited-branch shape (an edit in a non-first branch, more than
/// one edited branch, or an edited branch alongside a genuine removal)
/// refuses unconditionally and by name: the top-level diff cannot describe
/// a non-first branch's edit at all, so even a superficially "clean"
/// top-level diff must not be trusted (research §4's E-class trap,
/// generalized — an edit confined to, say, branch 1 leaves branch 0's own
/// diff looking like a no-op, which would otherwise silently classify
/// nothing while a real change went unrepresented).
///
/// **C4 reorder guard.** The name-keyed `select_list.is_noop()` alone is not
/// enough even in the no-edited-branches shape: a before-definition whose
/// two branches happen to share the same name-set/expressions but declare
/// them in mutually swapped orders can, after a branch reorder, make the
/// *name-keyed* top-level SELECT-list diff report a no-op (same names, same
/// expressions on both sides) while the branches' true *declared* column
/// order at the (real) first-branch position actually differs from before —
/// exactly the shape F1's positional `UNION ALL` binding cares about. The
/// declared-order equality check below (`before_order == after_order`)
/// closes that gap.
fn multi_branch_pure_diff_gate(comparable: &ComparableDiff) -> Option<AtomAnalysis> {
    let (edited, removed_len) = match &comparable.set_ops {
        SetOpDiff::NotApplicable => return None,
        SetOpDiff::Branches {
            added,
            removed,
            unchanged,
            edited,
        } => {
            let multi_branch = unchanged.len() + edited.len() + added.len() > 1
                || unchanged.len() + edited.len() + removed.len() > 1;
            if !multi_branch {
                return None;
            }
            (edited, removed.len())
        }
        SetOpDiff::Opaque { .. } => {
            return multi_branch_pure_diff_gate_clean_check(comparable);
        }
    };

    if edited.len() == 1 && edited[0].index == 0 && edited[0].after_index == 0 && removed_len == 0 {
        // Trusted edited-branch-0 admission: the top-level diff *is* this
        // pairing's own diff — before's own branch 0 paired against after's
        // own branch 0 (see this function's doc comment) — proceed exactly
        // as the single-branch case would.
        return None;
    }

    if !edited.is_empty() {
        let positions: Vec<String> = edited
            .iter()
            .map(|e| format!("before {} -> after {}", e.index, e.after_index))
            .collect();
        return Some(multi_branch_definition_changed_refusal(&format!(
            "branch pairing(s) {} carry their own edit, but the top-level (whole-definition) \
             SELECT-list/WHERE/skeleton diff only ever describes before's own branch 0 paired \
             against after's own branch 0 — an edit confined to a non-first before-position, an \
             edit paired against a non-first after-position (even when the before-position is \
             0 — a branch reorder can pair before's true first-branch edit against a later \
             after-position while after's own literal branch 0 matched a different, unchanged \
             before-branch), more than one edited branch, or an edited branch alongside a \
             genuine removal, is never visible there, even when that top-level diff itself \
             looks like a no-op",
            positions.join(", ")
        )));
    }

    multi_branch_pure_diff_gate_clean_check(comparable)
}

/// The no-edited-branches leg of [`multi_branch_pure_diff_gate`]: trusted
/// only when the top-level diff is a pure no-op (name/expression equality)
/// *and* the SELECT list's declared column order agrees between versions
/// (the C4 reorder guard — see the caller's doc comment).
fn multi_branch_pure_diff_gate_clean_check(comparable: &ComparableDiff) -> Option<AtomAnalysis> {
    let clean = comparable.select_list.is_noop()
        && comparable.where_clause.is_noop()
        && comparable.skeleton.is_noop();
    if !clean {
        return Some(multi_branch_definition_changed_refusal(
            "the SELECT-list/WHERE/skeleton diff (computed from each definition's own first \
             UNION ALL branch) shows a change, which cannot be trusted once more than one \
             branch is in play — a branch reorder alone can make the first-branch pair diverge \
             without any branch's own content actually changing",
        ));
    }

    let order_ok = match &comparable.select_list {
        SelectListDiff::Diffed {
            before_order,
            after_order,
            ..
        } => before_order == after_order,
        // `is_noop()` is `false` for `Opaque`, so `clean` above already
        // refused before this arm is ever reached — kept exhaustive and
        // fail-closed rather than unreachable.
        SelectListDiff::Opaque { .. } => false,
    };
    if !order_ok {
        return Some(multi_branch_definition_changed_refusal(
            "the first branch's own declared output column order differs between versions even \
             though the SELECT-list diff (name-keyed) reports no added/dropped/changed columns \
             — UNION ALL binds columns positionally, so a bare name-set comparison cannot be \
             trusted to prove the first branch's own declared order, only its column set, is \
             unchanged, once more than one branch is in play",
        ));
    }

    None
}

fn multi_branch_definition_changed_refusal(reason: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "multi-branch definition".to_string(),
            reason: format!(
                "multi-branch definition changed outside a pure branch add/remove — {reason}"
            ),
        }],
    }
}

fn unclassified_refusal_named(atom: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: atom.to_string(),
            reason: "the definition changed in a way this phase does not yet classify into an \
                     admissible technique (targeted-technique admission arrives in later phases)"
                .to_string(),
        }],
    }
}

// ===== Comparable-diff classification: SELECT-list B1/B2, plus the
// residual WHERE/set-operation/D-class/C1 catch-alls (research §4) =====

/// Classify a diff whose skeleton is proven [`SkeletonDiff::Unchanged`] —
/// only the SELECT list, `WHERE` clause, and/or set operations may differ.
/// One atom per added or renamed column (B1/B2), plus one coarse
/// [`AtomicChange::Unclassified`] atom per residual category that changed
/// (a changed column, an unpaired dropped column, a `WHERE`/set-operation
/// diff, or an opaque SELECT-list/`WHERE`/set-operation shape).
fn classify_comparable(comparable: &ComparableDiff, inputs: &BackbuildInputs) -> Vec<AtomAnalysis> {
    let mut atoms = Vec::new();

    let (representative_sources, after_columns, first_branch_order) = match &comparable.select_list
    {
        SelectListDiff::Opaque { reason } => {
            atoms.push(unclassified_refusal_named(&format!(
                "select-list: {reason}"
            )));
            (Vec::new(), None, None)
        }
        SelectListDiff::Diffed {
            added,
            dropped,
            changed,
            unchanged,
            after_order,
            ..
        } => {
            let representative_sources = representative_sources(unchanged);
            let changed_names: BTreeSet<String> = changed.iter().map(|c| c.name.clone()).collect();
            atoms.extend(classify_select_list(
                added,
                dropped,
                &representative_sources,
                inputs,
            ));
            for c in changed {
                atoms.push(classify_changed_column(
                    c,
                    &representative_sources,
                    &changed_names,
                    inputs,
                ));
            }
            let after_columns = after_column_names(unchanged, changed, added);
            // The after-definition's own branch-0 declared column order
            // (research §4 "H. Composites" positional-`UNION-ALL` note),
            // read directly off the diff's `after_order` — the after side's
            // own SELECT list in its true declared order, correct whether
            // or not branch 0 itself changed (unlike reading it off
            // `unchanged`, which reflects only `before`'s order and
            // silently drops any added/changed column's position — the
            // trusted edited-branch-0 admission path below relies on this
            // being right even when the SELECT list did change).
            let first_branch_order = after_order.clone();
            (
                representative_sources,
                Some(after_columns),
                Some(first_branch_order),
            )
        }
    };

    atoms.extend(classify_where_clause(
        &comparable.where_clause,
        &representative_sources,
        after_columns.as_deref(),
        inputs,
    ));

    atoms.extend(classify_set_ops(
        &comparable.set_ops,
        after_columns.as_deref(),
        first_branch_order.as_deref(),
        inputs,
    ));

    atoms
}

// ===== E-class: row-set (predicate) techniques — E1 (filter tightened) and
// E4 (time-horizon extension) only; E2/E3/F1 remain out of this phase's
// scope (research §4 E-class) =====

/// The `after`-definition's own output column names, keyed by the pieces
/// [`SelectListDiff::Diffed`] already exposes (`unchanged` + `changed` +
/// `added` — `dropped` columns are not in `after` at all). Sorted and
/// deduplicated: `assemble`'s H-ordering already guarantees any atom's own
/// `ALTER ADD` flushes before E4's `HSlot::Insert` step, so the INSERT/SELECT
/// column-list pairing here only needs internal self-consistency (the same
/// list on both sides of `emit::emit_difference_insert`), never physical
/// table column order.
fn after_column_names(
    unchanged: &[SelectColumn],
    changed: &[ChangedColumn],
    added: &[SelectColumn],
) -> Vec<String> {
    let mut names: Vec<String> = unchanged
        .iter()
        .map(|c| c.name.clone())
        .chain(changed.iter().map(|c| c.name.clone()))
        .chain(added.iter().map(|c| c.name.clone()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// E-class grain facts (research §4 intro + E-class section), read straight
/// from the CST via the enclosing `SelectStmt` rather than from the diff:
/// `SkeletonDiff::Unchanged` only proves GROUP BY/DISTINCT/LIMIT are
/// *identical* between versions, not that they're absent (research §4 E1's
/// brief: "if unchanged-LIMIT is invisible to the diff, you must detect
/// LIMIT presence from the CSTs"). Any conjunct's `Expr` (added or removed)
/// works as the anchor — both sides' grain is already proven identical by
/// `SkeletonDiff::Unchanged` by the time this is called.
struct EClassGrain {
    distinct: bool,
    has_limit: bool,
    has_group_by: bool,
    /// Bare-column-ref GROUP BY expressions' raw names — the E4 group-key
    /// carve-out compares a range predicate's column against this set. A
    /// GROUP BY expression that isn't a bare column reference (e.g.
    /// `date_trunc('day', ts)`) is simply absent here, which safely refuses
    /// the carve-out for it (fail-closed) without misreporting "no GROUP BY
    /// at all".
    ///
    /// Deliberately name-only, ignoring qualifiers — sound only when
    /// [`single_alias_from`](Self::single_alias_from) also holds, since a
    /// bare name is otherwise ambiguous across two different FROM-tree
    /// aliases exposing same-named columns (e.g. `GROUP BY o.report_date`
    /// vs. a range predicate on `d.report_date` from a joined dimension).
    group_by_column_names: BTreeSet<String>,
    /// Whether the model's FROM tree has at most one table reference in
    /// scope (no `JOIN` at all). `group_by_column_names` and
    /// [`RangeConjunctShape::name`] are both qualifier-blind (bare column
    /// names only), which is sound only when there is a single alias to
    /// begin with — `SkeletonDiff::Unchanged` proves the FROM tree
    /// *unchanged* between versions, not single-alias, so a pre-existing
    /// multi-table FROM with a same-named column on two different aliases
    /// would otherwise let the E4 group-key carve-out wrongly match a GROUP
    /// BY key against an unrelated column of a different table. The E4
    /// group-key carve-out requires this to be `true`; it never gates E1,
    /// the plain (no-group-by) E4 path, or the general GROUP BY refusal.
    single_alias_from: bool,
}

fn e_class_grain(expr: &Expr) -> Option<EClassGrain> {
    let stmt = expr.syntax().ancestors().find_map(SelectStmt::cast)?;
    let group_by = stmt.group_by_clause();
    let group_by_column_names = group_by
        .as_ref()
        .map(|g| {
            g.expressions()
                .filter_map(|e| e.as_column_ref().map(|c| c.name().to_string()))
                .collect()
        })
        .unwrap_or_default();
    let single_alias_from = stmt
        .from_clause()
        .map(|f| f.joins().count() == 0)
        .unwrap_or(true);
    Some(EClassGrain {
        distinct: stmt.is_distinct(),
        has_limit: stmt.limit_clause().is_some(),
        has_group_by: group_by.is_some(),
        group_by_column_names,
        single_alias_from,
    })
}

fn e_class_where_refusal(reason: String) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "where-clause".to_string(),
            reason,
        }],
    }
}

/// E-class dispatch for a `WHERE`-clause diff that actually changed
/// something (research §4 E1/E4) — replaces the coarse "where-clause"
/// catch-all for both call sites that classify a [`ComparableDiff`]'s
/// `WHERE` clause (`classify_comparable` and `classify_added_left_join_diff`
/// share this). `representatives` is the same stored 1:1 representative set
/// B1/D1 use (research §4 intro "one uniform rule") — E1/E4 both requalify
/// against it. `after_columns` is `None` only when the SELECT-list diff
/// itself was [`SelectListDiff::Opaque`], which blocks E4 specifically (its
/// difference `INSERT` needs an explicit column list) but not E1 (a `DELETE`
/// needs no column list at all).
fn classify_where_clause(
    where_clause: &ConjunctDiff,
    representative_sources: &[RepresentativeSource],
    after_columns: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    match where_clause {
        ConjunctDiff::Diffed { added, removed, .. } if added.is_empty() && removed.is_empty() => {
            Vec::new()
        }
        ConjunctDiff::Diffed { added, removed, .. } => classify_conjunct_diff(
            added,
            removed,
            representative_sources,
            after_columns,
            inputs,
        ),
        ConjunctDiff::Opaque { reason, .. } => vec![e_class_where_refusal(format!(
            "E-class (row-set/predicate techniques) refused: the WHERE-clause diff could not \
             be factored into a conjunct-set add/remove — a non-conjunctive rewrite (e.g. `a \
             OR b` -> `a`) is unsound for a conjunct-set add/remove framing (research §4 \
             E-class intro): {reason}"
        ))],
    }
}

fn classify_conjunct_diff(
    added: &[Expr],
    removed: &[Expr],
    representative_sources: &[RepresentativeSource],
    after_columns: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    let anchor = added.first().or_else(|| removed.first());
    let grain = anchor.and_then(e_class_grain);

    if grain.as_ref().is_some_and(|g| g.has_limit) {
        return vec![e_class_where_refusal(
            "E-class refuses when the definition has a LIMIT — row selection under LIMIT is \
             not stable under a predicate change (research §4 intro grain guards)"
                .to_string(),
        )];
    }
    if grain.as_ref().is_some_and(|g| g.distinct) {
        return vec![e_class_where_refusal(
            "E-class refuses under SELECT DISTINCT — a row-subset DELETE/INSERT would diverge \
             from a rebuild's DISTINCT-merged multiset (research §4 intro grain guards)"
                .to_string(),
        )];
    }

    // The only shape the group-by carve-out (or a plain, no-group-by E4)
    // ever admits: exactly one removed conjunct paired with exactly one
    // added conjunct, recognised as a provably-widened range predicate on
    // the same column.
    let e4_pair = if added.len() == 1 && removed.len() == 1 {
        Some(try_e4_pair(&removed[0], &added[0]))
    } else {
        None
    };

    if let Some(g) = &grain {
        if g.has_group_by {
            if let Some(Ok(proof)) = &e4_pair {
                if g.group_by_column_names.contains(&proof.column_name) {
                    if g.single_alias_from {
                        return vec![build_e4_atom(
                            proof,
                            representative_sources,
                            after_columns,
                            inputs,
                        )];
                    }
                    return vec![e_class_where_refusal(format!(
                        "E4's group-key carve-out requires the model's FROM tree to have a \
                         single table reference in scope — a bare column-name match between \
                         the GROUP BY key and the range predicate's column '{}' cannot be \
                         trusted to mean the same column when more than one alias is joined in \
                         (research §4 E-class grain precondition, group-key carve-out)",
                        proof.column_name
                    ))];
                }
            }
            return vec![e_class_where_refusal(
                "E-class refuses a WHERE-clause predicate change under GROUP BY — it \
                 re-partitions inputs to aggregates, not stored rows, and a slice INSERT would \
                 double-count groups that already exist while the identity anti-join guard \
                 would silently skip them instead of double-counting, which is worse (research \
                 §4 E-class grain precondition); only E4 with the range column itself a group \
                 key is admitted"
                    .to_string(),
            )];
        }
    }

    if let Some(Ok(proof)) = &e4_pair {
        return vec![build_e4_atom(
            proof,
            representative_sources,
            after_columns,
            inputs,
        )];
    }

    if removed.is_empty() {
        return added
            .iter()
            .enumerate()
            .map(|(index, conjunct)| {
                classify_e1_conjunct(index, conjunct, representative_sources, inputs)
            })
            .collect();
    }

    // E2 (research §4: "filter loosened") — a single removed conjunct with
    // no added conjunct alongside it is admitted unconditionally: no
    // disjoint-column proof is needed (there is nothing to be disjoint
    // *from*), so this short-circuits straight to `build_e2_atom` without
    // ever touching the column-name machinery below — `build_e2_atom`'s own
    // validator (`requalify::requalify`, inside `build_e2_option`) is the
    // only check this shape needs, and unlike the disjointness check it
    // never rejects a non-deterministic or unregistered-function predicate
    // (E2's difference `INSERT` evaluates the removed conjunct once,
    // directly — it has no B1/D1-style re-derivation requirement).
    if added.is_empty() && removed.len() == 1 {
        return vec![build_e2_atom(
            0,
            &removed[0],
            representative_sources,
            after_columns,
            inputs,
        )];
    }

    // E3 by disjoint-column composition (research §4: "E3 — arbitrary
    // predicate change = added ∧ removed conjuncts: compose E1 + E2"):
    // admitted when there is exactly one removed conjunct AND every added
    // conjunct's own referenced-column set ([`conjunct_column_names`] —
    // deliberately permissive; see its doc comment for why this must not be
    // `analysis::model_diff::collect_dependencies`, which would wrongly
    // reject a non-deterministic/unregistered-function added conjunct E1's
    // own classification would admit on its own merits) is disjoint from
    // the removed conjunct's. Disjointness is the middle path between "pure
    // loosen" (handled above) and the *same-column* pair E4 exclusively
    // owns: E1's DELETE and E2's INSERT are each independently sound in
    // isolation (E1 shrinks the table to the intersection of before/after
    // regardless of what else changed; E2's difference INSERT reads the
    // after-definition — which already reflects every added conjunct —
    // filtered by the removed conjunct's three-valued-safe complement), so
    // composing them never needs the added and removed conjuncts to
    // interact *unless they share a column*, in which case only a proof
    // like E4's range-widening (same column, same operator, provably
    // widened literal) can be trusted — an unrelated column swap on the
    // *same* column (e.g. an equality change, or a mixed-operator/non-
    // ISO-8601-literal pair E4 itself refuses, research §4 E4) is
    // deliberately NOT re-admitted here: disjointness fails for those (they
    // share the column), so they fall through to the refusal below
    // unchanged. Each added conjunct still goes through its own
    // `classify_e1_conjunct`/`try_e1` admission below — the disjointness
    // check here only decides *whether to attempt* the composition, never
    // substitutes for E1's own validation of any individual added conjunct.
    if removed.len() == 1 {
        let removed_cols = conjunct_column_names(&removed[0]);
        let disjoint = added
            .iter()
            .all(|conjunct| conjunct_column_names(conjunct).is_disjoint(&removed_cols));
        if disjoint {
            let mut atoms: Vec<AtomAnalysis> = added
                .iter()
                .enumerate()
                .map(|(index, conjunct)| {
                    classify_e1_conjunct(index, conjunct, representative_sources, inputs)
                })
                .collect();
            atoms.push(build_e2_atom(
                0,
                &removed[0],
                representative_sources,
                after_columns,
                inputs,
            ));
            return atoms;
        }
    }

    let reason = if removed.len() > 1 {
        format!(
            "the WHERE-clause diff removes {} conjuncts — this phase admits E2/E3 (filter \
             loosened / arbitrary predicate change, composing E1 + E2) only for a single \
             removed conjunct at a time (multiple simultaneous removed conjuncts risk a \
             double-counted difference INSERT without a stronger multi-conjunct proof)",
            removed.len()
        )
    } else {
        match e4_pair {
            // `added.len() == 1 && removed.len() == 1` (the only shape
            // `try_e4_pair` is ever attempted for) but the pair failed E4's
            // range-widening proof, AND the pair shares a column
            // (disjointness above already refused it otherwise it would
            // have composed): surface the E4-specific reason — it names
            // exactly why (mixed operators, a non-ISO-8601 literal, an
            // equality swap on the same column, …) — rather than the
            // generic message below, which would lose that detail.
            Some(Err(reason)) => format!(
                "the added+removed conjunct pair is not an admissible E4 (time-horizon \
                 extension) range-widening shape: {reason} — the pair shares a column, so the \
                 E3 disjoint-column composition (research §4 E3, E1 + E2) does not apply \
                 either; only a same-column proof like E4's range-widening can admit this shape"
            ),
            // `removed.len() == 1` but `added.len() != 1` (so `e4_pair` was
            // never attempted) and the disjointness check above failed — at
            // least one added conjunct shares a referenced column name with
            // the removed one.
            _ => "at least one added conjunct shares a referenced column with the removed \
                  conjunct — E3 (arbitrary predicate change, composing E1 + E2) only admits a \
                  removed conjunct whose column set is disjoint from every added conjunct's; a \
                  shared column needs a same-column proof like E4's range-widening instead"
                .to_string(),
        }
    };
    vec![e_class_where_refusal(reason)]
}

// ----- E2: filter loosened (research §4 E2) -----

fn build_e2_atom(
    index: usize,
    removed: &Expr,
    representative_sources: &[RepresentativeSource],
    after_columns: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::RemovedConjunct { index };
    match build_e2_option(removed, representative_sources, after_columns, inputs) {
        Ok(option) => AtomAnalysis {
            change: atom_change,
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(reason) => AtomAnalysis {
            change: atom_change,
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("removed conjunct #{index}"),
                reason,
            }],
        },
    }
}

/// E2 admission (research §4 E2): the after-definition's own difference
/// `INSERT`, scoped by the removed conjunct in its always-complement form
/// (`AND (<q>) IS NOT TRUE`) — exactly [`build_e4_option`]'s shape, minus the
/// range-widening proof (research §4 E4: "mechanically an E2 whose
/// removed/added conjuncts are range predicates on one column").
fn build_e2_option(
    removed: &Expr,
    representative_sources: &[RepresentativeSource],
    after_columns: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let after_columns = after_columns.ok_or_else(|| {
        "E2 (filter loosened) refused: the after-definition's output column list could not be \
         determined (the SELECT-list diff is opaque)"
            .to_string()
    })?;
    if after_columns.is_empty() {
        return Err(
            "E2 (filter loosened) refused: the after-definition has no output columns".to_string(),
        );
    }

    let requalified_removed = requalify::requalify(removed, representative_sources)
        .map_err(|reason| format!("E2 (filter loosened) refused: {reason}"))?;

    // Same key-addressability obligation as E4's identity anti-join guard
    // (research §4 intro): an equality anti-join never matches a NULL-keyed
    // row on either side, so a nullable identity column would let a rerun
    // silently re-insert. A declared identity that isn't provably NOT NULL
    // is treated exactly like no declared identity — the guard is omitted
    // and the option is recorded one-shot, never a refusal.
    let identity_columns = inputs.row_identity.as_ref().filter(|cols| {
        cols.iter()
            .all(|c| inputs.not_null_columns.contains(c.as_str()))
    });
    let statement = emit::emit_difference_insert(
        &inputs.table,
        after_columns,
        &inputs.after_sql,
        &requalified_removed,
        identity_columns.map(Vec::as_slice),
    );

    Ok(BackbuildOption {
        technique: Technique::FilterLoosenInsert,
        slot: Some(HSlot::Insert),
        statements: vec![statement],
        write_scope: WriteScope::RowSubset,
        reads_upstream: true,
        rerun_safe: identity_columns.is_some(),
    })
}

// ----- E1: filter tightened (research §4 E1) -----

fn classify_e1_conjunct(
    index: usize,
    conjunct: &Expr,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::AddedConjunct { index };
    match try_e1(conjunct, representative_sources, inputs) {
        Ok(option) => AtomAnalysis {
            change: atom_change,
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(reason) => AtomAnalysis {
            change: atom_change,
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("added conjunct #{index}"),
                reason,
            }],
        },
    }
}

/// E1 admission (research §4 E1): the added conjunct, requalified to stored
/// columns under the same uniform representative rule B1/D1 use. *Script*:
/// `DELETE FROM t WHERE (<q'>) IS NOT TRUE;` — no upstream read.
fn try_e1(
    conjunct: &Expr,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let requalified = requalify::requalify(conjunct, representative_sources)
        .map_err(|reason| format!("E1 (filter tightened) refused: {reason}"))?;

    let statement = emit::emit_predicate_delete(&inputs.table, &requalified);

    Ok(BackbuildOption {
        technique: Technique::PredicateTightenDelete,
        slot: Some(HSlot::Delete),
        statements: vec![statement],
        write_scope: WriteScope::RowSubset,
        reads_upstream: false,
        // A `DELETE` keyed on a predicate is naturally idempotent: rows a
        // prior run already deleted can never match the predicate again.
        rerun_safe: true,
    })
}

// ----- E4: time-horizon extension (research §4 E4) -----

/// The E4 range-widening proof's result: the range column's raw name (for
/// the group-key carve-out check) and the *removed* conjunct's whole
/// expression — E4's script always uses this verbatim (research §4: "the
/// emitted predicate is always the complement form"), never a
/// column/operator/literal reconstruction. The range view
/// ([`as_range_conjunct`]/[`try_e4_pair`]) is classification-only.
struct RangePairProof {
    column_name: String,
    removed_expr: Expr,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum RangeLiteralKind {
    Number,
    Text,
}

struct RangeConjunctShape {
    name: String,
    /// Canonical form: the column is always on the left, so a reversed
    /// source conjunct (`literal <op> column`) has its operator flipped
    /// (e.g. `X <= ts` becomes `ts >= X`).
    operator: String,
    literal_kind: RangeLiteralKind,
    literal_text: String,
}

/// Recognise `expr` as `column <op> literal` (or `literal <op> column`,
/// flipped to canonical form), `<op>` one of `>`, `>=`, `<`, `<=`. `None`
/// for any other shape — this is a permissive leaf classifier over one
/// already-bounded conjunct, not a derivability proof (mirrors
/// `references_alias`'s posture): failing to recognise a shape here simply
/// means E4 does not apply, never a hard refusal on its own.
fn as_range_conjunct(expr: &Expr) -> Option<RangeConjunctShape> {
    let bin = expr.as_binary()?;
    let op = bin.operator()?;
    if !matches!(op.as_str(), ">" | ">=" | "<" | "<=") {
        return None;
    }
    let left = bin.left()?;
    let right = bin.right()?;

    if let (Some(col), Some(lit)) = (left.as_column_ref(), as_bare_range_literal(&right)) {
        return Some(RangeConjunctShape {
            name: col.name().to_string(),
            operator: op,
            literal_kind: lit.0,
            literal_text: lit.1,
        });
    }
    if let (Some(lit), Some(col)) = (as_bare_range_literal(&left), right.as_column_ref()) {
        return Some(RangeConjunctShape {
            name: col.name().to_string(),
            operator: flip_comparison_operator(&op),
            literal_kind: lit.0,
            literal_text: lit.1,
        });
    }
    None
}

fn flip_comparison_operator(op: &str) -> String {
    match op {
        ">" => "<".to_string(),
        ">=" => "<=".to_string(),
        "<" => ">".to_string(),
        "<=" => ">=".to_string(),
        other => other.to_string(),
    }
}

/// `expr` is a *bare* literal (exactly one non-trivia token, a `NUMBER` or
/// `STRING` — no typed-literal prefix like `DATE '…'`) — E4's
/// range-widening proof needs this narrower shape than the crate's shared
/// constant-literal recognizer ([`walk::is_constant_literal`]) admits: a
/// comparison against a typed literal, a function call, another column, or
/// a parenthesised expression is not a shape E4 recognises (research §4 E4
/// only reasons about plain numeric/text bound arithmetic). A thin wrapper
/// over the shared recognizer, restricted to the single-token shape.
/// String literal text has its surrounding quotes stripped for comparison.
fn as_bare_range_literal(expr: &Expr) -> Option<(RangeLiteralKind, String)> {
    if !walk::is_constant_literal(expr) {
        return None;
    }
    let mut tokens = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia());
    let first = tokens.next()?;
    if tokens.next().is_some() {
        return None;
    }
    match first.kind() {
        SyntaxKind::NUMBER => Some((RangeLiteralKind::Number, first.text().to_string())),
        SyntaxKind::STRING => Some((
            RangeLiteralKind::Text,
            first.text().trim_matches('\'').to_string(),
        )),
        _ => None,
    }
}

/// The E4 range-widening proof (research §4 E4): `removed` and `added` must
/// both recognise as [`as_range_conjunct`] shapes over the *same* column
/// with the *same* (canonical) comparison operator, and `added`'s literal
/// must provably widen `removed`'s bound — mixed operators refuse rather
/// than trusting literal arithmetic at the boundary (research §4 E4: "mixed
/// operators like `>` -> `>=` refuse rather than trusting literal
/// arithmetic at the boundary").
fn try_e4_pair(removed: &Expr, added: &Expr) -> Result<RangePairProof, String> {
    let removed_shape = as_range_conjunct(removed).ok_or_else(|| {
        "the removed conjunct is not a recognised `column <op> literal` range predicate".to_string()
    })?;
    let added_shape = as_range_conjunct(added).ok_or_else(|| {
        "the added conjunct is not a recognised `column <op> literal` range predicate".to_string()
    })?;

    if removed_shape.name != added_shape.name {
        return Err("the removed and added conjuncts range over different columns".to_string());
    }

    if removed_shape.operator != added_shape.operator {
        return Err(format!(
            "mixed comparison operators (removed '{}' vs added '{}') are not admitted — \
             boundary semantics are not literal arithmetic (research §4 E4)",
            removed_shape.operator, added_shape.operator
        ));
    }

    if removed_shape.literal_kind != added_shape.literal_kind {
        return Err(
            "the removed and added conjuncts' literals are not the same kind (number vs text) \
             — widening cannot be proven"
                .to_string(),
        );
    }

    let widened = match removed_shape.operator.as_str() {
        ">" | ">=" => is_widened_lower_bound(
            removed_shape.literal_kind,
            &removed_shape.literal_text,
            &added_shape.literal_text,
        )?,
        // C1 fix (final-review finding): this call already computes the
        // upper-bound widening condition directly (`removed < added`, via
        // `is_widened_lower_bound(kind, b=added, a=removed) = a < b`) — it
        // must NOT be negated. The prior negated form silently admitted a
        // narrowing (`ts < '2025-01-01'` -> `ts < '2024-01-01'`, emitting a
        // no-op INSERT that leaves rows a rebuild would drop) while refusing
        // a genuine widening.
        "<" | "<=" => is_widened_lower_bound(
            removed_shape.literal_kind,
            &added_shape.literal_text,
            &removed_shape.literal_text,
        )?,
        // Unreachable: `as_range_conjunct` only ever produces one of the
        // four operators above.
        _ => false,
    };
    if !widened {
        return Err(
            "the added conjunct's literal does not provably widen the removed conjunct's bound \
             — E4 only admits a strictly widened range"
                .to_string(),
        );
    }

    Ok(RangePairProof {
        column_name: removed_shape.name,
        removed_expr: removed.clone(),
    })
}

/// A fixed-width ISO-8601 date/timestamp shape a text literal can match.
/// Two literals of the *same* shape compare lexicographically exactly like
/// they compare chronologically, since every field occupies the same fixed
/// character width at the same position in both. `sep` (the exact byte
/// separating the date and time parts, `b'T'` or `b' '`) is part of the
/// shape too — a differing separator byte at that shared fixed position
/// would make an ordinary string comparison compare unrelated bytes there
/// instead of comparable date/time content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Iso8601Shape {
    Date,
    Timestamp { sep: u8 },
    TimestampFractional { sep: u8, frac_width: usize },
}

/// Recognise `s` as a fixed-width `YYYY-MM-DD` date, optionally followed by
/// a fixed-width `[T ]HH:MM:SS[.fraction]` time. `None` for anything that
/// doesn't match byte-for-byte: a non-padded month/day (`2025-9-1`), a
/// quoted plain number (`'9'`, `'100'`), a named month, a timezone suffix,
/// or any other text shape. This is the *only* text-literal shape E4's
/// widening proof trusts lexicographic order for — fail-closed for
/// everything else, pending real type information at wiring (research §4
/// E4: a non-ISO-8601 quoted literal cannot be proven to widen from text
/// alone; a naive lexicographic compare on e.g. `'2025-9-1'` vs
/// `'2025-10-1'`, or `'9'` vs `'100'`, would silently misjudge the
/// chronological/numeric order).
fn iso8601_fixed_width_shape(s: &str) -> Option<Iso8601Shape> {
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let date_ok = b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit();
    if !date_ok {
        return None;
    }
    if b.len() == 10 {
        return Some(Iso8601Shape::Date);
    }
    if b.len() < 19 {
        return None;
    }
    let sep = b[10];
    if sep != b'T' && sep != b' ' {
        return None;
    }
    let time_ok = b[11].is_ascii_digit()
        && b[12].is_ascii_digit()
        && b[13] == b':'
        && b[14].is_ascii_digit()
        && b[15].is_ascii_digit()
        && b[16] == b':'
        && b[17].is_ascii_digit()
        && b[18].is_ascii_digit();
    if !time_ok {
        return None;
    }
    if b.len() == 19 {
        return Some(Iso8601Shape::Timestamp { sep });
    }
    if b[19] != b'.' {
        return None;
    }
    let frac = &b[20..];
    if frac.is_empty() || !frac.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(Iso8601Shape::TimestampFractional {
        sep,
        frac_width: frac.len(),
    })
}

/// Whether `a` is strictly less than `b`, for a lower-bound (`>`/`>=`)
/// widening check: `a < b` means moving the bound from `b` to `a` admits
/// more rows. Numeric literals compare as parsed `f64`s (fail-closed on an
/// unparseable literal — this should be unreachable given
/// `as_bare_range_literal` only ever tags `NUMBER`-kind tokens this way,
/// but a lexer accepting a
/// shape `f64::from_str` rejects is reported, not panicked, per fail-loud
/// discipline). Text literals compare lexicographically *only* when both
/// match the same [`iso8601_fixed_width_shape`] — refused otherwise
/// (fail-closed; see that function's doc comment for the counter-examples
/// this guards against).
fn is_widened_lower_bound(kind: RangeLiteralKind, b: &str, a: &str) -> Result<bool, String> {
    match kind {
        RangeLiteralKind::Number => {
            let b: f64 = b.parse().map_err(|_| {
                "the removed conjunct's literal is not a parseable number".to_string()
            })?;
            let a: f64 = a.parse().map_err(|_| {
                "the added conjunct's literal is not a parseable number".to_string()
            })?;
            Ok(a < b)
        }
        RangeLiteralKind::Text => {
            let shape_b = iso8601_fixed_width_shape(b).ok_or_else(|| {
                "a quoted literal is not a fixed-width ISO-8601 date/timestamp (e.g. \
                 'YYYY-MM-DD') — widening cannot be proven from a string literal without real \
                 type information, pending wiring-time type inference"
                    .to_string()
            })?;
            let shape_a = iso8601_fixed_width_shape(a).ok_or_else(|| {
                "a quoted literal is not a fixed-width ISO-8601 date/timestamp (e.g. \
                 'YYYY-MM-DD') — widening cannot be proven from a string literal without real \
                 type information, pending wiring-time type inference"
                    .to_string()
            })?;
            if shape_b != shape_a {
                return Err(
                    "the removed and added conjuncts' literals have different fixed-width \
                     date/timestamp shapes (date-only vs timestamp, or a differing separator or \
                     fractional-second precision) — lexicographic comparison is not sound \
                     across differing shapes"
                        .to_string(),
                );
            }
            Ok(a < b)
        }
    }
}

fn build_e4_atom(
    proof: &RangePairProof,
    representative_sources: &[RepresentativeSource],
    after_columns: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::RangePredicateChange {
        column: proof.column_name.clone(),
    };
    match build_e4_option(proof, representative_sources, after_columns, inputs) {
        Ok(option) => AtomAnalysis {
            change: atom_change,
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(reason) => AtomAnalysis {
            change: atom_change,
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("range predicate on '{}'", proof.column_name),
                reason,
            }],
        },
    }
}

/// Build the E4 difference-`INSERT` option once [`try_e4_pair`]'s range
/// proof holds. The emitted predicate is always the removed conjunct itself
/// (research §4 E4's complement form), requalified to stored columns under
/// the same rule E1 uses.
fn build_e4_option(
    proof: &RangePairProof,
    representative_sources: &[RepresentativeSource],
    after_columns: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let after_columns = after_columns.ok_or_else(|| {
        "E4 (time-horizon extension) refused: the after-definition's output column list could \
         not be determined (the SELECT-list diff is opaque)"
            .to_string()
    })?;
    if after_columns.is_empty() {
        return Err(
            "E4 (time-horizon extension) refused: the after-definition has no output columns"
                .to_string(),
        );
    }

    let requalified_removed = requalify::requalify(&proof.removed_expr, representative_sources)
        .map_err(|reason| {
            format!(
                "E4 (time-horizon extension) refused for range predicate on '{}': {reason}",
                proof.column_name
            )
        })?;

    // The identity anti-join guard is only sound when every identity column
    // is provably NOT NULL (research §4 intro "Key addressability" — the
    // shared obligation identity anti-join guards share with B3/B4/B6's
    // equality addressing): `t.<id> = __backbuild_diff.<id>` never matches a
    // NULL-keyed row on either side, so a nullable identity column would let
    // a rerun silently re-insert an already-inserted row rather than being
    // suppressed by the guard. A declared identity that isn't provably NOT
    // NULL is treated exactly like no declared identity at all — the guard
    // is omitted and the option is recorded one-shot, never a refusal (the
    // script itself is still correct on a single application).
    let identity_columns = inputs.row_identity.as_ref().filter(|cols| {
        cols.iter()
            .all(|c| inputs.not_null_columns.contains(c.as_str()))
    });
    let statement = emit::emit_difference_insert(
        &inputs.table,
        after_columns,
        &inputs.after_sql,
        &requalified_removed,
        identity_columns.map(Vec::as_slice),
    );

    Ok(BackbuildOption {
        technique: Technique::HorizonExtensionInsert,
        slot: Some(HSlot::Insert),
        statements: vec![statement],
        write_scope: WriteScope::RowSubset,
        reads_upstream: true,
        // An anti-join guard on a NOT-NULL-proven identity makes a rerun
        // safe; without one (undeclared, or declared but not proven NOT
        // NULL) this is documented one-shot (research §2 "Idempotence").
        rerun_safe: identity_columns.is_some(),
    })
}

// ===== F1: new UNION ALL branch (research §4 F1) =====

/// A named, fail-closed refusal whose `reason` is caller-supplied verbatim
/// (unlike [`unclassified_refusal_named`], whose reason is a fixed
/// boilerplate string) — used for the set-operation diff's refusal paths so
/// the reason text can name F1/F2 and the underlying diff reason directly,
/// the same posture [`e_class_where_refusal`] gives the E-class.
fn set_op_refusal(atom: &str, reason: String) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: atom.to_string(),
            reason,
        }],
    }
}

/// Classify a [`SetOpDiff`] into F1 atoms/refusals (research §4 F1) — shared
/// by [`classify_comparable`] and [`classify_added_left_join_diff`], which
/// otherwise duplicated this match. `after_columns` is the after-definition's
/// declared output column list (the branches' shared column names, research
/// §4 "H. Composites"' explicit-column-list requirement) — `None` only when
/// the SELECT-list diff itself is opaque, which blocks F1 (its `INSERT`
/// needs an explicit column list) exactly like it blocks E4. `first_branch_order`
/// is the after-definition's own branch-0 output column names in declared
/// order — read directly off the SELECT-list diff's `after_order` field
/// (C4 fix, final-review finding), so it stays correct even when branch 0
/// itself carries the diff's own edit (the trusted edited-branch-0
/// admission path in `multi_branch_pure_diff_gate`). `None` from either
/// call site that cannot supply it (the added-left-join paths, which the C3
/// gate already refuses ahead of reaching here whenever a genuine
/// multi-branch F1 shape is in play).
fn classify_set_ops(
    set_ops: &SetOpDiff,
    after_columns: Option<&[String]>,
    first_branch_order: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    match set_ops {
        SetOpDiff::Branches { added, removed, .. } if added.len() == 1 && removed.is_empty() => {
            vec![build_f1_atom(
                0,
                &added[0],
                after_columns,
                first_branch_order,
                inputs,
            )]
        }
        SetOpDiff::Branches {
            added,
            removed,
            unchanged,
            ..
        } if added.is_empty() && removed.len() == 1 => {
            vec![build_f2_atom(0, &removed[0], unchanged, inputs)]
        }
        SetOpDiff::Branches { added, removed, .. } if !added.is_empty() || !removed.is_empty() => {
            vec![set_op_refusal(
                "set-operations",
                format!(
                    "the set-operation branch diff changed in a way this phase does not yet \
                     classify into an admissible technique — F1 (added UNION ALL branch) only \
                     admits a single added branch with no removed branch, and F2 (removed UNION \
                     ALL branch) only admits a single removed branch with no added branch ({} \
                     added, {} removed here); more than one added or removed branch, or a mix \
                     of both, is refused rather than classified branch-by-branch",
                    added.len(),
                    removed.len()
                ),
            )]
        }
        SetOpDiff::Opaque { reason } => vec![set_op_refusal(
            "set-operations",
            format!(
                "branch add/remove refused: the set-operation diff could not be factored into a \
                 UNION ALL branch add (F1) or remove (F2) — {reason}"
            ),
        )],
        _ => Vec::new(),
    }
}

fn build_f2_atom(
    index: usize,
    removed_branch: &SelectStmt,
    unchanged: &[SelectStmt],
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::RemovedSetOpBranch { index };
    match build_f2_option(removed_branch, unchanged, inputs) {
        Ok(option) => AtomAnalysis {
            change: atom_change,
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(reason) => AtomAnalysis {
            change: atom_change,
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("removed set-operation branch #{index}"),
                reason,
            }],
        },
    }
}

/// F2 admission (research §4 F2): a removed `UNION ALL` branch needs a
/// provenance predicate distinguishing its rows in the stored table — a
/// discriminator column that is a distinct literal constant in *every*
/// branch of the before-definition (`find_branch_discriminator`, below).
/// `unchanged` is the before-definition's surviving branches (exact-text
/// matches — the multi-branch pure-diff gate already refuses any diff where
/// a surviving branch carries its own edit alongside a removal, so by the
/// time this is reached every non-removed before-branch is known to be
/// exactly one of `unchanged`). *Script*: `DELETE FROM t WHERE <column> =
/// <literal>;` — an equality predicate (not `PredicateTightenDelete`'s `IS
/// NOT TRUE`), justified because the discriminator is a proven non-NULL
/// literal that lands on exactly the removed branch's rows (see
/// [`Technique::DiscriminatedBranchDelete`]'s doc comment).
fn build_f2_option(
    removed_branch: &SelectStmt,
    unchanged: &[SelectStmt],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let (column, literal_sql) = find_branch_discriminator(removed_branch, unchanged)
        .map_err(|reason| format!("F2 (removed UNION ALL branch) refused: {reason}"))?;
    let statement = emit::emit_discriminated_branch_delete(&inputs.table, &column, &literal_sql);

    Ok(BackbuildOption {
        technique: Technique::DiscriminatedBranchDelete,
        slot: Some(HSlot::Delete),
        statements: vec![statement],
        write_scope: WriteScope::RowSubset,
        reads_upstream: false,
        // An equality `DELETE` keyed on a proven-distinct discriminator
        // constant is naturally idempotent: rows a prior run already
        // deleted can never match the predicate again.
        rerun_safe: true,
    })
}

/// The F2 discriminator proof (research §4 F2): find a SELECT-list output
/// column that is present, under the same output name, in `removed_branch`
/// and every one of `other_branches` (the before-definition's surviving
/// branches), whose expression is [`walk::is_constant_literal`] in every one
/// of those branches, and whose literal value is distinct across all of
/// them. Candidates are tried in `removed_branch`'s own declared column
/// order; the first fully-qualifying one wins. Returns the column name plus
/// `removed_branch`'s own literal's raw SQL text — the predicate's
/// right-hand side, spliced verbatim (a typed literal like `DATE
/// '2026-01-01'` already carries its own quoting/type prefix; a bare
/// `NUMBER`/`STRING` needs none added).
///
/// Deliberately does **not** consult `analysis::discriminants.rs`: that
/// module classifies an *aggregate combiner's* algebraic facts (is-monoid,
/// decomposable, monotonicity — `docs/specs/model_properties.md` §"Algebraic
/// discriminants") for maintenance-ladder purposes, and has no concept of a
/// per-branch literal-constant provenance column at all — the two "discriminant"
/// names are unrelated ideas that happen to share a word. The true sibling
/// is `analysis::walk`'s discriminated-union machinery
/// ([`walk::is_constant_literal`], [`walk::constant_literal_tag`], and
/// `walk::union_discriminated_grain`'s whole-`PropertyVector` counterpart):
/// this proof is the per-branch-`SelectStmt` shape of the same recognizer,
/// reused rather than re-derived, over each branch's own already-bounded
/// SELECT-list expression — consistent with the property-composition-walk
/// rule's "leaf classifier over one bounded node" carve-out
/// (`docs/specs/architecture.md` §"Property composition walk rule").
///
/// Fail-closed with a specific, actionable reason:
/// - no candidate column is ever a literal anywhere: "no provenance
///   predicate" (an actionable nudge to add a constant discriminator
///   column);
/// - a candidate is a literal in some branches but a non-constant expression
///   in at least one other: refused by name, since only a constant-per-branch
///   discriminator is provable from the definitions alone;
/// - a candidate is a literal in every branch but two branches share the
///   same value: refused by name, since the resulting equality predicate
///   would also delete a surviving branch's rows.
/// - a candidate is a literal in every branch but not the *same* literal
///   kind (e.g. `1` in one branch, `'1'` in another, or `DATE '2026-01-01'`
///   in one branch and a plain string in another): refused by name — a
///   `UNION ALL` coerces the column to a common supertype, so the emitted
///   equality predicate implicit-casts at execution and can match a
///   surviving branch's differently-typed-but-textually-equal rows even
///   though the two literals compare as distinct `(kind, text)` pairs here;
///   only a discriminator that is the *same* literal kind
///   ([`walk::constant_literal_tag`]'s `kind`) in every branch is kind-safe
///   to compare by value at all.
fn find_branch_discriminator(
    removed_branch: &SelectStmt,
    other_branches: &[SelectStmt],
) -> Result<(String, String), String> {
    let removed_items = branch_named_items(removed_branch)?;
    let mut other_items = Vec::with_capacity(other_branches.len());
    for branch in other_branches {
        other_items.push(branch_named_items(branch)?);
    }

    let mut nonconstant_candidate: Option<String> = None;
    let mut shared_candidate: Option<(String, String)> = None;
    let mut mixed_kind_candidate: Option<String> = None;

    for (name, removed_expr) in &removed_items {
        // Every other branch must declare this same output name, or it is
        // not a candidate at all (not a column common to every branch).
        let mut branch_exprs: Vec<&Expr> = vec![removed_expr];
        let mut present_everywhere = true;
        for items in &other_items {
            match items.iter().find(|(n, _)| n == name) {
                Some((_, e)) => branch_exprs.push(e),
                None => {
                    present_everywhere = false;
                    break;
                }
            }
        }
        if !present_everywhere {
            continue;
        }

        let literals: Vec<Option<(String, String)>> = branch_exprs
            .iter()
            .map(|e| walk::constant_literal_tag(e))
            .collect();
        let literal_count = literals.iter().filter(|l| l.is_some()).count();

        if literal_count == 0 {
            // Never a literal anywhere: not a discriminator candidate, and
            // uninformative for the "nonconstant" refusal either.
            continue;
        }
        if literal_count < literals.len() {
            // A literal in some branches, a non-constant expression in at
            // least one other — fail-closed, but keep looking for a
            // genuinely qualifying candidate before reporting this.
            nonconstant_candidate.get_or_insert_with(|| name.clone());
            continue;
        }

        // Every branch declares this column as a constant literal (the
        // `literal_count` check above proved every entry is `Some`, so
        // `flatten` drops nothing) — check the values are pairwise distinct.
        let values: Vec<(String, String)> = literals.into_iter().flatten().collect();

        // The column must be the *same* literal kind in every branch: a
        // `UNION ALL` coerces the column to one common supertype, so an
        // equality predicate against a same-text-different-kind pair (e.g.
        // `1` vs `'1'`) is not kind-safe — the emitted `DELETE` would
        // implicit-cast and match the surviving branch's row too, even
        // though the two literals are not `==` as a Rust tuple. Refused
        // by name, distinct from the "not a constant literal" and "not
        // distinct" refusals below, but kept looking for a genuinely
        // qualifying candidate before reporting it (same posture as the
        // other two fail-closed buckets).
        let first_kind = &values[0].0;
        if values.iter().any(|(kind, _)| kind != first_kind) {
            mixed_kind_candidate.get_or_insert_with(|| name.clone());
            continue;
        }

        let mut duplicate: Option<String> = None;
        'outer: for i in 0..values.len() {
            for j in (i + 1)..values.len() {
                if values[i].0 == values[j].0 && values[i].1 == values[j].1 {
                    duplicate = Some(values[i].1.clone());
                    break 'outer;
                }
            }
        }
        match duplicate {
            None => {
                // A qualifying discriminator — removed_branch's own value is
                // always `values[0]` (branch_exprs was built with
                // removed_expr first). Its raw text is already valid SQL to
                // splice verbatim into the equality predicate.
                return Ok((name.clone(), values[0].1.clone()));
            }
            Some(value) => {
                shared_candidate.get_or_insert((name.clone(), value));
            }
        }
    }

    if let Some((name, value)) = shared_candidate {
        return Err(format!(
            "candidate discriminator column '{name}' is not distinct per branch — its constant \
             value '{value}' is shared by more than one branch, so an equality predicate on it \
             would also delete a surviving branch's rows"
        ));
    }
    if let Some(name) = mixed_kind_candidate {
        return Err(format!(
            "discriminator column '{name}' mixes literal kinds across branches — column type is \
             union-coerced, equality is not kind-safe"
        ));
    }
    if let Some(name) = nonconstant_candidate {
        return Err(format!(
            "candidate discriminator column '{name}' is not a constant literal in every branch \
             — a non-constant expression in at least one branch means branch removal cannot be \
             proven targetable from the definitions alone"
        ));
    }
    Err(
        "no provenance predicate distinguishes the removed branch's rows in the stored table — \
         add a constant discriminator column (e.g. a literal `AS src` value distinct per \
         branch) to make branch removal targetable"
            .to_string(),
    )
}

/// `branch`'s own SELECT-list items as `(output name, expression)` pairs —
/// the F2 discriminator proof's per-branch view. Mirrors
/// [`branch_output_column_names`] (same wildcard/spread fail-closed
/// posture) but also keeps each item's expression, which the discriminator
/// proof needs to test with [`walk::constant_literal_tag`].
fn branch_named_items(branch: &SelectStmt) -> Result<Vec<(String, Expr)>, String> {
    let list = branch
        .select_list()
        .ok_or_else(|| "the branch has no SELECT list".to_string())?;
    let mut items = Vec::new();
    for entry in list.entries() {
        match entry {
            SelectEntry::Item(item) => {
                if item.is_wildcard() {
                    return Err(
                        "a wildcard SELECT item (`*`) in a branch is not resolvable to a named \
                         column — F2's discriminator proof needs an explicit name for every \
                         branch column"
                            .to_string(),
                    );
                }
                let name = item.column_name().ok_or_else(|| {
                    "a SELECT item in a branch has no derivable output column name".to_string()
                })?;
                let expr = item
                    .expression()
                    .ok_or_else(|| "a SELECT item in a branch has no expression".to_string())?;
                items.push((name, expr));
            }
            SelectEntry::Spread(_) => {
                return Err(
                    "a spread (`...expr`) SELECT item in a branch is not resolvable to a named \
                     column — F2's discriminator proof needs an explicit name for every branch \
                     column"
                        .to_string(),
                );
            }
        }
    }
    Ok(items)
}

fn build_f1_atom(
    index: usize,
    branch: &SelectStmt,
    after_columns: Option<&[String]>,
    first_branch_order: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::AddedSetOpBranch { index };
    match build_f1_option(branch, after_columns, first_branch_order, inputs) {
        Ok(option) => AtomAnalysis {
            change: atom_change,
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(reason) => AtomAnalysis {
            change: atom_change,
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("added set-operation branch #{index}"),
                reason,
            }],
        },
    }
}

/// F1 admission (research §4 F1): `UNION ALL` is additive, so the added
/// branch is exactly the delta — `INSERT INTO t (<cols>) SELECT <cols> FROM
/// (<branch>) …`, no difference predicate needed (unlike E2/E4).
///
/// C4 fix (final-review finding): `UNION ALL` binds columns *positionally*,
/// not by name — a rebuild's `CREATE TABLE ... AS <after>` (with a
/// multi-branch UNION ALL) takes every branch's column *names* from the
/// first branch alone; a later branch's own declared names are irrelevant
/// to the rebuild, only its column *order* matters. This emitter's `INSERT`
/// is name-based (`emit::emit_branch_insert` selects the branch's own output
/// columns by name into the target), so an added branch whose own output
/// names/order do not exactly match the first branch's would silently bind
/// different values under the same names than a real rebuild's positional
/// binding would — refused here rather than risking that divergence.
fn build_f1_option(
    branch: &SelectStmt,
    after_columns: Option<&[String]>,
    first_branch_order: Option<&[String]>,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let after_columns = after_columns.ok_or_else(|| {
        "F1 (added UNION ALL branch) refused: the after-definition's output column list could \
         not be determined (the SELECT-list diff is opaque)"
            .to_string()
    })?;
    if after_columns.is_empty() {
        return Err(
            "F1 (added UNION ALL branch) refused: the after-definition has no output columns"
                .to_string(),
        );
    }

    let first_branch_order = first_branch_order.ok_or_else(|| {
        "F1 (added UNION ALL branch) refused: the first branch's own declared output column \
         order could not be determined (the SELECT-list diff is opaque, or this diff shape is \
         not one the classifier can prove a stable first-branch order for)"
            .to_string()
    })?;
    let branch_columns = branch_output_column_names(branch)?;
    if branch_columns != first_branch_order {
        return Err(format!(
            "F1 (added UNION ALL branch) refused: the added branch's own output column names/order \
             ({branch_columns:?}) do not exactly match the first branch's declared order \
             ({first_branch_order:?}) — UNION ALL binds columns positionally, so a rebuild would \
             bind the added branch's values under the first branch's names, while this name-based \
             INSERT would bind them under the added branch's own names, silently diverging \
             whenever the two orders differ"
        ));
    }

    let branch_sql = branch_own_text(branch);

    // Same key-addressability obligation E2/E4's identity anti-join guard
    // shares (research §4 intro): a declared identity that isn't provably
    // NOT NULL is treated exactly like no declared identity — the guard is
    // omitted and the option is recorded one-shot, never a refusal.
    let identity_columns = inputs.row_identity.as_ref().filter(|cols| {
        cols.iter()
            .all(|c| inputs.not_null_columns.contains(c.as_str()))
    });
    let statement = emit::emit_branch_insert(
        &inputs.table,
        after_columns,
        &branch_sql,
        identity_columns.map(Vec::as_slice),
    );

    Ok(BackbuildOption {
        technique: Technique::UnionBranchInsert,
        slot: Some(HSlot::Insert),
        statements: vec![statement],
        write_scope: WriteScope::RowSubset,
        reads_upstream: true,
        rerun_safe: identity_columns.is_some(),
    })
}

/// `branch`'s own output column names, in declared order — C4's
/// positional-`UNION-ALL` validation (`build_f1_option`) needs this to
/// compare against the first branch's own declared order. Mirrors
/// `diff.rs`'s private `collect_named_columns` (out of this phase's
/// touch-scope; a small, self-contained SELECT-list walk, not the
/// dependency-collection machinery the "don't fork" rule is about), but only
/// ever needs the ordered name sequence, not each item's expression. Fails
/// closed (an `Err`, never a silently empty/partial list) on any item this
/// module cannot key by output name — a wildcard, a spread, or an item with
/// no derivable name — exactly like `diff.rs`'s comparator does for the
/// top-level SELECT list.
fn branch_output_column_names(branch: &SelectStmt) -> Result<Vec<String>, String> {
    let list = branch
        .select_list()
        .ok_or_else(|| "the added branch has no SELECT list".to_string())?;
    let mut names = Vec::new();
    for entry in list.entries() {
        match entry {
            SelectEntry::Item(item) => {
                if item.is_wildcard() {
                    return Err(
                        "a wildcard SELECT item (`*`) in the added branch is not resolvable to \
                         a named column — F1's positional-order validation needs an explicit \
                         name for every branch column"
                            .to_string(),
                    );
                }
                let name = item.column_name().ok_or_else(|| {
                    "a SELECT item in the added branch has no derivable output column name"
                        .to_string()
                })?;
                names.push(name);
            }
            SelectEntry::Spread(_) => {
                return Err(
                    "a spread (`...expr`) SELECT item in the added branch is not resolvable to \
                     a named column — F1's positional-order validation needs an explicit name \
                     for every branch column"
                        .to_string(),
                );
            }
        }
    }
    Ok(names)
}

/// `branch`'s own source text — just its own `SELECT`/`FROM`/`WHERE`/…
/// clauses, stopping before the `UNION`/`INTERSECT`/`EXCEPT` token that
/// introduces its set-operation tail (if any). Mirrors `diff.rs`'s
/// `branch_token_seq` (same direct-children walk, same stop condition) but
/// returns the branch's actual source substring rather than a token-kind
/// sequence — needed here to splice the branch verbatim into F1's `INSERT
/// … SELECT <branch>` script. Duplicated rather than exposed from `diff.rs`
/// (out of this phase's touch-scope) — a small, self-contained CST walk, not
/// the dependency-collection machinery the "don't fork" rule is about.
fn branch_own_text(branch: &SelectStmt) -> String {
    use smelt_parser::SyntaxKind::{EXCEPT_KW, INTERSECT_KW, UNION_KW};

    let node = branch.syntax();
    let node_start = u32::from(node.text_range().start());
    let full_text = node.text().to_string();

    let mut first = None;
    let mut last = None;
    'outer: for child in node.children_with_tokens() {
        if let Some(tok) = child.as_token() {
            if matches!(tok.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW) {
                // The set-operation tail (and the branch it introduces)
                // starts here — everything from here on belongs to a
                // different branch.
                break 'outer;
            }
            if !tok.kind().is_trivia() {
                if first.is_none() {
                    first = Some(tok.text_range().start());
                }
                last = Some(tok.text_range().end());
            }
            continue;
        }
        if let Some(child_node) = child.as_node() {
            for tok in child_node
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .filter(|t| !t.kind().is_trivia())
            {
                if first.is_none() {
                    first = Some(tok.text_range().start());
                }
                last = Some(tok.text_range().end());
            }
        }
    }

    match (first, last) {
        (Some(start), Some(end)) => {
            let start = (u32::from(start) - node_start) as usize;
            let end = (u32::from(end) - node_start) as usize;
            full_text[start..end].to_string()
        }
        _ => full_text,
    }
}

/// A dropped column left over after rename pairing (research §4 C1):
/// `ALTER TABLE t DROP COLUMN d;`, sequenced into the H "ALTER DROPs" slot
/// (`HSlot::Drop`, last — `assemble`'s composition order). This function
/// only *sequences* the drop; whether drops are permitted at all
/// (`--allow-column-removal`) is a policy decision that stays owned by
/// `docs/specs/schema_evolution.md` and is applied at wiring time, never
/// here (research §4 C1: "Classification and the opt-in flag doctrine ...
/// stay owned by schema_evolution.md; backbuild's job is only to *sequence*
/// the drop").
fn dropped_column_atom(table: &str, name: &str) -> AtomAnalysis {
    let stmt = emit::emit_alter_drop_column(table, name);
    AtomAnalysis {
        change: AtomicChange::DroppedColumn {
            name: name.to_string(),
        },
        options: vec![BackbuildOption {
            technique: Technique::ColumnDrop,
            slot: Some(HSlot::Drop),
            statements: vec![stmt],
            // Destructive, distinct from `ColumnScoped` (research §4 C1's
            // metadata requirement): this is what lets a wiring-time
            // chooser gate the technique behind `--allow-column-removal`
            // without re-classifying.
            write_scope: WriteScope::Destructive,
            reads_upstream: false,
            // DDL drop is not re-runnable — a second `ALTER TABLE ... DROP
            // COLUMN` on an already-dropped column errors (research §2
            // "Idempotence"), same posture as `Technique::Rename`.
            rerun_safe: false,
        }],
        inadmissible: Vec::new(),
    }
}

/// Rename pairing (B2) runs *before* add/drop classification, then every
/// remaining added column is classified as B1 or refused (research §4 B1,
/// B2, §7.2 "Rename-match ambiguity"). `representative_sources` is the
/// caller's already-computed representative set (research §4 intro "one
/// uniform rule") — shared with the sibling `changed`-column (D1)
/// classification so both draw from exactly the same stored-representative
/// provenance, resolved by [`resolve_representative`] (C2 fix), never by
/// bare output name alone.
fn classify_select_list(
    added: &[SelectColumn],
    dropped: &[SelectColumn],
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    let RenamePairing {
        atoms: mut rename_atoms,
        consumed_added,
    } = pair_renames(dropped, added, inputs);

    for col in added {
        if consumed_added.contains(&col.name) {
            // Already produced its own atom inside `pair_renames` (either
            // the rename atom itself, for the winner, or a "reads the
            // renamed column" B1 atom, for a tie-break loser).
            continue;
        }
        rename_atoms.push(classify_added_column(
            col,
            representative_sources,
            None,
            inputs,
        ));
    }

    rename_atoms
}

/// The stored 1:1 representative set (research §4 intro "Derivability
/// representatives — one uniform rule"), with full provenance: every
/// SELECT-list output column present, unchanged, in both definitions
/// (`unchanged`) *and* whose own expression is a bare column reference — a
/// bare pull-through, never a computed expression. A changed column is never
/// a representative by construction (it is not in `unchanged` at all). Every
/// derivability/grain-link proof in this module resolves a dependency
/// against this set via [`resolve_representative`] (never by bare output
/// name alone — the C2 fix, `super::resolve_representative`'s doc comment).
fn representative_sources(unchanged: &[SelectColumn]) -> Vec<RepresentativeSource> {
    unchanged
        .iter()
        .filter_map(|c| {
            let col_ref = c.expr.as_column_ref()?;
            let qualifier = col_ref.qualifier().map(|q| q.to_string());
            let raw_name = col_ref.name().to_string();
            let leaf = chase_leaf(c.expr.syntax(), qualifier.as_deref(), &raw_name);
            Some(RepresentativeSource {
                output_name: c.name.clone(),
                qualifier,
                raw_name,
                leaf,
            })
        })
        .collect()
}

fn classify_added_column(
    col: &SelectColumn,
    representative_sources: &[RepresentativeSource],
    join_admission: Option<&JoinAdmission>,
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::AddedColumn {
        name: col.name.clone(),
    };

    // B4 (research §4 B4) takes priority when every dependency of `col`'s
    // expression is qualified with a newly-added `LEFT JOIN`'s alias
    // ("added SELECT items reference the new alias"): B1/B3 would refuse
    // for such a column anyway (a brand-new alias never has a stored 1:1
    // pull-through representative bound to it), so attempting them first
    // would only produce confusing, redundant refusal text.
    if let Some(admission) = join_admission {
        if depends_only_on_join_alias(&col.expr, &admission.alias) {
            return match try_b4(col, admission, inputs) {
                Ok(options) => AtomAnalysis {
                    change: atom_change,
                    options,
                    inadmissible: Vec::new(),
                },
                Err(reason) => AtomAnalysis {
                    change: atom_change,
                    options: Vec::new(),
                    inadmissible: vec![BackbuildRefusal {
                        atom: format!("added column '{}'", col.name),
                        reason,
                    }],
                },
            };
        }
    }

    // B1 and B3 are attempted independently (research §2 "Options, not
    // choices"): an added column derivable both from stored columns and
    // from an upstream pull-through must carry *both* options on one atom,
    // mirroring the D-class dual-derivability symmetry
    // (`d_dual_derivable_yields_both_options`). A refusal on one side is
    // recorded regardless of whether the other side is admitted — options
    // and refusals are not mutually exclusive
    // (`b_dual_derivable_refusals_still_recorded`).
    let b1_result = try_b1(col, representative_sources, inputs);

    // B3 shares B1's expression-validity leaf check (subquery/window/opaque
    // function/non-determinism, research §4 intro) — when B1's failure came
    // from that shared leaf check, B3 would refuse identically, so it is
    // never attempted (avoids a redundant, confusing second refusal for the
    // exact same underlying reason). This only ever short-circuits when B1
    // *failed*; if B1 succeeded, B3 is still attempted independently.
    let expr_is_invalid = model_diff::collect_dependencies(&col.expr).is_err();
    let attempt_b3 = b1_result.is_ok() || !expr_is_invalid;
    let b3_result = if attempt_b3 {
        Some(try_b3(col, representative_sources, inputs))
    } else {
        None
    };

    // B5 (research §4 B5) is only attempted when `col`'s expression is
    // itself a registry-recognized aggregate call under a GROUP BY model
    // (`is_group_by_aggregate_shape`) — gated the same way B4's
    // `join_admission` branch above is, so an ordinary B1/B3-shaped added
    // column under a GROUP BY model (e.g. a bare pull-through of a stored
    // GROUP BY key) never picks up a spurious "not an aggregate" refusal.
    let b5_result = if is_group_by_aggregate_shape(&col.expr) {
        Some(try_b5(col, representative_sources, inputs))
    } else {
        None
    };

    // B6 (research §4 B6) is only attempted when `col`'s expression is
    // itself a window-function call (`OVER` present) — gated the same way
    // B4/B5's own shape gates are, so an ordinary non-window added column
    // never picks up a spurious B6 refusal alongside its own B1/B3
    // options. This is also the narrowing research §4's intro promises: a
    // window in an added column no longer refuses unconditionally (B1/B3's
    // shared leaf check still refuses it, via `model_diff::collect_dependencies`'s
    // window fail-close) — B6 is the dedicated admission path for exactly
    // this shape.
    let b6_result = if is_window_column_shape(&col.expr) {
        Some(try_b6(col, representative_sources, inputs))
    } else {
        None
    };

    let mut options = Vec::new();
    let mut inadmissible = Vec::new();
    match b1_result {
        Ok(option) => options.push(option),
        Err(reason) => inadmissible.push(BackbuildRefusal {
            atom: format!("added column '{}'", col.name),
            reason,
        }),
    }
    if let Some(b3_result) = b3_result {
        match b3_result {
            Ok(option) => options.push(option),
            Err(reason) => inadmissible.push(BackbuildRefusal {
                atom: format!("added column '{}'", col.name),
                reason,
            }),
        }
    }
    if let Some(b5_result) = b5_result {
        match b5_result {
            Ok(option) => options.push(option),
            Err(reason) => inadmissible.push(BackbuildRefusal {
                atom: format!("added column '{}'", col.name),
                reason,
            }),
        }
    }
    if let Some(b6_result) = b6_result {
        match b6_result {
            Ok(option) => options.push(option),
            Err(reason) => inadmissible.push(BackbuildRefusal {
                atom: format!("added column '{}'", col.name),
                reason,
            }),
        }
    }

    AtomAnalysis {
        change: atom_change,
        options,
        inadmissible,
    }
}

/// Render a `(qualifier, raw_name)` dependency for an error message —
/// `q.n` when qualified, bare `n` otherwise.
fn format_dependency(qualifier: &Option<String>, raw_name: &str) -> String {
    match qualifier {
        Some(q) => format!("{q}.{raw_name}"),
        None => raw_name.to_string(),
    }
}

/// B1 admission (research §4 B1): every dependency of `col`'s expression
/// must resolve to a stored 1:1 representative, matched by qualifier and raw
/// column name via [`resolve_representative`] (C2 fix) — never by bare
/// output name alone, which would let a qualified dependency collide with an
/// unrelated same-named representative. Uses
/// [`collect_qualified_dependencies`] — the same dependency walk B3/B4 use,
/// itself built over `analysis::model_diff::collect_dependencies`'s
/// validation (subquery/window/opaque-function/non-determinism fail-closed,
/// research §2 "Determinism caveat"; `docs/specs/architecture.md` §"Property
/// composition walk rule") — rather than the bare-name-only
/// `collect_dependencies` directly.
fn try_b1(
    col: &SelectColumn,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let deps = collect_qualified_dependencies(&col.expr).map_err(|reason| {
        format!(
            "B1 (self-derivable column add) refused for '{}': {reason}",
            col.name
        )
    })?;

    let mut missing: Vec<String> = deps
        .iter()
        .filter(|(q, n)| {
            resolve_representative(
                q.as_deref(),
                n,
                representative_sources,
                Some(col.expr.syntax()),
            )
            .is_none()
        })
        .map(|(q, n)| format_dependency(q, n))
        .collect();
    missing.sort();
    if let Some(dep) = missing.first() {
        return Err(format!(
            "B1 (self-derivable column add) refused for '{}': depends on '{dep}', which has no \
             1:1 stored representative (a bare pull-through unchanged between both definitions, \
             matched by qualifier and raw column name) in the model's own output — an \
             upstream-only dependency needs B3, not B1",
            col.name
        ));
    }

    let requalified =
        requalify::requalify(&col.expr, representative_sources).map_err(|reason| {
            format!(
                "B1 (self-derivable column add) refused for '{}': {reason}",
                col.name
            )
        })?;

    build_b1_option(col, &requalified, inputs)
}

/// Build the B1 `ALTER ADD` + in-place `UPDATE` option once `requalified_expr`
/// is known to be safe to splice into a self-read `UPDATE` (either the
/// generic dependency-derived text from [`try_b1`], or — for a B2 tie-break
/// loser — a bare reference to the rename's winning column name).
fn build_b1_option(
    col: &SelectColumn,
    requalified_expr: &str,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let sql_type = inputs.added_column_types.get(&col.name).ok_or_else(|| {
        format!(
            "B1 (self-derivable column add) refused for '{}': no declared SQL type in \
             BackbuildInputs::added_column_types",
            col.name
        )
    })?;

    let alter = emit::emit_alter_add_column(&inputs.table, &col.name, sql_type);
    let update = emit::emit_in_place_update(
        &inputs.table,
        &[(col.name.clone(), requalified_expr.to_string())],
    );

    Ok(BackbuildOption {
        technique: Technique::SelfDerivedColumnAdd,
        // Both statements sit in one H-slot bucket: the `ALTER ADD` must
        // still run before any rename-dependent add (`HSlot::Alter` flushes
        // strictly after `HSlot::Rename` — see `assemble`), and the paired
        // `UPDATE` only ever touches this atom's own freshly-added column,
        // so it never needs to be reordered relative to a sibling atom's
        // own alter/update pair.
        slot: Some(HSlot::Alter),
        statements: vec![alter, update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: false,
        // The `ALTER ADD` step is DDL and not re-runnable (research §2
        // "Idempotence": "DDL steps (ALTER ADD/RENAME) are not
        // re-runnable"), even though the paired `UPDATE` alone would be.
        rerun_safe: false,
    })
}

// ===== B3 / D2: upstream pull-through — one admission path, two triggers
// (research §4 B3, D2) =====

/// The result of the shared B3/D2 grain-link proof
/// ([`admit_upstream_pullthrough`]): the requalified expression text (every
/// dependency rewritten from the bound FROM-tree alias to the statement's
/// upstream alias, `requalify::requalify_upstream`), the upstream's physical
/// table name, and the `(target column, upstream column)` key-equality pairs
/// for the `WHERE` clause, one per `unique_key` component.
struct UpstreamPullthrough {
    requalified_expr: String,
    upstream_physical: String,
    key_pairs: Vec<(String, String)>,
}

/// The shared B3/D2 admission proof (research §4 B3/D2, intro "Key
/// addressability"): `expr`'s dependencies must resolve to columns of a
/// *single* FROM-tree alias (never a mix of aliases, never an unqualified
/// reference — the proof cannot bind an alias it cannot name); that alias
/// must have a `unique_key` declared in `BackbuildInputs::sources`; every
/// key column must have a 1:1 stored bare pull-through bound to that *exact
/// same* alias (`representative_sources` — never merely the same physical
/// table under a different alias, which is what makes a self-join's `o1`
/// pull-through refuse to license `o2`'s columns); and every key column must
/// be declared NOT NULL for that source. One admission path shared by B3
/// (an added column) and D2 (a changed column's new expression) — the two
/// callers differ only in which statements they wrap the result in (B3
/// prepends an `ALTER ADD`; D2 does not, since the column already exists).
fn admit_upstream_pullthrough(
    expr: &Expr,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<UpstreamPullthrough, String> {
    let deps = collect_qualified_dependencies(expr)?;
    if deps.is_empty() {
        return Err(
            "the expression has no column dependencies — nothing to bind a FROM-tree alias to"
                .to_string(),
        );
    }

    let aliases: BTreeSet<Option<String>> = deps.iter().map(|(q, _)| q.clone()).collect();
    if aliases.len() > 1 {
        return Err(format!(
            "depends on {} distinct FROM-tree aliases/qualifiers — not admissible as a single- \
             upstream pull-through (the grain-link proof binds to exactly one alias)",
            aliases.len()
        ));
    }
    let alias = aliases.into_iter().next().flatten().ok_or_else(|| {
        "depends on an unqualified column reference — the FROM-tree alias it reads from \
             cannot be determined; qualify it (e.g. `o.col`)"
            .to_string()
    })?;

    let source = inputs.sources.get(&alias).ok_or_else(|| {
        format!("no source declared for FROM-tree alias '{alias}' in BackbuildInputs::sources")
    })?;

    let unique_key = source
        .unique_key
        .as_ref()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            format!(
                "upstream '{alias}' has no declared unique_key — an equality backfill needs an \
                 addressable identity"
            )
        })?;

    let mut key_pairs = Vec::with_capacity(unique_key.len());
    for k in unique_key {
        let rep = representative_sources
            .iter()
            .find(|r| r.qualifier.as_deref() == Some(alias.as_str()) && &r.raw_name == k);
        let Some(rep) = rep else {
            return Err(format!(
                "no addressable identity — the output has no 1:1 stored pull-through of upstream \
                 '{alias}''s unique_key column '{k}' bound to that same alias (a self-join binds \
                 per FROM-tree alias, not per physical table)"
            ));
        };
        if !source.not_null_columns.contains(k) {
            return Err(format!(
                "upstream '{alias}''s unique_key column '{k}' is not declared NOT NULL \
                 (BackbuildInputs::sources[...].not_null_columns) — SQL UNIQUE admits NULLs, and \
                 an equality backfill never addresses a NULL-keyed row"
            ));
        }
        key_pairs.push((rep.output_name.clone(), rep.raw_name.clone()));
    }

    let requalified_expr = requalify::requalify_upstream(expr, &alias, "u")?;

    Ok(UpstreamPullthrough {
        requalified_expr,
        upstream_physical: source.physical_name.clone(),
        key_pairs,
    })
}

/// The B3/D2 leaf classifier: walk the same recognised expression shapes
/// [`model_diff::collect_dependencies`] already validated (subquery/window/
/// opaque-function/non-determinism fail-closed — reused, not re-derived,
/// per `docs/specs/architecture.md` §"Property composition walk rule"), but
/// preserve each column reference's qualifier instead of discarding it. The
/// grain-link proof needs to know *which* FROM-tree alias each dependency
/// reads from, not just its bare name — `model_diff::collect_dependencies`'s
/// `HashSet<String>` result has already thrown that away.
fn collect_qualified_dependencies(expr: &Expr) -> Result<Vec<(Option<String>, String)>, String> {
    model_diff::collect_dependencies(expr)?;
    let mut deps = Vec::new();
    walk_qualified(expr, &mut deps);
    Ok(deps)
}

/// Mirrors [`collect_qualified_dependencies`]'s caller-validated shapes.
/// Every subexpression reached here has already been proven (by the
/// `collect_dependencies` call in `collect_qualified_dependencies`) to be
/// one of these recognised shapes or a dependency-free literal, so this walk
/// is total — nothing left to fail closed on.
fn walk_qualified(expr: &Expr, out: &mut Vec<(Option<String>, String)>) {
    if let Some(col) = expr.as_column_ref() {
        out.push((
            col.qualifier().map(|q| q.to_string()),
            col.name().to_string(),
        ));
        return;
    }
    if let Some(func) = expr.as_function_call() {
        for arg in func.arguments() {
            walk_qualified(&arg, out);
        }
        return;
    }
    if let Some(bin) = expr.as_binary() {
        for side in [bin.left(), bin.right()].into_iter().flatten() {
            walk_qualified(&side, out);
        }
        return;
    }
    if let Some(case) = expr.as_case() {
        if let Some(value) = case.case_value() {
            walk_qualified(&value, out);
        }
        for when in case.when_clauses() {
            for arm in [when.condition(), when.result()].into_iter().flatten() {
                walk_qualified(&arm, out);
            }
        }
        if let Some(else_expr) = case.else_expr() {
            walk_qualified(&else_expr, out);
        }
        return;
    }
    if let Some(cast) = expr.as_cast() {
        if let Some(inner) = cast.expression() {
            walk_qualified(&inner, out);
        }
    }
    // Anything else is a dependency-free literal (already proven safe by
    // `collect_dependencies`) — nothing to record.
}

/// B3 (research §4 B3): an added column whose expression pulls through an
/// upstream already in the FROM tree, admitted via
/// [`admit_upstream_pullthrough`]. *Script*: `ALTER TABLE t ADD COLUMN c
/// <ty>;` (the column does not exist yet, unlike D2) followed by the
/// column-scoped `UPDATE ... FROM` backfill.
fn try_b3(
    col: &SelectColumn,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let pullthrough = admit_upstream_pullthrough(&col.expr, representative_sources, inputs)
        .map_err(|reason| {
            format!(
                "B3 (upstream pull-through) refused for '{}': {reason}",
                col.name
            )
        })?;

    let sql_type = inputs.added_column_types.get(&col.name).ok_or_else(|| {
        format!(
            "B3 (upstream pull-through) refused for '{}': no declared SQL type in \
             BackbuildInputs::added_column_types",
            col.name
        )
    })?;

    let alter = emit::emit_alter_add_column(&inputs.table, &col.name, sql_type);
    let update = emit::emit_column_backfill_update_from(
        &inputs.table,
        &[(col.name.clone(), pullthrough.requalified_expr)],
        &pullthrough.upstream_physical,
        "u",
        &pullthrough.key_pairs,
    );

    Ok(BackbuildOption {
        technique: Technique::UpstreamPullthrough,
        // Both statements sit in one H-slot bucket for the same reason B1's
        // do (see `build_b1_option`): the `ALTER ADD` must flush before any
        // rename-dependent add, and the paired `UPDATE` only ever touches
        // this atom's own freshly-added column.
        slot: Some(HSlot::Alter),
        statements: vec![alter, update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: true,
        // The `ALTER ADD` step is DDL and not re-runnable (research §2
        // "Idempotence"), same as B1.
        rerun_safe: false,
    })
}

/// D2 (research §4 D2): a changed column whose *new* expression pulls
/// through an upstream already in the FROM tree — same proof and script as
/// B3, triggered by an expression change rather than a column add. *Script*:
/// a single column-scoped `UPDATE ... FROM` — the column already exists, so
/// no `ALTER`.
fn try_d2(
    changed: &ChangedColumn,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let pullthrough = admit_upstream_pullthrough(&changed.after, representative_sources, inputs)
        .map_err(|reason| {
            format!(
                "D2 (upstream-read expression change) refused for '{}': {reason}",
                changed.name
            )
        })?;

    let update = emit::emit_column_backfill_update_from(
        &inputs.table,
        &[(changed.name.clone(), pullthrough.requalified_expr)],
        &pullthrough.upstream_physical,
        "u",
        &pullthrough.key_pairs,
    );

    Ok(BackbuildOption {
        technique: Technique::UpstreamPullthrough,
        slot: Some(HSlot::UpdateMerge),
        statements: vec![update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: true,
        // Plain DML, no DDL step — deterministic and re-runnable, same as
        // D1.
        rerun_safe: true,
    })
}

// ===== B5: new aggregate column at unchanged GROUP BY grain (research §4
// B5) =====

/// The B5 attempt gate: `expr` must itself be a registry-recognized
/// aggregate call (`SqlFunction::is_aggregate`, the same registry-backed
/// leaf classification `analysis::mod::classify_select_items` uses) sitting
/// inside a `GROUP BY` model. Deliberately checked *before* [`try_b5`] is
/// ever called — an ordinary B1/B3-shaped added column under a `GROUP BY`
/// model (e.g. a bare pull-through of a stored `GROUP BY` key) must never
/// pick up a spurious "not an aggregate" refusal alongside its admitted B1
/// option, mirroring how `depends_only_on_join_alias` gates the B4 attempt
/// above.
fn is_group_by_aggregate_shape(expr: &Expr) -> bool {
    let Some(func) = expr.as_function_call() else {
        return false;
    };
    let Some(name) = func.name() else {
        return false;
    };
    if !SqlFunction::from_name(&name).is_some_and(|f| f.is_aggregate()) {
        return false;
    }
    expr.syntax()
        .ancestors()
        .find_map(SelectStmt::cast)
        .and_then(|stmt| stmt.group_by_clause())
        .is_some()
}

/// One `GROUP BY` key's addressable identity (research §4 intro "Key
/// addressability" + "one uniform rule"): the key's own qualified source
/// text (spliced verbatim into the re-aggregation subquery's own `SELECT`
/// list, aliased to the representative's stored output name so the
/// backfill's join predicate can address it by that name) and the stored
/// output name the `UPDATE ... FROM` predicate matches on.
struct GroupKeyBinding {
    /// `<qualifier>.<raw_name> AS <output_name>` (or bare `<raw_name> AS
    /// <output_name>` when unqualified) — this key's own entry in the
    /// re-aggregation subquery's `SELECT` list.
    select_item: String,
    /// The model's own stored column name for this key — used on both
    /// sides of the `UPDATE ... FROM` predicate
    /// (`emit::emit_column_backfill_update_from_subquery`'s `key_pairs`).
    output_name: String,
}

/// Resolve one `GROUP BY` key expression to its [`GroupKeyBinding`]: it must
/// be a bare column reference with a stored 1:1 pull-through representative
/// (research §4 intro "one uniform rule"), and that representative's own
/// output column must be declared `NOT NULL`
/// (`BackbuildInputs::not_null_columns` — the shared key-addressability
/// obligation: an equality match never addresses a NULL-keyed row).
fn resolve_group_key(
    key_expr: &Expr,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<GroupKeyBinding, String> {
    let col_ref = key_expr.as_column_ref().ok_or_else(|| {
        format!(
            "GROUP BY key '{}' is not a bare column reference — no addressable identity can be \
             derived for a computed grouping key",
            key_expr.text().trim()
        )
    })?;
    let qualifier = col_ref.qualifier().map(|q| q.to_string());
    let raw_name = col_ref.name().to_string();

    let rep = resolve_representative(
        qualifier.as_deref(),
        &raw_name,
        representative_sources,
        Some(key_expr.syntax()),
    )
    .ok_or_else(|| {
        format!(
            "GROUP BY key '{}' has no 1:1 stored bare pull-through (unchanged between both \
                 definitions) in the model's own output — no addressable identity",
            format_dependency(&qualifier, &raw_name)
        )
    })?;

    if !inputs.not_null_columns.contains(rep.output_name.as_str()) {
        return Err(format!(
            "GROUP BY key '{}' is not declared NOT NULL (BackbuildInputs::not_null_columns) — \
             an equality match never addresses a NULL-keyed row",
            rep.output_name
        ));
    }

    Ok(GroupKeyBinding {
        select_item: format!("{} AS {}", key_expr.text().trim(), rep.output_name),
        output_name: rep.output_name.clone(),
    })
}

/// B5 (research §4 B5): an added SELECT item that is itself a registry-
/// recognized aggregate call over a `GROUP BY` model whose skeleton is
/// unchanged (`is_group_by_aggregate_shape` already proved both, as this
/// function's caller's gate). *Script*: `ALTER TABLE t ADD COLUMN c <ty>;`
/// then a matched-only column backfill from the after-definition's own
/// re-aggregation — full `FROM`/`WHERE` tree, `GROUP BY` keys — via
/// `emit::emit_column_backfill_update_from_subquery`: a bare `SELECT
/// <keys>, <agg> FROM <upstream> GROUP BY <keys>` would over-count rows the
/// model's own `WHERE` filters out (research §4 B5's named trap), so the
/// re-aggregation carries the model's `WHERE` verbatim. No insert arm: a
/// group absent from `t` is a group the row-set-unchanged proof already
/// guarantees cannot exist.
fn try_b5(
    col: &SelectColumn,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    // Determinism/registry validation of the aggregate's own arguments
    // (fail-closed on a volatile or unregistered nested call — research §2
    // "Determinism caveat": "a volatile or unrecognised function in an
    // added or changed expression refuses the atom"), reusing the same
    // leaf check B1/B3 use rather than re-deriving it.
    model_diff::collect_dependencies(&col.expr).map_err(|reason| {
        format!(
            "B5 (aggregate column add) refused for '{}': {reason}",
            col.name
        )
    })?;

    let stmt = col
        .expr
        .syntax()
        .ancestors()
        .find_map(SelectStmt::cast)
        .ok_or_else(|| {
            format!(
                "B5 (aggregate column add) refused for '{}': could not locate the enclosing \
                 SELECT statement",
                col.name
            )
        })?;

    let group_by = stmt.group_by_clause().ok_or_else(|| {
        format!(
            "B5 (aggregate column add) refused for '{}': the model has no GROUP BY clause",
            col.name
        )
    })?;
    if group_by.is_all() {
        return Err(format!(
            "B5 (aggregate column add) refused for '{}': GROUP BY ALL has no explicit \
             grouping-key expressions to re-aggregate on",
            col.name
        ));
    }

    let mut key_pairs = Vec::new();
    let mut select_items = Vec::new();
    for key_expr in group_by.expressions() {
        let binding =
            resolve_group_key(&key_expr, representative_sources, inputs).map_err(|reason| {
                format!(
                    "B5 (aggregate column add) refused for '{}': {reason}",
                    col.name
                )
            })?;
        key_pairs.push((binding.output_name.clone(), binding.output_name));
        select_items.push(binding.select_item);
    }
    if key_pairs.is_empty() {
        return Err(format!(
            "B5 (aggregate column add) refused for '{}': the GROUP BY clause has no explicit \
             grouping-key expressions",
            col.name
        ));
    }

    let from_text = stmt.from_clause().map(|f| f.text()).ok_or_else(|| {
        format!(
            "B5 (aggregate column add) refused for '{}': the model has no FROM clause",
            col.name
        )
    })?;
    let where_text = stmt.where_clause().map(|w| w.text()).unwrap_or_default();
    let group_by_text = group_by.syntax().text().to_string();

    let agg_text = col.expr.text();
    let mut subquery_parts = vec![
        format!(
            "SELECT {}, {} AS {}",
            select_items.join(", "),
            agg_text.trim(),
            col.name
        ),
        from_text.trim().to_string(),
    ];
    if !where_text.trim().is_empty() {
        subquery_parts.push(where_text.trim().to_string());
    }
    subquery_parts.push(group_by_text.trim().to_string());
    let subquery = subquery_parts.join(" ");

    let sql_type = inputs.added_column_types.get(&col.name).ok_or_else(|| {
        format!(
            "B5 (aggregate column add) refused for '{}': no declared SQL type in \
             BackbuildInputs::added_column_types",
            col.name
        )
    })?;

    let alter = emit::emit_alter_add_column(&inputs.table, &col.name, sql_type);
    let update = emit::emit_column_backfill_update_from_subquery(
        &inputs.table,
        &[(col.name.clone(), format!("s.{}", col.name))],
        &subquery,
        "s",
        &key_pairs,
    );

    Ok(BackbuildOption {
        technique: Technique::AggregateColumnBackfill,
        // Same H-slot bucket as B1/B3's `ALTER ADD` + paired `UPDATE`
        // (`build_b1_option`'s doc comment) — the `ALTER ADD` must flush
        // before any rename-dependent add, and the paired `UPDATE` only
        // ever touches this atom's own freshly-added column.
        slot: Some(HSlot::Alter),
        statements: vec![alter, update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: true,
        // The `ALTER ADD` step is DDL and not re-runnable (research §2
        // "Idempotence"), same as B1/B3.
        rerun_safe: false,
    })
}

// ===== B6: new window-function column over stored columns (research §4
// B6) =====

/// The B6 attempt gate: `expr` is itself a window-function call (`OVER`
/// present) — checked before [`try_b6`] is ever called, mirroring how
/// `is_group_by_aggregate_shape` gates B5's attempt so an ordinary
/// non-window added column never picks up a spurious "no OVER clause"
/// refusal alongside its own admitted B1/B3 options.
fn is_window_column_shape(expr: &Expr) -> bool {
    expr.as_function_call().is_some() && expr.window_spec().is_some()
}

/// Collect every column dependency of a window-function expression's own
/// pieces — the function's own arguments, and the `OVER` clause's
/// `PARTITION BY`/`ORDER BY` key expressions — preserving qualifiers
/// (`resolve_representative` needs qualifier + raw name, research §4 intro
/// "one uniform rule"). Unlike `model_diff::collect_dependencies`, this walk
/// does not fail closed on the window shape itself — proving *that* shape
/// safe is exactly B6's job — but every individual piece is validated
/// through [`collect_qualified_dependencies`] (registry determinism/
/// opaqueness leaf check, subquery/nested-window fail-close), so a volatile
/// or unrecognised function nested inside an argument or ordering
/// expression still refuses.
fn collect_window_dependencies(expr: &Expr) -> Result<Vec<(Option<String>, String)>, String> {
    let func = expr
        .as_function_call()
        .ok_or_else(|| "is not a window function call".to_string())?;
    let window = expr
        .window_spec()
        .ok_or_else(|| "has no OVER clause".to_string())?;

    let mut deps = Vec::new();
    for arg in func.arguments() {
        deps.extend(collect_qualified_dependencies(&arg)?);
    }
    if let Some(partition_by) = window.partition_by() {
        for e in partition_by.expressions() {
            deps.extend(collect_qualified_dependencies(&e)?);
        }
    }
    if let Some(order_by) = window.order_by() {
        for item in order_by.items() {
            if let Some(e) = item.expression() {
                deps.extend(collect_qualified_dependencies(&e)?);
            }
        }
    }
    Ok(deps)
}

/// Requalify a window-function expression's own pieces (the function's
/// arguments, `PARTITION BY`, `ORDER BY` key expressions) to their stored
/// 1:1 representative names, for splicing into B6's self-read `SELECT
/// <id...>, <window> AS c FROM t` projection (research §4 B6, §3 "Alias
/// requalification is a CST rewrite, not string surgery"). A window's
/// `OVER` clause sits outside `requalify.rs`'s recognised shapes — that
/// module never needed to rewrite one before B6 — and this phase's file
/// scope does not include `requalify.rs`, so this is a small, deliberately
/// local CST-fragment rewrite (mirroring `requalify::requalify`'s
/// positional-splice approach) scoped to exactly the pieces B6 needs.
fn requalify_window_expr(
    expr: &Expr,
    representative_sources: &[RepresentativeSource],
) -> Result<String, String> {
    let func = expr
        .as_function_call()
        .ok_or_else(|| "is not a window function call".to_string())?;
    let window = expr
        .window_spec()
        .ok_or_else(|| "has no OVER clause".to_string())?;

    let mut spans: Vec<(TextRange, String)> = Vec::new();
    for arg in func.arguments() {
        collect_window_replacements(&arg, representative_sources, &mut spans)?;
    }
    if let Some(partition_by) = window.partition_by() {
        for e in partition_by.expressions() {
            collect_window_replacements(&e, representative_sources, &mut spans)?;
        }
    }
    if let Some(order_by) = window.order_by() {
        for item in order_by.items() {
            if let Some(e) = item.expression() {
                collect_window_replacements(&e, representative_sources, &mut spans)?;
            }
        }
    }
    spans.sort_by_key(|(range, _)| range.start());
    Ok(splice_window_text(expr.syntax(), &spans))
}

/// Mirrors `requalify::collect_replacements`'s recognised shapes (column
/// reference, function call, binary, `CASE`, `CAST`) — duplicated locally
/// rather than imported, since `requalify.rs` is outside this phase's file
/// scope (see [`requalify_window_expr`]'s doc comment).
fn collect_window_replacements(
    expr: &Expr,
    representative_sources: &[RepresentativeSource],
    out: &mut Vec<(TextRange, String)>,
) -> Result<(), String> {
    if let Some(col) = expr.as_column_ref() {
        let rep = resolve_representative(
            col.qualifier(),
            col.name(),
            representative_sources,
            Some(expr.syntax()),
        )
        .ok_or_else(|| {
            format!(
                "column '{}' has no stored representative to requalify against",
                col.name()
            )
        })?;
        out.push((window_trimmed_range(expr.syntax()), rep.output_name.clone()));
        return Ok(());
    }
    if let Some(func) = expr.as_function_call() {
        for arg in func.arguments() {
            collect_window_replacements(&arg, representative_sources, out)?;
        }
        return Ok(());
    }
    if let Some(bin) = expr.as_binary() {
        for side in [bin.left(), bin.right()].into_iter().flatten() {
            collect_window_replacements(&side, representative_sources, out)?;
        }
        return Ok(());
    }
    if let Some(case) = expr.as_case() {
        if let Some(value) = case.case_value() {
            collect_window_replacements(&value, representative_sources, out)?;
        }
        for when in case.when_clauses() {
            for arm in [when.condition(), when.result()].into_iter().flatten() {
                collect_window_replacements(&arm, representative_sources, out)?;
            }
        }
        if let Some(else_expr) = case.else_expr() {
            collect_window_replacements(&else_expr, representative_sources, out)?;
        }
        return Ok(());
    }
    if let Some(cast) = expr.as_cast() {
        return match cast.expression() {
            Some(inner) => collect_window_replacements(&inner, representative_sources, out),
            None => Err("CAST has no inner expression to requalify".to_string()),
        };
    }
    let has_ident = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::IDENT);
    if has_ident {
        return Err(
            "expression shape is not recognised by the window requalification walk".to_string(),
        );
    }
    Ok(())
}

/// The range spanning `node`'s own first through last *non-trivia* token —
/// mirrors `requalify::trimmed_range` (duplicated locally per
/// [`requalify_window_expr`]'s doc comment).
fn window_trimmed_range(node: &SyntaxNode) -> TextRange {
    let mut first = None;
    let mut last = None;
    for tok in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
    {
        if first.is_none() {
            first = Some(tok.text_range().start());
        }
        last = Some(tok.text_range().end());
    }
    match (first, last) {
        (Some(start), Some(end)) => TextRange::new(start, end),
        _ => node.text_range(),
    }
}

/// Rebuild `node`'s own (trivia-trimmed) text, splicing in each recorded
/// `(range, replacement)` span — mirrors `requalify::splice` (duplicated
/// locally per [`requalify_window_expr`]'s doc comment).
fn splice_window_text(node: &SyntaxNode, spans: &[(TextRange, String)]) -> String {
    let node_start = u32::from(node.text_range().start());
    let full_text = node.text().to_string();
    let content = window_trimmed_range(node);
    let content_start = u32::from(content.start());
    let content_end = u32::from(content.end());

    let mut out = String::new();
    let mut cursor = content_start;
    for (range, replacement) in spans {
        let start = u32::from(range.start());
        let end = u32::from(range.end());
        if start < cursor {
            continue;
        }
        out.push_str(&full_text[(cursor - node_start) as usize..(start - node_start) as usize]);
        out.push_str(replacement);
        cursor = end;
    }
    out.push_str(&full_text[(cursor - node_start) as usize..(content_end - node_start) as usize]);
    out
}

/// B6 (research §4 B6): an added SELECT item that is itself a window-
/// function call (`is_window_column_shape` already proved the shape, as
/// this function's caller's gate). *Prove*: an explicit `ORDER BY` inside
/// the `OVER` clause (fail-closed refusal otherwise — a missing `ORDER BY`
/// leaves the window's draw within each partition underdetermined, and an
/// underdetermined draw can never be proven equal to a rebuild's own draw);
/// every dependency of the window's own arguments/`PARTITION BY`/`ORDER BY`
/// resolves to a stored 1:1 representative (the uniform representative
/// rule); a declared `BackbuildInputs::row_identity`, every column of which
/// is proven `NOT NULL` (key addressability — the backfill joins the
/// window's computed rows back to `t` by this identity). *Script*:
/// `ALTER TABLE t ADD COLUMN c <ty>;` then a self-read column backfill via
/// `emit::emit_column_backfill_update_from_subquery` whose source subquery
/// reads the deployed table `t` itself — `SELECT <id...>, <window> AS c
/// FROM t` — never an upstream: computing the window over the *stored* rows
/// is exactly what makes its draw match the rebuild's (research §2
/// "self-read scripts").
fn try_b6(
    col: &SelectColumn,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let window = col.expr.window_spec().ok_or_else(|| {
        format!(
            "B6 (window column add) refused for '{}': not a window function call",
            col.name
        )
    })?;

    // research §4 B6 + §2 "Determinism caveat": a window with no ORDER BY
    // has an underdetermined draw within each partition — a rank-family
    // function's result can differ run to run, so it can never be proven
    // equal to a rebuild's own draw. Named refusal, fail-closed.
    let has_order_by = window
        .order_by()
        .is_some_and(|o| o.items().next().is_some());
    if !has_order_by {
        return Err(format!(
            "B6 (window column add) refused for '{}': the window has no ORDER BY clause — an \
             underdetermined window draw can never be proven equal to a rebuild's draw",
            col.name
        ));
    }

    let deps = collect_window_dependencies(&col.expr).map_err(|reason| {
        format!(
            "B6 (window column add) refused for '{}': {reason}",
            col.name
        )
    })?;
    let mut missing: Vec<String> = deps
        .iter()
        .filter(|(q, n)| {
            resolve_representative(
                q.as_deref(),
                n,
                representative_sources,
                Some(col.expr.syntax()),
            )
            .is_none()
        })
        .map(|(q, n)| format_dependency(q, n))
        .collect();
    missing.sort();
    if let Some(dep) = missing.first() {
        return Err(format!(
            "B6 (window column add) refused for '{}': depends on '{dep}', which has no 1:1 \
             stored representative (a bare pull-through unchanged between both definitions, \
             matched by qualifier and raw column name) in the model's own output — a window can \
             only be backfilled from what the table already stores",
            col.name
        ));
    }

    // Key addressability (research §4 intro): the self-read backfill joins
    // the window's own computed rows back to `t` by row identity, so the
    // identity must be declared and every identity column proven NOT NULL.
    let identity = inputs
        .row_identity
        .as_ref()
        .filter(|cols| !cols.is_empty())
        .ok_or_else(|| {
            format!(
                "B6 (window column add) refused for '{}': no declared row_identity — a self-read \
                 window backfill needs an addressable identity to join the window's computed rows \
                 back to the stored table",
                col.name
            )
        })?;
    let mut nullable_identity: Vec<&String> = identity
        .iter()
        .filter(|c| !inputs.not_null_columns.contains(c.as_str()))
        .collect();
    nullable_identity.sort();
    if let Some(missing_not_null) = nullable_identity.first() {
        return Err(format!(
            "B6 (window column add) refused for '{}': row_identity column '{missing_not_null}' \
             is not declared NOT NULL — an equality match never addresses a NULL-keyed row",
            col.name
        ));
    }

    let requalified =
        requalify_window_expr(&col.expr, representative_sources).map_err(|reason| {
            format!(
                "B6 (window column add) refused for '{}': {reason}",
                col.name
            )
        })?;

    let sql_type = inputs.added_column_types.get(&col.name).ok_or_else(|| {
        format!(
            "B6 (window column add) refused for '{}': no declared SQL type in \
             BackbuildInputs::added_column_types",
            col.name
        )
    })?;

    // Self-read subquery: reads the deployed table `t` itself — the
    // proof-carrying construction that makes the window's draw match the
    // rebuild's (research §4 B6, §2's "self-read scripts" family):
    // `SELECT <id...>, <window> AS c FROM t`.
    let select_items: Vec<String> = identity
        .iter()
        .cloned()
        .chain(std::iter::once(format!("{requalified} AS {}", col.name)))
        .collect();
    let subquery = format!("SELECT {} FROM {}", select_items.join(", "), inputs.table);
    let key_pairs: Vec<(String, String)> =
        identity.iter().map(|c| (c.clone(), c.clone())).collect();

    let alter = emit::emit_alter_add_column(&inputs.table, &col.name, sql_type);
    let update = emit::emit_column_backfill_update_from_subquery(
        &inputs.table,
        &[(col.name.clone(), format!("s.{}", col.name))],
        &subquery,
        "s",
        &key_pairs,
    );

    Ok(BackbuildOption {
        technique: Technique::WindowColumnBackfill,
        // Same H-slot bucket as B1/B3/B5's `ALTER ADD` + paired `UPDATE`
        // (`build_b1_option`'s doc comment).
        slot: Some(HSlot::Alter),
        statements: vec![alter, update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: false,
        // The `ALTER ADD` step is DDL and not re-runnable (research §2
        // "Idempotence"), same as B1/B3/B5.
        rerun_safe: false,
    })
}

// ===== B4: new column via a newly-added LEFT JOIN — join-enrichment
// backfill with the row-set-preservation proof (research §4 B4) =====

/// A single added `LEFT JOIN`'s row-set-preservation proof, once admitted by
/// [`admit_added_left_join`]: the join's FROM-tree alias, its upstream's
/// physical table name, and the `(target output column, dimension raw
/// column)` key-equality pairs the emitted backfill's predicate is built
/// from — one per component of the dimension's declared `unique_key`.
struct JoinAdmission {
    alias: String,
    physical_name: String,
    key_pairs: Vec<(String, String)>,
}

/// The FROM-tree reference name an added join's own SQL uses to reach its
/// table — its alias when the join aliases it, otherwise its bare table
/// identifier (mirrors [`super::SourceRef`]'s doc comment: the same
/// per-FROM-tree-alias binding B3/D2's grain-link proof uses).
fn join_reference_name(join: &JoinClause) -> Option<String> {
    let table_ref = join.table_ref()?;
    table_ref.alias().or_else(|| table_ref.identifier())
}

/// Whether the model's `WHERE` clause references `alias` anywhere — the B4
/// "nothing else references it" sweep's WHERE leg (research §4 B4),
/// covering both shapes [`ConjunctDiff`] can take: a factored
/// [`ConjunctDiff::Diffed`] diff is swept over its `added` conjuncts only
/// (an `unchanged`/`removed` conjunct cannot reference a brand-new alias,
/// same structural argument as the other joins' conditions, since it is
/// byte-identical, modulo trivia, to a before-side conjunct that could not
/// have referenced an alias that did not exist yet); an unfactored
/// [`ConjunctDiff::Opaque`] diff (e.g. a top-level `OR`) has no `added` set
/// at all to check, so both retained raw sides (`before`/`after`) are swept
/// instead — a top-level `OR` is exactly the shape where "the conjunct-set
/// diff could not be factored" must never be misread as "nothing changed",
/// since `where_diff` returns `Opaque` independently of whether the alias
/// is actually referenced.
///
/// I1 fix (final-review finding): the sweep is qualifier-*and*-unqualified
/// aware — SQL permits an unqualified reference to resolve to any
/// unambiguous column in scope, including the newly-added join's own
/// column, so a sweep that only ever checked qualified references could miss
/// a real dim-side reference hiding behind a bare name. See
/// [`references_alias_or_unproven_bare`].
fn where_clause_references_alias(
    where_clause: &ConjunctDiff,
    alias: &str,
    representative_sources: &[RepresentativeSource],
) -> bool {
    match where_clause {
        ConjunctDiff::Diffed { added, .. } => added
            .iter()
            .any(|w| references_alias_or_unproven_bare(w, alias, representative_sources)),
        ConjunctDiff::Opaque { before, after, .. } => {
            before.as_ref().is_some_and(|e| {
                references_alias_or_unproven_bare(e, alias, representative_sources)
            }) || after.as_ref().is_some_and(|e| {
                references_alias_or_unproven_bare(e, alias, representative_sources)
            })
        }
    }
}

/// The row-set-preservation proof for a single added `LEFT JOIN` (research
/// §4 B4): LEFT (never removes a row) + at-most-one-match (the ON's
/// dim-side key columns exactly match the dimension's declared
/// `unique_key`) + the shared NOT NULL obligation on that key (research §4
/// intro "Key addressability") + the ON condition being a bare key equality
/// (no extra dim-side conjuncts, no non-equality comparison) + the
/// fact-side key columns having a stored bare representative (so the
/// emitted backfill can address `t` by them) + "nothing but the added
/// SELECT items reference the new alias" (a WHERE conjunct or a changed
/// column reading it means the join is not a pure enrichment — the row set
/// is no longer preserved by the join alone; checked for both a factored
/// [`ConjunctDiff::Diffed`] WHERE diff — via its `added` conjuncts — and an
/// unfactored [`ConjunctDiff::Opaque`] one — e.g. a top-level `OR`, where
/// "no added conjuncts" would otherwise be misread as "nothing changed", so
/// both retained raw sides are swept instead, see
/// [`where_clause_references_alias`]) — a pre-existing join's condition
/// referencing the new alias is impossible by construction, though:
/// `diff.rs`'s `join_equal` requires every non-added join's condition to be
/// byte-identical (modulo trivia) to its before-version, which could not
/// have referenced an alias that did not exist yet — so that leg needs no
/// separate check here.
fn admit_added_left_join(
    join: &JoinClause,
    changed: &[ChangedColumn],
    where_clause: &ConjunctDiff,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> Result<JoinAdmission, String> {
    // Defensive only: `diff.rs`'s `SkeletonDiff::AddedLeftJoins` is only
    // constructed for `LEFT JOIN`s (a non-LEFT added join routes to
    // `SkeletonDiff::Changed`'s G2 refusal before this function is ever
    // called) — never reachable in practice.
    if join.join_type() != Some(JoinType::Left) {
        return Err("B4 requires a LEFT JOIN — an INNER/other join type can drop rows".to_string());
    }

    let alias = join_reference_name(join).ok_or_else(|| {
        "the added join's table reference has no resolvable alias or identifier".to_string()
    })?;

    if where_clause_references_alias(where_clause, &alias, representative_sources) {
        return Err(format!(
            "the WHERE clause now references the added join's alias '{alias}' (directly, or via \
             an unqualified reference that cannot be proven fact-side) — the join is not a pure \
             enrichment (the row set is no longer preserved by the join alone)"
        ));
    }
    if let Some(c) = changed
        .iter()
        .find(|c| references_alias_or_unproven_bare(&c.after, &alias, representative_sources))
    {
        return Err(format!(
            "changed column '{}' now references the added join's alias '{alias}' (directly, or \
             via an unqualified reference that cannot be proven fact-side) — B4 only admits a \
             join that feeds solely added columns",
            c.name
        ));
    }

    let source = inputs.sources.get(&alias).ok_or_else(|| {
        format!(
            "no source declared for the added join's alias '{alias}' in \
             BackbuildInputs::sources"
        )
    })?;

    let condition = join
        .condition()
        .ok_or_else(|| "the added join has no ON/USING condition".to_string())?;
    if !condition.is_on() {
        return Err(
            "the added join's condition is not a bare ON key equality (USING is not yet \
             supported by B4)"
                .to_string(),
        );
    }
    let on_expr = condition
        .on_expression()
        .ok_or_else(|| "the added join's ON condition has no expression".to_string())?;

    let mut conjuncts = Vec::new();
    crate::analysis::expr_util::split_top_level_conjuncts(&on_expr, &mut conjuncts);

    let mut dim_cols = BTreeSet::new();
    let mut key_pairs_raw: Vec<(Option<String>, String, String)> = Vec::new();
    for conjunct in &conjuncts {
        let bin = conjunct
            .as_binary()
            .filter(|b| b.operator().as_deref() == Some("="));
        let Some(bin) = bin else {
            return Err(bare_key_equality_refusal());
        };
        let l = bin.left().and_then(|e| e.as_column_ref());
        let r = bin.right().and_then(|e| e.as_column_ref());
        let (Some(l), Some(r)) = (l, r) else {
            return Err(bare_key_equality_refusal());
        };
        let l_is_dim = l.qualifier() == Some(alias.as_str());
        let r_is_dim = r.qualifier() == Some(alias.as_str());
        let (dim_col, fact_qualifier, fact_col) = match (l_is_dim, r_is_dim) {
            (true, false) => (
                l.name().to_string(),
                r.qualifier().map(|q| q.to_string()),
                r.name().to_string(),
            ),
            (false, true) => (
                r.name().to_string(),
                l.qualifier().map(|q| q.to_string()),
                l.name().to_string(),
            ),
            _ => return Err(bare_key_equality_refusal()),
        };
        dim_cols.insert(dim_col.clone());
        key_pairs_raw.push((fact_qualifier, fact_col, dim_col));
    }

    // substrate-unification Phase 5: the at-most-one-match proof is F6's own
    // fan-out/cardinality proof (`analysis::join_shape::fan_out` +
    // `analysis::functional_dependency::functional_dependency_verdict`), not
    // a parallel `dim_cols != unique_key_set` re-implementation — the
    // declared `unique_key` below is a plain-data fact fed into that shared
    // verdict, the same posture `BackbuildInputs::sources[...].unique_key`
    // already has elsewhere (a schema fact `smelt-logical` has no catalog
    // access to derive itself). Consulting the SQL-walked
    // `functional_dependency_verdict_over_vector` route instead is not
    // available here: the dimension is an *external* source declared only
    // via `SourceRef`, with no SQL of its own in this module to walk.
    //
    // The whole declared `unique_key` must be registered as *one* composite
    // key-set (`with_composite_unique_key`, the same pattern
    // `maintenance/derive.rs`'s `join_context_for_edges`/
    // `join_context_for_sources` already use) — never one
    // `with_unique_key` call per column, which would register each column
    // as an *independently* sufficient single-column key and let `fan_out`
    // prove `OneToOne` off a join keyed on only *part* of a composite key
    // (`fd_verdict_shared_composite_key` is the killing test for this).
    // Once registered as one composite key-set, `fan_out`'s superset match
    // on the ON's equality columns against that key-set is equivalent to
    // the original `dim_cols != unique_key_set` exact-set check under the
    // bare-key-equality restriction above (which already forbids any ON
    // conjunct beyond the key, so by this point `dim_cols` is exactly the
    // ON's key columns) — see `fd_verdict_shared` for the equivalence this
    // delegation is proven not to change.
    let unique_key = source
        .unique_key
        .as_ref()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            format!(
                "upstream '{alias}' has no declared unique_key — the join's at-most-one-match \
                 proof needs an addressable identity"
            )
        })?;
    let unique_key_refs: Vec<&str> = unique_key.iter().map(String::as_str).collect();
    let join_ctx = crate::analysis::join_shape::JoinContext::new()
        .with_composite_unique_key(&alias, &unique_key_refs);
    let cardinality = crate::analysis::join_shape::fan_out(join, &join_ctx);
    if !crate::analysis::functional_dependency::functional_dependency_verdict(
        Some(cardinality),
        false,
    )
    .is_constant()
    {
        return Err(format!(
            "the added join's ON key columns ({dim_cols:?}) do not exactly match upstream \
             '{alias}''s declared unique_key ({unique_key:?}) — at-most-one-match is not proven"
        ));
    }

    for k in &dim_cols {
        if !source.not_null_columns.contains(k) {
            return Err(format!(
                "upstream '{alias}''s join key column '{k}' is not declared NOT NULL \
                 (BackbuildInputs::sources[...].not_null_columns) — SQL UNIQUE admits NULLs, and \
                 an equality join never matches a NULL-keyed row"
            ));
        }
    }

    let mut key_pairs = Vec::with_capacity(key_pairs_raw.len());
    for (fact_qualifier, fact_col, dim_col) in key_pairs_raw {
        let rep = representative_sources
            .iter()
            .find(|r| r.qualifier == fact_qualifier && r.raw_name == fact_col);
        let Some(rep) = rep else {
            return Err(format!(
                "the added join's key column '{fact_col}' has no stored bare representative in \
                 the model's own output — a join keyed on a column the model does not store is \
                 unaddressable; add it to the SELECT list to make this backfillable"
            ));
        };
        key_pairs.push((rep.output_name.clone(), dim_col));
    }

    Ok(JoinAdmission {
        alias,
        physical_name: source.physical_name.clone(),
        key_pairs,
    })
}

fn bare_key_equality_refusal() -> String {
    "the added join's ON condition contains something beyond a bare key equality (extra \
     dim-side conjuncts or a non-equality comparison are not reproduced)"
        .to_string()
}

/// Whether `expr`'s subtree contains any column reference that could
/// plausibly resolve to the newly-added join's alias `alias` — either
/// directly qualified with it, or unqualified and not provably resolvable to
/// the *fact* side instead (I1 fix, final-review finding: SQL permits an
/// unqualified reference to resolve to any unambiguous column in scope,
/// including a newly-added join's own column, so a sweep that only ever
/// checked qualified references could miss a real dim-side reference).
///
/// An unqualified reference is only provably fact-side when it resolves to a
/// *name-preserving* stored representative
/// ([`resolve_representative`]'s bare-dependency rule) — same posture as the
/// C2 derivability fix: fail closed (treat as possibly dim-side) rather than
/// assume. This is a permissive existence probe over one already-bounded
/// node (a `WHERE` conjunct or a changed column's expression), unlike the
/// fail-closed `collect_qualified_dependencies`/`model_diff::collect_dependencies`
/// walks elsewhere in this file: an unrecognised sub-shape is simply walked
/// through rather than refused outright — the *conclusion* it feeds
/// (`admit_added_left_join`'s "nothing else references it" sweep) is what
/// fails closed, by treating "cannot prove otherwise" as "references it".
/// Mirrors the `collect_column_refs`/`_rec` shape duplicated across this
/// crate's analysis modules (e.g. `analysis/fingerprint.rs`).
fn references_alias_or_unproven_bare(
    expr: &Expr,
    alias: &str,
    representative_sources: &[RepresentativeSource],
) -> bool {
    references_alias_or_unproven_bare_node(expr.syntax(), alias, representative_sources)
}

/// Every bare column name `expr` references anywhere in its subtree — the
/// E3 disjoint-column middle path's column-set extractor (research §4 E3),
/// used only to prove that an added and a removed `WHERE` conjunct could
/// not possibly address the same stored column. Deliberately more
/// permissive than `analysis::model_diff::collect_dependencies` (B1/D1's
/// derivability tool), same posture as [`references_alias_node`] just
/// above: no subquery/window fail-close and no determinism/registry check.
/// Those checks answer "can this expression be safely *re-derived* from
/// stored columns" — B1/D1's question, since a self-derived `UPDATE`
/// literally re-evaluates the expression — which is simply the wrong
/// question for E1/E2: a `WHERE` conjunct's `DELETE`/difference `INSERT`
/// evaluates the predicate once, directly, against real data (their own
/// validator, `requalify::requalify`, never checks determinism either), so
/// gating disjointness on `collect_dependencies` would wrongly refuse an
/// otherwise-admissible conjunct like `created_at < now() - INTERVAL '30
/// days'` purely because it *couldn't prove disjointness*, not because
/// either conjunct was actually inadmissible. A qualified reference's bare
/// name is collected the same as an unqualified one — disjointness only
/// needs "could these two possibly address the same stored column", and a
/// qualifier mismatch only makes two conjuncts *more* obviously unrelated,
/// never less; over-collecting (e.g. reaching into a subquery's own,
/// unrelated column references) only ever makes this proof *more*
/// conservative (a spurious shared name blocks composition, fail-closed),
/// never less sound.
fn conjunct_column_names(expr: &Expr) -> HashSet<String> {
    crate::analysis::expr_util::collect_column_names(expr)
}

fn references_alias_or_unproven_bare_node(
    node: &SyntaxNode,
    alias: &str,
    representative_sources: &[RepresentativeSource],
) -> bool {
    if node.kind() == SyntaxKind::EXPRESSION {
        if let Some(col) = Expr::cast(node.clone()).and_then(|e| e.as_column_ref()) {
            return match col.qualifier() {
                Some(q) => q == alias,
                // The lexer has no dedicated `TRUE`/`FALSE` keyword token —
                // both lex as plain `IDENT`, so a bare boolean literal is
                // otherwise indistinguishable from an unqualified column
                // reference at this AST layer (`ColumnRef::from_expr`).
                // Treat them as the literals they are, not a candidate
                // dim-side reference, so `WHERE d.active = TRUE` doesn't
                // spuriously fail-close every *other* added join too.
                None if col.name().eq_ignore_ascii_case("TRUE")
                    || col.name().eq_ignore_ascii_case("FALSE") =>
                {
                    false
                }
                None => {
                    resolve_representative(None, col.name(), representative_sources, Some(node))
                        .is_none()
                }
            };
        }
    }
    node.children()
        .any(|child| references_alias_or_unproven_bare_node(&child, alias, representative_sources))
}

/// Whether every dependency of `expr` is qualified with `alias` — the B4
/// detection gate (research §4 B4: "added SELECT items reference the new
/// alias"). `false` for an expression with no dependencies at all, or a
/// mix of alias-qualified and other dependencies (mixed fact/dimension
/// dependencies are not yet supported — such a column falls through to the
/// ordinary B1/B3 attempt, which will refuse on its own, unrelated,
/// merits).
fn depends_only_on_join_alias(expr: &Expr, alias: &str) -> bool {
    match collect_qualified_dependencies(expr) {
        Ok(deps) if !deps.is_empty() => deps.iter().all(|(q, _)| q.as_deref() == Some(alias)),
        _ => false,
    }
}

/// B4 (research §4 B4): an added SELECT item whose dependencies are wholly
/// qualified by a newly-added `LEFT JOIN`'s alias, admitted once
/// [`admit_added_left_join`]'s row-set-preservation proof holds for the
/// join. Shape enumeration is expression-driven (research §2 "Options, not
/// choices"): a bare column pull (`d.x`) admits both the `UPDATE ... FROM`
/// shape and the per-reference substituted scalar-subquery shape; any other
/// expression admits only the scalar-subquery shape — the NULL-extension
/// case (`COALESCE(d.x, 'none')`) is *wrong* under a whole-expression
/// subquery (a zero-row match nulls the entire expression before the
/// surrounding function ever runs), which is exactly why each dim-column
/// *reference* is individually substituted rather than the whole expression
/// (research §4 B4). See [`Technique::JoinEnrichmentUpdateFrom`] and
/// [`Technique::JoinEnrichmentScalarSubquery`]'s doc comments for the
/// safety-asymmetry metadata research §2 asks to be recorded per option.
fn try_b4(
    col: &SelectColumn,
    admission: &JoinAdmission,
    inputs: &BackbuildInputs,
) -> Result<Vec<BackbuildOption>, String> {
    let deps = collect_qualified_dependencies(&col.expr).map_err(|reason| {
        format!(
            "B4 (join-enrichment backfill) refused for '{}': {reason}",
            col.name
        )
    })?;
    if deps.is_empty()
        || !deps
            .iter()
            .all(|(q, _)| q.as_deref() == Some(admission.alias.as_str()))
    {
        return Err(format!(
            "B4 (join-enrichment backfill) refused for '{}': every dependency must be qualified \
             with the added join's alias '{}' — a mix of fact- and dimension-side dependencies \
             is not yet supported",
            col.name, admission.alias
        ));
    }

    let sql_type = inputs.added_column_types.get(&col.name).ok_or_else(|| {
        format!(
            "B4 (join-enrichment backfill) refused for '{}': no declared SQL type in \
             BackbuildInputs::added_column_types",
            col.name
        )
    })?;
    let alter = emit::emit_alter_add_column(&inputs.table, &col.name, sql_type);

    let mut options = Vec::new();

    // Bare column pull (`d.x`) additionally admits the `UPDATE ... FROM`
    // shape (research §4 B4).
    if let Some(bare) = col.expr.as_column_ref() {
        let update_from = emit::emit_column_backfill_update_from(
            &inputs.table,
            &[(
                col.name.clone(),
                format!("{}.{}", admission.alias, bare.name()),
            )],
            &admission.physical_name,
            &admission.alias,
            &admission.key_pairs,
        );
        options.push(BackbuildOption {
            technique: Technique::JoinEnrichmentUpdateFrom,
            slot: Some(HSlot::Alter),
            statements: vec![alter.clone(), update_from],
            write_scope: WriteScope::ColumnScoped,
            reads_upstream: true,
            // The `ALTER ADD` step is DDL and not re-runnable (research §2
            // "Idempotence"), same as B1/B3.
            rerun_safe: false,
        });
    }

    // The per-reference substituted scalar-subquery shape — the only option
    // for a general expression, and always offered alongside the bare-pull
    // `UPDATE ... FROM` shape above.
    let subquery_for = |dim_col: &str| {
        emit::emit_scalar_subquery_fragment(
            &inputs.table,
            dim_col,
            &admission.physical_name,
            &admission.alias,
            &admission.key_pairs,
        )
    };
    let scalar_expr =
        requalify::requalify_scalar_subquery(&col.expr, &admission.alias, &subquery_for).map_err(
            |reason| {
                format!(
                    "B4 (join-enrichment backfill) refused for '{}': {reason}",
                    col.name
                )
            },
        )?;
    let scalar_update =
        emit::emit_in_place_update(&inputs.table, &[(col.name.clone(), scalar_expr)]);
    options.push(BackbuildOption {
        technique: Technique::JoinEnrichmentScalarSubquery,
        slot: Some(HSlot::Alter),
        statements: vec![alter, scalar_update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: true,
        rerun_safe: false,
    });

    Ok(options)
}

/// A single, named refusal for the whole added-`LEFT JOIN` skeleton change
/// (research §4 B4/B7): used both when [`admit_added_left_join`]'s proof
/// fails outright (for the single-join B4 path, or for any one join in a
/// B7 chain — `reason` is prefixed with the failing join's own alias by
/// [`classify_b7_diff`] so the refusal names which join, per research §2
/// "Refusal posture"), when [`derive_join_order`] cannot resolve a
/// dependency order, and when the select list this phase cannot factor
/// (`Opaque`) arrives alongside an added join. Mirrors [`skeleton_refusal`]'s
/// shape — the shared B/D-class row-set-unchanged precondition (research §4
/// intro) fails for the *whole* diff once a join cannot be licensed, so (as
/// with any other skeleton refusal) no per-column classification is
/// attempted.
fn added_left_join_refusal(reason: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Skeleton {
            reason: reason.to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "skeleton".to_string(),
            reason: format!("B4 (join-enrichment backfill) refused: {reason}"),
        }],
    }
}

/// Dispatch a diff whose skeleton is [`SkeletonDiff::AddedLeftJoins`]:
/// exactly one added `LEFT JOIN` is B4 ([`classify_single_added_left_join`]);
/// two or more is B7 ([`classify_b7_diff`], research §4 B7) — each join
/// re-runs the same B4 admission proof, in reference-dependency order.
fn classify_added_left_join_diff(
    joins: &[JoinClause],
    comparable: &ComparableDiff,
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    if joins.len() == 1 {
        classify_single_added_left_join(&joins[0], comparable, inputs)
    } else {
        classify_b7_diff(joins, comparable, inputs)
    }
}

/// Classify a diff with exactly one added `LEFT JOIN` (research §4 B4):
/// admitted via [`admit_added_left_join`] once for the whole join, then
/// classified per atom — added SELECT items depending solely on the new
/// alias go through [`try_b4`] (via [`classify_added_column`]'s
/// `join_admission` parameter); everything else (rename pairing, ordinary
/// B1/B3 adds, D1/D2 changed columns, the WHERE/set-operation catch-alls)
/// runs through the same machinery [`classify_comparable`] uses for an
/// unchanged skeleton. An admission failure for the join refuses the *whole*
/// diff (no atom list), since the shared row-set-unchanged precondition
/// (research §4 intro) then fails for every atom, not just the join's own
/// added columns.
fn classify_single_added_left_join(
    join: &JoinClause,
    comparable: &ComparableDiff,
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    let (added, dropped, changed, unchanged) = match &comparable.select_list {
        SelectListDiff::Opaque { reason } => {
            return vec![unclassified_refusal_named(&format!(
                "select-list: {reason}"
            ))];
        }
        SelectListDiff::Diffed {
            added,
            dropped,
            changed,
            unchanged,
            ..
        } => (added, dropped, changed, unchanged),
    };

    let representative_sources = representative_sources(unchanged);

    let admission = match admit_added_left_join(
        join,
        changed,
        &comparable.where_clause,
        &representative_sources,
        inputs,
    ) {
        Ok(admission) => admission,
        Err(reason) => return vec![added_left_join_refusal(&reason)],
    };

    let mut atoms = Vec::new();

    let RenamePairing {
        atoms: mut rename_atoms,
        consumed_added,
    } = pair_renames(dropped, added, inputs);
    atoms.append(&mut rename_atoms);

    for col in added {
        if consumed_added.contains(&col.name) {
            continue;
        }
        atoms.push(classify_added_column(
            col,
            &representative_sources,
            Some(&admission),
            inputs,
        ));
    }

    let changed_names: BTreeSet<String> = changed.iter().map(|c| c.name.clone()).collect();
    for c in changed {
        atoms.push(classify_changed_column(
            c,
            &representative_sources,
            &changed_names,
            inputs,
        ));
    }

    let after_columns = after_column_names(unchanged, changed, added);
    atoms.extend(classify_where_clause(
        &comparable.where_clause,
        &representative_sources,
        Some(&after_columns),
        inputs,
    ));

    atoms.extend(classify_set_ops(
        &comparable.set_ops,
        Some(&after_columns),
        None,
        inputs,
    ));

    atoms
}

/// Derive B7's reference-dependency order over `joins` (research §4 B7; "H.
/// Composites"' one data-dependent within-slot ordering): join `i` must run
/// after join `j` whenever `i`'s `ON` condition references `j`'s own alias
/// anywhere (own-alias references, i.e. the dim-side of `i`'s own key
/// equality, are excluded — that is not a dependency on another join). This
/// is deliberately a syntactic, best-effort graph over *every* qualifier `i`
/// references, not just ones [`admit_added_left_join`] will ultimately prove
/// addressable — an edge to a join whose column turns out unstored (or
/// non-bare) still orders correctly, and [`admit_added_left_join`]'s own
/// proof (run afterward, per join, in this order) is what produces the
/// actual, actionable refusal for that case (research §4 B7; §7 open
/// question 7 "Multi-hop enrichment beyond B7" — the boundary this refusal
/// sits at).
///
/// A topological sort (Kahn's algorithm, always picking the lowest-index
/// ready join for determinism) over that graph; independent joins (no
/// reference relationship at all — research's `b7_independent_joins_either_
/// order`) end up in a deterministic, but not declaration-order-preserving,
/// relative order — since neither depends on the other, either order is
/// correct. A cycle (or, equivalently, any join whose dependency chain
/// cannot be fully resolved) refuses fail-closed: `Err` names the aliases
/// involved rather than guessing an order.
fn derive_join_order(joins: &[JoinClause], aliases: &[String]) -> Result<Vec<usize>, String> {
    let alias_index: BTreeMap<&str, usize> = aliases
        .iter()
        .enumerate()
        .map(|(i, a)| (a.as_str(), i))
        .collect();

    let mut predecessors: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); joins.len()];
    for (i, join) in joins.iter().enumerate() {
        let Some(on_expr) = join.condition().and_then(|c| c.on_expression()) else {
            // No resolvable ON expression at all — nothing to derive a
            // dependency edge from; `admit_added_left_join` will refuse this
            // join on its own merits ("no ON/USING condition"/"not a bare
            // ON key equality") once processing reaches it.
            continue;
        };
        // Every FROM-tree alias/qualifier referenced anywhere in the ON
        // condition's subtree — a permissive existence probe, same posture
        // as [`references_alias_node`] (which only tests membership; this
        // collects the whole set), used solely to build this reference-
        // dependency graph. Shape validation of the ON condition itself
        // stays `admit_added_left_join`'s job — this is graph-building
        // only, never a derivability proof, so an unrecognised sub-shape is
        // simply walked through rather than refused.
        let referenced = crate::analysis::expr_util::collect_referenced_qualifiers(&on_expr);
        for q in referenced {
            if q == aliases[i] {
                continue;
            }
            if let Some(&j) = alias_index.get(q.as_str()) {
                predecessors[i].insert(j);
            }
        }
    }

    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); joins.len()];
    let mut in_degree: Vec<usize> = vec![0; joins.len()];
    for (i, preds) in predecessors.iter().enumerate() {
        in_degree[i] = preds.len();
        for &p in preds {
            successors[p].push(i);
        }
    }

    let mut ready: BTreeSet<usize> = (0..joins.len()).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(joins.len());
    while let Some(&next) = ready.iter().next() {
        ready.remove(&next);
        order.push(next);
        for &succ in &successors[next] {
            in_degree[succ] -= 1;
            if in_degree[succ] == 0 {
                ready.insert(succ);
            }
        }
    }

    if order.len() != joins.len() {
        return Err(format!(
            "the added joins' ON conditions form a reference cycle (or another unresolvable \
             dependency order) among aliases {aliases:?} — B7 requires a strict dependency order \
             between sequential join steps"
        ));
    }

    Ok(order)
}

/// Classify a diff with two or more added `LEFT JOIN`s (research §4 B7,
/// "sequential multi-join enrichment"). Each join runs through the
/// *unmodified* B4 admission proof ([`admit_added_left_join`]) — LEFT +
/// at-most-one-match + the shared NOT NULL obligation + bare key equality +
/// "nothing but the added SELECT items reference the new alias" — in
/// [`derive_join_order`]'s reference-dependency order, with one extension:
/// the derivability environment (`representative_sources`) grows, one join
/// at a time, with the *earlier* steps' own bare pull-through added columns
/// — so a later join's `ON` condition may key on a column an earlier step
/// backfills (research §4 B7's "stored-by-then" extension). Bareness is
/// enforced purely by construction: only a bare column-ref added SELECT item
/// is ever turned into a representative here (mirroring
/// [`representative_sources`]'s own `unchanged`-side filter) — a wrapped
/// carrier (`COALESCE(d1.c, 0) AS c`) never becomes one, so a later join
/// keyed on it fails [`admit_added_left_join`]'s existing "no stored bare
/// representative" check with its existing, already-actionable message
/// (research §4 B7 bareness: a wrapped carrier stores a sentinel where the
/// rebuild has NULL, so a later join on it could hit a dim row the
/// rebuild's NULL key misses).
///
/// A cyclic/unresolvable order, or any one join's own admission failure,
/// refuses the *whole* diff (mirrors [`classify_single_added_left_join`]'s
/// posture) — the shared row-set-unchanged precondition (research §4 intro)
/// then fails for every atom, not just the failing join's own columns. A
/// per-join admission failure's refusal is prefixed with that join's own
/// alias, so it names which join (research §2 "Refusal posture").
///
/// Atoms for each join's own added columns are pushed onto the atom list in
/// that same dependency order — never SELECT-list declaration order.
/// `assemble` (`mod.rs`) buckets `HSlot::Alter` options by a pure pass over
/// `BackbuildOptions::atoms` in order (research §4 "H. Composites": "the
/// update slot is order-free by construction ... except B7, whose steps run
/// in their derived dependency order"), so pushing atoms in dependency
/// order here is what makes the assembled script's within-`HSlot::Alter`
/// statement order follow it — no change to `assemble`'s bucketing logic
/// itself is needed for this.
fn classify_b7_diff(
    joins: &[JoinClause],
    comparable: &ComparableDiff,
    inputs: &BackbuildInputs,
) -> Vec<AtomAnalysis> {
    let (added, dropped, changed, unchanged) = match &comparable.select_list {
        SelectListDiff::Opaque { reason } => {
            return vec![unclassified_refusal_named(&format!(
                "select-list: {reason}"
            ))];
        }
        SelectListDiff::Diffed {
            added,
            dropped,
            changed,
            unchanged,
            ..
        } => (added, dropped, changed, unchanged),
    };

    let mut aliases = Vec::with_capacity(joins.len());
    for join in joins {
        match join_reference_name(join) {
            Some(alias) => aliases.push(alias),
            None => {
                return vec![added_left_join_refusal(
                    "one of the added joins' table references has no resolvable alias or \
                     identifier",
                )];
            }
        }
    }

    let order = match derive_join_order(joins, &aliases) {
        Ok(order) => order,
        Err(reason) => return vec![added_left_join_refusal(&reason)],
    };

    let mut representative_sources = representative_sources(unchanged);

    let mut admissions: BTreeMap<String, JoinAdmission> = BTreeMap::new();
    for &idx in &order {
        let alias = &aliases[idx];
        let admission = match admit_added_left_join(
            &joins[idx],
            changed,
            &comparable.where_clause,
            &representative_sources,
            inputs,
        ) {
            Ok(admission) => admission,
            Err(reason) => {
                return vec![added_left_join_refusal(&format!(
                    "join '{alias}': {reason}"
                ))];
            }
        };

        // The stored-by-then extension (research §4 B7): a *bare* added
        // pull-through of this join's own alias becomes a representative a
        // later join's key may address. Never a wrapped one — see this
        // function's doc comment.
        for col in added {
            if let Some(bare) = col.expr.as_column_ref() {
                if bare.qualifier() == Some(alias.as_str()) {
                    representative_sources.push(RepresentativeSource {
                        output_name: col.name.clone(),
                        qualifier: Some(alias.clone()),
                        raw_name: bare.name().to_string(),
                        // No SELECT scope of its own to chase a leaf
                        // through — synthesized from a B4/B7-added column
                        // reading the newly-added join's alias, not from an
                        // `unchanged` item in either definition's own
                        // parsed CST.
                        leaf: None,
                    });
                }
            }
        }

        admissions.insert(alias.clone(), admission);
    }

    let mut atoms = Vec::new();

    let RenamePairing {
        atoms: mut rename_atoms,
        consumed_added,
    } = pair_renames(dropped, added, inputs);
    atoms.append(&mut rename_atoms);

    let mut handled: HashSet<String> = HashSet::new();
    for &idx in &order {
        let alias = &aliases[idx];
        let admission = &admissions[alias];
        for col in added {
            if consumed_added.contains(&col.name) || handled.contains(&col.name) {
                continue;
            }
            if depends_only_on_join_alias(&col.expr, alias) {
                handled.insert(col.name.clone());
                atoms.push(classify_added_column(
                    col,
                    &representative_sources,
                    Some(admission),
                    inputs,
                ));
            }
        }
    }

    // Every remaining added column depends on no added join's alias at all
    // — ordinary B1/B3 (a column depending on a mix of two aliases refuses
    // on its own merits inside `classify_added_column`/`try_b4`, exactly as
    // the single-join B4 path already does).
    for col in added {
        if consumed_added.contains(&col.name) || handled.contains(&col.name) {
            continue;
        }
        atoms.push(classify_added_column(
            col,
            &representative_sources,
            None,
            inputs,
        ));
    }

    let changed_names: BTreeSet<String> = changed.iter().map(|c| c.name.clone()).collect();
    for c in changed {
        atoms.push(classify_changed_column(
            c,
            &representative_sources,
            &changed_names,
            inputs,
        ));
    }

    let after_columns = after_column_names(unchanged, changed, added);
    atoms.extend(classify_where_clause(
        &comparable.where_clause,
        &representative_sources,
        Some(&after_columns),
        inputs,
    ));

    atoms.extend(classify_set_ops(
        &comparable.set_ops,
        Some(&after_columns),
        None,
        inputs,
    ));

    atoms
}

// ===== D1: changed existing-column expression, derivable from stored
// columns (research §4 D1) =====

/// Classify a `changed` column into its D1 and/or D2 options (research §4
/// D1, D2). The shared D-class grain guard (`SELECT DISTINCT`/`LIMIT`,
/// research §4 intro) is checked once, up front, for both techniques — a
/// grain-guard refusal is never technique-specific. Otherwise D1 and D2 are
/// attempted independently: D2 is *always* attempted alongside a successful
/// D1 (research: "a changed expression derivable both from stored columns
/// and from upstream yields both … options" — the dual-derivable case), and
/// is attempted after a failed D1 only when [`d1_failure_licenses_d2_attempt`]
/// says the failure was a genuine "no representative, might be upstream"
/// shape — never for an expression-validity failure (D2 shares the same
/// leaf check and would refuse identically) or a changed-sibling dependency
/// (interfaces note: "changed-sibling refuses standalone" — a same-model
/// reference has nothing to do with an upstream read). Whenever any
/// technique succeeds, `inadmissible` stays empty (the atom has an
/// admissible option; a sibling technique's own failure reason is not
/// worth reporting) — the same invariant every other atom kind upholds.
fn classify_changed_column(
    changed: &ChangedColumn,
    representative_sources: &[RepresentativeSource],
    changed_names: &BTreeSet<String>,
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::ChangedColumn {
        name: changed.name.clone(),
    };

    if let Some(reason) = grain_guard_refusal(&changed.after) {
        return AtomAnalysis {
            change: atom_change,
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("changed column '{}'", changed.name),
                reason: format!("D-class refused for '{}': {reason}", changed.name),
            }],
        };
    }

    let d1_result = try_d1(changed, representative_sources, changed_names, inputs);
    let attempt_d2 = match &d1_result {
        Ok(_) => true,
        Err(_) => d1_failure_licenses_d2_attempt(changed, representative_sources, changed_names),
    };
    let d2_result = if attempt_d2 {
        Some(try_d2(changed, representative_sources, inputs))
    } else {
        None
    };

    let mut options = Vec::new();
    let mut inadmissible = Vec::new();
    match d1_result {
        Ok(opt) => options.push(opt),
        Err(reason) => inadmissible.push(BackbuildRefusal {
            atom: format!("changed column '{}'", changed.name),
            reason,
        }),
    }
    if let Some(d2_result) = d2_result {
        match d2_result {
            Ok(opt) => options.push(opt),
            Err(reason) => inadmissible.push(BackbuildRefusal {
                atom: format!("changed column '{}'", changed.name),
                reason,
            }),
        }
    }
    if !options.is_empty() {
        // At least one technique succeeded — an admissible atom never also
        // carries stale refusal text for a sibling technique that wasn't
        // chosen (matches every other atom kind in this module).
        inadmissible.clear();
    }

    AtomAnalysis {
        change: atom_change,
        options,
        inadmissible,
    }
}

/// Whether a failed D1 licenses attempting D2 (see `classify_changed_column`
/// doc comment). Re-derives D1's own "first missing dependency" (same
/// qualifier-aware match order `try_d1` uses, C2 fix) rather than threading a
/// richer error type back from `try_d1` — `try_d1` already fully validated
/// the expression by the time this runs, so `collect_qualified_dependencies`
/// here is guaranteed `Ok` in every reachable case except the one it exists
/// to detect (an expression-validity failure, which also means "don't
/// attempt D2").
fn d1_failure_licenses_d2_attempt(
    changed: &ChangedColumn,
    representative_sources: &[RepresentativeSource],
    changed_names: &BTreeSet<String>,
) -> bool {
    let Ok(deps) = collect_qualified_dependencies(&changed.after) else {
        return false;
    };
    let mut missing: Vec<(Option<String>, String)> = deps
        .into_iter()
        .filter(|(q, n)| {
            resolve_representative(
                q.as_deref(),
                n,
                representative_sources,
                Some(changed.after.syntax()),
            )
            .is_none()
        })
        .collect();
    missing.sort();
    match missing.first() {
        Some((_, dep)) => !changed_names.contains(dep.as_str()),
        // No missing dependency at all means D1 should have succeeded —
        // unreachable from the `Err` branch that calls this, but false is
        // the safe (skip D2) answer either way.
        None => false,
    }
}

/// D1 admission (research §4 D1): every dependency of the column's *new*
/// expression must resolve to a stored 1:1 representative under exactly the
/// same uniform rule B1 uses (research §4 intro), matched by qualifier and
/// raw column name via [`resolve_representative`] (C2 fix) — representatives
/// are drawn only from `unchanged` output columns, so the changed column
/// itself and any changed sibling are never representatives by construction.
/// This is what makes self-substitution (a new expression reading the
/// column's own old value) and a mutual swap (`x AS a, y AS b` → `y AS a, x
/// AS b`) both refuse: neither `a` nor `b` is ever a representative, since
/// both are in `changed`, not `unchanged`.
///
/// The new expression is defined over *inputs*, not over the old column
/// value — this proof finds stored representatives of those inputs, it
/// never substitutes the old expression or the old column's own value.
///
/// `changed_names` (every output column name in this diff's `changed` set,
/// including `changed.name` itself) is used only to give a missing
/// dependency's refusal the *right* reason: a dependency that also changed
/// in this same edit is refused because a changed column is never a stored
/// representative (research §4 intro's uniform rule) — a completely
/// different cause from a dependency this model never stores at all, which
/// points at D2 (research §4 D2 is specifically "needs an upstream read";
/// an intra-model changed-sibling reference has no upstream involved, so it
/// must never be pointed at D2).
fn try_d1(
    changed: &ChangedColumn,
    representative_sources: &[RepresentativeSource],
    changed_names: &BTreeSet<String>,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    if let Some(reason) = grain_guard_refusal(&changed.after) {
        return Err(format!(
            "D1 (stored-derivable expression change) refused for '{}': {reason}",
            changed.name
        ));
    }

    let deps = collect_qualified_dependencies(&changed.after).map_err(|reason| {
        format!(
            "D1 (stored-derivable expression change) refused for '{}': {reason}",
            changed.name
        )
    })?;

    let mut missing: Vec<(Option<String>, String)> = deps
        .into_iter()
        .filter(|(q, n)| {
            resolve_representative(
                q.as_deref(),
                n,
                representative_sources,
                Some(changed.after.syntax()),
            )
            .is_none()
        })
        .collect();
    missing.sort();
    if let Some((qualifier, name)) = missing.first() {
        let dep = format_dependency(qualifier, name);
        return Err(if changed_names.contains(name.as_str()) {
            format!(
                "D1 (stored-derivable expression change) refused for '{}': depends on '{dep}', \
                 which also changed in this same edit — a changed column is never a stored \
                 representative (research §4 intro: a representative must be a bare \
                 pull-through unchanged between both definitions), so this cannot safely read \
                 its new value without risking an order-dependent or self-invalidating update",
                changed.name
            )
        } else {
            format!(
                "D1 (stored-derivable expression change) refused for '{}': depends on '{dep}', \
                 which has no 1:1 stored representative (a bare pull-through unchanged between \
                 both definitions, matched by qualifier and raw column name) in the model's own \
                 output — an upstream-only dependency needs D2, not D1",
                changed.name
            )
        });
    }

    let requalified =
        requalify::requalify(&changed.after, representative_sources).map_err(|reason| {
            format!(
                "D1 (stored-derivable expression change) refused for '{}': {reason}",
                changed.name
            )
        })?;

    let update = emit::emit_in_place_update(&inputs.table, &[(changed.name.clone(), requalified)]);

    Ok(BackbuildOption {
        technique: Technique::SelfDerivedColumnRewrite,
        // A single `UPDATE` — the H-ordering "UPDATEs/MERGEs" bucket
        // (research §4 "H. Composites"). No `ALTER` step, so unlike B1
        // there is nothing that must flush before it.
        slot: Some(HSlot::UpdateMerge),
        statements: vec![update],
        write_scope: WriteScope::ColumnScoped,
        reads_upstream: false,
        // Plain DML over representatives that are themselves unchanged
        // stored columns: re-running the same `UPDATE` reproduces the same
        // result (no DDL step, unlike B1's paired `ALTER ADD`).
        rerun_safe: true,
    })
}

/// The D-class shared grain guards (research §4 intro): `SELECT DISTINCT`
/// refuses every D-class atom outright (an `UPDATE` cannot merge two stored
/// rows that now agree on every column, while the rebuild's `DISTINCT`
/// would — the multisets diverge), and any `LIMIT` in the definition
/// refuses every non-A0 atom (row selection under `LIMIT` is not stable
/// under a value change). `classify_comparable` only reaches D-class atoms
/// once `SkeletonDiff::Unchanged` has already proven `DISTINCT`-ness and the
/// `LIMIT` clause identical between both versions, so walking up from
/// either version's own expression node to its enclosing `SelectStmt` (the
/// same recognised CST navigation `smelt-db`'s type inference already uses
/// to find an expression's enclosing SELECT — e.g.
/// `type_inference/function_call.rs`'s `is_grouped_query`) reports the same
/// verdict regardless of which version's `Expr` is passed in.
fn grain_guard_refusal(expr: &Expr) -> Option<String> {
    let stmt = expr.syntax().ancestors().find_map(SelectStmt::cast)?;
    if stmt.is_distinct() {
        return Some(
            "D-class refuses under SELECT DISTINCT — an UPDATE cannot merge rows a rebuild's \
             DISTINCT would (research §4 intro grain guards)"
                .to_string(),
        );
    }
    if stmt.limit_clause().is_some() {
        return Some(
            "D-class refuses when the definition has a LIMIT — row selection under LIMIT is not \
             stable under a value change (research §4 intro grain guards)"
                .to_string(),
        );
    }
    None
}

struct RenamePairing {
    atoms: Vec<AtomAnalysis>,
    consumed_added: HashSet<String>,
}

/// Pair dropped × added columns by expression equality (research §4 B2,
/// §7.2). Groups `dropped` into expression-equivalence clusters first,
/// since the ambiguity rule operates on the *dropped* side: a cluster of
/// two or more dropped columns sharing an identical expression can never be
/// safely paired to a rename, even when exactly one added column matches —
/// refuse rather than guess which dropped column it came from. A
/// single-dropped cluster matched by two or more identical added columns is
/// pinned deterministically: the lexicographically-first added name takes
/// the rename, the rest classify as B1 reading the renamed column.
fn pair_renames(
    dropped: &[SelectColumn],
    added: &[SelectColumn],
    inputs: &BackbuildInputs,
) -> RenamePairing {
    let mut atoms = Vec::new();
    let mut consumed_added = HashSet::new();

    for cluster in cluster_by_expr(dropped) {
        let candidates: Vec<&SelectColumn> = added
            .iter()
            .filter(|a| {
                crate::analysis::expr_util::same_modulo_trivia(
                    a.expr.syntax(),
                    cluster[0].expr.syntax(),
                )
            })
            .collect();

        if cluster.len() == 1 {
            let d = cluster[0];
            match candidates.len() {
                0 => atoms.push(dropped_column_atom(&inputs.table, &d.name)),
                1 => {
                    let winner = candidates[0];
                    atoms.push(rename_atom(&inputs.table, &d.name, &winner.name));
                    consumed_added.insert(winner.name.clone());
                }
                _ => {
                    let mut sorted = candidates;
                    sorted.sort_by(|a, b| a.name.cmp(&b.name));
                    let winner = sorted[0];
                    atoms.push(rename_atom(&inputs.table, &d.name, &winner.name));
                    consumed_added.insert(winner.name.clone());
                    for loser in &sorted[1..] {
                        consumed_added.insert(loser.name.clone());
                        atoms.push(classify_rename_loser(loser, &winner.name, inputs));
                    }
                }
            }
        } else if candidates.is_empty() {
            for d in &cluster {
                atoms.push(dropped_column_atom(&inputs.table, &d.name));
            }
        } else {
            let sibling_names: Vec<String> = cluster.iter().map(|d| d.name.clone()).collect();
            let candidate_names: Vec<String> = candidates.iter().map(|c| c.name.clone()).collect();
            for d in &cluster {
                atoms.push(ambiguous_rename_refusal(
                    &d.name,
                    &sibling_names,
                    &candidate_names,
                ));
            }
        }
    }

    RenamePairing {
        atoms,
        consumed_added,
    }
}

/// Group `dropped` into expression-equivalence clusters (pairwise
/// `analysis::expr_util::same_modulo_trivia`).
fn cluster_by_expr(dropped: &[SelectColumn]) -> Vec<Vec<&SelectColumn>> {
    let mut clusters: Vec<Vec<&SelectColumn>> = Vec::new();
    'outer: for d in dropped {
        for cluster in clusters.iter_mut() {
            if crate::analysis::expr_util::same_modulo_trivia(
                cluster[0].expr.syntax(),
                d.expr.syntax(),
            ) {
                cluster.push(d);
                continue 'outer;
            }
        }
        clusters.push(vec![d]);
    }
    clusters
}

fn rename_atom(table: &str, from: &str, to: &str) -> AtomAnalysis {
    let stmt = emit::emit_alter_rename_column(table, from, to);
    AtomAnalysis {
        change: AtomicChange::RenamedColumn {
            from: from.to_string(),
            to: to.to_string(),
        },
        options: vec![BackbuildOption {
            technique: Technique::Rename,
            slot: Some(HSlot::Rename),
            statements: vec![stmt],
            write_scope: WriteScope::None,
            reads_upstream: false,
            // DDL rename is not re-runnable (research §2 "Idempotence").
            rerun_safe: false,
        }],
        inadmissible: Vec::new(),
    }
}

fn ambiguous_rename_refusal(
    name: &str,
    siblings: &[String],
    candidates: &[String],
) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: format!("dropped column '{name}'"),
            reason: format!(
                "ambiguous rename: {} dropped columns ({}) share an identical expression \
                 matching added column(s) ({}) — refusing rather than guessing which dropped \
                 column renamed to which (research §7.2 'Rename-match ambiguity')",
                siblings.len(),
                siblings.join(", "),
                candidates.join(", ")
            ),
        }],
    }
}

/// A B2 tie-break loser (research §4 B2: "one dropped, two identical added
/// … the rest classify as B1 reading the renamed column"): rather than
/// re-deriving `loser`'s dependencies from scratch (which could refuse if
/// its underlying inputs have no independent representative), its script
/// reads the winning rename target directly — a bare reference, valid
/// regardless of how complex the shared expression was, since the winner
/// already stores exactly that value once the rename statement runs first
/// (`HSlot::Rename` precedes `HSlot::Alter` in `assemble`'s composition).
fn classify_rename_loser(
    loser: &SelectColumn,
    renamed_to: &str,
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    match build_b1_option(loser, renamed_to, inputs) {
        Ok(option) => AtomAnalysis {
            change: AtomicChange::AddedColumn {
                name: loser.name.clone(),
            },
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(reason) => AtomAnalysis {
            change: AtomicChange::AddedColumn {
                name: loser.name.clone(),
            },
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: format!("added column '{}'", loser.name),
                reason,
            }],
        },
    }
}

/// One chosen technique per atom, or the always-present `FullRefresh`
/// baseline — the input `assemble` needs to turn a [`BackbuildOptions`]
/// value into an ordered statement script (research §6: "assemble(options,
/// selection) applies the H ordering to one chosen option per atom").
/// Choosing *among* an atom's options is a future cost model's job
/// (research §2 "Options, not choices"); this phase's callers supply the
/// choice directly.
#[derive(Debug, Clone)]
pub enum Selection {
    /// Compose the targeted script: for every atom in
    /// `BackbuildOptions::atoms`, in order, the index into that atom's
    /// `options` to use. Must have the same length as `atoms`; if any
    /// chosen index does not name an admissible option — including an atom
    /// whose `options` is empty — `assemble` returns an empty script.
    /// Partial application is never offered (research §2 "Refusal
    /// posture").
    Targeted { atom_choices: Vec<usize> },
    /// The always-present model-level `FullRefresh` baseline.
    FullRefresh,
}

/// Turn a [`BackbuildOptions`] value plus a [`Selection`] into an ordered
/// list of statement strings, ready to execute in order. Statements are
/// never authored here — every string comes from a `BackbuildOption`
/// classification already produced (statement single-ownership,
/// `docs/specs/architecture.md` §"Constraints & Invariants" item 12).
///
/// The H-ordering slot structure (research §4 "H. Composites": `renames →
/// ALTER ADD/TYPE → DELETEs → UPDATEs/MERGEs → INSERTs → ALTER DROPs`) is a
/// property of this function alone — one bucketing-and-concatenation pass
/// over whatever [`Technique`]s the selection names, keyed purely by each
/// chosen option's [`HSlot`]. Every new technique a classifier admits only
/// ever needs to name its [`HSlot`]; this composition loop never changes
/// per-technique. [`Selection::Targeted`] still composes to an empty script
/// for the cases no technique is admissible for: zero atoms (the A0 no-op
/// case), or one or more atoms that all lack an admissible option (every
/// refusal case) — partial application is never offered (research §2).
pub fn assemble(options: &BackbuildOptions, selection: &Selection) -> Vec<String> {
    match selection {
        Selection::FullRefresh => options.full_refresh.statements.clone(),
        Selection::Targeted { atom_choices } => {
            if atom_choices.len() != options.atoms.len() {
                return Vec::new();
            }

            // Pass 1: every atom must name an admissible option, or the
            // whole composition is refused — partial application is never
            // offered (research §2).
            let mut chosen = Vec::with_capacity(options.atoms.len());
            for (atom, &choice) in options.atoms.iter().zip(atom_choices) {
                match atom.options.get(choice) {
                    Some(opt) => chosen.push(opt),
                    None => return Vec::new(),
                }
            }

            // Pass 2: bucket into the H-ordering slots, then concatenate in
            // slot order.
            let mut renames = Vec::new();
            let mut alters = Vec::new();
            let mut deletes = Vec::new();
            let mut update_merges = Vec::new();
            let mut inserts = Vec::new();
            let mut drops = Vec::new();
            for opt in chosen {
                let Some(slot) = opt.slot else {
                    // A model-level-only technique (FullRefresh) ended up
                    // in an atom's option set — a classifier bug, not a
                    // valid targeted composition. Refuse rather than emit
                    // a mis-ordered script.
                    return Vec::new();
                };
                let bucket = match slot {
                    HSlot::Rename => &mut renames,
                    HSlot::Alter => &mut alters,
                    HSlot::Delete => &mut deletes,
                    HSlot::UpdateMerge => &mut update_merges,
                    HSlot::Insert => &mut inserts,
                    HSlot::Drop => &mut drops,
                };
                bucket.extend(opt.statements.iter().cloned());
            }

            renames
                .into_iter()
                .chain(alters)
                .chain(deletes)
                .chain(update_merges)
                .chain(inserts)
                .chain(drops)
                .collect()
        }
    }
}
