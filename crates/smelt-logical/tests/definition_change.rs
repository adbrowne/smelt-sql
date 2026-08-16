//! TDD tests for `smelt_logical::analysis::definition_change` —
//! `classify_definition_change` (`docs/specs/model_properties.md`
//! §"Definition-change column classification"): the composed verdict
//! `SkeletonAdd{reason} | PureBackfill | UpstreamRederive` for one added
//! output column, replacing the `skeleton_columns.contains()` /
//! `mutation_sensitivity.is_empty()` proxies
//! `docs/plans/20260808-derived-maintenance-proofs.md` Phase 4 retires.

use std::collections::BTreeSet;

use smelt_logical::analysis::definition_change::{
    classify_definition_change, ClassifyRefusal, DefinitionChangeClass, DefinitionChangeCtx,
};
use smelt_logical::analysis::model_diff::ColumnDef;

fn column_def(name: &str, sql_expr: &str) -> ColumnDef {
    let sql = format!("SELECT {sql_expr} AS v FROM t");
    let parse = smelt_parser::parse(&sql);
    let file = smelt_parser::File::cast(parse.syntax()).expect("file");
    let select = file.select_stmt().expect("select");
    let item = select
        .select_list()
        .expect("select list")
        .items()
        .next()
        .expect("item");
    ColumnDef {
        name: name.to_string(),
        expr: item.expression().expect("expression"),
    }
}

fn empty_skeleton() -> BTreeSet<String> {
    BTreeSet::new()
}

/// An added column in `GROUP BY` position is a grain change (`SkeletonAdd`),
/// and the refusal reason names the `SkeletonRole` the flattened
/// `skeleton_columns.contains()` proxy lost.
#[test]
fn grouping_key_add_is_skeleton_add_with_role() {
    let sql = "SELECT pay_date, region, SUM(amount) AS revenue \
               FROM smelt.sources.payments GROUP BY pay_date, region";
    let old = vec![column_def("pay_date", "pay_date")];
    let added = column_def("region", "region");
    let skeleton = empty_skeleton();
    let ctx = DefinitionChangeCtx {
        old_columns: &old,
        declared_unique_key: &[],
        partition_col: Some("pay_date"),
        declared_skeleton_columns: &skeleton,
        monotone_dims: &[],
    };

    let verdict = classify_definition_change(&added, sql, &ctx).expect("classifies");
    match verdict {
        DefinitionChangeClass::SkeletonAdd { reason } => {
            assert!(
                reason.contains("Grouping"),
                "reason must name the SkeletonRole: {reason}"
            );
        }
        other => panic!("expected SkeletonAdd, got {other:?}"),
    }
}

/// An added column that is a pure function of already-stored columns (no
/// upstream read) is `PureBackfill` — admits an in-place `UPDATE`.
#[test]
fn pure_function_of_stored_columns_is_pure_backfill() {
    let sql = "SELECT event_id, referrer, SUBSTRING(referrer, 1, 10) AS referrer_domain \
               FROM smelt.sources.events";
    let old = vec![
        column_def("event_id", "event_id"),
        column_def("referrer", "referrer"),
    ];
    let added = column_def("referrer_domain", "SUBSTRING(referrer, 1, 10)");
    let skeleton = empty_skeleton();
    let ctx = DefinitionChangeCtx {
        old_columns: &old,
        declared_unique_key: &["event_id".to_string()],
        partition_col: None,
        declared_skeleton_columns: &skeleton,
        monotone_dims: &[],
    };

    let verdict = classify_definition_change(&added, sql, &ctx).expect("classifies");
    assert_eq!(verdict, DefinitionChangeClass::PureBackfill);
}

/// An added column that reads an append-only source non-aggregated (e.g. an
/// upstream column never before stored) must classify `UpstreamRederive`,
/// never `PureBackfill` — the group-emptiness proxy misclassified this case
/// (empty mutation-sensitivity != no upstream read); this is the corrected
/// misclassification this phase exists for. An aggregate over the source
/// (e.g. `COUNT(*)`) — which has zero *named* dependencies but is never a
/// pure function of a stored per-row value — is asserted the same way.
#[test]
fn append_only_source_read_is_upstream_rederive_not_pure_backfill() {
    let sql = "SELECT event_id, referrer, email \
               FROM smelt.sources.events JOIN smelt.sources.customers USING (user_id)";
    let old = vec![column_def("event_id", "event_id")];
    let added = column_def("email", "email");
    let skeleton = empty_skeleton();
    let ctx = DefinitionChangeCtx {
        old_columns: &old,
        declared_unique_key: &["event_id".to_string()],
        partition_col: None,
        declared_skeleton_columns: &skeleton,
        monotone_dims: &[],
    };

    let verdict = classify_definition_change(&added, sql, &ctx).expect("classifies");
    assert_eq!(verdict, DefinitionChangeClass::UpstreamRederive);

    // The aggregate case: zero named dependencies, but never pure.
    let agg_sql = "SELECT pay_date, COUNT(*) AS order_count \
                   FROM smelt.sources.payments GROUP BY pay_date";
    let agg_old = vec![column_def("pay_date", "pay_date")];
    let agg_added = column_def("order_count", "COUNT(*)");
    let agg_ctx = DefinitionChangeCtx {
        old_columns: &agg_old,
        declared_unique_key: &[],
        partition_col: Some("pay_date"),
        declared_skeleton_columns: &skeleton,
        monotone_dims: &[],
    };
    let agg_verdict =
        classify_definition_change(&agg_added, agg_sql, &agg_ctx).expect("classifies");
    assert_eq!(agg_verdict, DefinitionChangeClass::UpstreamRederive);
}

/// A change that is not a pure addition (the "added" column's name collides
/// with an existing stored column whose expression differs — a
/// redefinition, not an addition) fails closed rather than guessing a
/// verdict.
#[test]
fn non_additive_diff_refuses_classification() {
    let sql = "SELECT amount * 2 AS amount FROM t";
    let old = vec![column_def("amount", "amount")];
    let added = column_def("amount", "amount * 2");
    let skeleton = empty_skeleton();
    let ctx = DefinitionChangeCtx {
        old_columns: &old,
        declared_unique_key: &[],
        partition_col: None,
        declared_skeleton_columns: &skeleton,
        monotone_dims: &[],
    };

    let verdict = classify_definition_change(&added, sql, &ctx);
    assert!(
        matches!(verdict, Err(ClassifyRefusal::NotAdditive { .. })),
        "expected a fail-closed refusal, got {verdict:?}"
    );
}

/// A shape `skeleton_roles` cannot classify (no top-level `SELECT`) fails
/// closed rather than assuming the added column is safely `Payload`.
#[test]
fn unclassifiable_shape_fails_closed() {
    let sql = "DELETE FROM events WHERE event_id = 1";
    let old = vec![column_def("event_id", "event_id")];
    let added = column_def("region", "region");
    let skeleton = empty_skeleton();
    let ctx = DefinitionChangeCtx {
        old_columns: &old,
        declared_unique_key: &[],
        partition_col: None,
        declared_skeleton_columns: &skeleton,
        monotone_dims: &[],
    };

    let verdict = classify_definition_change(&added, sql, &ctx);
    assert!(
        matches!(verdict, Err(ClassifyRefusal::UnclassifiableShape)),
        "expected a fail-closed refusal, got {verdict:?}"
    );
}
