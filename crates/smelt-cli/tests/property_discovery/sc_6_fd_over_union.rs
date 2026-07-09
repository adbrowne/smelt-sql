//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `SC-6` (`docs/research/20260707-property-per-key-constancy.md` §3.8 /
//! §7 gap 3; hypothesis B4 in `docs/research/property-discovery/catalog.jsonl`).
//!
//! Hypothesis: `FunctionalDependency.key` is parsed but never read by the FD
//! verdict, and no union analysis exists — an FD `key → determines` that holds
//! in EACH `UNION ALL` branch does NOT hold in the union (the same key may
//! appear in both branches with different determined values), yet a declared
//! FD over a `UNION ALL` body widens the verdict to `Constant` today. This is
//! a linkB (analyzer-classification) cell: no once-write consumer is wired, so
//! the assertion is directly on the analyzer fact. Expected RED = the analyzer
//! widens a union it cannot prove key-disjoint (the proof-layer bug); GREEN =
//! the walk-derived property vector records the set-operation barrier and the
//! declared FD is refused (or at least not widened to `Constant`).

use smelt_logical::analysis::functional_dependency::functional_dependency_verdict_over_vector;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::model_property_vector;

/// SC-6's owning test. GREEN = a declared `customer_id → region` over a bare
/// `UNION ALL` body does NOT widen to `Constant` (the union manufactures
/// potential per-key variance the single-relation declaration cannot cover).
/// RED = the analyzer widens it, which — once a once-write consumer is wired —
/// would freeze whichever branch's row is folded first for a colliding key,
/// diverging from the full refresh that sees both branches.
#[test]
fn declared_fd_over_union_all_is_refused() {
    // Two append-only arms, each keyed on customer_id in its own source, but
    // the union is not proven key-disjoint (no literal discriminator in the
    // declared key). A concrete divergence: crm_a has (c1, 'EU'), crm_b has
    // (c1, 'US') — the union holds both, so customer_id → region is false on
    // the union even though it holds in each arm.
    let sql = "SELECT customer_id, region FROM crm_a \
               UNION ALL \
               SELECT customer_id, region FROM crm_b";

    let vector = model_property_vector(sql, &JoinContext::new())
        .expect("the model parses to a set-operation SELECT");

    let verdict = functional_dependency_verdict_over_vector(
        &["customer_id".to_string()],
        "region",
        &vector,
        // declared = true: this is exactly the declaration the widening rule
        // is meant to gate.
        true,
    );

    assert!(
        !verdict.is_constant(),
        "SC-6: a declared functional dependency (customer_id → region) over a bare UNION ALL \
         body must NOT widen to Constant — an FD holding in each branch does not hold in the \
         union (same key, different determined value across branches). \
         The walk must record the set-operation barrier and refuse the declaration. \
         Got verdict: {verdict:?}"
    );
}

/// Guard against over-narrowing (Phase 6 review checklist): the genuinely
/// undecidable single-branch case — a plain pass-through `determines` column
/// with no union and no fan-out join — is still widened by the declaration.
/// Refusing SC-6 must not refuse every declared FD.
#[test]
fn declared_fd_over_single_branch_still_widens() {
    let sql = "SELECT customer_id, region FROM crm_a";
    let vector = model_property_vector(sql, &JoinContext::new()).expect("parses");

    let verdict = functional_dependency_verdict_over_vector(
        &["customer_id".to_string()],
        "region",
        &vector,
        true,
    );

    assert!(
        verdict.is_constant(),
        "a declared FD over a single-branch model with an undecidable origin must still widen \
         to Constant (no over-narrowing); got {verdict:?}"
    );
}
