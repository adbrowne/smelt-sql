//! Generative metamorphic equivalence gate for the windowing transformers.
//!
//! The incremental core partitions a run's time domain into windows, wraps
//! each source scan in a pushdown filter (`inject_source_filters`), and
//! clamps the model's output to the write window (`inject_time_filter`).
//! The relation those transforms promise — and this gate proves executably —
//! is: for any partition of the domain into disjoint half-open windows, the
//! union of per-window runs equals the single unwindowed run. If any clamp
//! boundary is off by one, a pushdown margin under- or over-reads, or the
//! two transforms interact badly, disjointness or coverage breaks and the
//! two-way `EXCEPT ALL` comparison reports rows lost or invented.
//!
//! Three relations are asserted per generated case (model shape × data ×
//! window partition × pushdown margins):
//!
//!   A. clamp(M, full domain) == raw M minus NULL-event-time rows — pins
//!      the clamp's NULL semantics explicitly (`col >= s AND col < e` is
//!      NULL-rejecting).
//!   B. UNION ALL of clamp(M, wᵢ) over the partition == clamp(M, full).
//!   C. UNION ALL of clamp(pushdown(M, wᵢ ± margin), wᵢ) == clamp(M, full)
//!      — the composed production shape, for partition-aligned models
//!      (the event-time column passes through to the output), under
//!      randomized lookback/lookahead margins.
//!
//! Determinism: `TestRunner::deterministic()` throughout; no wall-clock or
//! ambient randomness. Case count via `SMELT_TRANSFORMER_CASES`.

#![cfg(feature = "duckdb")]

use duckdb::Connection;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use smelt_maintenance_testkit::oracle::except_all_row_count;
use smelt_runtime::{inject_source_filters, inject_time_filter, SourceBound, TimeRange};
use std::collections::HashMap;

const DEFAULT_CASES: usize = 64;
const SOURCE_REF: &str = "smelt.sources.events";
/// The date domain: day offsets 0..SPAN_DAYS map to 2024-01-01..2024-01-11.
const SPAN_DAYS: u32 = 10;

fn case_count() -> usize {
    std::env::var("SMELT_TRANSFORMER_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

fn date(offset: u32) -> String {
    format!("2024-01-{:02}", 1 + offset)
}

// ─── Recipe ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum WherePred {
    VPositive,
    KNotNullVSmall,
}

impl WherePred {
    fn sql(&self) -> &'static str {
        match self {
            WherePred::VPositive => "v > 0",
            WherePred::KNotNullVSmall => "k IS NOT NULL AND v < 3",
        }
    }
}

/// Partition-aligned model shapes: the event-time column `dt` passes
/// through to the output schema, licensing both the output clamp and the
/// per-window pushdown (a shape where it does not is refused upstream by
/// admission and is out of scope for this relation).
#[derive(Debug, Clone)]
enum Shape {
    /// SELECT dt, k, v FROM src [WHERE p]
    Projection,
    /// SELECT dt[, k], COUNT(*) as c, SUM(v) as s FROM src [WHERE p] GROUP BY dt[, k]
    Grouped { extra_key: bool },
}

/// One source row: (day offset or NULL, k, v). Aggregates stay exact
/// (COUNT/SUM over INTEGER), so fold order cannot fake a difference.
type Row = (Option<u32>, Option<i32>, Option<i32>);

#[derive(Debug, Clone)]
struct WindowRecipe {
    shape: Shape,
    where_pred: Option<WherePred>,
    rows: Vec<Row>,
    /// Interior cut points (day offsets in 1..SPAN_DAYS); with the domain
    /// endpoints they form the half-open window partition.
    cuts: Vec<u32>,
    before_days: u64,
    after_days: u64,
}

impl WindowRecipe {
    /// The model SQL, with the source written as a `smelt.<path>` ref
    /// exactly as `inject_source_filters` sees it in production.
    fn model_sql(&self) -> String {
        let where_clause = self
            .where_pred
            .as_ref()
            .map(|p| format!(" WHERE {}", p.sql()))
            .unwrap_or_default();
        match &self.shape {
            Shape::Projection => {
                format!("SELECT dt, k, v FROM {SOURCE_REF}{where_clause}")
            }
            Shape::Grouped { extra_key } => {
                let (key_cols, group_by) = if *extra_key {
                    ("dt, k", "GROUP BY dt, k")
                } else {
                    ("dt", "GROUP BY dt")
                };
                format!(
                    "SELECT {key_cols}, COUNT(*) as c, SUM(v) as s \
                     FROM {SOURCE_REF}{where_clause} {group_by}"
                )
            }
        }
    }

