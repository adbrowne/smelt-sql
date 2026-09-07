use super::*;
use crate::maintenance::{RowIdentity, Trigger};

fn key_identity(cols: &[&str]) -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::Key(cols.iter().map(|s| s.to_string()).collect()),
        proven_mismatch: None,
    }
}

fn whole_row_identity() -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    }
}

fn comparable(col: &str) -> ColumnComparability {
    ColumnComparability {
        output: col.to_string(),
        comparability: Comparability::Comparable,
    }
}

#[test]
fn region_write_variant_suppresses_over_a_proven_key_and_comparable_group() {
    let group = vec!["amount".to_string()];
    let comparability = vec![comparable("amount")];
    let identity = key_identity(&["region_id"]);
    let trigger = Trigger::NewData {
        source: "sources.payments".to_string(),
    };

    let resolved = resolve_region_write_variant(
        &group,
        &comparability,
        &identity,
        &trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("proven key + comparable group over a steady-state trigger admits suppression");
    assert_eq!(
        resolved,
        RegionWrite::Suppressed {
            key: vec!["region_id".to_string()],
            compared_columns: vec!["amount".to_string()],
        }
    );
}

#[test]
fn region_write_variant_is_unconditional_without_a_proven_key() {
    let group = vec!["amount".to_string()];
    let comparability = vec![comparable("amount")];
    let identity = whole_row_identity();
    let trigger = Trigger::NewData {
        source: "sources.payments".to_string(),
    };

    let resolved = resolve_region_write_variant(
        &group,
        &comparability,
        &identity,
        &trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert!(matches!(resolved, RegionWrite::Unconditional { .. }));
}

#[test]
fn region_write_variant_is_unconditional_on_a_first_build_trigger() {
    let group = vec!["amount".to_string()];
    let comparability = vec![comparable("amount")];
    let identity = key_identity(&["region_id"]);

    let resolved = resolve_region_write_variant(
        &group,
        &comparability,
        &identity,
        &Trigger::Backfill,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert!(matches!(resolved, RegionWrite::Unconditional { .. }));
}

#[test]
fn region_write_variant_propagates_a_refused_suppress_pin() {
    // No proven row identity — the P2/P3 proof itself refuses. A
    // `technique: suppress` pin cannot force suppression on over that,
    // and must surface as a hard `Err`, never a silent `Unconditional`.
    let group = vec!["amount".to_string()];
    let comparability = vec![comparable("amount")];
    let identity = whole_row_identity();
    let trigger = Trigger::NewData {
        source: "sources.payments".to_string(),
    };
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Suppress),
    };

    let err = resolve_region_write_variant(
        &group,
        &comparability,
        &identity,
        &trigger,
        false,
        &overrides,
    )
    .expect_err("pinning suppression over a refused P2/P3 proof must refuse");
    assert_eq!(
        err.pinned,
        PinnedRequest::Technique(CellTechnique::Suppress)
    );
}
