//! `derive_migration_plan`: turn a [`super::BackbuildOptions`] value into a
//! [`MigrationPlan`] — the per-group verdict/technique presentation
//! `smelt migrate` prints (`docs/specs/definition_deltas.md` §Overview).
//! Pure and total: every [`super::AtomAnalysis`] in the input becomes exactly
//! one [`ColumnGroupPlan`], and every [`super::Technique`] classification can
//! admit maps to exactly one [`Verdict`] via an exhaustive match — a new
//! `Technique` variant fails to compile here rather than silently defaulting
//! (fail-loud discipline, `docs/specs/architecture.md` §"Fail-loud
//! discipline"). This module never inspects SQL text or backend state; it
//! only re-shapes data classification has already proven.

use super::{
    AtomAnalysis, AtomicChange, BackbuildOption, BackbuildOptions, BackbuildRefusal, Technique,
    WriteScope,
};

/// This group's disposition, per `docs/specs/definition_deltas.md`
/// §Overview's worked example.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The diff is a no-op (research catalogue case A0) — nothing to plan.
    /// Only ever the whole [`MigrationPlan`]'s verdict, never a single
    /// group's (an eclipsed diff has no groups at all).
    Eclipsed,
    /// A self-derived add, rename, or rewrite — the cheapest family, never
    /// reading an upstream.
    BackfillInPlace,
    /// Every other admitted technique (an upstream/join read, a row-subset
    /// insert or delete, a self-read backfill), or a group with no
    /// admissible technique at all — presented as needing the full-refresh
    /// baseline, carrying its named refusals.
    ReDerive,
    /// The FROM/JOIN tree or grain changed — no targeted technique is ever
    /// admissible for this atom; rebuild is the only route.
    SkeletonChange,
}

/// Coarse cost/safety presentation, folding [`WriteScope`] with whether the
/// option reads an upstream relation — the two axes
/// `docs/specs/definition_deltas.md` §Overview's worked example prints a
/// technique's cost class by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostClass {
    /// [`WriteScope::None`] — a schema-only change (a rename), zero rows
    /// touched.
    Metadata,
    /// [`WriteScope::ColumnScoped`], no upstream read — a self-derived
    /// column update.
    LocalColumnUpdate,
    /// [`WriteScope::ColumnScoped`], reads an upstream — a join/backfill
    /// column update.
    UpstreamColumnUpdate,
    /// [`WriteScope::RowSubset`], no upstream read.
    LocalRowSubset,
    /// [`WriteScope::RowSubset`], reads an upstream.
    UpstreamRowSubset,
    /// [`WriteScope::Destructive`] — an irreversible schema change
    /// ([`Technique::ColumnDrop`]).
    Destructive,
    /// [`WriteScope::FullWrite`] — the model-level `FullRefresh` baseline.
    FullTable,
}

impl CostClass {
    fn from_option(option: &BackbuildOption) -> Self {
        match (option.write_scope, option.reads_upstream) {
            (WriteScope::None, _) => CostClass::Metadata,
            (WriteScope::ColumnScoped, false) => CostClass::LocalColumnUpdate,
            (WriteScope::ColumnScoped, true) => CostClass::UpstreamColumnUpdate,
            (WriteScope::RowSubset, false) => CostClass::LocalRowSubset,
            (WriteScope::RowSubset, true) => CostClass::UpstreamRowSubset,
            (WriteScope::Destructive, _) => CostClass::Destructive,
            (WriteScope::FullWrite, _) => CostClass::FullTable,
        }
    }
}

/// One admissible technique, presented as a candidate for its group —
/// options, not a choice (research §2 "Options, not choices").
#[derive(Debug, Clone)]
pub struct TechniqueCandidate {
    pub technique: Technique,
    pub cost_class: CostClass,
    pub statement_count: usize,
    pub reads_upstream: bool,
    pub rerun_safe: bool,
}

impl TechniqueCandidate {
    fn from_option(option: &BackbuildOption) -> Self {
        TechniqueCandidate {
            technique: option.technique,
            cost_class: CostClass::from_option(option),
            statement_count: option.statement_count(),
            reads_upstream: option.reads_upstream,
            rerun_safe: option.rerun_safe,
        }
    }
}

