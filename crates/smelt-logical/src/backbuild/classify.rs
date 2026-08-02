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

use std::collections::{BTreeSet, HashSet};

use smelt_parser::{Expr, SelectStmt};

use crate::analysis::model_diff;

use super::{
    AtomAnalysis, AtomicChange, BackbuildInputs, BackbuildOption, BackbuildOptions,
    BackbuildRefusal, ChangedColumn, ComparableDiff, ConjunctDiff, DefinitionDiff, HSlot,
    SelectColumn, SelectListDiff, SetOpDiff, SkeletonDiff, Technique, WriteScope,
};

use super::emit;
use super::requalify;

/// Derive the option set for a `(before, after)` diff (research §2, §4).
pub fn derive_backbuild_options(
    diff: &DefinitionDiff,
    inputs: &BackbuildInputs,
) -> BackbuildOptions {
    let full_refresh = full_refresh_option(inputs);

    let atoms = match diff {
        DefinitionDiff::Opaque { reason } => vec![whole_definition_refusal(reason)],
        DefinitionDiff::Comparable(comparable) => {
            if diff.is_noop() {
                // A0 — no-op: nothing changed, so there is nothing to
                // refuse or classify. The targeted script is trivially
                // empty (`assemble` over zero atoms); `FullRefresh` remains
                // available regardless.
                Vec::new()
            } else {
                match &comparable.skeleton {
                    SkeletonDiff::Changed { reason } => vec![skeleton_refusal(reason)],
                    SkeletonDiff::Unchanged => classify_comparable(comparable, inputs),
                    // Added LEFT JOIN(s) (B4/B7) are out of this phase's
                    // scope — a conservative catch-all rather than
                    // misclassifying a select-list change that arrived
                    // alongside a new join.
                    SkeletonDiff::AddedLeftJoins(_) => vec![unclassified_refusal()],
                }
            }
        }
    };

    BackbuildOptions {
        atoms,
        full_refresh,
    }
}

fn full_refresh_option(inputs: &BackbuildInputs) -> BackbuildOption {
    BackbuildOption {
        technique: Technique::FullRefresh,
        slot: None,
        statements: vec![format!(
            "CREATE OR REPLACE TABLE {} AS {}",
            inputs.table, inputs.after_sql
        )],
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

fn skeleton_refusal(reason: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Skeleton {
            reason: reason.to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "skeleton".to_string(),
            reason: classify_skeleton_reason(reason),
        }],
    }
}

/// Label a `SkeletonDiff::Changed` reason with its research §4 catalogue
/// case where the diff module's reason text identifies one: G1 (grain
/// change — `GROUP BY`/`DISTINCT`) or G2 (join-multiplicity change). Any
/// other skeleton change (FROM target, `HAVING`/`QUALIFY`/`WINDOW`/`ORDER
/// BY`/`LIMIT`) is still refused, just without a G1/G2 label — the
/// catalogue's G-class explicitly refuses those too ("Opaque expressions
/// ... LIMIT/ORDER BY changes: refuse with named reasons").
fn classify_skeleton_reason(reason: &str) -> String {
    let lower = reason.to_lowercase();
    if lower.contains("group by") || lower.contains("distinct") {
        format!("G1 (grain change) — {reason}")
    } else if lower.contains("join") {
        format!("G2 (join-multiplicity change) — {reason}")
    } else {
        format!("skeleton changed, not yet admissible — {reason}")
    }
}

