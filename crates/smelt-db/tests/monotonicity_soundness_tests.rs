//! Generative smelt-sql soundness oracle for the event-time monotonicity
//! trace (`smelt_logical::trace_event_time`).
//!
//! Spec: `docs/specs/model_properties.md` §"Event-time monotonicity trace"
//! (and the column-nullability-gate row).
//! Plan: `docs/plans/20260702-monotonicity-primitive-tested.md` Phase 2.
//!
//! Fixture-style unit tests (in `crates/smelt-logical/src/analysis/monotonicity.rs`)
//! only prove the primitive classifies ~15 *known* scenarios correctly. The
//! contract that actually matters is Constraint 12: for *every* expression
//! the primitive admits as `Traceable`, no input data can break the rule.
//! This module generates many `event_time` expressions (whitelist shapes),
//! compiles each into a smelt model, runs it through smelt's own compiler
//! (`smelt-runtime`) to get real backend SQL, and searches for input data
//! that breaks the commutation identity the trace claims:
//!
//! ```text
//! full   = { r in exec(S, D) | r.event_time in [lo, hi) }
//! pushed =   exec(S, { d in D | d.source_column in [lo-offset, hi-offset) })
//! assert multiset(full) == multiset(pushed)
//! ```
//!
//! Per the plan's confirmed simplification: since the compiled SQL `S`
//! never changes between the "full" and "pushed" runs, the property is
//! implemented purely by varying the *input data* fed to the same compiled
//! query (no `inject_time_filter`/pushdown-consumer machinery needed — that
//! is deferred to a later plan). The "full" clamp is applied as an outer SQL
//! WHERE wrapping the compiled query; the "pushed" clamp is applied by only
//! inserting the qualifying rows into the source table before running the
//! unmodified compiled query.
//!
//! DuckDB only for now: unlike `type_property_tests.rs`'s `SparkOracle`
//! (type-introspection via `DESCRIBE QUERY`), exercising this commutation
//! property on Spark needs full DDL + row-level query execution, which the
//! existing Spark harness does not provide. Wiring that seam is left to a
//! follow-on change rather than growing `SparkOracle` here.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::duckdb_oracle::DuckDbOracle;
use prop_helpers::monotonicity_gen::{
    gen_case_strategy, hazard_corpus, ts_literal, GenCase, Shape, SOURCE_COLUMN, SOURCE_NAME,
    SOURCE_REF, SOURCE_TABLE,
};

use smelt_core::config::{Config, Materialization, Target};
use smelt_logical::{trace_event_time, BoundContext, EventTimeTrace, Offset};
use smelt_runtime::{CompilerRegistry, EphemeralResolver};

use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestError, TestRunner};
use std::collections::HashMap;

// ---- Compile / DuckDB plumbing (mirrors crates/smelt-runtime/tests/compile_parity.rs) ----

