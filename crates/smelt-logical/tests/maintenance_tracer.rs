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
    emit_column_scoped_merge, emit_delete_insert, emit_in_place_update, emit_keyed_fold,
    MaintenanceDialect, Region,
};
use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::skeleton::skeleton_columns;
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
    let sql = "SELECT event_id, user_id, event_date, event_ts, page, referrer \
               FROM smelt.sources.events";
    let sources = vec![source(
        "events",
        MutationProfile::AppendOnly,
        Some("event_date"),
    )];
    // Derived, not hand-supplied (MP4): `event_id` is the declared unique
    // key, `event_date` the partition column — both skeleton by
    // declaration; the mutation-sensitivity grouping then partitions the
    // remaining payload columns by provenance × source `mutation_profile`.
    // `events` is append-only and none of `user_id`/`page`/`referrer`
    // aggregate over it, so the whole payload lands in one group with empty
    // sensitivity — the load-bearing append-only case
    // (`incremental_models.md` §Design "Factoring by mutation-sensitivity").
    let skeleton = skeleton_columns(sql, &["event_id".to_string()], Some("event_date"));
    assert_eq!(skeleton, set(&["event_id", "event_date"]));
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "degenerate: {:?}",
        grouping.degenerate
    );

    ModelInputs {
        sql,
        output: OutputSpec {
            table: "clickstream".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
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
fn ex02_delete_insert_carries_the_partition_predicate_on_the_delete_and_the_clamped_body() {
    let region = Region {
        start: "DATE '2026-01-01'".to_string(),
        end: "DATE '2026-01-08'".to_string(),
    };
    // The caller (the runtime's output clamp — `model_transforms.md` §"the
    // two clamps") is responsible for folding the region predicate into the
    // body it hands the emitter; the emitter does not add a second, outer
    // WHERE to the INSERT (`incremental_models.md` §"Statement emission
    // (single owner)").
    let body = "SELECT event_id, user_id, event_date, event_ts, page, referrer FROM events \
                WHERE event_date >= DATE '2026-01-01' AND event_date < DATE '2026-01-08'";
    let group = emit_delete_insert(
        "clickstream",
        "event_date",
        &region,
        body,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(group.statements.len(), 2);
    assert!(group.statements[0]
        .sql
        .starts_with("DELETE FROM clickstream WHERE"));
    for stmt in &group.statements {
        assert!(
            stmt.sql.contains("event_date >= DATE '2026-01-01'")
                && stmt.sql.contains("event_date < DATE '2026-01-08'"),
            "partition predicate missing from: {}",
            stmt.sql
        );
    }
}

// ---------------------------------------------------------------------------
// EX-07 — orders × unclocked mutable customer dim, inner-joined. `customers`
// is read both in a select item (`c.tier`, value sensitivity) and in the
// JOIN's ON predicate (row-admission position, membership sensitivity) — so
// EVERY payload group (`{tier}` and `{amount, user_id}`, which reads no
// customers column at all) is membership-sensitive to `customers` and gets
// a whole-row DeleteInsert recompute, never a column-scoped merge
// (`docs/specs/incremental_models.md` §"The plan matrix"). Without a
// declared full-scan acceptance the K8 guardrail refuses each group
// (partition-locality fails on `customers`).
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
    let sql = "SELECT o.order_date, o.order_id, o.user_id, o.amount, c.tier \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id";
    let sources = vec![
        source("orders", MutationProfile::AppendOnly, Some("order_date")),
        customers,
    ];
    // Derived, not hand-supplied (MP4): `amount` (and `user_id`) read only
    // the append-only `orders` join input without aggregating over it, so
    // they land in the value-empty-sensitivity group; `tier` reads the
    // mutable `customers` dimension and lands in its own group value-
    // sensitive to `customers`. `customers` is also read in the JOIN's ON
    // predicate, so BOTH groups carry membership sensitivity to it — an
    // inner join's dimension churn can retroactively un-admit an order row
    // no select item ever needed to read `customers` to produce.
    let skeleton = skeleton_columns(sql, &["order_id".to_string()], Some("order_date"));
    assert_eq!(skeleton, set(&["order_id", "order_date"]));
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "degenerate: {:?}",
        grouping.degenerate
    );
    assert!(
        grouping
            .groups
            .iter()
            .any(|g| g.columns.contains(&"tier".to_string())
                && g.mutation_sensitivity == set(&["customers"])),
        "groups: {:?}",
        grouping.groups
    );
    assert!(
        grouping
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["customers"])),
        "every payload group must be membership-sensitive to customers: {:?}",
        grouping.groups
    );

    ModelInputs {
        sql,
        output: OutputSpec {
            table: "orders_tiered".to_string(),
            grain: Grain::Partition {
                partition_col: "order_date".to_string(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
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
    // Every payload group (`{tier}` and `{amount, user_id}`) is now
    // membership-sensitive to `customers`, so each is its own admission
    // attempt and each refuses under the K8 guardrail — two refusals, not
    // one.
    assert_eq!(plan.refusals.len(), 2, "refusals: {:?}", plan.refusals);
    assert!(
        plan.refusals
            .iter()
            .all(|r| matches!(r, Refusal::ScanUnbounded { source, .. } if source == "customers")),
        "refusals: {:?}",
        plan.refusals
    );
}

#[test]
fn ex07_declared_full_scan_admits_the_membership_recompute_as_non_partition_local() {
    let inputs = ex07_inputs(true);
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "customers".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    // Both payload groups are membership-sensitive to `customers` (it is
    // read in the JOIN's ON predicate), so both get a whole-row recompute
    // cell — never a column-scoped merge, even for `{tier}`, which is ALSO
    // value-sensitive to `customers`: membership sensitivity dominates.
    assert_eq!(plan.cells.len(), 2, "cells: {:?}", plan.cells);
    for cell in &plan.cells {
        assert_eq!(cell.corner, Corner::RecomputeRegion);
        assert_eq!(cell.technique, Technique::DeleteInsert);
        assert!(
            matches!(&cell.partition_local, PartitionLocal::No { source, .. } if source == "customers")
        );
    }
    let groups: BTreeSet<String> = plan.cells.iter().map(|c| c.group.clone()).collect();
    assert_eq!(groups, set(&["{tier}", "{amount, user_id}"]));
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
    let sql = "SELECT pay_date, SUM(amount) AS revenue \
               FROM smelt.sources.payments GROUP BY pay_date";
    let sources = vec![source(
        "payments",
        MutationProfile::AppendOnly,
        Some("pay_date"),
    )];
    // Derived, not hand-supplied (MP4): `pay_date` is both the `GROUP BY`
    // key and the partition column, so it's skeleton either way; `revenue`
    // is an aggregate over the append-only `payments` source, so — unlike
    // EX-02/EX-07's non-aggregated append-only reads — new same-day rows
    // still change an already-created output row's value, and the
    // aggregate contributes `payments` sensitivity.
    let skeleton = skeleton_columns(sql, &[], Some("pay_date"));
    assert_eq!(skeleton, set(&["pay_date"]));
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "degenerate: {:?}",
        grouping.degenerate
    );
    assert_eq!(
        grouping.groups,
        vec![ColumnGroup {
            columns: strings(&["revenue"]),
            mutation_sensitivity: set(&["payments"]),
            membership_sensitivity: BTreeSet::new(),
        }]
    );

    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "daily_revenue".to_string(),
            grain: Grain::Partition {
                partition_col: "pay_date".to_string(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
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
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), combiner)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
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
    // `payments` declares no `unique_key`, so the repair narrowing's own
    // affected-key discovery fails closed too, pushing an additive
    // `RepairKeysNotDiscoverable` refusal alongside the pre-existing one
    // (`incremental_models.md` §"The repair family" — fail-closed refusal is
    // additive, never a replacement).
    assert!(plan
        .refusals
        .iter()
        .any(|r| matches!(r, Refusal::NoAdmissibleTechnique { .. })));
    assert!(plan
        .refusals
        .iter()
        .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { .. })));
}

