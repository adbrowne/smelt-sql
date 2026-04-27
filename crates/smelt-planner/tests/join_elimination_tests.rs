//! Phase 34 — `EliminateUnusedLeftJoin` rule tests.
//!
//! These tests are pure (no Salsa dependency) — they construct `Arc<LogicalNode>`
//! trees directly and drive the rule API through the public interface in
//! `smelt_planner::logical_plan_rules`.

use std::sync::Arc;

use smelt_planner::logical::{Cardinality, LogicalNode};
use smelt_planner::logical_plan_rules::{
    EliminateUnusedLeftJoin, PlannerRule, RuleContext, RuleResult,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_table_ref(name: &str) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::TableRef {
        name: name.to_string(),
    })
}

fn make_left_join(
    lhs: Arc<LogicalNode>,
    rhs: Arc<LogicalNode>,
    join_columns: Vec<&str>,
    cardinality: Cardinality,
    output_columns: Vec<&str>,
) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::LeftJoin {
        lhs,
        rhs,
        join_columns: join_columns.into_iter().map(str::to_string).collect(),
        cardinality,
        output_columns: output_columns.into_iter().map(str::to_string).collect(),
    })
}

fn make_select(projections: Vec<&str>, from: Arc<LogicalNode>) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::Select {
        projections: projections.into_iter().map(str::to_string).collect(),
        from: Some(from),
        filter: None,
    })
}

// ---------------------------------------------------------------------------
// Test 1 — rule fires when no RHS column is consumed
// ---------------------------------------------------------------------------
//
// Select { projections: ["order_id", "total"] }   <-- no dim columns
//   from: LeftJoin {
//     lhs: TableRef("orders"),
//     rhs: TableRef("dim_customer"),
//     join_columns: ["customer_id"],
//     cardinality: OneToOne,
//     output_columns: ["customer_name", "customer_tier"],
//   }
//
// Expected: rule fires, `from` replaced with lhs (TableRef("orders")).

#[test]
fn join_elimination_fires_when_no_downstream_consumer() {
    let join = make_left_join(
        make_table_ref("orders"),
        make_table_ref("dim_customer"),
        vec!["customer_id"],
        Cardinality::OneToOne,
        vec!["customer_name", "customer_tier"],
    );
    let plan = make_select(vec!["order_id", "total"], join);

    let rule = EliminateUnusedLeftJoin;
    let result = rule.apply(plan, &RuleContext::default());

    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => {
            panic!("Expected Changed: join should be elided when no RHS column is used")
        }
    };

    // The new from must be TableRef("orders"), not a LeftJoin.
    match new_plan.as_ref() {
        LogicalNode::Select {
            from, projections, ..
        } => {
            assert_eq!(
                projections,
                &vec!["order_id".to_string(), "total".to_string()],
                "projections must be preserved"
            );
            let from_node = from.as_ref().expect("Select must have a from child");
            match from_node.as_ref() {
                LogicalNode::TableRef { name } => {
                    assert_eq!(
                        name, "orders",
                        "from must be the LHS table ref after elimination"
                    );
                }
                other => panic!("Expected TableRef(orders) after join elimination, got: {other:?}"),
            }
        }
        other => panic!("Expected Select at top level, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 2 — rule blocked when a RHS column appears in projections
// ---------------------------------------------------------------------------
//
// Same plan but projections: ["order_id", "customer_name"].
// "customer_name" is an RHS output column → rule must return Unchanged.

#[test]
fn join_elimination_blocked_when_column_used() {
    let join = make_left_join(
        make_table_ref("orders"),
        make_table_ref("dim_customer"),
        vec!["customer_id"],
        Cardinality::OneToOne,
        vec!["customer_name", "customer_tier"],
    );
    let plan = make_select(vec!["order_id", "customer_name"], join);

    let rule = EliminateUnusedLeftJoin;
    match rule.apply(plan, &RuleContext::default()) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => panic!(
            "Expected Unchanged when a RHS column ('customer_name') is projected, got Changed: {p:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// Test 3 — rule blocked when cardinality is OneToMany
// ---------------------------------------------------------------------------
//
// Same plan (no dim columns in projections) but cardinality: OneToMany.
// Rule must return Unchanged — dropping the join could change the row count.

#[test]
fn join_elimination_requires_declared_cardinality() {
    let join = make_left_join(
        make_table_ref("orders"),
        make_table_ref("dim_customer"),
        vec!["customer_id"],
        Cardinality::OneToMany,
        vec!["customer_name", "customer_tier"],
    );
    let plan = make_select(vec!["order_id", "total"], join);

    let rule = EliminateUnusedLeftJoin;
    match rule.apply(plan, &RuleContext::default()) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => panic!(
            "Expected Unchanged for OneToMany cardinality (join may change row count), got Changed: {p:?}"
        ),
    }
}
