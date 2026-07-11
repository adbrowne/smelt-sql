//! Tests for D-53 scale-factor correctness:
//!   - effective_row_count uses floor() not round()
//!   - zero effective row count is a configuration error
//!   - FK generator bound equals the effective (scaled) row count at any scale factor
//!   - zero FK-target count is a configuration error, not a panic

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use smelt_datagen::config::{effective_row_count, FkCounts, GeneratorSpec};
use smelt_datagen::generic::{apply_spec, RowContext};
use std::collections::HashMap;

// ── effective_row_count ──────────────────────────────────────────────────────

#[test]
fn scale_factor_uses_floor_not_round() {
    // num_rows=1, scale_factor=0.9 → floor(0.9)=0 → error
    // With round(): round(0.9)=1 → Ok(1) — this distinguishes floor from round.
    let result = effective_row_count(1, 0.9);
    assert!(
        result.is_err(),
        "expected Err(zero-row) for num_rows=1, scale_factor=0.9 (floor=0), got Ok({})",
        result.unwrap()
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("effective row count is 0"),
        "error should mention zero row count; got: {msg}"
    );
}

#[test]
fn zero_effective_row_count_is_error() {
    // num_rows=1, scale_factor=0.4 → floor(0.4)=0 → error
    let result = effective_row_count(1, 0.4);
    assert!(
        result.is_err(),
        "expected Err for zero effective row count, got Ok"
    );
}

#[test]
fn normal_scale_produces_floor_value() {
    // num_rows=100, scale_factor=0.505 → floor(50.5)=50 (not round→51)
    let count = effective_row_count(100, 0.505).expect("should not be zero");
    assert_eq!(count, 50, "expected floor(100*0.505)=50, got {count}");
}

#[test]
fn scale_factor_one_passes_through() {
    let count = effective_row_count(42, 1.0).expect("scale=1.0 must succeed");
    assert_eq!(count, 42);
}

#[test]
fn scale_factor_greater_than_one_works() {
    // num_rows=100, scale_factor=2.0 → floor(200)=200
    let count = effective_row_count(100, 2.0).expect("scale>1 must succeed");
    assert_eq!(count, 200);
}

// ── FK generator bound at any scale ─────────────────────────────────────────

/// Helper: generate N FK values from a `ForeignKey { dataset: "devices" }` spec
/// with `fk_counts["devices"] = target_count`.
fn generate_fk_values(target_count: usize, n: usize) -> Vec<i32> {
    let spec = GeneratorSpec::ForeignKey {
        dataset: "devices".to_string(),
    };
    let mut fk: FkCounts = HashMap::new();
    fk.insert("devices".to_string(), target_count);
    let pools = HashMap::new();
    let pool_samples = HashMap::new();
    let ctx = RowContext {
        row_index: 0,
        fk_counts: &fk,
        pools: &pools,
        pool_samples: &pool_samples,
        row_so_far: &[],
        partition_col: None,
    };
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    (0..n)
        .map(
            |_| match apply_spec(&mut rng, &spec, &ctx).expect("fk generation ok") {
                smelt_datagen::generic::GenericValue::Int(v) => v,
                other => panic!("expected Int, got {other:?}"),
            },
        )
        .collect()
}

#[test]
fn fk_bound_at_scale_less_than_one() {
    // devices: num_rows=100, scale_factor=0.5 → effective=50
    // All FK values must be in [1, 50].
    let effective = effective_row_count(100, 0.5).unwrap(); // =50
    let values = generate_fk_values(effective, 500);
    assert!(
        values.iter().all(|&v| (1..=50).contains(&v)),
        "FK values out of [1,50]: {:?}",
        values
            .iter()
            .filter(|&&v| !(1..=50).contains(&v))
            .collect::<Vec<_>>()
    );
}

#[test]
fn fk_bound_at_scale_greater_than_one() {
    // devices: num_rows=100, scale_factor=2.0 → effective=200
    // All FK values must be in [1, 200].
    let effective = effective_row_count(100, 2.0).unwrap(); // =200
    let values = generate_fk_values(effective, 500);
    assert!(
        values.iter().all(|&v| (1..=200).contains(&v)),
        "FK values out of [1,200]: {:?}",
        values
            .iter()
            .filter(|&&v| !(1..=200).contains(&v))
            .collect::<Vec<_>>()
    );
}

// ── FK with zero target count is an error, not a panic ─────────────────────

#[test]
fn fk_zero_count_fallback_hardened() {
    // After hardening, apply_spec with a FK whose target has count=0 should
    // return Err rather than % 0 panic (current code: unwrap_or(1) masks this).
    // This test documents the desired behavior — the hardened path returns Err.
    let spec = GeneratorSpec::ForeignKey {
        dataset: "devices".to_string(),
    };
    let mut fk: FkCounts = HashMap::new();
    fk.insert("devices".to_string(), 0usize); // zero count — must error, not panic
    let pools = HashMap::new();
    let pool_samples = HashMap::new();
    let ctx = RowContext {
        row_index: 0,
        fk_counts: &fk,
        pools: &pools,
        pool_samples: &pool_samples,
        row_so_far: &[],
        partition_col: None,
    };
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let result = apply_spec(&mut rng, &spec, &ctx);
    assert!(
        result.is_err(),
        "expected Err for zero FK target count, got Ok"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("devices") && (msg.contains("zero") || msg.contains("0")),
        "error should name the dataset and mention zero count; got: {msg}"
    );
}
