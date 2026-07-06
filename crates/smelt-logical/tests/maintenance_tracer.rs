//! Tracer bullet for the maintenance-plan framework
//! (`docs/research/20260705-refresh-as-maintenance-plan/`): encode the
//! catalogue's key examples (EX-02, EX-07, EX-13, EX-24, EX-36, EX-39,
//! EX-40 of `07-example-catalogue.md`) as derivation inputs and assert each
//! trigger lands in its expected 2×2 corner, refusals fire where the
//! catalogue refuses, and the emitted SQL carries the partition predicate on
//! both the scan and the write target.

use std::collections::BTreeSet;

use smelt_logical::analysis::model_diff::{additive_only_diff, ColumnDef};
use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, emit_in_place_update, emit_keyed_fold, Region,
};
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, PartitionLocal, Refusal, SourceFacts,
    Technique, Trigger,
};
use smelt_types::SqlFunction;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn source(name: &str, mutation: MutationProfile, partition_col: Option<&str>) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation,
        partition_col: partition_col.map(|s| s.to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

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

// ---------------------------------------------------------------------------
// EX-02 — clickstream landing: append-only clocked source, partition grain.
// New data and backfill both land in recompute-region, partition-local.
// ---------------------------------------------------------------------------

fn ex02_inputs() -> ModelInputs<'static> {
    ModelInputs {
        sql: "SELECT event_id, user_id, event_date, event_ts, page, referrer \
              FROM smelt.sources.events",
        output: OutputSpec {
            table: "clickstream".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: set(&["event_id", "event_date"]),
        },
        sources: vec![source(
            "events",
            MutationProfile::AppendOnly,
            Some("event_date"),
        )],
        column_groups: vec![ColumnGroup {
            columns: strings(&["page", "referrer"]),
            mutation_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        column_add_proof: None,
    }
}

#[test]
fn ex02_new_data_and_backfill_are_partition_local_recompute_region() {
    let inputs = ex02_inputs();
    let plan = derive_maintenance_plan(
        &inputs,
        &[
            Trigger::NewData {
                source: "events".to_string(),
            },
            Trigger::Backfill,
        ],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells.len(), 2);
    for cell in &plan.cells {
        assert_eq!(cell.corner, Corner::RecomputeRegion);
        assert_eq!(cell.technique, Technique::DeleteInsert);
        assert_eq!(cell.partition_local, PartitionLocal::Yes);
        assert!(!cell.ledger_catch_up);
    }
}

#[test]
fn ex02_delete_insert_carries_the_partition_predicate_on_both_statements() {
    let region = Region {
        start: "DATE '2026-01-01'".to_string(),
        end: "DATE '2026-01-08'".to_string(),
    };
    let stmts = emit_delete_insert(
        "clickstream",
        "event_date",
        &region,
        "SELECT event_id, user_id, event_date, event_ts, page, referrer FROM events",
    );
    assert_eq!(stmts.len(), 2);
    assert!(stmts[0].starts_with("DELETE FROM clickstream WHERE"));
    for stmt in &stmts {
        assert!(
            stmt.contains("event_date >= DATE '2026-01-01'")
                && stmt.contains("event_date < DATE '2026-01-08'"),
            "partition predicate missing from: {stmt}"
        );
    }
}

// ---------------------------------------------------------------------------
// EX-07 — orders × unclocked mutable customer dim. Dimension churn drives a
// column-scoped merge of {tier}; without a declared full-scan acceptance the
// K8 guardrail refuses (partition-locality fails on `customers`).
// ---------------------------------------------------------------------------

fn ex07_inputs(allow_full_scan: bool) -> ModelInputs<'static> {
    let mut customers = SourceFacts {
        name: "customers".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: None,
        unique_key: strings(&["user_id"]),
        allow_full_scan: false,
    };
    customers.allow_full_scan = allow_full_scan;
    ModelInputs {
        sql: "SELECT o.order_date, o.order_id, o.user_id, o.amount, c.tier \
              FROM smelt.sources.orders o \
              JOIN smelt.sources.customers c ON c.user_id = o.user_id",
        output: OutputSpec {
            table: "orders_tiered".to_string(),
            grain: Grain::Partition {
                partition_col: "order_date".to_string(),
            },
            skeleton_columns: set(&["order_id", "order_date"]),
        },
        sources: vec![
            source("orders", MutationProfile::AppendOnly, Some("order_date")),
            customers,
        ],
        column_groups: vec![
            ColumnGroup {
                columns: strings(&["amount"]),
                mutation_sensitivity: BTreeSet::new(),
            },
            ColumnGroup {
                columns: strings(&["tier"]),
                mutation_sensitivity: set(&["customers"]),
            },
        ],
        fold: None,
        column_add_proof: None,
    }
}

#[test]
fn ex07_dimension_churn_without_full_scan_acceptance_refuses_scan_unbounded() {
    let inputs = ex07_inputs(false);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "customers".to_string(),
        }],
    );
    assert!(plan.cells.is_empty(), "cells: {:?}", plan.cells);
    assert!(
        matches!(&plan.refusals[..], [Refusal::ScanUnbounded { source, .. }] if source == "customers"),
        "refusals: {:?}",
        plan.refusals
    );
}

