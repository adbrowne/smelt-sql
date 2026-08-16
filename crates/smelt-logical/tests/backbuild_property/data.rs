//! Fixed source pool (research `docs/research/20260802-backbuild-synthesis.md`
//! §6, `docs/plans/20260802-backbuild-followups.md` Phase 7) plus generated
//! fact-table data for the generative backbuild conformance gate.
//!
//! The fact table (`src_orders`) is randomly generated per case (bounded
//! ints/dates, deliberate NULLs in `status`, duplicate payload rows via
//! small bounded domains, plus a fixed set of predicate-boundary rows always
//! appended). The two dimension tables (`src_dim`, `src_dim2`) are a small
//! fixed fixture — deterministic enough to make join fan-out/miss shapes
//! (zero matches, multiple matches, a NULL dimension attribute) predictable
//! by construction, per `recipe.rs`/`render.rs`'s reliance on them.
//!
//! Both this module's `CREATE TABLE` DDL and `render.rs`'s `BackbuildInputs`
//! facts (unique keys, `NOT NULL` columns) are rendered from the *same*
//! `recipe::schema` `TableSchema` constants (`orders_schema`/`dim_schema`/
//! `dim2_schema`) — one set of column facts, two consumers, so a name, type,
//! or nullability edit can only be made in one place (research §4 intro "Key
//! addressability": declared facts must be true by construction).

#![allow(dead_code)]

use proptest::prelude::*;

use super::recipe::schema;

/// `src_dim` DDL, rendered from [`schema::dim_schema`]: `dim_id INTEGER NOT
/// NULL /*unique*/, region VARCHAR, score INTEGER`. `dim_id` 1..=4 —
/// `src_orders.customer_id` ranges 1..=6, so customer_ids 5 and 6 always
/// miss (LEFT JOIN NULL-extension coverage); dim_id 3's `region` is NULL (a
/// nullable dimension attribute reaching a matched fact row).
pub fn dim_ddl() -> String {
    format!(
        "{}\nINSERT INTO src_dim VALUES (1, 'north', 10), (2, 'south', 20), (3, NULL, 30), \
         (4, 'east', 5);",
        schema::dim_schema().create_table_sql()
    )
}

/// `src_dim2` DDL, rendered from [`schema::dim2_schema`]: `zone_id INTEGER
/// NOT NULL /*unique*/, zone VARCHAR` — the B7 second-hop / F1-F2 branch
/// source. `zone_id` values (101..=103) are disjoint from
/// `src_orders.order_id`'s range so a UNION ALL branch built from this table
/// never collides with the fact branch's row identity.
pub fn dim2_ddl() -> String {
    format!(
        "{}\nINSERT INTO src_dim2 VALUES (101, 'zoneA'), (102, NULL), (103, 'zoneC');",
        schema::dim2_schema().create_table_sql()
    )
}

/// `src_orders(order_id INTEGER NOT NULL /*unique*/, customer_id INTEGER NOT
/// NULL, amount INTEGER NOT NULL, qty INTEGER NOT NULL, status VARCHAR, ts
/// DATE NOT NULL)`. `status` is the pool's only nullable fact column
/// (three-valued-logic coverage for the E-class).
#[derive(Debug, Clone)]
pub struct OrderRow {
    pub order_id: i64,
    pub customer_id: i64,
    pub amount: i64,
    pub qty: i64,
    pub status: Option<&'static str>,
    pub ts: &'static str,
}

fn arb_status() -> impl Strategy<Value = Option<&'static str>> {
    prop_oneof![
        3 => Just(Some("open")),
        3 => Just(Some("closed")),
        1 => Just(None),
    ]
}

/// A small, fixed pool of dates spanning the E4 horizon-widening boundary
/// (`WhereKind::TsHorizon`'s two candidate bounds, 2024-01-01 and
/// 2025-01-01, per `recipe.rs`) so random draws routinely land on both sides
/// of both bounds.
fn arb_ts() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "2023-03-15",
        "2023-11-02",
        "2024-06-10",
        "2024-12-31",
        "2025-06-01",
        "2025-09-20",
        "2026-02-14",
        "2026-08-01",
    ])
}

/// 10 randomly generated fact rows, `order_id` 1..=10 — bounded
/// `customer_id` (1..=6, so 5/6 always miss `src_dim`), bounded
/// `amount`/`qty` (including negative/zero, boundary-adjacent), and
/// `status` with deliberate NULLs. Small bounded domains make duplicate
/// payload rows (same amount/qty/status/ts under different `order_id`s)
/// routine without needing to force them explicitly.
pub fn arb_order_rows() -> impl Strategy<Value = Vec<OrderRow>> {
    proptest::collection::vec(
        (1i64..=6, -5i64..=50, 0i64..=20, arb_status(), arb_ts()),
        10,
    )
    .prop_map(|draws| {
        draws
            .into_iter()
            .enumerate()
            .map(|(i, (customer_id, amount, qty, status, ts))| OrderRow {
                order_id: (i as i64) + 1,
                customer_id,
                amount,
                qty,
                status,
                ts,
            })
            .collect()
    })
}

/// Fixed predicate-boundary rows (research §4 "E4 widening"), always
/// appended alongside the random draw — `order_id` 91..=94, disjoint from
/// the random rows' 1..=10 range. `customer_id` 5/6 pin a guaranteed
/// zero-match dimension lookup; `amount`/`qty` 0 pin the `> 0` boundary;
/// `ts` pins both E4 horizon bounds exactly.
pub fn boundary_rows() -> Vec<OrderRow> {
    vec![
        OrderRow {
            order_id: 91,
            customer_id: 1,
            amount: 0,
            qty: 5,
            status: Some("open"),
            ts: "2024-01-01",
        },
        OrderRow {
            order_id: 92,
            customer_id: 2,
            amount: -1,
            qty: 0,
            status: None,
            ts: "2025-01-01",
        },
        OrderRow {
            order_id: 93,
            customer_id: 5,
            amount: 10,
            qty: 3,
            status: Some("closed"),
            ts: "2023-12-31",
        },
        OrderRow {
            order_id: 94,
            customer_id: 6,
            amount: 1,
            qty: 1,
            status: Some("open"),
            ts: "2026-01-01",
        },
    ]
}

fn render_orders_ddl_and_data(rows: &[OrderRow]) -> String {
    let mut sql = schema::orders_schema().create_table_sql();
    sql.push('\n');
    sql.push_str("INSERT INTO src_orders VALUES\n");
    let values: Vec<String> = rows
        .iter()
        .map(|r| {
            let status = match r.status {
                Some(s) => format!("'{s}'"),
                None => "NULL".to_string(),
            };
            format!(
                "({}, {}, {}, {}, {status}, DATE '{}')",
                r.order_id, r.customer_id, r.amount, r.qty, r.ts
            )
        })
        .collect();
    sql.push_str(&values.join(",\n"));
    sql.push(';');
    sql
}

/// The full staging script for one case: `src_orders` (random rows + fixed
/// boundary rows) plus the fixed `src_dim`/`src_dim2` fixtures — every
/// table `render.rs`'s recipes might reference, staged unconditionally
/// (an unused table costs nothing).
pub fn full_staging_sql(random_rows: &[OrderRow]) -> String {
    let mut all_rows = random_rows.to_vec();
    all_rows.extend(boundary_rows());
    format!(
        "{}\n{}\n{}\n",
        render_orders_ddl_and_data(&all_rows),
        dim_ddl(),
        dim2_ddl()
    )
}
