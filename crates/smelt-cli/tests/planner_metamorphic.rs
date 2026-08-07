//! Generative metamorphic equivalence gate for planner rewrites.
//!
//! The planner's promise (README differentiator #2) is that a rewrite either
//! preserves correctness or refuses with a diagnostic — never a silent
//! approximation. This gate proves that promise executably for the
//! `cube_split` rewrite: it draws structural query recipes (never free SQL
//! text), stages randomized data in an in-memory DuckDB, runs the original
//! query and the planner's rewritten `ExecutionStep` plan side by side, and
//! asserts the two results are multiset-equal (two-way `EXCEPT ALL`, so
//! multiplicity bugs cannot hide).
//!
//! Recipes deliberately include shapes the rewrite cannot preserve (HAVING,
//! ORDER BY + LIMIT, SELECT DISTINCT, QUALIFY). For those the gate asserts
//! the planner *refuses* — an error naming the model — rather than emitting
//! a plan that silently drops the clause.
//!
//! Determinism: `TestRunner::deterministic()` throughout; no wall-clock or
//! ambient randomness. Case count via `SMELT_METAMORPHIC_CASES`.

#![cfg(feature = "duckdb")]

use duckdb::Connection;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use smelt_maintenance_testkit::oracle::except_all_row_count;
use smelt_planner::{ModelGraph, ModelInfo, Planner, Transformation};

const DEFAULT_CASES: usize = 64;

fn case_count() -> usize {
    std::env::var("SMELT_METAMORPHIC_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

// ─── Recipe ─────────────────────────────────────────────────────────────────

/// A grouping key: (expression over the events table, alias).
#[derive(Debug, Clone, PartialEq)]
enum GroupKey {
    K1,
    K2,
    K1Mod2,
}

impl GroupKey {
    fn expr(&self) -> &'static str {
        match self {
            GroupKey::K1 => "k1",
            GroupKey::K2 => "k2",
            GroupKey::K1Mod2 => "k1 % 2",
        }
    }
}

/// Argument of a COUNT(DISTINCT ...) item.
#[derive(Debug, Clone)]
enum DistinctArg {
    D1,
    D2,
    V,
    D1Plus1,
}

impl DistinctArg {
    fn expr(&self) -> &'static str {
        match self {
            DistinctArg::D1 => "d1",
            DistinctArg::D2 => "d2",
            DistinctArg::V => "v",
            DistinctArg::D1Plus1 => "d1 + 1",
        }
    }
}

/// A non-distinct aggregate item. Exact-integer aggregates only: the split
/// plan may fold rows in a different order, so order-sensitive floating
/// aggregates (SUM(DOUBLE), AVG) would produce false differences.
#[derive(Debug, Clone)]
enum OtherAgg {
    CountStar,
    SumV,
    MinD1,
    MaxD2,
}

impl OtherAgg {
    fn expr(&self) -> &'static str {
        match self {
            OtherAgg::CountStar => "COUNT(*)",
            OtherAgg::SumV => "SUM(v)",
            OtherAgg::MinD1 => "MIN(d1)",
            OtherAgg::MaxD2 => "MAX(d2)",
        }
    }
}

#[derive(Debug, Clone)]
enum WherePred {
    VPositive,
    D2NotNull,
    K1NullOrVSmall,
}

impl WherePred {
    fn sql(&self) -> &'static str {
        match self {
            WherePred::VPositive => "v > 0",
            WherePred::D2NotNull => "d2 IS NOT NULL",
            WherePred::K1NullOrVSmall => "k1 IS NULL OR v < 3",
        }
    }
}

/// Clauses the cube_split rewrite cannot reproduce. A recipe carrying one of
/// these must be *refused* by the planner, never rewritten.
#[derive(Debug, Clone, PartialEq)]
enum UnsupportedClause {
    Having,
    OrderByLimit,
    Distinct,
}

/// One row of the events table. `None` renders as NULL.
type Row = (
    Option<i32>,
    Option<&'static str>,
    Option<i32>,
    Option<&'static str>,
    Option<i32>,
);