#[test]
fn ex24_keyed_fold_sql_combines_matched_and_inserts_unseen_keys() {
    let group = emit_keyed_fold(
        "lifetime_spend",
        &strings(&["user_id"]),
        &[(
            "lifetime_spend".to_string(),
            "target.lifetime_spend + delta.lifetime_spend".to_string(),
        )],
        "SELECT user_id, SUM(amount) AS lifetime_spend FROM payments_delta GROUP BY user_id",
        None,
        MaintenanceDialect::DuckDb,
    );
    let sql = &group.statements[0].sql;
    assert!(sql.contains("ON target.user_id = delta.user_id"));
    assert!(sql.contains("lifetime_spend = target.lifetime_spend + delta.lifetime_spend"));
    assert!(sql.contains("WHEN NOT MATCHED THEN INSERT *"));
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
        // A registry-recognised pure function (backbuild Phase 3 tightened
        // `collect_dependencies`'s opaqueness check — an unregistered
        // function name now fails closed rather than being silently
        // treated as having no dependencies; `SUBSTRING` stands in for the
        // original `regexp_extract` illustration of "pure function of an
        // existing stored column", which this test's assertions never
        // depended on the specific function for).
        column_def("referrer_domain", "SUBSTRING(referrer, 1, 10)"),
    ];
    let proof = additive_only_diff(&old, &new, &[]);
    assert!(proof.is_additive_only());

    // `classify_definition_change` resolves the added column's own
    // expression from the model's *current* SQL (a `ColumnAdded` trigger
    // fires because `referrer_domain` already exists there) and the
    // additive-only diff's `old_columns` from `ModelInputs::old_columns` —
    // the retired `column_add_proof`'s replacement
    // (`docs/plans/20260808-derived-maintenance-proofs.md` Phase 4).
    let sql = "SELECT event_id, user_id, event_date, event_ts, page, referrer, \
               SUBSTRING(referrer, 1, 10) AS referrer_domain \
               FROM smelt.sources.events";
    let mut inputs = ex02_inputs();
    inputs.sql = sql;
    inputs.column_groups.push(ColumnGroup {
        columns: strings(&["referrer_domain"]),
        mutation_sensitivity: BTreeSet::new(),
        membership_sensitivity: BTreeSet::new(),
    });
    inputs.old_columns = old;
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
        membership_sensitivity: BTreeSet::new(),
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
        [Refusal::DefinitionChangeNotBackfillable { .. }]
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
        Some(("event_date", &region)),
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
        matches!(&plan.refusals[..], [Refusal::SkeletonChanged { column }] if column == "region")
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
                membership_sensitivity: BTreeSet::new(),
            },
            // The added field is co-sensitive with {revenue} but starts at
            // S = ∅ — its own catch-up group until convergence (EX-40).
            ColumnGroup {
                columns: strings(&["order_count"]),
                mutation_sensitivity: set(&["payments"]),
                membership_sensitivity: BTreeSet::new(),
            },
        ],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
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