/// One [`AtomAnalysis`], re-shaped into its printable verdict, candidate
/// techniques (options, not choices), and every named refusal.
#[derive(Debug, Clone)]
pub struct ColumnGroupPlan {
    /// A short, human-readable label for the underlying atom (mirrors
    /// `classify.rs`'s own per-site refusal-naming convention).
    pub label: String,
    pub verdict: Verdict,
    /// Empty exactly when every technique was refused for this atom (or the
    /// atom is a skeleton change, which is never per-technique) — the
    /// full-refresh baseline is always available regardless (see
    /// [`MigrationPlan::full_refresh`]).
    pub candidates: Vec<TechniqueCandidate>,
    pub refusals: Vec<BackbuildRefusal>,
}

/// Every [`Technique`] classification can admit maps to exactly one
/// [`Verdict`] — exhaustive over [`Technique`], so a new variant is a
/// compile error here, not a silently-defaulted verdict.
fn verdict_for_technique(technique: Technique) -> Verdict {
    match technique {
        Technique::SelfDerivedColumnAdd | Technique::Rename | Technique::SelfDerivedColumnRewrite => {
            Verdict::BackfillInPlace
        }
        Technique::UpstreamPullthrough
        | Technique::JoinEnrichmentUpdateFrom
        | Technique::JoinEnrichmentScalarSubquery
        | Technique::PredicateTightenDelete
        | Technique::HorizonExtensionInsert
        | Technique::FilterLoosenInsert
        | Technique::UnionBranchInsert
        | Technique::DiscriminatedBranchDelete
        | Technique::AggregateColumnBackfill
        | Technique::WindowColumnBackfill
        | Technique::ColumnDrop
        // The model-level baseline is itself a full re-derivation; it never
        // appears among an atom's own `options` (see `classify.rs`'s
        // `full_refresh_option`, which is only ever stored in
        // `BackbuildOptions::full_refresh`), but the match stays exhaustive
        // over every `Technique` variant regardless.
        | Technique::FullRefresh => Verdict::ReDerive,
    }
}

/// A short, human-readable label for an [`AtomicChange`] — mirrors
/// `classify.rs`'s private `atom_change_label` (not reused directly: that
/// helper is `pub(crate)` to `classify.rs`'s own module, and duplicating a
/// four-line match is cheaper than widening its visibility for one caller).
fn atom_label(change: &AtomicChange) -> String {
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

fn derive_group_plan(atom: &AtomAnalysis) -> ColumnGroupPlan {
    let label = atom_label(&atom.change);

    if matches!(atom.change, AtomicChange::Skeleton { .. }) {
        return ColumnGroupPlan {
            label,
            verdict: Verdict::SkeletonChange,
            candidates: Vec::new(),
            refusals: atom.inadmissible.clone(),
        };
    }

    let candidates: Vec<TechniqueCandidate> = atom
        .options
        .iter()
        .map(TechniqueCandidate::from_option)
        .collect();
    let verdict = candidates
        .first()
        .map(|c| verdict_for_technique(c.technique))
        .unwrap_or(Verdict::ReDerive);

    ColumnGroupPlan {
        label,
        verdict,
        candidates,
        refusals: atom.inadmissible.clone(),
    }
}

/// The printable migration plan for one (before, after) diff — one group per
/// atom, plus the always-present model-level `FullRefresh` baseline.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    /// The diff is a no-op — `groups` is always empty in this case, and
    /// vice versa (research catalogue case A0).
    pub eclipsed: bool,
    pub groups: Vec<ColumnGroupPlan>,
    /// The model-level `CREATE OR REPLACE TABLE t AS <after>` baseline —
    /// always present, regardless of `groups`.
    pub full_refresh: BackbuildOption,
}