    /// Output column list, for name-projected comparison.
    fn columns(&self) -> &'static str {
        match &self.shape {
            Shape::Projection => "dt, k, v",
            Shape::Grouped { extra_key: true } => "dt, k, c, s",
            Shape::Grouped { extra_key: false } => "dt, c, s",
        }
    }

    /// Window boundaries: domain start, sorted deduped interior cuts, domain end.
    fn boundaries(&self) -> Vec<u32> {
        let mut b = vec![0];
        let mut cuts = self.cuts.clone();
        cuts.sort_unstable();
        cuts.dedup();
        b.extend(cuts);
        b.push(SPAN_DAYS);
        b
    }
}

// ─── Generators ─────────────────────────────────────────────────────────────

fn arb_row() -> impl Strategy<Value = Row> {
    (
        prop::option::weighted(0.9, 0u32..SPAN_DAYS),
        prop::option::weighted(0.85, 0i32..3),
        prop::option::weighted(0.85, -5i32..5),
    )
}

fn arb_recipe() -> impl Strategy<Value = WindowRecipe> {
    (
        prop_oneof![
            Just(Shape::Projection),
            Just(Shape::Grouped { extra_key: false }),
            Just(Shape::Grouped { extra_key: true }),
        ],
        prop::option::weighted(
            0.5,
            prop_oneof![Just(WherePred::VPositive), Just(WherePred::KNotNullVSmall)],
        ),
        prop::collection::vec(arb_row(), 0..50),
        prop::collection::vec(1u32..SPAN_DAYS, 0..=3),
        0u64..=2,
        0u64..=2,
    )
        .prop_map(
            |(shape, where_pred, rows, cuts, before_days, after_days)| WindowRecipe {
                shape,
                where_pred,
                rows,
                cuts,
                before_days,
                after_days,
            },
        )
}

// ─── Staging and execution ──────────────────────────────────────────────────

fn stage_events(conn: &Connection, rows: &[Row]) {
    conn.execute_batch("CREATE TABLE events (dt DATE, k INTEGER, v INTEGER)")
        .expect("create events");
    if rows.is_empty() {
        return;
    }
    let values: Vec<String> = rows
        .iter()
        .map(|(d, k, v)| {
            let d = d
                .map(|off| format!("DATE '{}'", date(off)))
                .unwrap_or_else(|| "NULL".into());
            let k = k.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
            let v = v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into());
            format!("({d}, {k}, {v})")
        })
        .collect();
    conn.execute_batch(&format!("INSERT INTO events VALUES {}", values.join(", ")))
        .expect("insert events");
}

/// Resolve the `smelt.<path>` ref to the staged table, as compilation would.
fn resolve(sql: &str) -> String {
    sql.replace(SOURCE_REF, "events")
}

fn materialize(conn: &Connection, table: &str, sql: &str) {
    conn.execute_batch(&format!("CREATE TABLE {table} AS {sql}"))
        .unwrap_or_else(|e| panic!("materializing {table} failed: {e}\nSQL: {sql}"));
}

fn assert_multiset_eq(
    conn: &Connection,
    left: &str,
    right: &str,
    relation: &str,
    recipe: &WindowRecipe,
) {
    let missing = except_all_row_count(conn, left, right);
    let extra = except_all_row_count(conn, right, left);
    assert!(
        missing == 0 && extra == 0,
        "{relation} violated ({missing} rows lost, {extra} rows invented)\n\
         left:  {left}\nright: {right}\nrecipe: {recipe:#?}\nmodel SQL: {}",
        recipe.model_sql(),
    );
}

// ─── The gate ───────────────────────────────────────────────────────────────

