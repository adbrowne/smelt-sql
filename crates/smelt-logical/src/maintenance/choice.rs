//! Technique choice among admissible alternatives — the override ladder
//! (`defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
//! scope winning) plus the cost-model hook `smelt bakeoff` measures into
//! (`incremental_models.md` §Surface "Frontmatter", §Semantics
//! "Interchangeability and choice", §Design "Offline cost measurement is
//! first-class").
//!
//! `derive_maintenance_plan` (`derive.rs`) admits exactly one [`Technique`]
//! per cell today — there is no multi-technique admission set inside the
//! pure plan yet (see that module's doc comment). The second live
//! alternative that exists for every cell whose admitted technique realizes
//! the top-right/bottom-left corners (fold-a-delta, column-scoped
//! re-derivation) is the always-admissible whole-region recompute
//! (`Technique::DeleteInsert`): a recompute is contract-agnostic and
//! unconditionally valid over replayable input
//! (`incremental_models.md` §Semantics "The plan matrix"). This module treats
//! `{the cell's own admitted technique, RegionRecompute}` as the resolvable
//! set and applies the override ladder over it — pure data in, pure data
//! out, per the "Maintenance-plan purity" invariant (root `CLAUDE.md`).
//!
//! A `technique:` pin naming a technique outside that resolvable set is an
//! admission failure ([`ChoiceRefusal`]), never a silent override — the
//! spec's "a pin bypasses the cost model, never admission."

use smelt_core::config::{
    CellTechnique, MaintenanceCellConfig, MaintenanceDefaults, TechniquePreference,
};

use crate::analysis::walk::{ColumnComparability, Comparability};

use super::{MaintenancePlan, RowIdentity, RowIdentityVerdict, Technique, Trigger};

/// The technique the ladder resolves to for one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChosenTechnique {
    /// The cell's own admitted technique (fold family / column-scoped merge
    /// / in-place update — whichever `derive_maintenance_plan` picked).
    Admitted(Technique),
    /// The always-available whole-region recompute (`DELETE`+`INSERT`),
    /// chosen either because it is the only resolvable member or because
    /// the ladder/cost-model preferred it.
    RegionRecompute,
}

/// Why a requested technique choice could not be honoured: `cells[].technique`
/// (or a soft `prefer`, when it disagrees with every resolvable member) names
/// a technique outside `{the cell's own admitted technique, RegionRecompute}`
/// — a pin bypasses the cost model, never admission
/// (`incremental_models.md` §Surface "Frontmatter").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRefusal {
    pub trigger: String,
    pub pinned: CellTechnique,
    pub why: String,
}

impl std::fmt::Display for ChoiceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MaintenanceUnboundedFootprint: pinned technique '{:?}' for {} is not in the \
             admissible set — {}",
            self.pinned, self.trigger, self.why
        )
    }
}

/// The effective per-cell override once the ladder narrows: `cells[].technique`
/// (a hard pin) if present, else `cells[].prefer` if present, else
/// `defaults.prefer` — narrower scope always wins over broader
/// (`incremental_models.md` §Surface "Frontmatter": "The override ladder is
/// `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
/// scope winning").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectiveOverride {
    /// Soft bias — the cost model may still choose a different resolvable
    /// technique. `None`/`Auto` both mean "no soft bias": the resolver falls
    /// through to its own deterministic default.
    pub prefer: Option<TechniquePreference>,
    /// Hard pin — bypasses the cost model, never bypasses admission.
    pub technique: Option<CellTechnique>,
}

/// Match one `maintenance.cells[]` entry against `trigger_address` (the
/// cell's `on:` value — a source address or the literal `backfill`) and
/// `group_columns` (any member of the cell's derived column group — the
/// `cells[].columns` match is "names any member", not "equals exactly",
/// per §Surface "Frontmatter").
fn matching_cell<'a>(
    cells: &'a [MaintenanceCellConfig],
    trigger_address: &str,
    group_columns: &[String],
) -> Option<&'a MaintenanceCellConfig> {
    cells.iter().find(|c| {
        c.on == trigger_address
            && c.columns
                .iter()
                .any(|col| group_columns.iter().any(|g| g == col))
    })
}

