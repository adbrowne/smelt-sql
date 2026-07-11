//! TDD tests for `smelt_logical::maintenance::skeleton` — skeleton-role
//! extraction (`docs/specs/model_properties.md` §"Skeleton-role
//! extraction").

use smelt_logical::maintenance::skeleton::{skeleton_columns, skeleton_roles, SkeletonRole};

/// `GROUP BY` keys and `SELECT DISTINCT` dedup keys occupy a
/// row-membership position — they must classify as skeleton (not
/// `Payload`), because a field added there is a grain change, never a
/// column backfill (`maintenance_plan.md` §"The definition-change
/// trigger").
#[test]
fn group_by_and_dedup_columns_are_skeleton() {
    let sql = "SELECT pay_date, SUM(amount) AS revenue, COUNT(*) AS order_count \
               FROM smelt.sources.payments GROUP BY pay_date";
    let roles = skeleton_roles(sql, &[], None).expect("classifies");
    let pay_date_role = roles
        .iter()
        .find(|(c, _)| c == "pay_date")
        .map(|(_, r)| *r)
        .expect("pay_date present");
    assert_eq!(pay_date_role, SkeletonRole::Grouping);

    let revenue_role = roles
        .iter()
        .find(|(c, _)| c == "revenue")
        .map(|(_, r)| *r)
        .expect("revenue present");
    assert_eq!(revenue_role, SkeletonRole::Payload);

    let columns = skeleton_columns(sql, &[], None);
    assert!(columns.contains("pay_date"));
    assert!(!columns.contains("revenue"));
    assert!(!columns.contains("order_count"));

    // SELECT DISTINCT: the dedup key is the whole projected row.
    let distinct_sql = "SELECT DISTINCT user_id, tier FROM smelt.sources.customers";
    let distinct_roles = skeleton_roles(distinct_sql, &[], None).expect("classifies");
    assert!(
        distinct_roles
            .iter()
            .all(|(_, role)| *role == SkeletonRole::Dedup),
        "every column of a DISTINCT projection is a dedup key: {distinct_roles:?}"
    );
}

/// A declared unique key is always `Identity`, regardless of whether the
/// query itself groups or dedups on it.
#[test]
fn declared_unique_key_is_identity() {
    let sql = "SELECT event_id, event_date, page FROM smelt.sources.events";
    let roles = skeleton_roles(sql, &["event_id".to_string()], None).expect("classifies");
    let event_id_role = roles
        .iter()
        .find(|(c, _)| c == "event_id")
        .map(|(_, r)| *r)
        .expect("event_id present");
    assert_eq!(event_id_role, SkeletonRole::Identity);

    let page_role = roles
        .iter()
        .find(|(c, _)| c == "page")
        .map(|(_, r)| *r)
        .expect("page present");
    assert_eq!(page_role, SkeletonRole::Payload);
}

/// `ORDER BY` keys occupy a row-ordering position.
#[test]
fn order_by_columns_are_skeleton() {
    let sql = "SELECT user_id, rank FROM smelt.sources.leaderboard ORDER BY rank";
    let roles = skeleton_roles(sql, &[], None).expect("classifies");
    let rank_role = roles
        .iter()
        .find(|(c, _)| c == "rank")
        .map(|(_, r)| *r)
        .expect("rank present");
    assert_eq!(rank_role, SkeletonRole::Ordering);
}