fn unclassified_refusal() -> AtomAnalysis {
    unclassified_refusal_named("whole-definition")
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

    match &comparable.select_list {
        SelectListDiff::Opaque { reason } => {
            atoms.push(unclassified_refusal_named(&format!(
                "select-list: {reason}"
            )));
        }
        SelectListDiff::Diffed {
            added,
            dropped,
            changed,
            unchanged,
        } => {
            let representatives = representative_names(unchanged);
            let representative_sources = representative_sources(unchanged);
            let changed_names: BTreeSet<String> = changed.iter().map(|c| c.name.clone()).collect();
            atoms.extend(classify_select_list(
                added,
                dropped,
                &representatives,
                &representative_sources,
                inputs,
            ));
            for c in changed {
                atoms.push(classify_changed_column(
                    c,
                    &representatives,
                    &representative_sources,
                    &changed_names,
                    inputs,
                ));
            }
        }
    }

    match &comparable.where_clause {
        ConjunctDiff::Diffed { added, removed, .. } if !added.is_empty() || !removed.is_empty() => {
            atoms.push(unclassified_refusal_named("where-clause"));
        }
        ConjunctDiff::Opaque { reason } => {
            atoms.push(unclassified_refusal_named(&format!(
                "where-clause: {reason}"
            )));
        }
        _ => {}
    }

    match &comparable.set_ops {
        SetOpDiff::Branches { added, removed, .. } if !added.is_empty() || !removed.is_empty() => {
            atoms.push(unclassified_refusal_named("set-operations"));
        }
        SetOpDiff::Opaque { reason } => {
            atoms.push(unclassified_refusal_named(&format!(
                "set-operations: {reason}"
            )));
        }
        _ => {}
    }

    atoms
}

fn dropped_column_unclassified(name: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: format!("dropped column '{name}'"),
            reason: "a dropped column not paired into a rename (research §4 C1) is not yet \
                     classified into an admissible technique by this phase"
                .to_string(),
        }],
    }
}

/// Rename pairing (B2) runs *before* add/drop classification, then every
/// remaining added column is classified as B1 or refused (research §4 B1,
/// B2, §7.2 "Rename-match ambiguity"). `representatives` is the caller's
/// already-computed [`representative_names`] set — shared with the sibling
/// `changed`-column (D1) classification so both draw from exactly the same
/// stored-representative set (research §4 intro "one uniform rule").
fn classify_select_list(
    added: &[SelectColumn],
    dropped: &[SelectColumn],
    representatives: &BTreeSet<String>,
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
            representatives,
            representative_sources,
            inputs,
        ));
    }

    rename_atoms
}

/// The stored 1:1 representative output-column names (research §4 intro
/// "Derivability representatives — one uniform rule"): every SELECT-list
/// output column present, unchanged, in both definitions (`unchanged`) *and*
/// whose own expression is a bare column reference — a bare pull-through,
/// never a computed expression. A changed column is never a representative
/// by construction (it is not in `unchanged` at all).
fn representative_names(unchanged: &[SelectColumn]) -> BTreeSet<String> {
    unchanged
        .iter()
        .filter(|c| c.expr.as_column_ref().is_some())
        .map(|c| c.name.clone())
        .collect()
}

/// One stored 1:1 representative's own upstream provenance: the FROM-tree
/// qualifier its bare pull-through expression reads from (`None` for an
/// unqualified reference) and the raw column name at that qualifier,
/// alongside the representative's own output name. Built from the same
/// `unchanged` SELECT items [`representative_names`] draws from — the B3/D2
/// grain-link proof (research §4 B3/D2: "the pulled-through key and the
/// added expression must resolve to the SAME alias") needs the qualifier
/// `representative_names`'s plain name set discards.
struct RepresentativeSource {
    output_name: String,
    qualifier: Option<String>,
    raw_name: String,
}

fn representative_sources(unchanged: &[SelectColumn]) -> Vec<RepresentativeSource> {
    unchanged
        .iter()
        .filter_map(|c| {
            let col_ref = c.expr.as_column_ref()?;
            Some(RepresentativeSource {
                output_name: c.name.clone(),
                qualifier: col_ref.qualifier().map(|q| q.to_string()),
                raw_name: col_ref.name().to_string(),
            })
        })
        .collect()
}

