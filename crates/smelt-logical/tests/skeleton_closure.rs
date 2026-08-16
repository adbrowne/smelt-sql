//! TDD tests for `smelt_logical::analysis::skeleton_closure` — the P1
//! skeleton-source-closure proof (`docs/specs/model_properties.md`
//! §"Skeleton-source closure").

use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::skeleton_closure::{
    skeleton_source_closure, RowPreservation, SkeletonSourceClosure,
};

/// Driving fact `LEFT JOIN` dimension, payload-only enrichment columns, no
/// membership predicate on the enrichment side — all five conjuncts prove
/// and the closure is `Closed`.
#[test]
fn left_join_dimension_payload_only_closes() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.id";
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert_eq!(
        verdict,
        SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::JoinShape
        },
        "{verdict:?}"
    );
}

/// Inner JOIN **with** a declared `referential_integrity` on the dimension
/// source, payload-only, no membership predicate — row preservation (conjunct
/// 4) is licensed by the declaration instead of a `LEFT JOIN`.
#[test]
fn inner_join_with_declared_referential_integrity_closes() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               JOIN smelt.sources.customers d ON f.customer_id = d.id";
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let ri = vec!["id".to_string()];
    let verdict = skeleton_source_closure(sql, "customers", Some(&ri), &ctx);
    assert_eq!(
        verdict,
        SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::DeclaredReferentialIntegrity {
                source: "customers".to_string()
            }
        },
        "{verdict:?}"
    );
}

/// A bare inner JOIN with no `referential_integrity` declared and no `LEFT
/// JOIN` cannot prove row preservation — stays `Open`. This is the
/// discriminating case the proof must catch (see also
/// `skeleton_closure_pinned.rs`).
#[test]
fn bare_inner_join_stays_open() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               JOIN smelt.sources.customers d ON f.customer_id = d.id";
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert!(!verdict.is_closed(), "{verdict:?}");
    match verdict {
        SkeletonSourceClosure::Open { reason } => {
            assert!(reason.contains("row preservation"), "{reason}");
        }
        _ => unreachable!(),
    }
}

/// A `WHERE dim.col = …` membership predicate on an enrichment-side column
/// makes output row-membership depend on the enrichment source — stays
/// `Open` (conjunct 5).
#[test]
fn membership_predicate_on_enrichment_column_stays_open() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.id \
               WHERE d.tier = 'gold'";
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert!(!verdict.is_closed(), "{verdict:?}");
    match verdict {
        SkeletonSourceClosure::Open { reason } => {
            assert!(reason.contains("membership predicate"), "{reason}");
        }
        _ => unreachable!(),
    }
}

/// An enrichment column feeding a `GROUP BY` is the v1 aggregation-scope
/// restriction — `Open` regardless of the five conjuncts, checked first.
#[test]
fn enrichment_column_feeding_group_by_stays_open() {
    let sql = "SELECT d.tier, COUNT(*) AS n \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.id \
               GROUP BY d.tier";
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert!(!verdict.is_closed(), "{verdict:?}");
    match verdict {
        SkeletonSourceClosure::Open { reason } => {
            assert!(reason.contains("v1 scope restriction"), "{reason}");
        }
        _ => unreachable!(),
    }
}

/// A fan-out (non-1:1) join — no declared unique key on the dimension for
/// the join equality — stays `Open` (conjunct 3), even under a `LEFT JOIN`.
#[test]
fn fan_out_join_stays_open() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.region";
    // The declared unique key is `id`, not `region` — the join's equality
    // does not cover any declared key-set.
    let ctx = JoinContext::new().with_unique_key("d", "id");
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert!(!verdict.is_closed(), "{verdict:?}");
    match verdict {
        SkeletonSourceClosure::Open { reason } => {
            assert!(reason.contains("one-to-one"), "{reason}");
        }
        _ => unreachable!(),
    }
}

/// A fan-out join with no declared unique key at all fails closed the same
/// way (fan_out's own conservative default).
#[test]
fn fan_out_join_with_no_declared_key_stays_open() {
    let sql = "SELECT f.order_id, f.amount, d.tier \
               FROM smelt.sources.orders f \
               LEFT JOIN smelt.sources.customers d ON f.customer_id = d.id";
    let ctx = JoinContext::new(); // no declared unique keys at all
    let verdict = skeleton_source_closure(sql, "customers", None, &ctx);
    assert!(!verdict.is_closed(), "{verdict:?}");
}
