//! Black-box coverage for the Phase 2 substrate unification
//! (`docs/plans/20260808-substrate-unification.md`): `analysis::
//! expr_util::{collect_column_refs, collect_column_refs_ungated,
//! collect_column_names, collect_referenced_qualifiers,
//! split_top_level_conjuncts}` replaced eight independently-copied helpers
//! (five column-ref collectors, three conjunct splitters) with one home
//! each. Those functions are deliberately `pub(crate)` — not part of this
//! crate's external API — so the direct, table-driven characterization
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
//! | Input | Gated (fingerprint/classify shape) | Ungated (old skeleton_closure/grouping shape) |
//! |---|---|---|
//! | `a.b` | `[a.b]` | `[a.b]` (agree) |
//! | `CASE WHEN a.x > 1 THEN a.y ELSE b.z END` | `[a.x, a.y, b.z]` | `[a.x, a.y, b.z]` (agree) |
//! | `foo(a.x, b.y)` | `[a.x, b.y]` | `[foo]` — **disagreement**: false column `foo`, real refs dropped |
//! | `SUM(a.x) OVER (PARTITION BY a.y ORDER BY a.z)` | `[a.x, a.y, a.z]` | `[SUM, a.y, a.z]` — **disagreement**: `a.x` silently dropped |
//! | `CAST(a.x AS INTEGER)` | `[a.x]` | `[a.x]` (agree) |
//! | `a.x BETWEEN b.y AND b.z` | `[a.x, b.y, b.z]` | `[a.x, b.y, b.z]` (agree) |
//!
//! Default resolution: the gated (correct) shape, `collect_column_refs`,
//! became the crate-wide implementation. `analysis::skeleton_closure` was
//! repointed to it with zero conformance-suite regression (its
//! `expr_references_alias` WHERE/ON-conjunct membership check simply
//! becomes more precise). `maintenance::grouping` could **not** be: fixing
//! it flips `maintenance_conformance::
//! keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery` and
//! `keyed_enriched_recipe_admits_suppressed_column_scoped_merge` to a
//! different maintenance technique — an admission-verdict change Phase 2's
//! contract forbids. `maintenance::grouping` therefore keeps calling
//! `collect_column_refs_ungated` (the historical, bug-preserving shape),
//! tracked as a follow-up in `docs/TODO.md`. The two conjunct-splitter
//! copies unified here (`backbuild::diff`'s `split_conjuncts`,
//! `backbuild::classify`'s `split_top_level_and`) were byte-identical in
//! logic — no disagreement to reconcile; `analysis::walk`'s range-based
//! `collect_self_conjunct_ranges` now consumes the shared splitter
//! internally (see its doc comment) rather than folding into the same
//! signature, since its own output (text ranges for region carving) is a
//! genuinely different shape.

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

/// `maintenance::grouping` deliberately still uses `collect_column_refs_
/// ungated` (see this file's module doc and `docs/TODO.md`): a column that
/// is a pure function call over a mutation-sensitive source's own column
/// (`UPPER(d.tier)`) is *not* attributed to that source's sensitivity here
/// — the ungated collector reads `UPPER` as a bare (unresolvable) column
/// name and drops `d.tier` entirely, so the derivation cannot resolve any
/// FROM-alias qualifier for the column and — per the fail-closed collapse
/// rule — the whole model's non-skeleton columns degenerate into one group
/// sensitive to every declared source, rather than a soundly narrower
/// per-column group. This pins today's behaviour, not the target: fixing
/// `maintenance::grouping` to use the gated collector is the tracked
/// follow-up.
#[test]
fn function_wrapped_source_column_still_collapses_under_the_preserved_bug() {
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
        !result.degenerate.is_empty(),
        "expected the unresolvable bare `UPPER` reference to force a degenerate collapse, got {result:?}"
    );
}