#[test]
fn ex07_declared_full_scan_admits_the_column_merge_as_non_partition_local() {
    let inputs = ex07_inputs(true);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "customers".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.technique, Technique::ColumnScopedMerge);
    assert_eq!(cell.group, "{tier}");
    assert!(
        matches!(&cell.partition_local, PartitionLocal::No { source, .. } if source == "customers")
    );
}

#[test]
fn ex07_backfill_locality_names_the_unclocked_lookup() {
    let inputs = ex07_inputs(false);
    let plan = derive_maintenance_plan(&inputs, &[Trigger::Backfill]);
    let cell = &plan.cells[0];
    assert_eq!(cell.corner, Corner::RecomputeRegion);
    assert!(
        matches!(&cell.partition_local, PartitionLocal::No { source, .. } if source == "customers")
    );
}

// ---------------------------------------------------------------------------
// EX-13 — daily revenue (additive agg, GROUP BY = partition col). Today's
// admitted cell is recompute-region, partition-local.
// ---------------------------------------------------------------------------

#[test]
fn ex13_new_day_is_partition_local_recompute_region() {
    let inputs = ModelInputs {
        sql: "SELECT pay_date, SUM(amount) AS revenue \
              FROM smelt.sources.payments GROUP BY pay_date",
        output: OutputSpec {
            table: "daily_revenue".to_string(),
            grain: Grain::Partition {
                partition_col: "pay_date".to_string(),
            },
            skeleton_columns: set(&["pay_date"]),
        },
        sources: vec![source(
            "payments",
            MutationProfile::AppendOnly,
            Some("pay_date"),
        )],
        column_groups: vec![ColumnGroup {
            columns: strings(&["revenue"]),
            mutation_sensitivity: set(&["payments"]),
        }],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::NewData {
            source: "payments".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty());
    assert_eq!(plan.cells[0].corner, Corner::RecomputeRegion);
    assert_eq!(plan.cells[0].partition_local, PartitionLocal::Yes);
}

// ---------------------------------------------------------------------------
// EX-24 — keyed lifetime spend: fold-a-delta into key-addressed state.
// Admission needs an append-only source and a monoid combiner; a holistic
// combiner or a mutable source refuses (fail closed).
// ---------------------------------------------------------------------------

fn ex24_inputs(
    combiner: SqlFunction,
    mutation: MutationProfile,
) -> (ModelInputs<'static>, Trigger) {
    let inputs = ModelInputs {
        sql: "SELECT user_id, SUM(amount) AS lifetime_spend \
              FROM smelt.sources.payments GROUP BY user_id",
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![source("payments", mutation, Some("pay_date"))],
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["payments"]),
        }],
        fold: Some(FoldSpec {
            add_columns: strings(&["lifetime_spend"]),
            combiner,
        }),
        column_add_proof: None,
    };
    let trigger = Trigger::NewData {
        source: "payments".to_string(),
    };
    (inputs, trigger)
}

#[test]
fn ex24_additive_fold_over_append_only_is_admitted() {
    let (inputs, trigger) = ex24_inputs(SqlFunction::Sum, MutationProfile::AppendOnly);
    let plan = derive_maintenance_plan(&inputs, &[trigger]);
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert_eq!(cell.corner, Corner::FoldDelta);
    assert_eq!(cell.technique, Technique::KeyedFold);
}

#[test]
fn ex24_holistic_combiner_refuses_the_fold() {
    let (inputs, trigger) = ex24_inputs(SqlFunction::Median, MutationProfile::AppendOnly);
    let plan = derive_maintenance_plan(&inputs, &[trigger]);
    assert!(plan.cells.is_empty());
    assert!(matches!(
        &plan.refusals[..],
        [Refusal::NoAdmissibleTechnique { .. }]
    ));
}

#[test]
fn ex24_mutable_source_fails_the_faithful_fold_condition() {
    let (inputs, trigger) = ex24_inputs(SqlFunction::Sum, MutationProfile::MutableSnapshot);
    let plan = derive_maintenance_plan(&inputs, &[trigger]);
    assert!(plan.cells.is_empty());
    assert!(matches!(
        &plan.refusals[..],
        [Refusal::NoAdmissibleTechnique { .. }]
    ));
}

#[test]
fn ex24_keyed_fold_sql_combines_matched_and_inserts_unseen_keys() {
    let stmts = emit_keyed_fold(
        "lifetime_spend",
        &strings(&["user_id"]),
        &strings(&["lifetime_spend"]),
        &strings(&["user_id", "lifetime_spend"]),
        "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments_delta GROUP BY user_id",
    );
    let sql = &stmts[0];
    assert!(sql.contains("ON t.user_id = s.user_id"));
    assert!(sql.contains("lifetime_spend = t.lifetime_spend + s.lifetime_spend"));
    assert!(sql.contains("WHEN NOT MATCHED THEN INSERT (user_id, lifetime_spend)"));
}

// ---------------------------------------------------------------------------
// EX-36 / EX-39 / EX-40 — the definition-change trigger.
// ---------------------------------------------------------------------------