/// Resolve the effective override for one cell, applying the narrower-wins
/// ladder. `cells` is the model's `maintenance.cells[]` frontmatter (already
/// scoped to this model — there is no project-level default for the
/// technique ladder, unlike `scan_bounds`).
pub fn effective_override(
    defaults: Option<&MaintenanceDefaults>,
    cells: &[MaintenanceCellConfig],
    trigger_address: &str,
    group_columns: &[String],
) -> EffectiveOverride {
    let broad_prefer = defaults.and_then(|d| d.prefer);
    let narrow = matching_cell(cells, trigger_address, group_columns);
    EffectiveOverride {
        prefer: narrow.and_then(|c| c.prefer).or(broad_prefer),
        technique: narrow.and_then(|c| c.technique),
    }
}

/// Human-readable trigger label for diagnostics — mirrors the `{trigger:?}`
/// convention `derive.rs`'s own refusals use.
fn trigger_label(trigger: &Trigger) -> String {
    format!("{trigger:?}")
}

/// Whether `technique` is a member of the cell's resolvable set: the cell's
/// own admitted technique (only when the backend can actually run it) or the
/// always-available region recompute.
fn admits(
    pin: CellTechnique,
    admitted: Option<&Technique>,
    backend_supports_column_scoped_merge: bool,
) -> bool {
    match pin {
        CellTechnique::Recompute => true,
        CellTechnique::Fold => matches!(
            admitted,
            Some(Technique::KeyedFold) | Some(Technique::InPlaceUpdate)
        ),
        CellTechnique::RederiveColumns => {
            admitted == Some(&Technique::ColumnScopedMerge) && backend_supports_column_scoped_merge
        }
    }
}

/// Resolve which technique executes for `trigger`, given the plan, the
/// effective override (already narrowed by [`effective_override`]), and
/// whether the target backend can run a column-scoped `MERGE` at all.
///
/// Mirrors `incremental_models.md` §"Per-cell admission": a `technique:` pin
/// bypasses the cost model, **never** admission — pinning a technique the
/// resolvable set does not contain is a hard, fail-loud [`ChoiceRefusal`],
/// not a silent fallback to `RegionRecompute`. A soft `prefer` never
/// refuses: it only nudges the choice among what IS resolvable, falling back
/// silently to the deterministic default when the preferred family isn't
/// resolvable (that is what "soft" means — `cells[].prefer`'s doc comment:
/// "the cost model may still choose a different admissible technique").
/// Absent any override, the cell's own admitted+live technique is preferred
/// over region recompute (the point of admitting it at all); otherwise
/// region recompute is the safe default.
pub fn resolve_cell_choice(
    plan: &MaintenancePlan,
    trigger: &Trigger,
    overrides: &EffectiveOverride,
    backend_supports_column_scoped_merge: bool,
) -> Result<ChosenTechnique, ChoiceRefusal> {
    let cell = plan.cell_for(trigger);
    let admitted_technique = cell.map(|c| &c.technique);
    let live_technique = admitted_technique.filter(|t| match t {
        Technique::ColumnScopedMerge => backend_supports_column_scoped_merge,
        _ => true,
    });

    if let Some(pin) = overrides.technique {
        return if admits(
            pin,
            admitted_technique,
            backend_supports_column_scoped_merge,
        ) {
            match pin {
                CellTechnique::Recompute => Ok(ChosenTechnique::RegionRecompute),
                CellTechnique::Fold | CellTechnique::RederiveColumns => {
                    Ok(ChosenTechnique::Admitted(
                        admitted_technique
                            .expect(
                                "admits() already proved `admitted_technique` is Some for this pin",
                            )
                            .clone(),
                    ))
                }
            }
        } else {
            Err(ChoiceRefusal {
                trigger: trigger_label(trigger),
                pinned: pin,
                why: format!(
                    "the derived plan's resolvable set for this cell is {{{}}} — a pin \
                     bypasses the cost model, never admission",
                    resolvable_set_label(admitted_technique, backend_supports_column_scoped_merge)
                ),
            })
        };
    }

    // No hard pin: a soft `prefer` nudges among what IS resolvable, but
    // never refuses.
    match overrides.prefer {
        Some(TechniquePreference::Recompute) => Ok(ChosenTechnique::RegionRecompute),
        _ => match live_technique {
            Some(t) => Ok(ChosenTechnique::Admitted(t.clone())),
            None => Ok(ChosenTechnique::RegionRecompute),
        },
    }
}

fn resolvable_set_label(
    admitted_technique: Option<&Technique>,
    backend_supports_column_scoped_merge: bool,
) -> String {
    let mut members = vec!["recompute".to_string()];
    if let Some(t) = admitted_technique {
        let live = match t {
            Technique::ColumnScopedMerge => backend_supports_column_scoped_merge,
            _ => true,
        };
        if live {
            members.push(format!("{t:?}"));
        }
    }
    members.join(", ")
}

