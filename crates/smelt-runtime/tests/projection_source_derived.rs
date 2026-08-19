//! Pins Phase 2 of `docs/plans/20260819-source-derived-projection.md`: a
//! model's projection (its output column names and their inferred types) is
//! derived once from the **source** `SelectStmt`, before dialect lowering —
//! never recovered by re-parsing the dialect-printed SQL. See
//! `docs/specs/multi_backend.md` §"Output-schema type conformance" and
//! §"Whole-row MERGE".
//!
//! These tests exercise `SqlCompiler::compile` (the one entry point Phase 2
//! routes through the new single-owner projection) via the public
//! `CompilerRegistry`.

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
        name: "projection_source_derived".to_string(),
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

const MEDIAN_MODEL_SQL: &str = "SELECT d, MEDIAN(val) AS med_val FROM events GROUP BY d";

/// Test 1: compiling a `MEDIAN` model for BigQuery must yield
/// `output_columns == ["d", "med_val"]` and a cast wrap naming `med_val` —
/// the printed `ARRAY_AGG`/`CASE` form BigQuery's `MEDIAN` lowering produces
/// does not read back as smelt SQL, so recovering the projection from it
/// silently loses the alias.
#[test]
fn bigquery_median_model_yields_source_derived_output_columns() {
    let registry = registry();
    let compiler = registry.get("bigquery");
    let model = make_model("median_model", MEDIAN_MODEL_SQL);

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert_eq!(
        compiled.output_columns,
        vec!["d".to_string(), "med_val".to_string()],
        "output_columns must be derived from the source select list, not the \
         printed ARRAY_AGG lowering: sql = {}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("med_val"),
        "the cast wrap (if any) or the projection itself must still name the \
         column med_val: {}",
        compiled.sql
    );
}

/// Test 2: the same model compiled for every backend agrees on
/// `output_columns` and on the column name(s) any cast wrap emits — a
/// dialect-invariant projection is the whole point of deriving it from
/// source, not from each backend's own lowering.
#[test]
fn output_columns_are_dialect_invariant_across_backends() {
    let registry = registry();
    let model = make_model("median_model", MEDIAN_MODEL_SQL);

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

    assert_eq!(
        duckdb.output_columns,
        vec!["d".to_string(), "med_val".to_string()]
    );
    assert_eq!(
        duckdb.output_columns, spark.output_columns,
        "duckdb vs spark: {:?} vs {:?}",
        duckdb.output_columns, spark.output_columns
    );
    assert_eq!(
        duckdb.output_columns, bigquery.output_columns,
        "duckdb vs bigquery: {:?} vs {:?}",
        duckdb.output_columns, bigquery.output_columns
    );

    // If a cast wrap is present at all, every backend's wrap must name the
    // same columns — the cast wrap's column names ride the same projection.
    for compiled in [&duckdb, &spark, &bigquery] {
        if compiled.sql.contains("_smelt_typed") {
            assert!(
                compiled.sql.contains("med_val"),
                "cast wrap must still name med_val: {}",
                compiled.sql
            );
        }
    }
}

/// Test 3: a model whose source projection is a bare `*` still yields an
/// EMPTY `output_columns` — empty means *unknown*, per
/// `docs/specs/multi_backend.md` §"Whole-row MERGE", and that fail-closed
/// contract must survive the refactor to a source-derived projection.
#[test]
fn bare_wildcard_projection_still_yields_empty_output_columns() {
    let registry = registry();
    let compiler = registry.get("duckdb");
    let model = make_model("wildcard_model", "SELECT * FROM events");

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert!(
        compiled.output_columns.is_empty(),
        "a bare wildcard select must yield an empty (unknown) output_columns \
         list, not a partial or guessed one: {:?}",
        compiled.output_columns
    );
}

/// Test 4 (regression guard for commit 970ef87a): a BigQuery `MEDIAN` must
/// still emit no *narrowing* cast — division-promotion silently adopting the
/// *other*, known operand's type for an unresolved BigQuery-native type
/// spelling (`FLOAT64`) previously produced `CAST(med_val AS SMALLINT)`,
/// rounding an exact median before it left the warehouse. `MEDIAN`'s
/// registry-declared return type is `Double` regardless of its operand, so a
/// `CAST(med_val AS FLOAT64)` (BigQuery's spelling of `Double`) is the
/// *correct*, exact-width cast here — it is not the regression; a narrowing
/// integer cast (`SMALLINT`, `INT64`, …) would be.
#[test]
fn bigquery_median_emits_no_narrowing_cast() {
    let registry = registry();
    let compiler = registry.get("bigquery");
    let model = make_model(
        "holistic",
        "SELECT CAST(d AS DATE) AS d, MEDIAN(val) AS med_val FROM raw.events GROUP BY d",
    );

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert!(
        !compiled.sql.contains("_col"),
        "no positional fallback name may reach the backend: {}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("med_val"),
        "the model's own alias must survive: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("CAST(med_val AS SMALLINT")
            && !compiled.sql.contains("CAST(med_val AS INT")
            && !compiled.sql.contains("CAST(med_val AS NUMERIC")
            && !compiled.sql.contains("CAST(med_val AS BIGNUMERIC"),
        "an unresolved median type must not be cast to a guessed narrower \
         integer/fixed-point type, silently rounding interpolated medians: {}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("CAST(med_val AS FLOAT64)"),
        "MEDIAN's registry-declared Double return type should still surface \
         as an exact-width FLOAT64 cast on BigQuery: {}",
        compiled.sql
    );
}

/// A concrete, independently-reproduced instance of the same defect (kept
/// alongside the plan's four named tests as direct evidence the projection
/// really was being recovered from printed SQL): Spark's `supports_qualify
/// = false` lowers `QUALIFY` to `SELECT * FROM (<original>) _q WHERE
/// <predicate>` — an outer wildcard select. Reading the projection off that
/// printed form (as the old `output_column_names(sql: &str)` did) makes a
/// perfectly well-named source projection (`id, rn`) look unknowable. The
/// source-derived projection must see through the lowering: the model's own
/// `SelectStmt` has no wildcard at all.
#[test]
fn qualify_lowering_does_not_erase_output_columns() {
    let registry = registry();
    let compiler = registry.get("spark");
    let sql = "SELECT id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY ts DESC) AS rn \
               FROM t QUALIFY rn = 1";
    let model = make_model("qualify_model", sql);

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert_eq!(
        compiled.output_columns,
        vec!["id".to_string(), "rn".to_string()],
        "the QUALIFY-to-subquery lowering's outer `SELECT *` must not make \
         a statically-known source projection look unknown: sql = {}",
        compiled.sql
    );
}