/// `emit_column_scoped_merge`'s production shape (`docs/specs/
/// incremental_models.md` §"Statement emission (single owner)"): `UPDATE
/// SET *`, keyed on `unique_key`, no column-list `SET` and no predicate of
/// its own on either the scan or the write target — partition-scoping, when
/// the technique is not the declared full-scan case, is folded into
/// `source_select` by the caller (the same convention `emit_delete_insert`'s
/// `body` follows). The caller therefore carries the sibling column
/// (`revenue`) through unchanged in the projection rather than relying on
/// the emitter to leave it untouched via an explicit column list — this
/// replaces the old column-list `SET` form, which never matched production.
#[test]
fn ex40_column_merge_sql_is_set_star_over_the_callers_full_row_projection() {
    let region = Region {
        start: "DATE '2026-01-01'".to_string(),
        end: "DATE '2026-01-02'".to_string(),
    };
    // The caller projects the full target row — `revenue` passed through
    // unchanged from the existing state via a join, `order_count`
    // re-derived — and folds the region predicate into the scan itself.
    let source_select = format!(
        "SELECT p.pay_date, d.revenue, COUNT(*) AS order_count \
         FROM payments p JOIN daily_revenue d ON d.pay_date = p.pay_date \
         WHERE p.pay_date >= {start} AND p.pay_date < {end} \
         GROUP BY p.pay_date, d.revenue",
        start = region.start,
        end = region.end,
    );
    let group = emit_column_scoped_merge(
        "daily_revenue",
        &strings(&["pay_date"]),
        &source_select,
        &[],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(group.statements.len(), 1);
    let sql = &group.statements[0].sql;
    assert!(sql.contains("pay_date >= DATE '2026-01-01'"));
    assert!(sql.contains("ON target.pay_date = source.pay_date"));
    assert!(sql.contains("WHEN MATCHED THEN UPDATE SET *"));
    assert!(sql.contains("WHEN NOT MATCHED THEN INSERT *"));
    // No explicit column-list SET — `revenue` flows through the source
    // projection itself, not through emitter-side sibling exclusion.
    assert!(!sql.contains("order_count = s.order_count"));
}

/// A `GROUP BY` key absent from the declared `output.skeleton_columns` set,
/// added in a group whose `mutation_sensitivity` is non-empty, must still
/// refuse as `SkeletonChanged` — a grain change, never a column
/// backfill (`model_properties.md` §"Definition-change column
/// classification": "SkeletonAdd — refused, a grain change, never a column
/// backfill"). Before this test's fix, `derive_column_added`'s only
/// skeleton guard was the declared-set check in the early refusal loop;
/// `classify_definition_change` (whose leg 1 derives skeleton roles from
/// the SQL itself) only ran inside the *empty*-sensitivity branch, so a
/// non-empty-sensitivity group dispatched straight to `ColumnScopedMerge`
/// without ever deriving the added column's skeleton role.
#[test]
fn ex39b_underived_skeleton_add_in_sensitive_group_still_refuses() {
    let inputs = ModelInputs {
        sql: "SELECT pay_date, region, SUM(amount) AS revenue \
              FROM smelt.sources.payments GROUP BY pay_date, region",
        output: OutputSpec {
            table: "daily_revenue".to_string(),
            grain: Grain::Partition {
                partition_col: "pay_date".to_string(),
            },
            // `region` is a `GROUP BY` key but was never hand-declared —
            // only the derived check catches it.
            skeleton_columns: set(&["pay_date"]),
        },
        sources: vec![source(
            "payments",
            MutationProfile::MutableSnapshot,
            Some("pay_date"),
        )],
        column_groups: vec![
            ColumnGroup {
                columns: strings(&["revenue"]),
                mutation_sensitivity: set(&["payments"]),
                membership_sensitivity: BTreeSet::new(),
            },
            // The added column's own group — non-empty sensitivity, so
            // this exercises the `ColumnScopedMerge` branch, not the
            // in-place-update branch `classify_definition_change` already
            // guarded.
            ColumnGroup {
                columns: strings(&["region"]),
                mutation_sensitivity: set(&["payments"]),
                membership_sensitivity: BTreeSet::new(),
            },
        ],
        fold: None,
        old_columns: vec![
            column_def("pay_date", "pay_date"),
            column_def("revenue", "SUM(amount)"),
        ],
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: strings(&["region"]),
        }],
    );
    assert!(
        matches!(&plan.refusals[..], [Refusal::SkeletonChanged { column }] if column == "region"),
        "refusals: {:?}",
        plan.refusals
    );
    assert!(
        plan.cells
            .iter()
            .all(|c| c.technique != Technique::ColumnScopedMerge),
        "no ColumnScopedMerge cell should be emitted for a skeleton-position add: {:?}",
        plan.cells
    );
}

