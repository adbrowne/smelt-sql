//! The state-structure half of the contract lattice's fail-loud rule
//! (`docs/specs/state.md` §"Declarations stay fail-loud"): which declared
//! contract point requires which [`StateStructure`], and whether a given
//! [`StateAvailability`] can honour it. Pure — no I/O, no posture/backend
//! resolution; the caller (`smelt-db`'s `check_file_diagnostics`) resolves
//! the effective posture and passes the already-computed availability.
//!
//! `frozen_horizon` is deliberately absent from [`required_state_structures`]:
//! its probe baseline is observability-class and degrades with
//! `ProbeBaselineUnavailable` rather than refusing
//! (`docs/outcomes/20260816-state-residency/outcome.md` phase 1 decision
//! log). `deferral` is the one point whose semantics *are* a statement
//! about state — its lag is measured against the interval ledger and
//! landed-delta record, so a declaration without that structure is an
//! unmeasurable promise, not a degradable one.

use smelt_core::config::ContractConfig;

use crate::maintenance::availability::{StateAvailability, StateStructure};

/// One declared contract point that requires a state structure this
/// [`ContractConfig`] cannot supply, resolved against a caller's
/// [`StateAvailability`] — the `DeclaredContractRequiresState` diagnostic's
/// own data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractStateRefusal {
    /// Names the declaration: `"contract.deferral"` for the model-level
    /// default, `"contract.cells[on: <address>].deferral"` for a
    /// cell-level override.
    pub declaration: String,
    pub missing_structure: StateStructure,
    pub why: String,
}

/// One declared contract point plus the state structure its semantics
/// require, independent of any [`StateAvailability`] — the "ideal"
/// requirement [`validate_contract_state`] checks against a specific
/// availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRequirement {
    pub declaration: String,
    pub structure: StateStructure,
}

/// Every state structure `cfg`'s declared contract points require,
/// independent of any backend or posture. `frozen_horizon` never appears
/// (see module docs); a model-level `deferral` yields one requirement
/// named `"contract.deferral"`, and each `contract.cells[]` entry with its
/// own `deferral` yields one more, named by its `on:` address.
pub fn required_state_structures(cfg: &ContractConfig) -> Vec<StateRequirement> {
    let mut requirements = Vec::new();
    if cfg.deferral.is_some() {
        requirements.push(StateRequirement {
            declaration: "contract.deferral".to_string(),
            structure: StateStructure::IntervalFrontier,
        });
    }
    for cell in &cfg.cells {
        if cell.deferral.is_some() {
            requirements.push(StateRequirement {
                declaration: format!("contract.cells[on: {}].deferral", cell.on),
                structure: StateStructure::IntervalFrontier,
            });
        }
    }
    requirements
}

/// Resolve `cfg`'s [`required_state_structures`] against `availability`,
/// returning one [`ContractStateRefusal`] per requirement `availability`
/// cannot supply. Empty when every requirement is available (including the
/// vacuous case: no `deferral` declared at all).
pub fn validate_contract_state(
    cfg: &ContractConfig,
    availability: &StateAvailability,
) -> Vec<ContractStateRefusal> {
    required_state_structures(cfg)
        .into_iter()
        .filter_map(|req| {
            let available = match req.structure {
                StateStructure::IntervalFrontier => availability.interval_frontier,
                StateStructure::ReconciliationLedger => availability.reconciliation_ledger,
                StateStructure::FrontierRecord => availability.frontier_record,
            };
            if available {
                return None;
            }
            Some(ContractStateRefusal {
                declaration: req.declaration.clone(),
                missing_structure: req.structure,
                why: format!(
                    "{} promises a lag measured against the interval ledger and landed-delta \
                     record, which the effective posture or target backend does not supply",
                    req.declaration
                ),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::{ContractCellConfig, DataLatency};

    fn cfg_with_model_deferral() -> ContractConfig {
        ContractConfig {
            frozen_horizon: None,
            deferral: DataLatency::parse("6 hours"),
            cells: vec![],
        }
    }

    #[test]
    fn deferral_requires_the_interval_frontier() {
        let reqs = required_state_structures(&cfg_with_model_deferral());
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].declaration, "contract.deferral");
        assert_eq!(reqs[0].structure, StateStructure::IntervalFrontier);
    }

    #[test]
    fn cell_deferral_requires_the_interval_frontier() {
        let cfg = ContractConfig {
            frozen_horizon: None,
            deferral: None,
            cells: vec![ContractCellConfig {
                columns: vec!["amount".to_string()],
                on: "sources.raw.events".to_string(),
                deferral: DataLatency::parse("1 day"),
            }],
        };
        let reqs = required_state_structures(&cfg);
        assert_eq!(reqs.len(), 1);
        assert_eq!(
            reqs[0].declaration,
            "contract.cells[on: sources.raw.events].deferral"
        );
        assert_eq!(reqs[0].structure, StateStructure::IntervalFrontier);
    }

    #[test]
    fn frozen_horizon_requires_no_state() {
        let cfg = ContractConfig {
            frozen_horizon: DataLatency::parse("90 days"),
            deferral: None,
            cells: vec![],
        };
        assert!(required_state_structures(&cfg).is_empty());
    }

    #[test]
    fn available_structure_yields_no_refusal() {
        let refusals =
            validate_contract_state(&cfg_with_model_deferral(), &StateAvailability::all());
        assert!(refusals.is_empty());
    }

    #[test]
    fn absent_structure_refuses_naming_declaration_and_structure() {
        let refusals =
            validate_contract_state(&cfg_with_model_deferral(), &StateAvailability::none());
        assert_eq!(refusals.len(), 1);
        assert_eq!(refusals[0].declaration, "contract.deferral");
        assert_eq!(
            refusals[0].missing_structure,
            StateStructure::IntervalFrontier
        );
        assert!(refusals[0].why.contains("contract.deferral"));
    }
}
