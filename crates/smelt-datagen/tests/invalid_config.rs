//! Tests that malformed datagen configs (where validate_config failed to catch a
//! bad linked_choice reference) return typed errors instead of panicking.
//!
//! Each test exercises one of the three LinkedChoice panic sites that the
//! hardening plan (Phase 7) converts to Result::Err.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use smelt_datagen::config::{FkCounts, GeneratorSpec};
use smelt_datagen::generic::{apply_spec, LinkedPool, PoolSamples, RowContext};
use std::collections::HashMap;
use std::sync::Arc;

fn empty_pool() -> LinkedPool {
    LinkedPool {
        rows: vec![],
        field_index: indexmap::IndexMap::new(),
    }
}

fn one_entry_pool(field: &str) -> LinkedPool {
    use smelt_datagen::generic::GenericValue;
    let mut fi = indexmap::IndexMap::new();
    fi.insert(field.to_string(), 0);
    LinkedPool {
        rows: vec![vec![GenericValue::Str("val".to_string())]],
        field_index: fi,
    }
}

#[test]
fn missing_pool_sample_returns_err() {
    // pool_samples map does not contain "my_pool" — first LinkedChoice panic site.
    let spec = GeneratorSpec::LinkedChoice {
        pool: "my_pool".to_string(),
        field: "status".to_string(),
    };
    let mut pools: HashMap<String, Arc<LinkedPool>> = HashMap::new();
    pools.insert("my_pool".to_string(), Arc::new(empty_pool()));
    let pool_samples: PoolSamples<'_> = HashMap::new(); // empty — no sample for "my_pool"
    let fk = FkCounts::new();
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
        "expected Err for missing pool sample, got Ok"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("my_pool"),
        "error should name the pool; got: {msg}"
    );
}

#[test]
fn missing_pool_ref_returns_err() {
    // pools map does not contain "my_pool" — second LinkedChoice panic site.
    let spec = GeneratorSpec::LinkedChoice {
        pool: "my_pool".to_string(),
        field: "status".to_string(),
    };
    let pools: HashMap<String, Arc<LinkedPool>> = HashMap::new(); // empty — pool not built
    let mut pool_samples: PoolSamples<'_> = HashMap::new();
    pool_samples.insert("my_pool", 0);
    let fk = FkCounts::new();
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
    assert!(result.is_err(), "expected Err for missing pool ref, got Ok");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("my_pool"),
        "error should name the pool; got: {msg}"
    );
}

#[test]
fn missing_field_in_pool_returns_err() {
    // pool exists and is sampled, but "bad_field" is not in the pool — third panic site.
    let spec = GeneratorSpec::LinkedChoice {
        pool: "my_pool".to_string(),
        field: "bad_field".to_string(),
    };
    let mut pools: HashMap<String, Arc<LinkedPool>> = HashMap::new();
    pools.insert(
        "my_pool".to_string(),
        Arc::new(one_entry_pool("good_field")),
    );
    let mut pool_samples: PoolSamples<'_> = HashMap::new();
    pool_samples.insert("my_pool", 0);
    let fk = FkCounts::new();
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
        "expected Err for missing field in pool, got Ok"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("bad_field"),
        "error should name the field; got: {msg}"
    );
}