#[derive(Debug, Clone)]
struct CubeRecipe {
    group_keys: Vec<GroupKey>,
    distincts: Vec<DistinctArg>,
    others: Vec<OtherAgg>,
    where_pred: Option<WherePred>,
    unsupported: Option<UnsupportedClause>,
    rows: Vec<Row>,
}

impl CubeRecipe {
    /// Render the model SQL with the `-- smelt:cube_split` annotation.
    fn sql(&self) -> String {
        let mut items: Vec<String> = Vec::new();
        for (i, k) in self.group_keys.iter().enumerate() {
            items.push(format!("{} as g{}", k.expr(), i));
        }
        for (i, d) in self.distincts.iter().enumerate() {
            items.push(format!("COUNT(DISTINCT {}) as c{}", d.expr(), i));
        }
        for (i, o) in self.others.iter().enumerate() {
            items.push(format!("{} as o{}", o.expr(), i));
        }

        let distinct_kw = if self.unsupported == Some(UnsupportedClause::Distinct) {
            "DISTINCT "
        } else {
            ""
        };
        let mut sql = format!("SELECT {}{} FROM events", distinct_kw, items.join(", "));
        if let Some(p) = &self.where_pred {
            sql.push_str(&format!(" WHERE {}", p.sql()));
        }
        if !self.group_keys.is_empty() {
            let exprs: Vec<&str> = self.group_keys.iter().map(|k| k.expr()).collect();
            sql.push_str(&format!(" GROUP BY {}", exprs.join(", ")));
        }
        match &self.unsupported {
            Some(UnsupportedClause::Having) => sql.push_str(" HAVING COUNT(*) > 1"),
            Some(UnsupportedClause::OrderByLimit) => sql.push_str(" ORDER BY 1 LIMIT 2"),
            Some(UnsupportedClause::Distinct) | None => {}
        }
        sql.push_str(" -- smelt:cube_split");
        sql
    }

    /// Column aliases in the canonical (key, distinct, other) order.
    fn aliases(&self) -> Vec<String> {
        let mut cols = Vec::new();
        for i in 0..self.group_keys.len() {
            cols.push(format!("g{}", i));
        }
        for i in 0..self.distincts.len() {
            cols.push(format!("c{}", i));
        }
        for i in 0..self.others.len() {
            cols.push(format!("o{}", i));
        }
        cols
    }
}

// ─── Generators ─────────────────────────────────────────────────────────────

fn arb_group_keys() -> impl Strategy<Value = Vec<GroupKey>> {
    prop_oneof![
        1 => Just(vec![]),
        4 => prop_oneof![
            Just(vec![GroupKey::K1]),
            Just(vec![GroupKey::K2]),
            Just(vec![GroupKey::K1Mod2]),
        ],
        3 => prop_oneof![
            Just(vec![GroupKey::K1, GroupKey::K2]),
            Just(vec![GroupKey::K2, GroupKey::K1Mod2]),
        ],
    ]
}

fn arb_distinct() -> impl Strategy<Value = DistinctArg> {
    prop_oneof![
        Just(DistinctArg::D1),
        Just(DistinctArg::D2),
        Just(DistinctArg::V),
        Just(DistinctArg::D1Plus1),
    ]
}

fn arb_other() -> impl Strategy<Value = OtherAgg> {
    prop_oneof![
        Just(OtherAgg::CountStar),
        Just(OtherAgg::SumV),
        Just(OtherAgg::MinD1),
        Just(OtherAgg::MaxD2),
    ]
}

fn arb_row() -> impl Strategy<Value = Row> {
    (
        prop::option::weighted(0.85, 0i32..3),
        prop::option::weighted(0.85, prop_oneof![Just("a"), Just("b")]),
        prop::option::weighted(0.85, 0i32..4),
        prop::option::weighted(0.85, prop_oneof![Just("x"), Just("y"), Just("z")]),
        prop::option::weighted(0.85, -5i32..5),
    )
}