// ---------------------------------------------------------------------------
// Keyed-enriched shape (`docs/plans/20260808-membership-sensitivity.md`
// Phase 1): a keyed model over an append-only fact joined to an unclocked
// mutable dimension whose columns appear ONLY in the JOIN's ON predicate —
// never in any select item. Membership sensitivity is what makes `dim`'s
// mutations maintainable at all (`docs/specs/model_properties.md`
// §"Per-column mutation-sensitivity / column provenance", membership
// paragraph): value sensitivity alone would leave `dim` invisible to the
// derivation entirely (no cell, no refusal — a quiet equivalence hole).
// ---------------------------------------------------------------------------

#[test]
fn keyed_enriched_dim_mutation_is_membership_sensitive_recompute_never_column_merge() {
    let sql = "SELECT f.id, COUNT(f.val) AS val_count \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.id = d.id \
               GROUP BY f.id";
    let sources = vec![
        SourceFacts {
            name: "fact".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "dim".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: strings(&["id"]),
            allow_full_scan: true,
        },
    ];
    let skeleton = set(&["id"]);
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "degenerate: {:?}",
        grouping.degenerate
    );
    assert!(
        grouping
            .groups
            .iter()
            .any(|g| g.columns.contains(&"val_count".to_string())
                && g.membership_sensitivity == set(&["dim"])),
        "groups: {:?}",
        grouping.groups
    );

    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "fact_dim".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["id"]),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "dim".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells.len(), 1, "cells: {:?}", plan.cells);
    let cell = &plan.cells[0];
    assert_eq!(cell.group, "{val_count}");
    assert_eq!(cell.corner, Corner::RecomputeRegion);
    assert_eq!(cell.technique, Technique::DeleteInsert);
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::ColumnScopedMerge),
        "a membership-sensitive dim cell must never resolve to ColumnScopedMerge: {:?}",
        plan.cells
    );
}