fn classify_added_column(
    col: &SelectColumn,
    representatives: &BTreeSet<String>,
    representative_sources: &[RepresentativeSource],
    inputs: &BackbuildInputs,
) -> AtomAnalysis {
    let atom_change = AtomicChange::AddedColumn {
        name: col.name.clone(),
    };
    match try_b1(col, representatives, inputs) {
        Ok(option) => AtomAnalysis {
            change: atom_change,
            options: vec![option],
            inadmissible: Vec::new(),
        },
        Err(b1_reason) => {
            // B3 shares B1's expression-validity leaf check (subquery/
            // window/opaque function/non-determinism, research §4 intro) —
            // when that is what refused B1, B3 would refuse identically, so
            // it is never attempted (avoids a redundant, confusing second
            // refusal for the exact same underlying reason).
            if model_diff::collect_dependencies(&col.expr).is_err() {
                return AtomAnalysis {
                    change: atom_change,
                    options: Vec::new(),
                    inadmissible: vec![BackbuildRefusal {
                        atom: format!("added column '{}'", col.name),
                        reason: b1_reason,
                    }],
                };
            }
            match try_b3(col, representative_sources, inputs) {
                Ok(option) => AtomAnalysis {
                    change: atom_change,
                    options: vec![option],
                    inadmissible: Vec::new(),
                },
                Err(b3_reason) => AtomAnalysis {
                    change: atom_change,
                    options: Vec::new(),
                    inadmissible: vec![
                        BackbuildRefusal {
                            atom: format!("added column '{}'", col.name),
                            reason: b1_reason,
                        },
                        BackbuildRefusal {
                            atom: format!("added column '{}'", col.name),
                            reason: b3_reason,
                        },
                    ],
                },
            }
        }
    }
}