fn arb_recipe() -> impl Strategy<Value = CubeRecipe> {
    (
        arb_group_keys(),
        prop::collection::vec(arb_distinct(), 2..=4),
        prop::collection::vec(arb_other(), 0..=2),
        prop::option::weighted(
            0.5,
            prop_oneof![
                Just(WherePred::VPositive),
                Just(WherePred::D2NotNull),
                Just(WherePred::K1NullOrVSmall),
            ],
        ),
        prop::option::weighted(
            0.25,
            prop_oneof![
                Just(UnsupportedClause::Having),
                Just(UnsupportedClause::OrderByLimit),
                Just(UnsupportedClause::Distinct),
            ],
        ),
        prop::collection::vec(arb_row(), 0..40),
    )
        .prop_map(
            |(group_keys, distincts, others, where_pred, unsupported, rows)| CubeRecipe {
                group_keys,
                distincts,
                others,
                where_pred,
                unsupported,
                rows,
            },
        )
}

// ─── Staging and execution ──────────────────────────────────────────────────

fn stage_events(conn: &Connection, rows: &[Row]) {
    conn.execute_batch(
        "CREATE TABLE events (k1 INTEGER, k2 VARCHAR, d1 INTEGER, d2 VARCHAR, v INTEGER)",
    )
    .expect("create events");
    if rows.is_empty() {
        return;
    }
    fn int(v: Option<i32>) -> String {
        v.map(|x| x.to_string()).unwrap_or_else(|| "NULL".into())
    }
    fn txt(v: Option<&str>) -> String {
        v.map(|s| format!("'{s}'")).unwrap_or_else(|| "NULL".into())
    }
    let values: Vec<String> = rows
        .iter()
        .map(|(k1, k2, d1, d2, v)| {
            format!(
                "({}, {}, {}, {}, {})",
                int(*k1),
                txt(*k2),
                int(*d1),
                txt(*d2),
                int(*v)
            )
        })
        .collect();
    conn.execute_batch(&format!("INSERT INTO events VALUES {}", values.join(", ")))
        .expect("insert events");
}

/// Run the planner over a single-model graph; return `Ok(steps)` when the
/// rewrite fired, `Err(errors)` when the planner refused.
fn plan_recipe(recipe: &CubeRecipe) -> Result<Vec<smelt_planner::ExecutionStep>, Vec<String>> {
    let model = ModelInfo {
        name: "cube_model".to_string(),
        sql: recipe.sql(),
        refs: vec![],
        timeseries_config: None,
        incremental_config: None,
    };
    let mut graph = ModelGraph::new();
    graph.add_model(model);
    let (transformations, errors) = Planner::new().plan(&graph);
    if !errors.is_empty() {
        return Err(errors);
    }
    let steps = transformations
        .into_iter()
        .find_map(|t| match t {
            Transformation::ReplaceWithPlan { steps, .. } => Some(steps),
            _ => None,
        })
        .ok_or_else(|| vec!["planner produced no ReplaceWithPlan".to_string()])?;
    Ok(steps)
}

/// Execute the rewritten plan on `conn`, materializing the final query as
/// table `split_result`.
fn execute_steps(conn: &Connection, steps: &[smelt_planner::ExecutionStep]) {
    use smelt_planner::ExecutionStep;
    for step in steps {
        match step {
            ExecutionStep::CreateTemp { name, sql } => {
                conn.execute_batch(&format!("CREATE TEMP TABLE {name} AS {sql}"))
                    .unwrap_or_else(|e| panic!("CreateTemp {name} failed: {e}\nSQL: {sql}"));
            }
            ExecutionStep::AppendToTemp { name, sql } => {
                conn.execute_batch(&format!("INSERT INTO {name} {sql}"))
                    .unwrap_or_else(|e| panic!("AppendToTemp {name} failed: {e}\nSQL: {sql}"));
            }
            ExecutionStep::FinalQuery { sql } => {
                conn.execute_batch(&format!("CREATE TABLE split_result AS {sql}"))
                    .unwrap_or_else(|e| panic!("FinalQuery failed: {e}\nSQL: {sql}"));
            }
            ExecutionStep::DropTemp { name } => {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS {name}"))
                    .unwrap_or_else(|e| panic!("DropTemp {name} failed: {e}"));
            }
        }
    }
}

