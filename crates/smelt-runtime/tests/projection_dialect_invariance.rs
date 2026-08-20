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