/// B1 admission (research §4 B1): every dependency of `col`'s expression
/// must resolve to a stored 1:1 representative. Reuses
/// `analysis::model_diff::collect_dependencies` — the same dependency walk
/// `additive_only_diff` uses, not a fork of it (`docs/specs/architecture.md`
/// §"Property composition walk rule") — which already fails closed on
/// subqueries, window `OVER` clauses, unregistered functions, and
/// non-deterministic functions (research §2 "Determinism caveat").
fn try_b1(
    col: &SelectColumn,
    representatives: &BTreeSet<String>,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    let deps = model_diff::collect_dependencies(&col.expr).map_err(|reason| {
        format!(
            "B1 (self-derivable column add) refused for '{}': {reason}",
            col.name
        )
    })?;

    let mut missing: Vec<&String> = deps
        .iter()
        .filter(|d| !representatives.contains(*d))
        .collect();
    missing.sort();
    if let Some(dep) = missing.first() {
        return Err(format!(
            "B1 (self-derivable column add) refused for '{}': depends on '{dep}', which has no \
             1:1 stored representative (a bare pull-through unchanged between both definitions) \
             in the model's own output — an upstream-only dependency needs B3, not B1",
            col.name
        ));
    }

    let requalified = requalify::requalify(&col.expr, representatives).map_err(|reason| {
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
    representatives: &BTreeSet<String>,
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

    let d1_result = try_d1(changed, representatives, changed_names, inputs);
    let attempt_d2 = match &d1_result {
        Ok(_) => true,
        Err(_) => d1_failure_licenses_d2_attempt(changed, representatives, changed_names),
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
/// doc comment). Re-derives D1's own "first missing dependency" (same sort
/// order `try_d1` uses) rather than threading a richer error type back from
/// `try_d1` — `try_d1` already fully validated the expression by the time
/// this runs, so `collect_dependencies` here is guaranteed `Ok` in every
/// reachable case except the one it exists to detect (an expression-validity
/// failure, which also means "don't attempt D2").
fn d1_failure_licenses_d2_attempt(
    changed: &ChangedColumn,
    representatives: &BTreeSet<String>,
    changed_names: &BTreeSet<String>,
) -> bool {
    let Ok(deps) = model_diff::collect_dependencies(&changed.after) else {
        return false;
    };
    let mut missing: Vec<&String> = deps
        .iter()
        .filter(|d| !representatives.contains(*d))
        .collect();
    missing.sort();
    match missing.first() {
        Some(dep) => !changed_names.contains(dep.as_str()),
        // No missing dependency at all means D1 should have succeeded —
        // unreachable from the `Err` branch that calls this, but false is
        // the safe (skip D2) answer either way.
        None => false,
    }
}

/// D1 admission (research §4 D1): every dependency of the column's *new*
/// expression must resolve to a stored 1:1 representative under exactly the
/// same uniform rule B1 uses (research §4 intro) — representatives are drawn
/// only from `unchanged` output columns, so the changed column itself and
/// any changed sibling are never representatives by construction. This is
/// what makes self-substitution (a new expression reading the column's own
/// old value) and a mutual swap (`x AS a, y AS b` → `y AS a, x AS b`) both
/// refuse: neither `a` nor `b` is ever in `representatives`, since both are
/// in `changed`, not `unchanged`.
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
    representatives: &BTreeSet<String>,
    changed_names: &BTreeSet<String>,
    inputs: &BackbuildInputs,
) -> Result<BackbuildOption, String> {
    if let Some(reason) = grain_guard_refusal(&changed.after) {
        return Err(format!(
            "D1 (stored-derivable expression change) refused for '{}': {reason}",
            changed.name
        ));
    }

    let deps = model_diff::collect_dependencies(&changed.after).map_err(|reason| {
        format!(
            "D1 (stored-derivable expression change) refused for '{}': {reason}",
            changed.name
        )
    })?;

    let mut missing: Vec<&String> = deps
        .iter()
        .filter(|d| !representatives.contains(*d))
        .collect();
    missing.sort();
    if let Some(dep) = missing.first() {
        return Err(if changed_names.contains(dep.as_str()) {
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
                 both definitions) in the model's own output — an upstream-only dependency \
                 needs a later phase's D2, not D1",
                changed.name
            )
        });
    }

    let requalified = requalify::requalify(&changed.after, representatives).map_err(|reason| {
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
            .filter(|a| expr_equal_modulo_trivia(&a.expr, &cluster[0].expr))
            .collect();

        if cluster.len() == 1 {
            let d = cluster[0];
            match candidates.len() {
                0 => atoms.push(dropped_column_unclassified(&d.name)),
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
                atoms.push(dropped_column_unclassified(&d.name));
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
/// `expr_equal_modulo_trivia`).
fn cluster_by_expr(dropped: &[SelectColumn]) -> Vec<Vec<&SelectColumn>> {
    let mut clusters: Vec<Vec<&SelectColumn>> = Vec::new();
    'outer: for d in dropped {
        for cluster in clusters.iter_mut() {
            if expr_equal_modulo_trivia(&cluster[0].expr, &d.expr) {
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

/// Trivia-insensitive structural equality of two expressions. Duplicates
/// `diff.rs`'s private `same_modulo_trivia` (over `Expr` rather than
/// `SyntaxNode`) rather than exposing it: `diff.rs` is out of this phase's
/// touch-scope, and this is a small, self-contained comparator, not the
/// dependency walk the "don't fork" rule is about.
fn expr_equal_modulo_trivia(a: &Expr, b: &Expr) -> bool {
    let mut ta = a
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .map(|t| (t.kind(), t.text().to_string()));
    let mut tb = b
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .map(|t| (t.kind(), t.text().to_string()));
    loop {
        match (ta.next(), tb.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => continue,
            _ => return false,
        }
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
/// This phase's classifier only ever produces atoms with empty option
/// sets, so [`Selection::Targeted`] composes to an empty script in every
/// case it can reach today: zero atoms (the A0 no-op case), or one or more
/// atoms that all lack an admissible option (every refusal case this phase
/// derives). The H-ordering slot structure (research §4 "H. Composites":
/// `renames → ALTER ADD/TYPE → DELETEs → UPDATEs/MERGEs → INSERTs → ALTER
/// DROPs`) is built out in full regardless, so later phases — which
/// populate atom options with real techniques — only need to give each new
/// [`Technique`] an [`HSlot`]; this function's bucketing loop does not
/// change.
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
