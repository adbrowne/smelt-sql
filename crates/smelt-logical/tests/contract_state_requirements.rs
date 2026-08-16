//! Phase 6 (`docs/outcomes/20260816-state-residency/phases/06-plan.md`):
//! `required_state_structures`/`validate_contract_state`
//! (`crates/smelt-logical/src/contract/state_requirements.rs`) — the pure
//! oracle behind `DeclaredContractRequiresState`
//! (`docs/specs/state.md` §"Declarations stay fail-loud").

use smelt_core::config::{ContractCellConfig, ContractConfig, DataLatency};
use smelt_logical::contract::state_requirements::{
    required_state_structures, validate_contract_state,
};
use smelt_logical::maintenance::availability::{StateAvailability, StateStructure};

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
    let refusals = validate_contract_state(&cfg_with_model_deferral(), &StateAvailability::all());
    assert!(refusals.is_empty());
}

#[test]
fn absent_structure_refuses_naming_declaration_and_structure() {
    let refusals = validate_contract_state(&cfg_with_model_deferral(), &StateAvailability::none());
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].declaration, "contract.deferral");
    assert_eq!(
        refusals[0].missing_structure,
        StateStructure::IntervalFrontier
    );
    assert!(refusals[0].why.contains("contract.deferral"));
}
