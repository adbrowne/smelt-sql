//! The degradation contract's second step (`docs/specs/state.md` §"The
//! degradation contract"): a pure resolution pass over an already-derived
//! [`MaintenancePlan`] that downgrades a cell whose technique needs a state
//! structure the target backend cannot build, recording the downgrade as an
//! advisory [`StateDowngrade`] rather than silently running the ideal
//! technique or refusing outright. The ideal plan itself is never mutated —
//! a caller keeps both (`state.md`: "the ideal plan must exist as a derived
//! object even when it will not run").

use super::{MaintenancePlan, Refusal, Technique};

/// A state structure a maintenance technique may depend on
/// (`docs/specs/state.md` §"The reconciliation ledger").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateStructure {
    /// The reconciliation ledger's additive grading (`_smelt_ledger`) —
    /// [`Technique::KeyedFold`]'s never-fold-twice premise. Losing this is a
    /// correctness premise, not bookkeeping: a cell that needs it and has no
    /// admissible fallback is dropped with a refusal, never silently run.
    ReconciliationLedger,
    /// The reconciliation ledger's frontier record (`_smelt_frontier`) —
    /// bookkeeping only. [`Technique::DeleteInsert`]'s region recompute is
    /// self-certifying (it always writes what it read); losing the frontier
    /// record loses only the never-fold-twice audit trail, never
    /// correctness, so the technique is unaffected and the downgrade is
    /// advisory.
    FrontierRecord,
    /// The run's interval ledger and landed-delta record
    /// (`docs/specs/run_state.md` §"Interval ledger", §"Relationship to the
    /// reconciliation ledger") — observability-class, withheld under
    /// `state.mode: stateless` (`docs/specs/state.md` §"`state.mode` and
    /// what each posture provides"). `contract.deferral`'s lag is measured
    /// against this pair, not against [`FrontierRecord`]; losing it makes
    /// the declared lag unmeasurable, so a declaration that needs it is
    /// refused fail-loud rather than downgraded
    /// (`docs/specs/state.md` §"Declarations stay fail-loud").
    IntervalFrontier,
}

/// Per-structure availability on a target backend
/// (`docs/specs/state.md` §"`state.mode` and what each posture provides").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateAvailability {
    pub reconciliation_ledger: bool,
    pub frontier_record: bool,
    /// The run's interval ledger and landed-delta record — withheld under
    /// `state.mode: stateless` regardless of backend
    /// (`docs/specs/state.md` §"`state.mode` and what each posture
    /// provides"). Unlike the other two fields this is a posture property,
    /// not a backend-capability one; [`StateStructure::IntervalFrontier`]
    /// is consulted only by contract-declaration validation, never by
    /// [`resolve_state_availability`]'s plan-derivation downgrade.
    pub interval_frontier: bool,
}

impl StateAvailability {
    /// Every structure available — the value a caller that does not (yet)
    /// know its target backend passes. [`resolve_state_availability`] with
    /// this value downgrades zero cells: byte-identical to not resolving at
    /// all.
    pub fn all() -> Self {
        Self {
            reconciliation_ledger: true,
            frontier_record: true,
            interval_frontier: true,
        }
    }

    /// No structure available — the ledger-less/frontier-less backend
    /// (every dialect but DuckDB today), under `state.mode: stateless`.
    pub fn none() -> Self {
        Self {
            reconciliation_ledger: false,
            frontier_record: false,
            interval_frontier: false,
        }
    }
}

/// The state structure `technique` depends on, if any.
pub fn required_state_structure(technique: Technique) -> Option<StateStructure> {
    match technique {
        Technique::KeyedFold => Some(StateStructure::ReconciliationLedger),
        Technique::DeleteInsert => Some(StateStructure::FrontierRecord),
        Technique::ColumnScopedMerge | Technique::InPlaceUpdate | Technique::PerGroupRecompute => {
            None
        }
    }
}

