//! Standing gate: a model's projection — its output column names, and the
//! column names any type-cast wrap emits — is dialect-invariant. See
//! `docs/specs/multi_backend.md` §"Output-schema type conformance": the
//! projection is derived once, from the model's source select list, before
//! dialect lowering, and every consumer reads that single derivation rather
//! than reading back the dialect printer's rendered output.
//!
//! This is deliberately load-bearing: dialect lowering is exactly what
//! differs between backends (`MEDIAN` becomes an `ARRAY_AGG`-indexing
//! expression on BigQuery, `%` becomes `MOD()`, `QUALIFY` becomes an outer
//! subquery-and-`WHERE`, …), so if a compile entry point ever again recovers
//! its projection by re-parsing that printed, dialect-lowered SQL instead of
//! reading the source-derived projection, this test fails immediately —
//! the three backends' lowerings disagree by construction, so a printed-SQL
//! read-back cannot produce the same names on all three.
//!
//! No live warehouse is needed: every assertion is against the compiled SQL
//! text, so this runs per-PR with no gating flag or environment variable.

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::ModelFile;
use smelt_runtime::CompilerRegistry;
use std::collections::HashMap;

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

fn spark_target() -> Target {
    Target {
        target_type: "spark".to_string(),
        database: None,
        schema: "default".to_string(),
        connect_url: Some("sc://localhost".to_string()),
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

fn bigquery_target() -> Target {
    Target {
        target_type: "bigquery".to_string(),
        database: None,
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: Some("p".to_string()),
        dataset: Some("main".to_string()),
        location: Some("US".to_string()),
    }
}

fn test_config() -> Config {
    let mut targets = HashMap::new();
    targets.insert("duckdb".to_string(), duckdb_target());
    targets.insert("spark".to_string(), spark_target());
    targets.insert("bigquery".to_string(), bigquery_target());
    Config {
        name: "projection_dialect_invariance".to_string(),
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

fn make_model(name: &str, sql: &str) -> ModelFile {
    let parse = smelt_parser::parse(sql);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let path = std::path::PathBuf::from(format!("models/{name}.sql"));
    ModelFile {
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

fn registry() -> CompilerRegistry {
    let config = test_config();
    let targets = config.targets.clone();
    CompilerRegistry::new(&config, &targets)
}

/// One model exercising every construct the dialect printer lowers:
/// `MEDIAN` (window position), `%`, `**`, `QUALIFY`, a `DATE` literal, a
/// `::` cast, and an array literal.
const EVERY_LOWERED_CONSTRUCT_SQL: &str = "SELECT \
    id, \
    MEDIAN(val) OVER (PARTITION BY id) AS med_val, \
    (val % 3) AS remainder, \
    (val ** 2) AS squared, \
    DATE '2024-01-01' AS d, \
    val::DOUBLE AS val_double, \
    ARRAY[1, 2, 3] AS arr, \
    ROW_NUMBER() OVER (PARTITION BY id ORDER BY val DESC) AS rn \
    FROM events \
    QUALIFY rn = 1";

const EXPECTED_OUTPUT_COLUMNS: &[&str] = &[
    "id",
    "med_val",
    "remainder",
    "squared",
    "d",
    "val_double",
    "arr",
    "rn",
];

/// The gate. Compiling the same model, unchanged, for DuckDB, Spark and
/// BigQuery must yield byte-identical `output_columns`, and — wherever a
/// type-cast wrap is present — every wrapped column name must appear in
/// every backend's compiled SQL identically. A dialect's own lowering of
/// `MEDIAN`, `%`, `QUALIFY`, etc. must never leak into either.
#[test]
fn output_columns_and_cast_wrap_names_are_byte_identical_across_backends() {
    let registry = registry();
    let model = make_model("every_lowered_construct", EVERY_LOWERED_CONSTRUCT_SQL);

    let duckdb = registry
        .get("duckdb")
        .compile(&model, "main")
        .expect("duckdb compile should succeed");
    let spark = registry
        .get("spark")
        .compile(&model, "main")
        .expect("spark compile should succeed");
    let bigquery = registry
        .get("bigquery")
        .compile(&model, "main")
        .expect("bigquery compile should succeed");

    let expected: Vec<String> = EXPECTED_OUTPUT_COLUMNS
        .iter()
        .map(|s| s.to_string())
        .collect();

    assert_eq!(
        duckdb.output_columns, expected,
        "duckdb output_columns diverged from the source select list: sql = {}",
        duckdb.sql
    );
    assert_eq!(
        duckdb.output_columns, spark.output_columns,
        "duckdb vs spark output_columns: {:?} vs {:?}\nduckdb sql = {}\nspark sql = {}",
        duckdb.output_columns, spark.output_columns, duckdb.sql, spark.sql
    );
    assert_eq!(
        duckdb.output_columns, bigquery.output_columns,
        "duckdb vs bigquery output_columns: {:?} vs {:?}\nduckdb sql = {}\nbigquery sql = {}",
        duckdb.output_columns, bigquery.output_columns, duckdb.sql, bigquery.sql
    );

    // Wherever a cast wrap is present, every backend's wrap must name every
    // expected column identically — the cast wrap's column names ride the
    // same source-derived projection as `output_columns` above.
    for compiled in [&duckdb, &spark, &bigquery] {
        if compiled.sql.contains("_smelt_typed") {
            for name in EXPECTED_OUTPUT_COLUMNS {
                assert!(
                    compiled.sql.contains(name),
                    "cast wrap must name every projection column identically \
                     across backends; missing {name:?} in: {}",
                    compiled.sql
                );
            }
        }
    }

    // No dialect-lowering artifact (a positional fallback name, or a raw
    // lowered function's own naming) may leak into any backend's output
    // column list or cast wrap.
    for compiled in [&duckdb, &spark, &bigquery] {
        assert!(
            !compiled.sql.contains("_col1")
                && !compiled.sql.contains("_col2")
                && !compiled.sql.contains("_col3"),
            "no positional fallback name may reach the backend: {}",
            compiled.sql
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Statement-level lowering: the restructure planner
// (`docs/specs/multi_backend.md` §"Statement-level lowering") rewrites a
// query block around a synthesised CTE. Its own admissibility rules are
// unit-tested against the pure planner in `smelt-dialect`; this leg proves
// the *derived projection* survives that rewrite through the real compile
// path, for both restructure shapes, exactly the way the leg above proves it
// survives ordinary dialect lowering.
// ─────────────────────────────────────────────────────────────────────────

/// `WindowToCte` (an aggregate-only built-in reached with a whole-partition
/// `OVER`): admissible on DuckDB and Spark, where the ordered-set
/// `PERCENTILE_CONT` has no window form; native on BigQuery, which accepts
/// `PERCENTILE_CONT` with an `OVER` clause directly.
const WINDOW_TO_CTE_SQL: &str = "SELECT id, g, \
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med \
    FROM tbl";
const WINDOW_TO_CTE_COLUMNS: &[&str] = &["id", "g", "med"];

/// `AnalyticToCte` (an analytic-only built-in reached under `GROUP BY`):
/// admissible on BigQuery, which requires an `OVER` clause and rejects
/// `PERCENTILE_CONT` under `GROUP BY` outright; native on DuckDB and Spark,
/// which have the ordered-set aggregate directly.
const ANALYTIC_TO_CTE_SQL: &str = "SELECT g, \
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med, \
    COUNT(*) AS n \
    FROM tbl GROUP BY g";
const ANALYTIC_TO_CTE_COLUMNS: &[&str] = &["g", "med", "n"];

/// A plain `SELECT *`, admissibility rule 4's positive case: no restructure
/// call is present, so the wildcard must compile unremarkably on every
/// backend, with the same (empty — an unexpanded wildcard has no resolvable
/// per-column name, `output_column_names`'s documented "untrustworthy as a
/// whole" rule) `output_columns` everywhere.
const SELECT_STAR_SQL: &str = "SELECT * FROM tbl";

/// A model using both restructure directions and the wildcard admissibility
/// rule must derive the same, unlowered projection on every backend — the
/// same invariant `output_columns_and_cast_wrap_names_are_byte_identical_across_backends`
/// proves for ordinary dialect lowering, extended to statement-level
/// restructuring.
#[test]
fn decorrelated_model_output_columns_are_identical() {
    let registry = registry();

    for (name, sql, expected) in [
        ("window_to_cte", WINDOW_TO_CTE_SQL, WINDOW_TO_CTE_COLUMNS),
        (
            "analytic_to_cte",
            ANALYTIC_TO_CTE_SQL,
            ANALYTIC_TO_CTE_COLUMNS,
        ),
        ("select_star", SELECT_STAR_SQL, &[]),
    ] {
        let model = make_model(name, sql);
        let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();

        let duckdb = registry
            .get("duckdb")
            .compile(&model, "main")
            .unwrap_or_else(|e| panic!("{name}: duckdb compile should succeed: {e}"));
        let spark = registry
            .get("spark")
            .compile(&model, "main")
            .unwrap_or_else(|e| panic!("{name}: spark compile should succeed: {e}"));
        let bigquery = registry
            .get("bigquery")
            .compile(&model, "main")
            .unwrap_or_else(|e| panic!("{name}: bigquery compile should succeed: {e}"));

        assert_eq!(
            duckdb.output_columns, expected,
            "{name}: duckdb output_columns diverged from the source select list: sql = {}",
            duckdb.sql
        );
        assert_eq!(
            duckdb.output_columns, spark.output_columns,
            "{name}: duckdb vs spark output_columns: {:?} vs {:?}\nduckdb sql = {}\nspark sql = {}",
            duckdb.output_columns, spark.output_columns, duckdb.sql, spark.sql
        );
        assert_eq!(
            duckdb.output_columns, bigquery.output_columns,
            "{name}: duckdb vs bigquery output_columns: {:?} vs {:?}\nduckdb sql = {}\nbigquery sql = {}",
            duckdb.output_columns, bigquery.output_columns, duckdb.sql, bigquery.sql
        );

        for compiled in [&duckdb, &spark, &bigquery] {
            if compiled.sql.contains("_smelt_typed") {
                for col in &expected {
                    assert!(
                        compiled.sql.contains(col.as_str()),
                        "{name}: cast wrap must name every projection column identically \
                         across backends; missing {col:?} in: {}",
                        compiled.sql
                    );
                }
            }
        }
    }
}

/// The restructure actually fires where it must: a leg that only compared
/// projections would pass just as well if no backend ever restructured
/// anything.
#[test]
fn each_direction_actually_restructures_on_the_backend_that_needs_it() {
    let registry = registry();

    let window_model = make_model("window_to_cte", WINDOW_TO_CTE_SQL);
    let duckdb = registry
        .get("duckdb")
        .compile(&window_model, "main")
        .expect("compile");
    assert!(
        duckdb.sql.contains("__smelt_base") && duckdb.sql.contains("IS NOT DISTINCT FROM"),
        "DuckDB has no window form of the ordered-set aggregate; it must restructure: {}",
        duckdb.sql
    );

    let analytic_model = make_model("analytic_to_cte", ANALYTIC_TO_CTE_SQL);
    let bigquery = registry
        .get("bigquery")
        .compile(&analytic_model, "main")
        .expect("compile");
    assert!(
        bigquery.sql.contains("ANY_VALUE") && bigquery.sql.contains("OVER"),
        "GoogleSQL rejects PERCENTILE_CONT under GROUP BY; it must restructure: {}",
        bigquery.sql
    );
    let duckdb_analytic = registry
        .get("duckdb")
        .compile(&analytic_model, "main")
        .expect("compile");
    assert!(
        !duckdb_analytic.sql.contains("__smelt_r0"),
        "DuckDB has the ordered-set aggregate natively; restructuring it would be a \
         needless rewrite: {}",
        duckdb_analytic.sql
    );
}

/// Admissibility rule 4 — no unexpanded wildcard — proven through the real
/// compile path, not just the pure planner: a `SELECT *` sharing a query
/// block with a call that would otherwise restructure must refuse rather
/// than restructure, because the wildcard would expand against the
/// *restructured* `FROM` and pick up the synthesised columns.
#[test]
fn wildcard_alongside_a_restructure_candidate_is_refused() {
    let model = make_model(
        "wildcard_with_restructure_candidate",
        "SELECT *, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med FROM tbl GROUP BY g",
    );
    let err = registry()
        .get("bigquery")
        .compile(&model, "main")
        .expect_err(
            "a wildcard sharing the block with an aggregate-position PERCENTILE_CONT on \
             BigQuery must refuse rather than restructure",
        );
    let msg = format!("{err}");
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code: {msg}"
    );
}