fn duckdb_target() -> Target {
    Target {
        target_type: "duckdb".to_string(),
        database: Some("test.duckdb".to_string()),
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

fn test_config() -> Config {
    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target());
    Config {
        name: "monotonicity_soundness".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    }
}

fn make_model(name: &str, sql: &str) -> smelt_core::ModelFile {
    let parse = smelt_parser::parse(sql);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let path = std::path::PathBuf::from(format!("models/{name}.sql"));
    smelt_core::ModelFile {
        name: name.to_string(),
        path: path.clone(),
        content: sql.to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(path),
        address_segments: vec![name.to_string()],
    }
}

/// Compile a generated `event_time` expression into real DuckDB backend SQL
/// via smelt's own compiler (never hand-assembled — owner correction
/// 2026-07-02). The model selects `event_time` and `payload` from the
/// single-segment source ref `smelt.mono_src`, aliased `t` (so
/// `Shape::QualifiedColumn`'s `t.event_ts` resolves).
fn compile_event_time_sql(expr_sql: &str) -> String {
    let model_sql = format!("SELECT {expr_sql} AS event_time, payload FROM {SOURCE_REF} AS t");
    let model = make_model("mono_gen", &model_sql);

    let config = test_config();
    let targets = config.targets.clone();
    let registry = CompilerRegistry::new(&config, &targets);
    let compiler = registry.get("default");
    let resolver = EphemeralResolver::empty();
    let compiled = compiler
        .compile_with_sql_and_ephemerals(&model, "main", &model_sql, &resolver)
        .expect("compile should succeed for generated monotonicity smelt-sql");
    compiled.sql
}

/// Parse `sql_expr` as a SELECT-list item and return the parsed `Expr`, the
/// same recipe `monotonicity.rs`'s own unit tests use.
fn parse_event_time_expr(sql_expr: &str) -> smelt_parser::Expr {
    let sql = format!("SELECT {sql_expr} AS event_time FROM t");
    let parse = smelt_parser::parse(&sql);
    let file = smelt_parser::ast::File::cast(parse.syntax()).expect("file cast");
    let select = file.select_stmt().expect("select stmt");
    let select_list = select.select_list().expect("select list");
    let item = select_list.items().next().expect("first select item");
    item.expression().expect("item expression")
}

fn bound_ctx() -> BoundContext {
    BoundContext::new().with_source(SOURCE_NAME, SOURCE_COLUMN)
}

/// (Re)create `SOURCE_TABLE` and insert `rows` — `(stable_id, value)` pairs,
/// `value = None` meaning a NULL row. The id must be assigned once against
/// the *original* full data set (by the caller) and carried through
/// filtering unchanged, so the same logical row gets the same `payload`
/// value whether it appears in the "full" or "pushed" run — otherwise the
/// result multisets would spuriously differ on `payload` alone whenever
/// filtering reorders/drops rows, independent of the property being tested.
fn setup_table(oracle: &DuckDbOracle, rows: &[(usize, Option<i64>)]) -> Result<(), String> {
    let mut ddl = format!(
        "DROP TABLE IF EXISTS {SOURCE_TABLE}; \
         CREATE TABLE {SOURCE_TABLE} ({SOURCE_COLUMN} TIMESTAMP, other_ts TIMESTAMP, payload INTEGER);"
    );
    if !rows.is_empty() {
        let values: Vec<String> = rows
            .iter()
            .map(|(id, v)| {
                let ts = match v {
                    Some(secs) => ts_literal(*secs),
                    None => "NULL".to_string(),
                };
                format!("({ts}, NULL, {id})")
            })
            .collect();
        ddl.push_str(&format!(
            " INSERT INTO {SOURCE_TABLE} VALUES {};",
            values.join(", ")
        ));
    }
    oracle.execute_ddl(&ddl)
}

/// The core commutation check: given the compiled SQL `S`, the claimed
/// `offset_seconds`, a window `[lo, hi)`, and the full data set `D`, assert
/// `full == pushed` (sorted row multisets — `DuckDbOracle::execute_query`
/// already sorts).
fn check_commutation(
    compiled_sql: &str,
    offset_seconds: i64,
    lo: i64,
    hi: i64,
    data: &[Option<i64>],
) -> Result<(), String> {
    let oracle = DuckDbOracle::new();

    // Assign stable ids against the original data set once, so `payload`
    // identifies the same logical row in both the "full" and "pushed" runs.
    let ided: Vec<(usize, Option<i64>)> = data.iter().copied().enumerate().collect();

    // full = exec(S, D) filtered to event_time in [lo, hi) by an outer WHERE.
    setup_table(&oracle, &ided)?;
    let full_sql = format!(
        "SELECT event_time, payload FROM ({compiled_sql}) __mono_full \
         WHERE event_time >= {} AND event_time < {}",
        ts_literal(lo),
        ts_literal(hi)
    );
    let full_rows = oracle.execute_query(&full_sql)?;

    // pushed = exec(S, { d in D | d.source_column in [lo-offset, hi-offset) }).
    let pushed_lo = lo - offset_seconds;
    let pushed_hi = hi - offset_seconds;
    let pushed_subset: Vec<(usize, Option<i64>)> = ided
        .into_iter()
        .filter(|(_, v)| matches!(v, Some(x) if *x >= pushed_lo && *x < pushed_hi))
        .collect();
    setup_table(&oracle, &pushed_subset)?;
    let pushed_rows = oracle.execute_query(compiled_sql)?;

    if full_rows == pushed_rows {
        Ok(())
    } else {
        Err(format!(
            "commutation violated: offset={offset_seconds}s window=[{lo},{hi}) \
             pushed_window=[{pushed_lo},{pushed_hi})\n  \
             full ({} rows) != pushed ({} rows)\n  full={:?}\n  pushed={:?}\n  sql={compiled_sql}",
            full_rows.len(),
            pushed_rows.len(),
            full_rows,
            pushed_rows,
        ))
    }
}

/// Run the full generated case end-to-end: classify with the real
/// primitive, assert it is `Traceable` with the expected offset, then run
/// the commutation check against the resulting compiled SQL.
fn run_whitelist_case(case: &GenCase) -> Result<(), String> {
    let expr_sql = case.shape.sql_expr(SOURCE_COLUMN);
    let expr = parse_event_time_expr(&expr_sql);
    let ctx = bound_ctx();
    let verdict = trace_event_time(&expr, &ctx);

    let (offset_seconds, source_column) = match &verdict {
        EventTimeTrace::Traceable {
            source_column,
            offset,
            ..
        } => {
            let secs = match offset {
                Offset::Seconds(s) => s.0 as i64,
                Offset::Symbolic(sym) => {
                    return Err(format!(
                        "shape {:?} produced an unexpected Symbolic offset {sym:?} \
                         (generator is scoped to seconds-only offsets)",
                        case.shape
                    ));
                }
                Offset::Integer(n) => {
                    return Err(format!(
                        "shape {:?} produced an unexpected Integer offset {n} \
                         (generator is scoped to seconds-only offsets)",
                        case.shape
                    ));
                }
            };
            (secs, source_column.clone())
        }
        other => {
            return Err(format!(
                "whitelist shape {:?} (expr `{expr_sql}`) was not classified Traceable: {other:?}",
                case.shape
            ));
        }
    };

    if source_column != SOURCE_COLUMN {
        return Err(format!(
            "shape {:?} traced to column `{source_column}`, expected `{SOURCE_COLUMN}`",
            case.shape
        ));
    }
    let expected_offset = case.shape.expected_offset_seconds();
    if offset_seconds != expected_offset {
        return Err(format!(
            "shape {:?} traced offset {offset_seconds}s, expected {expected_offset}s",
            case.shape
        ));
    }

    let compiled_sql = compile_event_time_sql(&expr_sql);
    check_commutation(&compiled_sql, offset_seconds, case.lo, case.hi, &case.data)
}

// ---- gen_traceable_commutes_on_duckdb ----

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// For every whitelist shape the primitive claims `Traceable`, the
    /// output-clamp-on-event_time identity must equal the source-filter
    /// identity, for arbitrary generated data (incl. NULL and
    /// boundary-straddling rows) and arbitrary windows. Must find zero
    /// counterexamples — any failure is either a false-`Traceable` bug in
    /// the primitive or a generator bug; per the implementer brief, a
    /// genuine primitive bug is reported, not silently patched.
    #[test]
    fn gen_traceable_commutes_on_duckdb(case in gen_case_strategy()) {
        let result = run_whitelist_case(&case);
        prop_assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}

// ---- gen_never_false_positive_on_seed_hazards ----

/// Deterministic seed corpus (fast, no DuckDB execution needed): every
/// blacklist/StaticSeed shape, plus the named research hazards (P3 NULL
/// seed, Q5 non-commuting CASE body, J3/J4 multi-clock two-column
/// arithmetic — see `hazard_corpus()`), must never classify `Traceable`.
/// False negatives (`NotTraceable`/`StaticSeed` when a smarter classifier
/// could safely prove `Traceable`) are allowed by Constraint 12; only false
/// positives fail this test.
#[test]
fn gen_never_false_positive_on_seed_hazards() {
    let ctx = bound_ctx();
    for (name, expr_sql) in hazard_corpus() {
        let expr = parse_event_time_expr(&expr_sql);
        let verdict = trace_event_time(&expr, &ctx);
        match &verdict {
            EventTimeTrace::Traceable { .. } => {
                panic!(
                    "hazard `{name}` (expr `{expr_sql}`) was classified Traceable — \
                     false positive: {verdict:?}"
                );
            }
            EventTimeTrace::StaticSeed { reason } | EventTimeTrace::NotTraceable { reason, .. } => {
                assert!(
                    !reason.is_empty(),
                    "hazard `{name}` (expr `{expr_sql}`) produced an empty reason"
                );
            }
        }
    }
}

/// Also fold in the original whitelist/blacklist shapes' own blacklist
/// members (defence in depth: exercises the exact same expression heads the
/// primitive's own unit tests cover) — never Traceable.
#[test]
fn gen_never_false_positive_on_shape_blacklist() {
    let ctx = bound_ctx();
    let blacklist = [
        Shape::TwoColumnArithmetic,
        Shape::ModFn,
        Shape::ExtractFn,
        Shape::CaseExpr,
        Shape::CoalesceConst,
        Shape::GreatestConst,
        Shape::UnknownUdf,
        Shape::ConstLiteral,
        Shape::NullLiteral,
        Shape::NowFn,
        Shape::CastVarchar,
    ];
    for shape in blacklist {
        assert!(
            !shape.is_whitelist(),
            "shape {shape:?} should not be self-classified whitelist"
        );
        let expr_sql = shape.sql_expr(SOURCE_COLUMN);
        let expr = parse_event_time_expr(&expr_sql);
        let verdict = trace_event_time(&expr, &ctx);
        assert!(
            !matches!(verdict, EventTimeTrace::Traceable { .. }),
            "shape {shape:?} (expr `{expr_sql}`) was classified Traceable: {verdict:?}"
        );
    }
}

// ---- gen_shrinks_report_expression ----

/// Planted-unsound "arm": forces a `GREATEST(col, const)` clamp expression
/// to be treated as `Traceable{ offset: 0 }` — exactly the shape the real
/// primitive correctly rejects (`GreatestConst` in the blacklist, verified
/// above). This bypasses `trace_event_time` entirely (never touches
/// `crates/smelt-logical/src/analysis/monotonicity.rs`) and feeds the
/// falsely-claimed offset straight into `check_commutation`, to prove the
/// oracle actually falsifies unsound whitelist arms — and that proptest
/// shrinks the failure to a small case — rather than passing vacuously.
fn planted_unsound_commutes(lo: i64, hi: i64, data: &[Option<i64>]) -> Result<(), String> {
    // A clamp anchored well inside the generated data/window range, so a
    // window straddling the clamp point commonly exists.
    const CLAMP_SECS: i64 = 3 * prop_helpers::monotonicity_gen::DAY;
    let expr_sql = format!("GREATEST({SOURCE_COLUMN}, {})", ts_literal(CLAMP_SECS));
    let compiled_sql = compile_event_time_sql(&expr_sql);
    // Falsely claim offset = 0 (as if this were a bare column/shift chain).
    check_commutation(&compiled_sql, 0, lo, hi, data)
}

#[test]
fn gen_shrinks_report_expression() {
    let strategy = (
        (-6 * prop_helpers::monotonicity_gen::DAY..6 * prop_helpers::monotonicity_gen::DAY)
            .prop_flat_map(|lo| {
                (
                    Just(lo),
                    (lo + 1)..(lo + 4 * prop_helpers::monotonicity_gen::DAY),
                )
            }),
        prop::collection::vec(
            prop_oneof![
                1 => Just(None),
                4 => (-6 * prop_helpers::monotonicity_gen::DAY..6 * prop_helpers::monotonicity_gen::DAY).prop_map(Some),
            ],
            8..16,
        ),
    );

    let mut runner = TestRunner::new(ProptestConfig::with_cases(128));
    let result = runner.run(&strategy, |((lo, hi), data)| {
        planted_unsound_commutes(lo, hi, &data).map_err(TestCaseError::fail)
    });

    match result {
        Err(TestError::Fail(_, minimal)) => {
            // Falsified and shrunk — exactly what a sound oracle must do
            // when an unsound whitelist arm is planted. Sanity-check the
            // shrunk case is still a valid (non-empty-window) counterexample.
            let ((lo, hi), _data) = minimal;
            assert!(lo < hi, "shrunk counterexample should keep a valid window");
        }
        Ok(()) => panic!(
            "expected the planted-unsound GREATEST(...) arm to be falsified by the oracle, \
             but the commutation check passed vacuously for every generated case"
        ),
        Err(other) => {
            panic!("unexpected proptest error while falsifying the planted arm: {other:?}")
        }
    }
}

// ---- Smoke tests (deterministic, fast) ----

#[test]
fn smoke_bare_column_commutes() {
    let case = GenCase {
        shape: Shape::BareColumn,
        lo: 0,
        hi: 10 * prop_helpers::monotonicity_gen::DAY,
        data: vec![
            Some(-1),
            Some(0),
            Some(1),
            Some(10 * prop_helpers::monotonicity_gen::DAY - 1),
            Some(10 * prop_helpers::monotonicity_gen::DAY),
            None,
        ],
    };
    run_whitelist_case(&case).expect("bare column commutation should hold");
}

#[test]
fn smoke_date_trunc_day_commutes_on_aligned_window() {
    let case = GenCase {
        shape: Shape::DateTruncDay,
        lo: 0,
        hi: 3 * prop_helpers::monotonicity_gen::DAY,
        data: vec![
            Some(-3600),    // previous day
            Some(0),        // exact lower boundary
            Some(3600 * 5), // mid first day
            Some(prop_helpers::monotonicity_gen::DAY + 100),
            Some(3 * prop_helpers::monotonicity_gen::DAY - 1), // last second inside window
            Some(3 * prop_helpers::monotonicity_gen::DAY),     // exact upper boundary (excluded)
            None,
        ],
    };
    run_whitelist_case(&case)
        .expect("DATE_TRUNC('day', ...) commutation should hold on an aligned window");
}

#[test]
fn smoke_interval_shift_hours_commutes() {
    let case = GenCase {
        shape: Shape::IntervalShiftHours(3),
        lo: -2 * prop_helpers::monotonicity_gen::HOUR,
        hi: 5 * prop_helpers::monotonicity_gen::HOUR,
        data: vec![
            Some(-10 * prop_helpers::monotonicity_gen::HOUR),
            Some(-5 * prop_helpers::monotonicity_gen::HOUR),
            Some(0),
            Some(prop_helpers::monotonicity_gen::HOUR),
            Some(10 * prop_helpers::monotonicity_gen::HOUR),
            None,
        ],
    };
    run_whitelist_case(&case).expect("col + INTERVAL '3 hours' commutation should hold");
}

#[test]
fn hazard_corpus_covers_every_blacklist_entry() {
    // Sanity: the deterministic corpus is non-empty and every entry has a
    // non-empty expression (guards against accidental empty-string shapes).
    let corpus = hazard_corpus();
    assert!(
        corpus.len() >= 12,
        "expected a substantial hazard corpus, got {}",
        corpus.len()
    );
    for (name, expr) in corpus {
        assert!(
            !expr.trim().is_empty(),
            "hazard `{name}` has an empty expression"
        );
    }
}