/// One cell's recorded downgrade — the `MaintenanceStateDowngraded`
/// diagnostic's own data (`docs/specs/diagnostics.md` §"Maintenance").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDowngrade {
    pub cell_group: String,
    pub trigger: String,
    pub ideal_technique: Technique,
    pub resolved_technique: Technique,
    pub missing_structure: StateStructure,
    pub why: String,
}

/// The plan resolved against `availability`: `plan.cells` is [`MaintenancePlan::cells`]
/// with every downgraded cell rewritten to its resolved technique (and, for
/// a keyed fold with no admissible fallback, dropped in favour of a pushed
/// [`Refusal::NoAdmissibleTechnique`]); `downgrades` names every cell that
/// moved, advisory or not.
#[derive(Debug, Clone)]
pub struct ResolvedPlan {
    pub plan: MaintenancePlan,
    pub downgrades: Vec<StateDowngrade>,
}

/// Resolve `ideal` against `availability` (`docs/specs/state.md` §"The
/// degradation contract"). Pure, and total over every technique
/// [`required_state_structure`] names — never panics, never silently drops a
/// cell without a refusal.
pub fn resolve_state_availability(
    ideal: &MaintenancePlan,
    availability: &StateAvailability,
) -> ResolvedPlan {
    let mut resolved = ideal.clone();
    let mut downgrades = Vec::new();
    let mut kept_cells = Vec::with_capacity(resolved.cells.len());

    for mut cell in resolved.cells.drain(..) {
        let Some(structure) = required_state_structure(cell.technique) else {
            kept_cells.push(cell);
            continue;
        };
        let available = match structure {
            StateStructure::ReconciliationLedger => availability.reconciliation_ledger,
            StateStructure::FrontierRecord => availability.frontier_record,
            // `required_state_structure` never yields this variant — no
            // `Technique` depends on it, only a declared contract point.
            StateStructure::IntervalFrontier => {
                unreachable!("no Technique requires the interval/landed-delta frontier")
            }
        };
        if available {
            kept_cells.push(cell);
            continue;
        }

        match structure {
            StateStructure::FrontierRecord => {
                // Bookkeeping only — the technique is unaffected, but the
                // loss is still reported so `smelt explain` never silently
                // drops it.
                downgrades.push(StateDowngrade {
                    cell_group: cell.group.clone(),
                    trigger: format!("{:?}", cell.trigger),
                    ideal_technique: cell.technique,
                    resolved_technique: cell.technique,
                    missing_structure: structure,
                    why: "no engine-resident frontier builder for this backend; the region \
                          recompute's frontier record is not recorded"
                        .to_string(),
                });
                kept_cells.push(cell);
            }
            StateStructure::ReconciliationLedger => {
                if let Some(fallback) = cell.recompute_fallback.clone() {
                    let ideal_technique = cell.technique;
                    downgrades.push(StateDowngrade {
                        cell_group: cell.group.clone(),
                        trigger: format!("{:?}", cell.trigger),
                        ideal_technique,
                        resolved_technique: fallback.technique,
                        missing_structure: structure,
                        why: "no reconciliation ledger on this backend; downgraded to the \
                              per-group recompute fallback"
                            .to_string(),
                    });
                    cell.technique = fallback.technique;
                    cell.scans = fallback.scans;
                    cell.key_scope = fallback.key_scope;
                    kept_cells.push(cell);
                } else {
                    // No admissible fallback was derived for this cell —
                    // never run the keyed fold anyway. Fail-loud, naming the
                    // missing structure, same as any other admission
                    // failure.
                    resolved.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{:?}", cell.trigger),
                        why: format!(
                            "cell '{}' requires the reconciliation ledger \
                             (Technique::KeyedFold), which the target backend cannot build, \
                             and no admissible recompute fallback was derived for it",
                            cell.group
                        ),
                    });
                }
            }
            StateStructure::IntervalFrontier => {
                unreachable!("no Technique requires the interval/landed-delta frontier")
            }
        }
    }

    resolved.cells = kept_cells;
    ResolvedPlan {
        plan: resolved,
        downgrades,
    }
}
