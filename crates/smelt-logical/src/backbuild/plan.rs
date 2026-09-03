//! Folding a [`BackbuildOptions`] atom set into a per-column-group migration
//! plan, and a stable hash over it (research
//! `docs/research/20260802-backbuild-synthesis.md`; phase-1 decision: "hash
//! the plan data structure the emitters consume — verdicts, techniques,
//! statements, input facts — never a caller-invented comparator").
//!
//! This module authors no new SQL statement text — every string in a
//! [`BackbuildOption`] already came from `classify.rs`/`emit.rs`; folding and
//! hashing only read and re-group them (statement single-ownership,
//! `docs/specs/architecture.md` §"Constraints & Invariants" item 12).

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{
    AtomAnalysis, AtomicChange, BackbuildInputs, BackbuildOption, BackbuildRefusal, DefinitionDiff,
    SourceRef,
};
use crate::backbuild::classify::{assemble, derive_backbuild_options, Selection};

/// The whole-plan (or per-group) migration verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationVerdict {
    /// The diff is a no-op (or, at the whole-plan level, every group folded
    /// to nothing) — nothing to migrate.
    Eclipsed,
    /// Every admitted option for this group only touches the stored table
    /// itself (`reads_upstream: false` on every option) — an in-place
    /// backfill suffices.
    BackfillInPlace,
    /// At least one admitted option for this group reads an upstream —
    /// re-deriving from source data is required.
    Rederive,
    /// No admissible technique exists for this group — full refresh is the
    /// only honest route (a genuine skeleton/grain change, or a change this
    /// phase does not yet classify).
    SkeletonChange,
}

/// One column group's migration verdict, candidate techniques, and named
/// refusals — one per [`AtomAnalysis`] `derive_backbuild_options` produced.
#[derive(Debug, Clone)]
pub struct ColumnGroupPlan {
    /// Column name(s) this group covers. Empty for a group with no
    /// column-level identity (a whole-definition or skeleton change).
    pub columns: Vec<String>,
    pub verdict: MigrationVerdict,
    /// Every admissible option for this group (mirrors `AtomAnalysis::options`
    /// — no new statement authoring, these are exactly the `BackbuildOption`s
    /// `derive_backbuild_options` already produced).
    pub options: Vec<BackbuildOption>,
    pub refusals: Vec<BackbuildRefusal>,
}

/// The whole per-model migration plan: one [`ColumnGroupPlan`] per atom
/// `derive_backbuild_options` classified, plus the always-present
/// model-level full-refresh baseline.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub model: String,
    pub table: String,
    pub groups: Vec<ColumnGroupPlan>,
    pub full_refresh: BackbuildOption,
    /// The targeted script `assemble` composes from the first admitted
    /// option per group (`Selection::Targeted` with an all-zero choice
    /// vector) — empty when any group admits no option (a skeleton change),
    /// since partial application is never offered.
    pub statements: Vec<String>,
}

impl MigrationPlan {
    /// Whether every group's chosen (first-admitted) option is
    /// [`BackbuildOption::rerun_safe`] — `true` for a plan with no groups
    /// (nothing to re-run). An interrupted apply of a plan that is *not*
    /// fully rerun-safe cannot simply be resumed by re-running `statements`
    /// from the start.
    pub fn all_rerun_safe(&self) -> bool {
        self.groups
            .iter()
            .all(|g| g.options.first().is_some_and(|o| o.rerun_safe))
    }
}

impl MigrationPlan {
    /// Whole-plan verdict: `Eclipsed` when there is nothing to do (no
    /// groups); otherwise the "worst" group verdict in priority order
    /// `SkeletonChange > Rederive > BackfillInPlace` — a plan mixing group
    /// kinds is summarized by its most consequential group.
    pub fn verdict(&self) -> MigrationVerdict {
        if self.groups.is_empty() {
            return MigrationVerdict::Eclipsed;
        }
        if self
            .groups
            .iter()
            .any(|g| g.verdict == MigrationVerdict::SkeletonChange)
        {
            return MigrationVerdict::SkeletonChange;
        }
        if self
            .groups
            .iter()
            .any(|g| g.verdict == MigrationVerdict::Rederive)
        {
            return MigrationVerdict::Rederive;
        }
        MigrationVerdict::BackfillInPlace
    }
}

/// Column name(s) an atom's [`AtomicChange`] identifies — empty for a
/// whole-definition or skeleton change, which has no column-level identity.
fn atom_columns(change: &AtomicChange) -> Vec<String> {
    match change {
        AtomicChange::AddedColumn { name }
        | AtomicChange::DroppedColumn { name }
        | AtomicChange::ChangedColumn { name } => vec![name.clone()],
        AtomicChange::RenamedColumn { from, to } => vec![from.clone(), to.clone()],
        AtomicChange::WholeDefinition { .. }
        | AtomicChange::Skeleton { .. }
        | AtomicChange::AddedConjunct { .. }
        | AtomicChange::RangePredicateChange { .. }
        | AtomicChange::RemovedConjunct { .. }
        | AtomicChange::AddedSetOpBranch { .. }
        | AtomicChange::RemovedSetOpBranch { .. }
        | AtomicChange::Unclassified => vec![],
    }
}

