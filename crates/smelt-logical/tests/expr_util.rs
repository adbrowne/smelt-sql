//! Black-box coverage for `analysis::expr_util::{collect_column_refs,
//! collect_column_names, collect_referenced_qualifiers,
//! split_top_level_conjuncts}`, which replaced eight independently-copied
//! helpers (five column-ref collectors, three conjunct splitters) with one
//! home each. Those functions are deliberately `pub(crate)` — not part of
//! this crate's external API — so the direct, table-driven characterization
//! (the battery of qualified refs / aliases / function args / `CASE` /
//! window `OVER` clauses for column-ref collection, and nested parens /
//! `OR` guards / `BETWEEN` for conjunct splitting) lives as unit tests
//! colocated in `crates/smelt-logical/src/analysis/expr_util.rs`
//! (`mod tests`), which has same-crate access. This file demonstrates the
//! same reconciliation *end to end*, through the public call sites that
//! consume those helpers, so the observable behaviour is pinned from
//! outside the crate too.
//!
//! **The disagreement found and how it was reconciled.** Two of the five
//! pre-unification column-ref-collection copies
//! (`analysis::skeleton_closure`, `maintenance::grouping`) had silently
//! diverged from the other three (`analysis::fingerprint`,
//! `backbuild::classify`'s two copies): they omitted an `EXPRESSION`-kind
//! guard, so a bare function call anywhere in the expression (e.g.
//! `SUM(a.x) OVER (...)`, `UPPER(d.tier)`) was misread as itself a column
//! reference (the call's own name read as a bare column) *and* — because
//! the match short-circuits the recursion — every real column reference
//! among that call's arguments was silently dropped.
//!
//! | Input | Result |
//! |---|---|
//! | `a.b` | `[a.b]` |
//! | `CASE WHEN a.x > 1 THEN a.y ELSE b.z END` | `[a.x, a.y, b.z]` |
//! | `foo(a.x, b.y)` | `[a.x, b.y]` (not the false column `foo`) |
//! | `SUM(a.x) OVER (PARTITION BY a.y ORDER BY a.z)` | `[a.x, a.y, a.z]` (not `[SUM, a.y, a.z]`) |
//! | `CAST(a.x AS INTEGER)` | `[a.x]` |
//! | `a.x BETWEEN b.y AND b.z` | `[a.x, b.y, b.z]` |
//!
//! `collect_column_refs`, the guarded (correct) shape, is the crate-wide
//! implementation — every call site, including `maintenance::grouping`'s
//! per-column provenance derivation, uses it
//! (`docs/plans/20260808-membership-sensitivity.md` Phase 1). The two
//! conjunct-splitter copies unified here (`backbuild::diff`'s
//! `split_conjuncts`, `backbuild::classify`'s `split_top_level_and`) were
//! byte-identical in logic — no disagreement to reconcile;
//! `analysis::walk`'s range-based `collect_self_conjunct_ranges` now
//! consumes the shared splitter internally (see its doc comment) rather
//! than folding into the same signature, since its own output (text ranges
//! for region carving) is a genuinely different shape.

use std::collections::BTreeSet;

use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::skeleton_closure::{skeleton_source_closure, SkeletonSourceClosure};
use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::{MutationProfile, SourceFacts};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn source(name: &str, mutation: MutationProfile, unique_key: &[&str]) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation,
        partition_col: None,
        unique_key: unique_key.iter().map(|s| s.to_string()).collect(),
        allow_full_scan: false,
    }
}

/// `analysis::skeleton_closure` now uses the gated (fixed) collector: a
/// membership predicate wrapped in a function call over an enrichment-side
/// column is correctly recognised as referencing that column, so conjunct
/// 5 (no membership predicate on the enrichment side) correctly refuses —
/// `Open`, not a falsely-`Closed` verdict.
#[test]
fn function_wrapped_membership_predicate_on_enrichment_column_stays_open() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.id \
               WHERE UPPER(d.tier) = 'GOLD'";
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert!(!verdict.is_closed(), "{verdict:?}");
    match verdict {
        SkeletonSourceClosure::Open { reason } => {
            assert!(reason.contains("membership predicate"), "{reason}");
        }
        other => panic!("expected Open, got {other:?}"),
    }
}

/// `maintenance::grouping` now uses the gated `collect_column_refs`
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 1 — the
/// collector-swap that closed out this file's originally-documented
/// divergence): a column that is a pure function call over a
/// mutation-sensitive source's own column (`UPPER(d.tier)`) correctly
/// resolves the FROM-alias qualifier for `d.tier` inside the call's
/// arguments, so the derivation produces a soundly narrower per-column
/// group instead of collapsing. `d` is also read in the `LEFT JOIN`'s `ON`
/// predicate, so it additionally carries membership sensitivity — deriving
/// fail-closed for outer joins, per this plan's Phase 1 scope.
#[test]
fn function_wrapped_source_column_no_longer_collapses() {
    let sources = vec![
        source("customers", MutationProfile::MutableSnapshot, &["id"]),
        source("orders", MutationProfile::AppendOnly, &[]),
    ];
    let sql = "SELECT f.order_id, UPPER(d.tier) AS tier_u \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.id";
    let skeleton = set(&["order_id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        result.degenerate.is_empty(),
        "UPPER(d.tier) must resolve via the gated collector, not collapse: {result:?}"
    );
    let tier_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"tier_u".to_string()))
        .expect("tier_u is grouped");
    assert_eq!(
        tier_group.mutation_sensitivity,
        set(&["customers"]),
        "UPPER(d.tier) must contribute d.tier's value sensitivity"
    );
    assert_eq!(
        tier_group.membership_sensitivity,
        set(&["customers"]),
        "d is also read in the LEFT JOIN's ON predicate — membership \
         sensitivity, deriving fail-closed for outer joins"
    );
}