#[test]
fn window_partition_union_equals_unwindowed_run() {
    let mut runner = TestRunner::deterministic();
    let strat = arb_recipe();

    let mut multi_window_cases = 0usize;
    let mut null_dt_cases = 0usize;

    for case in 0..case_count() {
        let recipe = strat
            .new_tree(&mut runner)
            .expect("generate recipe")
            .current();

        let conn = Connection::open_in_memory().expect("open duckdb");
        stage_events(&conn, &recipe.rows);

        let model_sql = recipe.model_sql();
        let cols = recipe.columns();
        let boundaries = recipe.boundaries();
        let windows: Vec<(String, String)> = boundaries
            .windows(2)
            .map(|w| (date(w[0]), date(w[1])))
            .collect();
        let full_range = TimeRange {
            start: date(0),
            end: date(SPAN_DAYS),
        };

        // Baselines: the raw resolved model, and the full-domain clamp.
        materialize(&conn, "raw_result", &resolve(&model_sql));
        let full_clamped = inject_time_filter(&resolve(&model_sql), "dt", &full_range)
            .unwrap_or_else(|e| panic!("case {case}: full-domain clamp refused: {e:?}"));
        materialize(&conn, "full_result", &full_clamped);

        // Relation A: the full-domain clamp drops exactly the NULL-dt rows.
        assert_multiset_eq(
            &conn,
            &format!("SELECT {cols} FROM full_result"),
            &format!("SELECT {cols} FROM raw_result WHERE dt IS NOT NULL"),
            &format!("case {case}, relation A (full clamp == raw minus NULL dt)"),
            &recipe,
        );

        let bounds: HashMap<String, SourceBound> = HashMap::from([(
            SOURCE_REF.to_string(),
            SourceBound {
                partition_col: "dt".to_string(),
                before_secs: recipe.before_days * 86_400,
                after_secs: recipe.after_days * 86_400,
            },
        )]);

        // Per-window runs: clamp-only (relation B) and pushdown+clamp
        // (relation C), in the production order (source filters, then the
        // outer output clamp).
        for (i, (start, end)) in windows.iter().enumerate() {
            let range = TimeRange {
                start: start.clone(),
                end: end.clone(),
            };
            let clamped = inject_time_filter(&model_sql, "dt", &range)
                .unwrap_or_else(|e| panic!("case {case}: clamp refused: {e:?}"));
            materialize(&conn, &format!("clamp_{i}"), &resolve(&clamped));

            let pushed = inject_source_filters(&model_sql, &bounds, &range);
            let composed = inject_time_filter(&pushed, "dt", &range)
                .unwrap_or_else(|e| panic!("case {case}: composed clamp refused: {e:?}"));
            materialize(&conn, &format!("composed_{i}"), &resolve(&composed));
        }

        let union_of = |prefix: &str| -> String {
            (0..windows.len())
                .map(|i| format!("SELECT {cols} FROM {prefix}_{i}"))
                .collect::<Vec<_>>()
                .join(" UNION ALL ")
        };

        assert_multiset_eq(
            &conn,
            &union_of("clamp"),
            &format!("SELECT {cols} FROM full_result"),
            &format!("case {case}, relation B (union of window clamps == full clamp)"),
            &recipe,
        );
        assert_multiset_eq(
            &conn,
            &union_of("composed"),
            &format!("SELECT {cols} FROM full_result"),
            &format!(
                "case {case}, relation C (union of pushdown+clamp windows == full clamp, \
                 margins -{}/+{} days)",
                recipe.before_days, recipe.after_days,
            ),
            &recipe,
        );

        if windows.len() > 1 {
            multi_window_cases += 1;
        }
        if recipe.rows.iter().any(|(d, _, _)| d.is_none()) {
            null_dt_cases += 1;
        }
    }

    // Generator health: the interesting corners must actually be exercised.
    assert!(
        multi_window_cases >= case_count() / 4,
        "only {multi_window_cases}/{} cases had more than one window",
        case_count(),
    );
    assert!(
        null_dt_cases >= case_count() / 8,
        "only {null_dt_cases}/{} cases carried NULL event times",
        case_count(),
    );
}

/// The clamp refuses a qualified event-time column — the wrapping projection
/// has no inner aliases to qualify by. Pinned so the refusal contract stays.
#[test]
fn qualified_clamp_column_is_refused() {
    let range = TimeRange {
        start: date(0),
        end: date(SPAN_DAYS),
    };
    let result = inject_time_filter("SELECT e.dt FROM events e", "e.dt", &range);
    assert!(result.is_err(), "qualified clamp column must be refused");
}
