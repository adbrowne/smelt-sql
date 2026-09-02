//! Coverage-matrix conformance sweep (`docs/plans/20260707-maintenance-plan-impl.md`
//! phase MP17): the two named gap-pinning cells this phase's own Known
//! Divergences entries in `docs/specs/incremental_models.md` promise by exact
//! path (`crates/smelt-cli/tests/property_discovery/coverage_matrix_gaps.rs`).
//! These use the pure `smelt_logical::maintenance` derivation directly (no
//! `LinkCProject`/DuckDB execution needed — both cells are refusal-shaped,
//! not equivalence-shaped) rather than the `smelt-maintenance-testkit`
//! harness this file's siblings use.
//!
//! See `crates/smelt-logical/tests/maintenance_plan_conformance.rs::coverage_matrix_is_inhabited`
//! for the full inhabited-cell inventory this phase claims/defers.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts, Technique, Trigger,
};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// EX-08 — orders enriched with an unclocked change-feed dimension
// (`07-example-catalogue.md` EX-08; the "inner-join enrichment / unclocked
// lookup-dim" and "inner-join enrichment / change feed" matrix cells).
// UNSUPPORTED-TODAY per the catalogue.
//
// `incremental_models.md` §Known Divergences ("The trigger-list builder's
// `explicitly_mutable` scoping misses `change_feed`-declared sources
// entirely"): the PRODUCTION query builder
// (`smelt-db::queries::maintenance::derive_model_maintenance_plan`) never
// even constructs an `UpstreamMutation` trigger for a `change_feed` source
// today — "no cell to even refuse" is the gap at that layer. What IS
// directly testable at the pure-derivation level (this test) is the K8
// guardrail that WOULD fire the moment such a trigger is constructed: an
// unclocked source (no derivable partition link) with no `allow_full_scan`
// acceptance refuses with the named `ScanUnbounded` diagnostic rather than
// silently emitting an unbounded full-table technique. This is the
// concrete refusal this construct is one declaration away from today.
// ---------------------------------------------------------------------------

