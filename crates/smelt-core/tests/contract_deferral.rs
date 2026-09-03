//! Phase 4 (`docs/outcomes/20260809-contract-lattice-v1/phases/04-plan.md`):
//! `contract.deferral` and `contract.cells[]` frontmatter parsing — format
//! validity, disambiguated from `contract.frozen_horizon`'s own dedicated
//! error.

use smelt_core::metadata::{extract_file_metadata, FileMetadata, MetadataError};

#[test]
fn contract_deferral_parses() {
    let sql = "---\ncontract:\n  deferral: '6 hours'\n---\nSELECT 1";
    let metadata = match extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => *metadata,
        other => panic!("expected Single metadata, got {other:?}"),
    };
    let contract = metadata.contract.expect("contract must be Some");
    let deferral = contract.deferral.expect("deferral must be Some");
    assert_eq!(deferral.seconds, 6 * 3600);
}

#[test]
fn contract_cells_parse() {
    let sql = "---\ncontract:\n  cells:\n    - columns: [revenue]\n      on: 'orders'\n      deferral: '1 day'\n---\nSELECT 1";
    let metadata = match extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => *metadata,
        other => panic!("expected Single metadata, got {other:?}"),
    };
    let contract = metadata.contract.expect("contract must be Some");
    assert_eq!(contract.cells.len(), 1);
    let cell = &contract.cells[0];
    assert_eq!(cell.columns, vec!["revenue".to_string()]);
    assert_eq!(cell.on, "orders");
    assert_eq!(
        cell.deferral
            .as_ref()
            .expect("cell deferral must be Some")
            .seconds,
        86400
    );
}

#[test]
fn contract_deferral_unparseable_is_named_error() {
    let sql = "---\ncontract:\n  deferral: 'soonish'\n---\nSELECT 1";
    let err = extract_file_metadata(sql).expect_err("must be a fail-loud error, not Ok");
    assert!(
        matches!(err, MetadataError::ContractDeferralInvalid { .. }),
        "expected ContractDeferralInvalid, got {err:?}"
    );
}

#[test]
fn contract_cells_deferral_unparseable_is_named_error() {
    let sql = "---\ncontract:\n  cells:\n    - columns: [revenue]\n      on: 'orders'\n      deferral: 'soonish'\n---\nSELECT 1";
    let err = extract_file_metadata(sql).expect_err("must be a fail-loud error, not Ok");
    assert!(
        matches!(err, MetadataError::ContractDeferralInvalid { .. }),
        "expected ContractDeferralInvalid, got {err:?}"
    );
}

#[test]
fn frozen_horizon_unparseable_is_not_misattributed_to_deferral() {
    let sql =
        "---\ncontract:\n  frozen_horizon: '90 fortnights'\n  deferral: '6 hours'\n---\nSELECT 1";
    let err = extract_file_metadata(sql).expect_err("must be a fail-loud error, not Ok");
    assert!(
        matches!(err, MetadataError::ContractFrozenHorizonInvalid { .. }),
        "expected ContractFrozenHorizonInvalid, got {err:?}"
    );
}

#[test]
fn retain_departed_malformed_raises_contract_error() {
    let sql = "---\ncontract:\n  retain_departed: 'yes'\n---\nSELECT 1";
    let err = extract_file_metadata(sql).expect_err("must be a fail-loud error, not Ok");
    assert!(
        matches!(err, MetadataError::ContractRetainDepartedInvalid { .. }),
        "expected ContractRetainDepartedInvalid, got {err:?}"
    );
}

#[test]
fn retain_departed_parses_both_forms() {
    let sql = "---\ncontract:\n  retain_departed: true\n---\nSELECT 1";
    let metadata = match extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => *metadata,
        other => panic!("expected Single metadata, got {other:?}"),
    };
    let contract = metadata.contract.expect("contract must be Some");
    assert_eq!(
        contract.retain_departed,
        Some(smelt_core::config::RetainDeparted::Bool(true))
    );

    let sql = "---\ncontract:\n  retain_departed:\n    tombstone: is_departed\n---\nSELECT 1";
    let metadata = match extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => *metadata,
        other => panic!("expected Single metadata, got {other:?}"),
    };
    let contract = metadata.contract.expect("contract must be Some");
    assert_eq!(
        contract.retain_departed,
        Some(smelt_core::config::RetainDeparted::Tombstone {
            tombstone: "is_departed".to_string()
        })
    );
}
