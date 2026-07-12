//! Technique choice among admissible alternatives — the override ladder
//! (`defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
//! scope winning) plus the cost-model hook `smelt bakeoff` measures into
//! (`maintenance_plan.md` §Surface "Frontmatter", §Semantics
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
//! (`maintenance_plan.md` §Semantics "The plan matrix"). This module treats
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

use super::{MaintenancePlan, Technique, Trigger};

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
/// (`maintenance_plan.md` §Surface "Frontmatter").
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
/// (`maintenance_plan.md` §Surface "Frontmatter": "The override ladder is
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
/// Mirrors `maintenance_plan.md` §"Per-cell admission": a `technique:` pin
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
    let live = admitted_technique.is_some_and(|t| match t {
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
        // infallible: `live` is computed via `admitted_technique.is_some_and(..)`
        // above — `Option::is_some_and` only evaluates its closure (and can
        // only return true) when the receiver is `Some`, so `live == true`
        // structurally implies `admitted_technique.is_some()` regardless of
        // what the closure itself decides.
        Some(TechniquePreference::Fold) if live => Ok(ChosenTechnique::Admitted(
            admitted_technique.expect("live implies Some").clone(),
        )),
        _ if live => Ok(ChosenTechnique::Admitted(
            admitted_technique.expect("live implies Some").clone(),
        )),
        _ => Ok(ChosenTechnique::RegionRecompute),
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
            }],
            refusals: vec![],
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