/// Derive a [`MigrationPlan`] from a [`BackbuildOptions`] value — pure,
/// total, and exhaustive over every [`Technique`] classification can admit.
pub fn derive_migration_plan(options: &BackbuildOptions) -> MigrationPlan {
    let eclipsed = options.atoms.is_empty();
    let groups = options.atoms.iter().map(derive_group_plan).collect();
    MigrationPlan {
        eclipsed,
        groups,
        full_refresh: options.full_refresh.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backbuild::HSlot;

    fn full_refresh_option() -> BackbuildOption {
        BackbuildOption {
            technique: Technique::FullRefresh,
            slot: None,
            statements: vec!["CREATE OR REPLACE TABLE t AS SELECT 1".to_string()],
            write_scope: WriteScope::FullWrite,
            reads_upstream: true,
            rerun_safe: true,
        }
    }

    fn option(
        technique: Technique,
        write_scope: WriteScope,
        reads_upstream: bool,
    ) -> BackbuildOption {
        BackbuildOption {
            technique,
            slot: Some(HSlot::UpdateMerge),
            statements: vec!["UPDATE t SET c = 1".to_string()],
            write_scope,
            reads_upstream,
            rerun_safe: true,
        }
    }

    #[test]
    fn noop_diff_is_eclipsed_with_no_groups() {
        let options = BackbuildOptions {
            atoms: Vec::new(),
            full_refresh: full_refresh_option(),
        };

        let plan = derive_migration_plan(&options);

        assert!(plan.eclipsed);
        assert!(plan.groups.is_empty());
    }

    #[test]
    fn technique_verdict_mapping_covers_every_technique() {
        let self_derived = [
            Technique::SelfDerivedColumnAdd,
            Technique::Rename,
            Technique::SelfDerivedColumnRewrite,
        ];
        for technique in self_derived {
            assert_eq!(
                verdict_for_technique(technique),
                Verdict::BackfillInPlace,
                "{technique:?} should be BackfillInPlace"
            );
        }

        let re_derive = [
            Technique::UpstreamPullthrough,
            Technique::JoinEnrichmentUpdateFrom,
            Technique::JoinEnrichmentScalarSubquery,
            Technique::PredicateTightenDelete,
            Technique::HorizonExtensionInsert,
            Technique::FilterLoosenInsert,
            Technique::UnionBranchInsert,
            Technique::DiscriminatedBranchDelete,
            Technique::AggregateColumnBackfill,
            Technique::WindowColumnBackfill,
            Technique::ColumnDrop,
            Technique::FullRefresh,
        ];
        for technique in re_derive {
            assert_eq!(
                verdict_for_technique(technique),
                Verdict::ReDerive,
                "{technique:?} should be ReDerive"
            );
        }
    }

    #[test]
    fn group_with_no_admissible_option_falls_back_to_full_refresh() {
        let atom = AtomAnalysis {
            change: AtomicChange::ChangedColumn {
                name: "total".to_string(),
            },
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: "changed column 'total'".to_string(),
                reason: "needs an upstream read this phase does not admit".to_string(),
            }],
        };
        let options = BackbuildOptions {
            atoms: vec![atom],
            full_refresh: full_refresh_option(),
        };

        let plan = derive_migration_plan(&options);

        assert!(!plan.eclipsed);
        assert_eq!(plan.groups.len(), 1);
        let group = &plan.groups[0];
        assert_eq!(group.verdict, Verdict::ReDerive);
        assert!(group.candidates.is_empty());
        assert_eq!(group.refusals.len(), 1);
    }

    #[test]
    fn skeleton_refusal_is_a_skeleton_change_verdict() {
        let atom = AtomAnalysis {
            change: AtomicChange::Skeleton {
                reason: "GROUP BY changed".to_string(),
            },
            options: Vec::new(),
            inadmissible: vec![BackbuildRefusal {
                atom: "skeleton".to_string(),
                reason: "G1 (grain change) — GROUP BY changed".to_string(),
            }],
        };
        let options = BackbuildOptions {
            atoms: vec![atom],
            full_refresh: full_refresh_option(),
        };

        let plan = derive_migration_plan(&options);

        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].verdict, Verdict::SkeletonChange);
        assert!(plan.groups[0].candidates.is_empty());
    }

    #[test]
    fn multiple_admissible_techniques_are_presented_as_candidates() {
        let atom = AtomAnalysis {
            change: AtomicChange::AddedColumn {
                name: "region_u".to_string(),
            },
            options: vec![
                option(
                    Technique::JoinEnrichmentUpdateFrom,
                    WriteScope::ColumnScoped,
                    true,
                ),
                option(
                    Technique::JoinEnrichmentScalarSubquery,
                    WriteScope::ColumnScoped,
                    true,
                ),
            ],
            inadmissible: Vec::new(),
        };
        let options = BackbuildOptions {
            atoms: vec![atom],
            full_refresh: full_refresh_option(),
        };

        let plan = derive_migration_plan(&options);

        assert_eq!(plan.groups.len(), 1);
        let group = &plan.groups[0];
        assert_eq!(group.verdict, Verdict::ReDerive);
        assert_eq!(group.candidates.len(), 2);
        assert!(group
            .candidates
            .iter()
            .any(|c| c.technique == Technique::JoinEnrichmentUpdateFrom));
        assert!(group
            .candidates
            .iter()
            .any(|c| c.technique == Technique::JoinEnrichmentScalarSubquery));
    }
}