/// Whether a `Technique::ColumnScopedMerge` cell's matched arm may write
/// conditionally (T1, `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase C4) — the interchangeable alternative to always
/// rewriting every matched row: [`emit::emit_column_scoped_merge_suppressed`]
/// versus the unconditional [`emit::emit_column_scoped_merge`]
/// (`super::emit`). Both variants are members of the same resolvable
/// `ColumnScopedMerge` technique; this only decides which matched-arm shape
/// is safe to emit for one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteSuppression {
    /// Every compared column is proven `Comparable` across runs (P3) over a
    /// proven, non-`WholeRow` row identity (P2) — the matched arm may be
    /// guarded by `IS DISTINCT FROM` over exactly these columns.
    Suppressed { compared_columns: Vec<String> },
    /// Fail-closed refusal of the conditional variant: at least one compared
    /// column is not proven comparable, no row identity is proven, or the
    /// group is empty. `why` names the specific column(s) or condition that
    /// refused, so a caller (`smelt explain`) can show the reason rather
    /// than only ever seeing the safe fallback.
    Unconditional { why: String },
}

/// Resolve whether a column-scoped `MERGE` cell's write may be suppressed
/// for unchanged rows (`incremental_models.md` §"Known Divergences" — this
/// phase narrows the "every emitted MERGE writes all matched rows
/// unconditionally" divergence).
///
/// Fail-closed over two independent proofs, either alone refusing:
/// - **P2, row identity** (`super::RowIdentityVerdict`, `derive::row_identity`):
///   a [`RowIdentity::WholeRow`] cell has no proven per-row join identity to
///   compare on safely, so suppression refuses regardless of column
///   comparability.
/// - **P3, per-column change comparability** (`crate::analysis::walk::
///   Comparability`, the property-walk fold): every column in `group_columns`
///   must carry a `Comparable` verdict in `comparability` — a column absent
///   from the vector is treated exactly like an explicit `Incomparable` verdict
///   (fail-closed: absence of a proof is never trusted as a pass), matching
///   `Comparability::default()`'s own fail-closed convention.
///
/// `group_columns` is the cell's own mutation-sensitive column group (the
/// caller resolves this from the same derived plan the cell came from —
/// e.g. `ColumnGroup::columns` matching `PlanCell::group`'s display name);
/// an empty group has nothing to compare and refuses.
pub fn resolve_write_suppression(
    group_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
) -> WriteSuppression {
    if matches!(row_identity.identity, RowIdentity::WholeRow) {
        return WriteSuppression::Unconditional {
            why: "no proven row identity (P2 verdict is WholeRow) — a conditional write cannot \
                  safely address individual rows to compare, so the matched arm falls back to \
                  unconditional rewrite"
                .to_string(),
        };
    }

    if group_columns.is_empty() {
        return WriteSuppression::Unconditional {
            why: "the cell's column group is empty — there is nothing to compare".to_string(),
        };
    }

    let incomparable: Vec<String> = group_columns
        .iter()
        .filter(|col| {
            let verdict = comparability
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(col));
            match verdict {
                Some(c) => c.comparability == Comparability::Incomparable,
                // Fail-closed: no proof at all for this column is never
                // trusted as a pass.
                None => true,
            }
        })
        .cloned()
        .collect();

    if incomparable.is_empty() {
        WriteSuppression::Suppressed {
            compared_columns: group_columns.to_vec(),
        }
    } else {
        WriteSuppression::Unconditional {
            why: format!(
                "column(s) {} are not proven comparable across runs (P3) — the conditional \
                 write refuses fail-closed and falls back to the unconditional matched-arm \
                 rewrite",
                incomparable.join(", ")
            ),
        }
    }
}

