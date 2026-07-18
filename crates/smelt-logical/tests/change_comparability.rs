//! P3 change-comparability (`docs/specs/model_properties.md` §"Change
//! comparability"): does a projected column's value derive purely from
//! processed inputs, so it is comparable across separate runs (not merely
//! stable within one run) — the fact a future write-suppression compare
//! consumes. No consumer is wired yet; these tests exercise the derived
//! verdict itself against the public walk + maintenance-plumbing API.

use std::collections::BTreeMap;

use smelt_core::metadata::Contract;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::{model_property_vector, Comparability};
use smelt_logical::maintenance::column_comparability_with_contract;

fn comparability_of(sql: &str, output: &str) -> Comparability {
    let v = model_property_vector(sql, &JoinContext::new()).expect("model parses to a SELECT");
    v.comparability
        .iter()
        .find(|c| c.output == output)
        .unwrap_or_else(|| {
            panic!(
                "no comparability verdict for column {output}; got {:?}",
                v.comparability
            )
        })
        .comparability
}

/// A run-pinned clock (`NOW()`) is comparable *within* one run (it is a
/// per-run constant, `Determinism::Run`) but not comparable *across* runs —
/// the value legitimately differs run to run even though nothing about the
/// processed inputs changed, so a naive compare would treat it as a false
/// change (or, symmetrically, mask a real one). The within-run/across-run
/// distinction that motivates change-comparability as its own proof (not a
/// reuse of the determinism verdict wholesale) must land Incomparable here.
#[test]
fn pinned_now_is_incomparable_across_runs_though_deterministic_within_one() {
    let verdict = comparability_of(
        "SELECT customer_id, now() AS loaded_at FROM orders",
        "loaded_at",
    );
    assert_eq!(
        verdict,
        Comparability::Incomparable,
        "a run-pinned NOW() column must be Incomparable across runs"
    );
}

/// A plain payload column carried straight through is comparable.
#[test]
fn plain_column_is_comparable() {
    let verdict = comparability_of("SELECT customer_id, amount FROM orders", "amount");
    assert_eq!(verdict, Comparability::Comparable);
}

/// `columns.<c>.contract: plausible` forces its column Incomparable
/// regardless of what the walk itself proved — the override is applied
/// where the derived vector meets the model's declared metadata, not
/// inside the walk (the walk has no visibility into frontmatter).
#[test]
fn plausible_contract_column_is_incomparable_regardless_of_sql() {
    let v = model_property_vector(
        "SELECT customer_id, amount FROM orders",
        &JoinContext::new(),
    )
    .expect("model parses to a SELECT");
    assert_eq!(
        v.comparability
            .iter()
            .find(|c| c.output == "amount")
            .map(|c| c.comparability),
        Some(Comparability::Comparable),
        "sanity: the walk alone proves amount Comparable"
    );

    let mut contracts = BTreeMap::new();
    contracts.insert("amount".to_string(), Contract::Plausible);
    let overridden = column_comparability_with_contract(&v.comparability, &contracts);

    assert_eq!(
        overridden
            .iter()
            .find(|c| c.output == "amount")
            .map(|c| c.comparability),
        Some(Comparability::Incomparable),
        "a plausible-contract column must be forced Incomparable even though its \
         SQL alone is a pure passthrough"
    );
}