#[test]
fn ex08_unclocked_change_feed_dimension_scan_unbounded() {
    let sql = "SELECT o.order_date, o.order_id, o.amount, d.tier \
               FROM smelt.sources.orders o JOIN smelt.sources.customers_cdc d \
                 ON d.user_id = o.user_id";
    let sources = vec![
        SourceFacts {
            name: "orders".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("order_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "customers_cdc".to_string(),
            // change_feed's admission-time posture (`derive.rs::source_shape`
            // doc comment) — mutable, unclocked lookup, no full-scan
            // acceptance declared.
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        },
    ];
    let skeleton = set(&["order_date", "order_id"]);
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "orders_enriched".to_string(),
            grain: Grain::Partition {
                partition_col: "order_date".to_string(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: vec![ColumnGroup {
            columns: vec!["tier".to_string()],
            mutation_sensitivity: set(&["customers_cdc"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
    };

    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "customers_cdc".to_string(),
        }],
    );
    assert!(
        plan.cells.is_empty(),
        "no technique should be admitted for an unbounded scan: {:?}",
        plan.cells
    );
    assert_eq!(plan.refusals.len(), 1);
    assert!(matches!(
        &plan.refusals[0],
        Refusal::ScanUnbounded { source, .. } if source == "customers_cdc"
    ));
}

// ---------------------------------------------------------------------------
// EX-41/EX-42 — `INTERSECT`/`EXCEPT` set operations
// (`docs/specs/model_properties.md` §Known Divergences: set-op distribution
// classifies `UNION ALL` only; `incremental_models.md` §Known Divergences
// records the maintenance-plan-level consequence directly). Not a matrix
// row in `07-example-catalogue.md` today — added here to pin exactly the
// gap the spec names.
//
// `grouping::derive_column_groups` fails closed on ANY set operation
// (`select.has_set_operation()`), collapsing every payload column into one
// group sensitive to every declared source — the SAME collapse an
// unrecognised CTE/derived-table shape gets, never a silent narrower
// group. This test proves that collapse directly, then shows its
// consequence for the plan: because the production query builder only
// ever constructs `NewData`/`Backfill` triggers for a clocked source
// (`incremental_models.md`'s trigger-list-builder divergence entry — an
// `UpstreamMutation` trigger is never built for a clocked append-only
// source regardless of column grouping), every reachable cell for a
// typical `INTERSECT`/`EXCEPT` model (two clocked, append-only arms) is
// `DeleteInsert` region recompute, matching the spec's "every admitted
// cell is `DeleteInsert` region recompute regardless of source property"
// exactly.
// ---------------------------------------------------------------------------

#[test]
fn ex41_ex42_intersect_no_payload_column_still_delete_insert() {
    for op in ["INTERSECT", "EXCEPT"] {
        let sql = format!(
            "SELECT order_id, order_date FROM smelt.sources.web_orders \
             {op} \
             SELECT order_id, order_date FROM smelt.sources.mobile_orders"
        );
        let sources = vec![
            SourceFacts {
                name: "web_orders".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("order_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
            SourceFacts {
                name: "mobile_orders".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("order_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
        ];
        let skeleton = set(&["order_id", "order_date"]);

        // The classification-level proof: a set operation collapses to the
        // whole-model fail-closed group, never a silently narrower one.
        let grouping = derive_column_groups(&sql, &sources, &skeleton);
        assert!(
            grouping.groups.is_empty() && grouping.degenerate.is_empty(),
            "{op}: both output columns are skeleton here (a pass-through set-op), \
             so there is no payload column to collapse — degenerate: {:?}",
            grouping.degenerate
        );

        // The plan-level consequence: only the recompute family is ever
        // reached for this shape (no `UpstreamMutation` trigger is
        // constructed by the production query builder for a clocked
        // append-only source — `NewData`/`Backfill` are the only triggers
        // that occur in practice).
        let inputs = ModelInputs {
            sql: &sql,
            output: OutputSpec {
                table: "orders_unified".to_string(),
                grain: Grain::Partition {
                    partition_col: "order_date".to_string(),
                },
                skeleton_columns: skeleton,
            },
            sources,
            column_groups: vec![],
            fold: None,
            old_columns: Vec::new(),
            old_sql: None,
            keyed_time_axis: None,
        };
        let plan = derive_maintenance_plan(
            &inputs,
            &[
                Trigger::NewData {
                    source: "web_orders".to_string(),
                },
                Trigger::NewData {
                    source: "mobile_orders".to_string(),
                },
                Trigger::Backfill,
            ],
        );
        assert!(plan.refusals.is_empty(), "{op}: {:?}", plan.refusals);
        assert!(
            plan.cells
                .iter()
                .all(|c| c.technique == Technique::DeleteInsert),
            "{op}: every admitted cell must be DeleteInsert region recompute: {:?}",
            plan.cells
        );
    }
}

// ---------------------------------------------------------------------------
// A genuinely mutation-sensitive `INTERSECT` payload column DOES hit the
// fail-closed whole-model collapse (the case the classification-level
// doc comment above describes in general terms) — this variant proves it
// with a payload column present, complementing the pass-through case above.
// ---------------------------------------------------------------------------

#[test]
fn ex41_ex42_intersect_with_payload_column_collapses_whole_model() {
    let sql = "SELECT order_id, order_date, amount FROM smelt.sources.web_orders \
               INTERSECT \
               SELECT order_id, order_date, amount FROM smelt.sources.mobile_orders";
    let sources = vec![
        SourceFacts {
            name: "web_orders".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: Some("order_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "mobile_orders".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("order_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
    ];
    let skeleton = set(&["order_id", "order_date"]);
    let grouping = derive_column_groups(sql, &sources, &skeleton);
    assert_eq!(grouping.groups.len(), 1);
    assert_eq!(grouping.groups[0].columns, vec!["amount".to_string()]);
    assert_eq!(
        grouping.groups[0].mutation_sensitivity,
        set(&["web_orders", "mobile_orders"]),
        "collapses to sensitive-to-every-declared-source, never a silently narrower group"
    );
    assert!(!grouping.degenerate.is_empty());
    assert!(grouping.degenerate[0].reason.contains("set operation"));
}