/// Which physical write mechanism realizes a keyed-fold cell's conditional
/// write (T1/T2, `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase C5): the ordinary keyed `MERGE`
/// ([`super::emit::emit_keyed_fold`]/[`super::emit::
/// emit_keyed_fold_suppressed`]) on a backend that can run `MERGE` at all,
/// else the merge-less **staged-candidate conditional DELETE+INSERT**
/// ([`super::emit::emit_staged_candidate_conditional`]) — the keyed-shaped
/// realisation for a backend without `MERGE` (a documented gap:
/// Spark-over-Parquet). `MERGE` is preferred whenever the backend has it;
/// the staged-candidate mechanism is never a silent substitute on a backend
/// that *can* run `MERGE` (`docs/specs/model_transforms.md` §"Change-
/// suppressed MERGE and the staged-candidate conditional DELETE+INSERT").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyedWriteMechanism {
    /// The keyed `MERGE`, carrying its own resolved [`WriteSuppression`]
    /// verdict (suppressed or the plain unconditional matched arm).
    Merge(WriteSuppression),
    /// The merge-less staged-candidate conditional write, over exactly
    /// `compared_columns` — only ever produced when [`resolve_write_suppression`]
    /// proved the group fully comparable over a proven row identity (this
    /// phase's staged-candidate emitter has no unconditional shape, so an
    /// `Unconditional` verdict on a `MERGE`-less backend cannot resolve to
    /// this mechanism — see [`resolve_keyed_write_mechanism`]'s doc comment
    /// for the fallback that case requires from the caller).
    StagedCandidate { compared_columns: Vec<String> },
}

/// Resolve which mechanism realizes a keyed-fold cell's write, given its
/// already-resolved [`WriteSuppression`] verdict
/// ([`resolve_write_suppression`]) and whether the target backend can run
/// `MERGE` at all.
///
/// `None` means neither mechanism this function knows about is admissible:
/// the backend cannot run `MERGE`, and the compare group was not fully
/// comparable (or no row identity was proven) — `WriteSuppression::
/// Unconditional`. There is no merge-less *unconditional* keyed-fold emitter
/// in this catalogue (the staged-candidate shape's `DELETE`+`INSERT` only
/// makes sense restricted to the rows whose effect is not the identity —
/// see [`super::emit::emit_staged_candidate_conditional`]'s panic contract);
/// a caller reaching `None` must fall back to a backend-agnostic mechanism
/// outside this function's scope (e.g. the always-available whole-region
/// recompute, [`ChosenTechnique::RegionRecompute`]), never invent a
/// merge-less unconditional MERGE substitute.
///
/// This does not yet consult a `write:` pin (the open write-pattern
/// registry and `maintenance.cells[].write` — tracked by a later phase);
/// today it always prefers `MERGE` when the backend has it, matching
/// "never a silent substitution" — the staged-candidate mechanism is
/// reachable *only* through a genuine capability gap, not a preference.
pub fn resolve_keyed_write_mechanism(
    suppression: &WriteSuppression,
    backend_supports_merge: bool,
) -> Option<KeyedWriteMechanism> {
    if backend_supports_merge {
        return Some(KeyedWriteMechanism::Merge(suppression.clone()));
    }
    match suppression {
        WriteSuppression::Suppressed { compared_columns } => {
            Some(KeyedWriteMechanism::StagedCandidate {
                compared_columns: compared_columns.clone(),
            })
        }
        WriteSuppression::Unconditional { .. } => None,
    }
}

#[cfg(test)]
mod keyed_write_mechanism_tests {
    use super::*;

    fn suppressed() -> WriteSuppression {
        WriteSuppression::Suppressed {
            compared_columns: vec!["event_count".to_string()],
        }
    }

    fn unconditional() -> WriteSuppression {
        WriteSuppression::Unconditional {
            why: "column(s) notes are not proven comparable".to_string(),
        }
    }

    #[test]
    fn merge_capable_backend_always_resolves_to_merge_never_staged_candidate() {
        // Even a fully-comparable group stays on MERGE when the backend
        // can run one — the staged-candidate mechanism is never a silent
        // substitute for a MERGE the backend could have executed.
        let resolved = resolve_keyed_write_mechanism(&suppressed(), true);
        assert_eq!(resolved, Some(KeyedWriteMechanism::Merge(suppressed())));

        let resolved_unconditional = resolve_keyed_write_mechanism(&unconditional(), true);
        assert_eq!(
            resolved_unconditional,
            Some(KeyedWriteMechanism::Merge(unconditional()))
        );
    }

    #[test]
    fn merge_less_backend_with_comparable_group_admits_staged_candidate() {
        let resolved = resolve_keyed_write_mechanism(&suppressed(), false);
        assert_eq!(
            resolved,
            Some(KeyedWriteMechanism::StagedCandidate {
                compared_columns: vec!["event_count".to_string()]
            })
        );
    }

    #[test]
    fn merge_less_backend_with_no_admissible_suppression_resolves_to_none() {
        // Fail-closed: no merge-less unconditional keyed-fold mechanism
        // exists in this catalogue — the caller must fall back further
        // (e.g. region recompute), never invent a substitute here.
        let resolved = resolve_keyed_write_mechanism(&unconditional(), false);
        assert_eq!(resolved, None);
    }
}