/// The closure-pruning rule (`docs/plans/20260809-sensitivity-precision.md`
/// Phase 4; `model_properties.md` §"Semantics"): the SAME fact+mutable-dim
/// shape as the test above, but the enrichment join is a `LEFT JOIN` in a
/// non-aggregating scope with no membership predicate on `dim` — every
/// skeleton-source-closure conjunct proves `Closed` with row preservation
/// established by the join shape itself, so `dim`'s own `ON` read
/// contributes no membership sensitivity. The `UpstreamMutation(dim)` cell
/// for the (purely value-sensitive) `attr` group must now derive
/// `Corner::ColumnMerge`/`Technique::ColumnScopedMerge` instead of the
/// recompute-region `DeleteInsert` the un-pruned (aggregating, inner-join)
/// shape gets.
#[test]
fn closed_outer_enrichment_join_upstream_mutation_derives_column_scoped_merge() {
    let sql = "SELECT f.id, f.amount, d.attr AS attr \
               FROM smelt.sources.fact f \
               LEFT JOIN smelt.sources.dim d ON f.id = d.id";
    let sources = vec![
        SourceFacts {
            name: "fact".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "dim".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: strings(&["id"]),
            allow_full_scan: true,
        },
    ];
    let skeleton = set(&["id"]);
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "degenerate: {:?}",
        grouping.degenerate
    );
    let attr_group = grouping
        .groups
        .iter()
        .find(|g| g.columns.contains(&"attr".to_string()))
        .expect("attr group");
    assert!(
        attr_group.membership_sensitivity.is_empty(),
        "the closed LEFT JOIN's own ON read must contribute no membership \
         sensitivity: {attr_group:?}"
    );
    assert_eq!(attr_group.mutation_sensitivity, set(&["dim"]));

    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "fact_dim".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["id"]),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "dim".to_string(),
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    let cell = plan
        .cells
        .iter()
        .find(|c| c.group == "{attr}")
        .unwrap_or_else(|| panic!("no {{attr}} cell: {:?}", plan.cells));
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.technique, Technique::ColumnScopedMerge);
}
