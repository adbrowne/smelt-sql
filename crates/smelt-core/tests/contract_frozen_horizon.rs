//! Phase 2 (`docs/outcomes/20260809-contract-lattice-v1/phases/02-plan.md`):
//! `contract.frozen_horizon` frontmatter parsing — format validity and the
//! fail-loud staging of `deferral` until phase 4.

use smelt_core::metadata::{extract_file_metadata, FileMetadata, MetadataError};

#[test]
fn frozen_horizon_parses_into_metadata() {
    let sql = "---\ncontract:\n  frozen_horizon: '90 days'\n---\nSELECT 1";
    let metadata = match extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => *metadata,
        other => panic!("expected Single metadata, got {other:?}"),
    };
    let contract = metadata.contract.expect("contract must be Some");
    let frozen_horizon = contract
        .frozen_horizon
        .expect("frozen_horizon must be Some");
    assert_eq!(frozen_horizon.seconds, 90 * 86400);
}

#[test]
fn unparseable_frozen_horizon_is_a_metadata_error() {
    let sql = "---\ncontract:\n  frozen_horizon: '90 fortnights'\n---\nSELECT 1";
    let err = extract_file_metadata(sql).expect_err("must be a fail-loud error, not Ok");
    assert!(
        matches!(err, MetadataError::ContractFrozenHorizonInvalid { .. }),
        "expected ContractFrozenHorizonInvalid, got {err:?}"
    );
}

#[test]
fn deferral_key_is_refused_until_wired() {
    let sql = "---\ncontract:\n  deferral: '6 hours'\n---\nSELECT 1";
    let err =
        extract_file_metadata(sql).expect_err("deferral must be refused, not silently accepted");
    let message = err.to_string();
    assert!(
        message.contains("deferral"),
        "unknown-key error must name `deferral`, got: {message}"
    );
}