#[cfg(test)]
mod write_suppression_tests {
    use super::*;
    use crate::maintenance::RowIdentity;

    fn key_identity(cols: &[&str]) -> RowIdentityVerdict {
        RowIdentityVerdict {
            identity: RowIdentity::Key(cols.iter().map(|s| s.to_string()).collect()),
            proven_mismatch: None,
        }
    }

    fn comparable(col: &str) -> ColumnComparability {
        ColumnComparability {
            output: col.to_string(),
            comparability: Comparability::Comparable,
        }
    }

    fn incomparable(col: &str) -> ColumnComparability {
        ColumnComparability {
            output: col.to_string(),
            comparability: Comparability::Incomparable,
        }
    }

    #[test]
    fn fully_comparable_group_admits_suppression() {
        let group = vec!["tier".to_string(), "email".to_string()];
        let comparability = vec![comparable("tier"), comparable("email")];
        let identity = key_identity(&["id"]);

        let resolved = resolve_write_suppression(&group, &comparability, &identity);
        assert_eq!(
            resolved,
            WriteSuppression::Suppressed {
                compared_columns: group.clone()
            }
        );
    }

    #[test]
    fn one_incomparable_column_refuses_named() {
        let group = vec!["tier".to_string(), "notes".to_string()];
        let comparability = vec![comparable("tier"), incomparable("notes")];
        let identity = key_identity(&["id"]);

        let resolved = resolve_write_suppression(&group, &comparability, &identity);
        match resolved {
            WriteSuppression::Unconditional { why } => {
                assert!(
                    why.contains("notes"),
                    "refusal reason must name the incomparable column; got: {why}"
                );
            }
            other => panic!("expected Unconditional refusal, got {other:?}"),
        }
    }

    #[test]
    fn column_missing_from_comparability_vector_fails_closed() {
        // No proof at all for 'tier' — absence must not be trusted as a pass.
        let group = vec!["tier".to_string()];
        let comparability: Vec<ColumnComparability> = vec![];
        let identity = key_identity(&["id"]);

        let resolved = resolve_write_suppression(&group, &comparability, &identity);
        match resolved {
            WriteSuppression::Unconditional { why } => assert!(why.contains("tier")),
            other => panic!("expected Unconditional refusal, got {other:?}"),
        }
    }

    #[test]
    fn whole_row_identity_refuses_regardless_of_comparability() {
        let group = vec!["tier".to_string()];
        let comparability = vec![comparable("tier")];
        let identity = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };

        let resolved = resolve_write_suppression(&group, &comparability, &identity);
        match resolved {
            WriteSuppression::Unconditional { why } => {
                assert!(why.contains("row identity") || why.contains("WholeRow"));
            }
            other => panic!("expected Unconditional refusal, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance::{Corner, PartitionLocal, PlanCell};
    use smelt_core::config::MaintenanceCellConfig;

    fn admitted_plan(source: &str, technique: Technique, corner: Corner) -> MaintenancePlan {
        MaintenancePlan {
            cells: vec![PlanCell {
                group: "{tier}".to_string(),
                trigger: Trigger::UpstreamMutation {
                    source: source.to_string(),
                },
                corner,
                technique,
                partition_local: PartitionLocal::Yes,
                scans: vec![],
                ledger_catch_up: false,
                row_identity: crate::maintenance::RowIdentityVerdict {
                    identity: crate::maintenance::RowIdentity::WholeRow,
                    proven_mismatch: None,
                },
            }],
            refusals: vec![],
            key_locality: None,
        }
    }

    #[test]
    fn pin_bypasses_cost_model_but_not_admission() {
        let plan = admitted_plan("users", Technique::ColumnScopedMerge, Corner::ColumnMerge);
        let trigger = Trigger::UpstreamMutation {
            source: "users".to_string(),
        };

        // A pin naming the admitted technique succeeds, bypassing whatever
        // the cost model would otherwise have chosen.
        let overrides = EffectiveOverride {
            prefer: None,
            technique: Some(CellTechnique::RederiveColumns),
        };
        let resolved = resolve_cell_choice(&plan, &trigger, &overrides, true)
            .expect("pin naming the admitted technique must resolve");
        assert_eq!(
            resolved,
            ChosenTechnique::Admitted(Technique::ColumnScopedMerge)
        );

        // Pinning a technique the plan did NOT admit for this cell (a keyed
        // fold, when the cell only admits column-scoped merge) is a hard
        // error, never a silent override.
        let bad_overrides = EffectiveOverride {
            prefer: None,
            technique: Some(CellTechnique::Fold),
        };
        let err = resolve_cell_choice(&plan, &trigger, &bad_overrides, true)
            .expect_err("pinning an unadmitted technique must refuse");
        assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));

        // Pinning `rederive_columns` when the backend cannot run it is the
        // same refusal shape — a capability gap is indistinguishable from
        // an unadmitted cell.
        let err2 = resolve_cell_choice(&plan, &trigger, &overrides, false)
            .expect_err("pin naming a capability-gapped backend must refuse");
        assert!(err2.to_string().contains("MaintenanceUnboundedFootprint"));

        // `recompute` is always in the resolvable set — pinning it always
        // succeeds, admitted or not.
        let recompute_overrides = EffectiveOverride {
            prefer: None,
            technique: Some(CellTechnique::Recompute),
        };
        let resolved = resolve_cell_choice(&plan, &trigger, &recompute_overrides, true)
            .expect("recompute is always resolvable");
        assert_eq!(resolved, ChosenTechnique::RegionRecompute);
    }