fn group_verdict(atom: &AtomAnalysis) -> MigrationVerdict {
    if atom.options.is_empty() {
        return MigrationVerdict::SkeletonChange;
    }
    if atom.options.iter().any(|o| o.reads_upstream) {
        MigrationVerdict::Rederive
    } else {
        MigrationVerdict::BackfillInPlace
    }
}

/// Fold `derive_backbuild_options`'s atoms into a per-column-group migration
/// plan. Pure — no new statement authoring; every SQL string in every
/// group's `options` came from `classify.rs`/`emit.rs`.
pub fn derive_migration_plan(
    model: &str,
    diff: &DefinitionDiff,
    inputs: &BackbuildInputs,
) -> MigrationPlan {
    let options = derive_backbuild_options(diff, inputs);
    let groups: Vec<ColumnGroupPlan> = options
        .atoms
        .iter()
        .map(|atom| ColumnGroupPlan {
            columns: atom_columns(&atom.change),
            verdict: group_verdict(atom),
            options: atom.options.clone(),
            refusals: atom.inadmissible.clone(),
        })
        .collect();

    // Compose the targeted script from the first admitted option per group
    // (`assemble` returns an empty script if any group admits none —
    // partial application is never offered).
    let atom_choices = vec![0; options.atoms.len()];
    let statements = assemble(&options, &Selection::Targeted { atom_choices });

    MigrationPlan {
        model: model.to_string(),
        table: inputs.table.clone(),
        groups,
        full_refresh: options.full_refresh,
        statements,
    }
}

/// Stable hash over the plan's derived shape (verdicts, techniques,
/// statements, write scopes, rerun-safety) plus the input facts the plan was
/// derived from (`table`, `after_sql`, `row_identity`, `not_null_columns`,
/// `added_column_types`, `sources`) — excluding region enumeration (there is
/// no region field anywhere in [`MigrationPlan`]/[`BackbuildInputs`], so
/// nothing to exclude; region enumeration is resolved at apply time, per the
/// phase-1 decision).
pub fn plan_hash(plan: &MigrationPlan, inputs: &BackbuildInputs) -> String {
    let mut s = String::new();

    let _ = writeln!(s, "table={}", plan.table);
    let _ = writeln!(s, "model={}", plan.model);

    for group in &plan.groups {
        let _ = writeln!(s, "group.columns={}", group.columns.join(","));
        let _ = writeln!(s, "group.verdict={:?}", group.verdict);
        for opt in &group.options {
            write_option(&mut s, opt);
        }
        for refusal in &group.refusals {
            let _ = writeln!(s, "refusal.atom={}", refusal.atom);
            let _ = writeln!(s, "refusal.reason={}", refusal.reason);
        }
    }

    write_option(&mut s, &plan.full_refresh);

    let _ = writeln!(s, "statements={}", plan.statements.join("\n---\n"));

    // Input facts.
    let _ = writeln!(s, "inputs.table={}", inputs.table);
    let _ = writeln!(s, "inputs.after_sql={}", inputs.after_sql);
    let _ = writeln!(
        s,
        "inputs.row_identity={}",
        inputs
            .row_identity
            .as_ref()
            .map(|cols| cols.join(","))
            .unwrap_or_default()
    );
    let _ = writeln!(
        s,
        "inputs.not_null_columns={}",
        inputs
            .not_null_columns
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    );
    for (name, ty) in &inputs.added_column_types {
        let _ = writeln!(s, "inputs.added_column_types.{name}={ty}");
    }
    for (alias, source) in &inputs.sources {
        write_source_ref(&mut s, alias, source);
    }

    let digest = Sha256::digest(s.as_bytes());
    format!("sha256:{digest:x}")
}

fn write_option(s: &mut String, opt: &BackbuildOption) {
    let _ = writeln!(s, "option.technique={:?}", opt.technique);
    let _ = writeln!(s, "option.slot={:?}", opt.slot);
    let _ = writeln!(s, "option.statements={}", opt.statements.join("\n---\n"));
    let _ = writeln!(s, "option.write_scope={:?}", opt.write_scope);
    let _ = writeln!(s, "option.reads_upstream={}", opt.reads_upstream);
    let _ = writeln!(s, "option.rerun_safe={}", opt.rerun_safe);
}

fn write_source_ref(s: &mut String, alias: &str, source: &SourceRef) {
    let _ = writeln!(s, "source.{alias}.physical_name={}", source.physical_name);
    let _ = writeln!(
        s,
        "source.{alias}.unique_key={}",
        source
            .unique_key
            .as_ref()
            .map(|cols| cols.join(","))
            .unwrap_or_default()
    );
    let _ = writeln!(
        s,
        "source.{alias}.not_null_columns={}",
        source
            .not_null_columns
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    );
}