#[test]
fn ex36_pure_function_field_add_is_in_place_update_with_ledger_catch_up() {
    let old = vec![
        column_def("event_id", "event_id"),
        column_def("referrer", "referrer"),
    ];
    let new = vec![
        column_def("event_id", "event_id"),
        column_def("referrer", "referrer"),
        column_def(
            "referrer_domain",
            "regexp_extract(referrer, '://([^/]+)', 1)",
        ),
    ];
    let proof = additive_only_diff(&old, &new, &[]);
    assert!(proof.is_additive_only());

    let mut inputs = ex02_inputs();
    inputs.column_groups.push(ColumnGroup {
        columns: strings(&["referrer_domain"]),
        mutation_sensitivity: BTreeSet::new(),
    });
    inputs.column_add_proof = Some(&proof);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["referrer_domain"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = &plan.cells[0];
    assert_eq!(cell.corner, Corner::FoldDelta, "top-left, empty delta");
    assert_eq!(cell.technique, Technique::InPlaceUpdate);
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert!(cell.ledger_catch_up, "S starts at ∅ over existing regions");
}

#[test]
fn ex36_without_the_additive_only_proof_fails_closed() {
    let mut inputs = ex02_inputs();
    inputs.column_groups.push(ColumnGroup {
        columns: strings(&["referrer_domain"]),
        mutation_sensitivity: BTreeSet::new(),
    });
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["referrer_domain"]),
        }],
    );
    assert!(plan.cells.is_empty());
    assert!(matches!(
        &plan.refusals[..],
        [Refusal::NoAdmissibleTechnique { .. }]
    ));
}

#[test]
fn ex36_in_place_update_reads_nothing_upstream() {
    let region = Region {
        start: "DATE '2026-01-01'".to_string(),
        end: "DATE '2026-02-01'".to_string(),
    };
    let stmts = emit_in_place_update(
        "clickstream",
        &[(
            "referrer_domain".to_string(),
            "regexp_extract(referrer, '://([^/]+)', 1)".to_string(),
        )],
        "event_date",
        &region,
    );
    let sql = &stmts[0];
    assert!(sql.starts_with("UPDATE clickstream SET referrer_domain ="));
    assert!(sql.contains("event_date >= DATE '2026-01-01'"));
    assert!(!sql.contains("FROM"), "no upstream read: {sql}");
}

#[test]
fn ex39_skeleton_position_add_refuses_as_grain_change() {
    let mut inputs = ex02_inputs();
    inputs.output.skeleton_columns.insert("region".to_string());
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["region"]),
        }],
    );
    assert!(plan.cells.is_empty());
    assert!(
        matches!(&plan.refusals[..], [Refusal::SkeletonColumnAdded { column }] if column == "region")
    );
}

#[test]
fn ex40_aggregate_field_add_is_column_merge_with_ledger_catch_up() {
    let inputs = ModelInputs {
        sql: "SELECT pay_date, SUM(amount) AS revenue, COUNT(*) AS order_count \
              FROM smelt.sources.payments GROUP BY pay_date",
        output: OutputSpec {
            table: "daily_revenue".to_string(),
            grain: Grain::Partition {
                partition_col: "pay_date".to_string(),
            },
            skeleton_columns: set(&["pay_date"]),
        },
        sources: vec![source(
            "payments",
            MutationProfile::AppendOnly,
            Some("pay_date"),
        )],
        column_groups: vec![
            ColumnGroup {
                columns: strings(&["revenue"]),
                mutation_sensitivity: set(&["payments"]),
            },
            // The added field is co-sensitive with {revenue} but starts at
            // S = ∅ — its own catch-up group until convergence (EX-40).
            ColumnGroup {
                columns: strings(&["order_count"]),
                mutation_sensitivity: set(&["payments"]),
            },
        ],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["order_count"]),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells.len(), 1, "only the added group gets an op");
    let cell = &plan.cells[0];
    assert_eq!(cell.group, "{order_count}");
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.technique, Technique::ColumnScopedMerge);
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
    assert!(cell.ledger_catch_up);
}

#[test]
fn ex40_column_merge_sql_prunes_scan_and_target_and_leaves_siblings_alone() {
    let region = Region {
        start: "DATE '2026-01-01'".to_string(),
        end: "DATE '2026-01-02'".to_string(),
    };
    let stmts = emit_column_scoped_merge(
        "daily_revenue",
        &strings(&["pay_date"]),
        &strings(&["order_count"]),
        "SELECT pay_date, COUNT(*) AS order_count FROM payments GROUP BY pay_date",
        Some("pay_date"),
        Some(&region),
    );
    let sql = &stmts[0];
    // Partition predicate on the scan side and the write target.
    assert_eq!(sql.matches("pay_date >= DATE '2026-01-01'").count(), 2);
    assert!(sql.contains("t.pay_date >= DATE '2026-01-01'"));
    // Writes only the new field; the sibling column is untouched.
    assert!(sql.contains("UPDATE SET order_count = s.order_count"));
    assert!(!sql.contains("revenue = "));
}