    #[test]
    fn unadmitted_cell_pin_refuses() {
        // No cell at all for this trigger (the plan refused it upstream).
        let plan = MaintenancePlan::default();
        let trigger = Trigger::UpstreamMutation {
            source: "users".to_string(),
        };
        let overrides = EffectiveOverride {
            prefer: None,
            technique: Some(CellTechnique::RederiveColumns),
        };
        let err = resolve_cell_choice(&plan, &trigger, &overrides, true)
            .expect_err("a pin naming a cell the plan never admitted must refuse");
        assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));

        // Absent a pin, the safe default resolves with no error.
        let resolved = resolve_cell_choice(&plan, &trigger, &EffectiveOverride::default(), true)
            .expect("no pin + unadmitted cell must fall back safely, not error");
        assert_eq!(resolved, ChosenTechnique::RegionRecompute);
    }

    fn cell_cfg(
        on: &str,
        columns: &[&str],
        prefer: Option<TechniquePreference>,
        technique: Option<CellTechnique>,
    ) -> MaintenanceCellConfig {
        MaintenanceCellConfig {
            columns: columns.iter().map(|s| s.to_string()).collect(),
            on: on.to_string(),
            prefer,
            technique,
        }
    }

    #[test]
    fn ladder_narrower_scope_wins() {
        // `defaults.prefer: fold` is the broad default; a `cells[]` entry
        // scoped to this exact cell prefers `recompute` instead — the
        // narrower scope must win.
        let defaults = MaintenanceDefaults {
            prefer: Some(TechniquePreference::Fold),
        };
        let cells = vec![cell_cfg(
            "sources.users",
            &["tier"],
            Some(TechniquePreference::Recompute),
            None,
        )];

        let effective = effective_override(
            Some(&defaults),
            &cells,
            "sources.users",
            &["tier".to_string()],
        );
        assert_eq!(effective.prefer, Some(TechniquePreference::Recompute));

        // A cell with no matching `cells[]` entry falls back to the broad
        // default.
        let effective_unmatched = effective_override(
            Some(&defaults),
            &cells,
            "sources.other",
            &["other_col".to_string()],
        );
        assert_eq!(effective_unmatched.prefer, Some(TechniquePreference::Fold));

        // A `cells[].technique` hard pin coexists with — and, since it's
        // even narrower, wins the same way over — a `cells[].prefer` soft
        // bias on the same entry.
        let cells_with_pin = vec![cell_cfg(
            "sources.users",
            &["tier"],
            Some(TechniquePreference::Recompute),
            Some(CellTechnique::RederiveColumns),
        )];
        let effective_pin = effective_override(
            Some(&defaults),
            &cells_with_pin,
            "sources.users",
            &["tier".to_string()],
        );
        assert_eq!(
            effective_pin.technique,
            Some(CellTechnique::RederiveColumns)
        );

        // End-to-end: the ladder's resolved override feeds
        // `resolve_cell_choice` and actually changes the outcome versus the
        // broad default alone.
        let plan = admitted_plan(
            "sources.users",
            Technique::ColumnScopedMerge,
            Corner::ColumnMerge,
        );
        let trigger = Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        };
        let resolved = resolve_cell_choice(&plan, &trigger, &effective, true)
            .expect("recompute is always resolvable");
        assert_eq!(resolved, ChosenTechnique::RegionRecompute);
    }
}