/// Assert the naive and rewritten results are multiset-equal under a
/// name-keyed projection (canonical alias order on both sides, so a column
/// reordering in the rewrite cannot mask or fake a difference).
fn assert_multiset_equal(conn: &Connection, recipe: &CubeRecipe) {
    let cols = recipe.aliases().join(", ");
    let left = format!("SELECT {cols} FROM naive_result");
    let right = format!("SELECT {cols} FROM split_result");
    let missing = except_all_row_count(conn, &left, &right);
    let extra = except_all_row_count(conn, &right, &left);
    assert!(
        missing == 0 && extra == 0,
        "cube_split rewrite diverges from the naive query \
         ({missing} rows lost, {extra} rows invented)\nmodel SQL: {}\nrecipe: {recipe:#?}",
        recipe.sql(),
    );
}

// ─── The gate ───────────────────────────────────────────────────────────────

/// Every admitted rewrite is result-equivalent; every recipe carrying a
/// clause the rewrite cannot reproduce is refused with an error naming the
/// model. Admission floors at the bottom guard against the generator
/// silently degenerating into all-refused (or all-clean) pools.
#[test]
fn cube_split_rewrite_upholds_result_equivalence() {
    let mut runner = TestRunner::deterministic();
    let strat = arb_recipe();

    let mut fired = 0usize;
    let mut refused_unsupported = 0usize;

    for case in 0..case_count() {
        let recipe = strat
            .new_tree(&mut runner)
            .expect("generate recipe")
            .current();

        match plan_recipe(&recipe) {
            Ok(steps) => {
                assert!(
                    recipe.unsupported.is_none(),
                    "case {case}: planner rewrote a query whose {:?} clause the \
                     cube_split plan cannot reproduce — this silently drops the \
                     clause instead of refusing.\nmodel SQL: {}",
                    recipe.unsupported,
                    recipe.sql(),
                );
                let conn = Connection::open_in_memory().expect("open duckdb");
                stage_events(&conn, &recipe.rows);
                conn.execute_batch(&format!("CREATE TABLE naive_result AS {}", recipe.sql()))
                    .unwrap_or_else(|e| panic!("naive query failed: {e}\nSQL: {}", recipe.sql()));
                execute_steps(&conn, &steps);
                assert_multiset_equal(&conn, &recipe);
                fired += 1;
            }
            Err(errors) => {
                assert!(
                    recipe.unsupported.is_some(),
                    "case {case}: planner refused a supported recipe: {errors:?}\nmodel SQL: {}",
                    recipe.sql(),
                );
                assert!(
                    errors.iter().any(|e| e.contains("cube_model")),
                    "case {case}: refusal does not name the model: {errors:?}",
                );
                refused_unsupported += 1;
            }
        }
    }

    // Generator-health floors: both legs must actually be exercised.
    assert!(
        fired >= case_count() / 4,
        "only {fired}/{} recipes fired the rewrite — generator degenerated",
        case_count(),
    );
    assert!(
        refused_unsupported > 0,
        "no unsupported-clause recipe was generated — refusal leg untested",
    );
}

/// The annotation with a single COUNT(DISTINCT) is a hard error (existing
/// behaviour) — pinned here so the refusal contract stays uniform.
#[test]
fn single_distinct_annotation_is_refused_not_rewritten() {
    let recipe = CubeRecipe {
        group_keys: vec![GroupKey::K1],
        distincts: vec![DistinctArg::D1],
        others: vec![],
        where_pred: None,
        unsupported: None,
        rows: vec![],
    };
    let result = plan_recipe(&recipe);
    let errors = result.expect_err("one COUNT(DISTINCT) must refuse");
    assert!(
        errors.iter().any(|e| e.contains("cube_model")),
        "refusal does not name the model: {errors:?}"
    );
}

/// Without the annotation the planner must not touch the model at all.
#[test]
fn unannotated_model_is_never_rewritten() {
    let model = ModelInfo {
        name: "plain".to_string(),
        sql: "SELECT k1, COUNT(DISTINCT d1) as c0, COUNT(DISTINCT d2) as c1 \
              FROM events GROUP BY k1"
            .to_string(),
        refs: vec![],
        timeseries_config: None,
        incremental_config: None,
    };
    let mut graph = ModelGraph::new();
    graph.add_model(model);
    let (transformations, errors) = Planner::new().plan(&graph);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert!(
        !transformations
            .iter()
            .any(|t| matches!(t, Transformation::ReplaceWithPlan { .. })),
        "planner rewrote an unannotated model",
    );
}
