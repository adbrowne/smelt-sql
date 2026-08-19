//! A model's projection (its output column names and their inferred types)
//! is derived once from the **source** `SelectStmt`, before dialect
//! lowering — never recovered by re-parsing the dialect-printed SQL. See
//! `docs/specs/multi_backend.md` §"Output-schema type conformance" and
//! §"Whole-row MERGE".
//!
//! These tests exercise every public compile entry point on `SqlCompiler`
//! (`compile`, `compile_with_sql_and_ephemerals`, `compile_with_ephemerals`)
//! via the public `CompilerRegistry` — each routes through the same
//! source-derived projection, so the same invariants must hold for all of
//! them, including across the ephemeral-CTE `WITH` prepend (which operates
//! on already-printed SQL, by design, but must not disturb the projection
//! derived from source before it).

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::ModelFile;
use smelt_runtime::{CompilerRegistry, EphemeralResolver};
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

/// The `CAST(d AS DATE)` gives the type-cast wrapper a concrete type to
/// engage on for `d` too, so the wrap actually fires (mirrors
/// `bigquery_median_emits_no_narrowing_cast`'s fixture above).
const MEDIAN_HOLISTIC_SQL: &str =
    "SELECT CAST(d AS DATE) AS d, MEDIAN(val) AS med_val FROM raw.events GROUP BY d";
/// Exercises `%` (MOD) and `**` (POWER) lowering, alongside `MEDIAN`.
const MOD_POW_MODEL_SQL: &str =
    "SELECT d, (val % 3) AS remainder, (val ** 2) AS squared FROM events";

/// A lowered construct's `output_columns` must be dialect-invariant through
/// `compile_with_sql_and_ephemerals`, exactly as `compile` already proves
/// above.
#[test]
fn compile_with_sql_and_ephemerals_yields_dialect_invariant_output_columns() {
    let registry = registry();
    let model = make_model("median_model", MEDIAN_MODEL_SQL);
    let resolver = EphemeralResolver::empty();

    let duckdb = registry
        .get("duckdb")
        .compile_with_sql_and_ephemerals(&model, "main", MEDIAN_MODEL_SQL, &resolver)
        .expect("duckdb compile should succeed");
    let spark = registry
        .get("spark")
        .compile_with_sql_and_ephemerals(&model, "main", MEDIAN_MODEL_SQL, &resolver)
        .expect("spark compile should succeed");
    let bigquery = registry
        .get("bigquery")
        .compile_with_sql_and_ephemerals(&model, "main", MEDIAN_MODEL_SQL, &resolver)
        .expect("bigquery compile should succeed");

    let expected = vec!["d".to_string(), "med_val".to_string()];
    assert_eq!(duckdb.output_columns, expected);
    assert_eq!(
        duckdb.output_columns, spark.output_columns,
        "duckdb vs spark: {:?} vs {:?}",
        duckdb.output_columns, spark.output_columns
    );
    assert_eq!(
        duckdb.output_columns, bigquery.output_columns,
        "duckdb vs bigquery: {:?} vs {:?}; sql = {}",
        duckdb.output_columns, bigquery.output_columns, bigquery.sql
    );
}

/// Same invariant, through the other public ephemeral-aware entry point.
#[test]
fn compile_with_ephemerals_yields_dialect_invariant_output_columns() {
    let registry = registry();
    let model = make_model("median_model", MEDIAN_MODEL_SQL);
    let resolver = EphemeralResolver::empty();

    let duckdb = registry
        .get("duckdb")
        .compile_with_ephemerals(&model, "main", &resolver)
        .expect("duckdb compile should succeed");
    let bigquery = registry
        .get("bigquery")
        .compile_with_ephemerals(&model, "main", &resolver)
        .expect("bigquery compile should succeed");

    let expected = vec!["d".to_string(), "med_val".to_string()];
    assert_eq!(duckdb.output_columns, expected);
    assert_eq!(
        duckdb.output_columns, bigquery.output_columns,
        "duckdb vs bigquery: {:?} vs {:?}; sql = {}",
        duckdb.output_columns, bigquery.output_columns, bigquery.sql
    );
}

/// `%`/`**` lowering (MOD/POWER) must not disturb `output_columns` either.
#[test]
fn compile_with_sql_and_ephemerals_mod_and_pow_are_dialect_invariant() {
    let registry = registry();
    let model = make_model("mod_pow_model", MOD_POW_MODEL_SQL);
    let resolver = EphemeralResolver::empty();

    let duckdb = registry
        .get("duckdb")
        .compile_with_sql_and_ephemerals(&model, "main", MOD_POW_MODEL_SQL, &resolver)
        .expect("duckdb compile should succeed");
    let bigquery = registry
        .get("bigquery")
        .compile_with_sql_and_ephemerals(&model, "main", MOD_POW_MODEL_SQL, &resolver)
        .expect("bigquery compile should succeed");

    let expected = vec![
        "d".to_string(),
        "remainder".to_string(),
        "squared".to_string(),
    ];
    assert_eq!(duckdb.output_columns, expected);
    assert_eq!(
        duckdb.output_columns, bigquery.output_columns,
        "duckdb vs bigquery: {:?} vs {:?}; sql = {}",
        duckdb.output_columns, bigquery.output_columns, bigquery.sql
    );
}

/// Regression guard for `970ef87a` (the same one pinned above for `compile`
/// in `bigquery_median_emits_no_narrowing_cast`), exercised through
/// `compile_with_sql_and_ephemerals`: the type-cast wrapper must derive
/// `med_val`'s type from the source `MEDIAN(val)` expression, never from the
/// printed `ARRAY_AGG`/`CASE`/`FLOAT64` lowering — a re-parse of the latter
/// leaves the type unresolved and lets division promotion narrow the cast to
/// the other, known operand's `SmallInt`.
#[test]
fn compile_with_sql_and_ephemerals_median_emits_no_narrowing_cast() {
    let registry = registry();
    let model = make_model("holistic", MEDIAN_HOLISTIC_SQL);
    let resolver = EphemeralResolver::empty();

    let compiled = registry
        .get("bigquery")
        .compile_with_sql_and_ephemerals(&model, "main", MEDIAN_HOLISTIC_SQL, &resolver)
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

/// The case most likely to regress: the ephemeral-CTE prepend happens on
/// already-printed SQL, textually merging a `WITH` preamble in front of the
/// model's own top-level SELECT. `output_columns` must still come out right
/// — proving the projection derived before printing survives a
/// transformation applied entirely after it.
#[test]
fn ephemeral_cte_prepend_does_not_disturb_output_columns() {
    let registry = registry();
    let ephemeral_sql = "SELECT id, name FROM raw_users WHERE active = true";
    let sql = "SELECT d, MEDIAN(val) AS med_val FROM smelt.staging_users GROUP BY d";
    let model = make_model("final_model", sql);

    let bigquery_compiler = registry.get("bigquery");
    let resolver = bigquery_compiler.build_ephemeral_resolver(
        &[("staging_users".to_string(), ephemeral_sql.to_string())],
        "main",
    );

    let compiled = bigquery_compiler
        .compile_with_sql_and_ephemerals(&model, "main", sql, &resolver)
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("WITH"),
        "the ephemeral CTE prepend must actually have run: {}",
        compiled.sql
    );
    assert_eq!(
        compiled.output_columns,
        vec!["d".to_string(), "med_val".to_string()],
        "output_columns must survive the post-print WITH-clause merge: sql = {}",
        compiled.sql
    );
}

/// Same case, through `compile_with_ephemerals`.
#[test]
fn ephemeral_cte_prepend_does_not_disturb_output_columns_via_compile_with_ephemerals() {
    let registry = registry();
    let ephemeral_sql = "SELECT id, name FROM raw_users WHERE active = true";
    let sql = "SELECT d, MEDIAN(val) AS med_val FROM smelt.staging_users GROUP BY d";
    let model = make_model("final_model", sql);

    let bigquery_compiler = registry.get("bigquery");
    let resolver = bigquery_compiler.build_ephemeral_resolver(
        &[("staging_users".to_string(), ephemeral_sql.to_string())],
        "main",
    );

    let compiled = bigquery_compiler
        .compile_with_ephemerals(&model, "main", &resolver)
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("WITH"),
        "the ephemeral CTE prepend must actually have run: {}",
        compiled.sql
    );
    assert_eq!(
        compiled.output_columns,
        vec!["d".to_string(), "med_val".to_string()],
        "output_columns must survive the post-print WITH-clause merge: sql = {}",
        compiled.sql
    );
}

/// A concrete, independently-reproduced instance of the same defect pinned
/// above for `compile` (`qualify_lowering_does_not_erase_output_columns`),
/// through the ephemeral-aware entry point: Spark's `supports_qualify =
/// false` lowers `QUALIFY` to an outer `SELECT * FROM (<original>) _q WHERE
/// <predicate>` — a wildcard select item. Reading `output_columns` off
/// *that* printed form makes a perfectly well-named source projection
/// (`id, rn`) look unknowable.
#[test]
fn compile_with_sql_and_ephemerals_qualify_lowering_does_not_erase_output_columns() {
    let registry = registry();
    let sql = "SELECT id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY ts DESC) AS rn \
               FROM t QUALIFY rn = 1";
    let model = make_model("qualify_model", sql);
    let resolver = EphemeralResolver::empty();

    let compiled = registry
        .get("spark")
        .compile_with_sql_and_ephemerals(&model, "main", sql, &resolver)
        .expect("compile should succeed");

    assert_eq!(
        compiled.output_columns,
        vec!["id".to_string(), "rn".to_string()],
        "the QUALIFY-to-subquery lowering's outer `SELECT *` must not make a \
         statically-known source projection look unknown: sql = {}",
        compiled.sql
    );
}

/// Same regression, through `compile_with_ephemerals`.
#[test]
fn compile_with_ephemerals_qualify_lowering_does_not_erase_output_columns() {
    let registry = registry();
    let sql = "SELECT id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY ts DESC) AS rn \
               FROM t QUALIFY rn = 1";
    let model = make_model("qualify_model", sql);
    let resolver = EphemeralResolver::empty();

    let compiled = registry
        .get("spark")
        .compile_with_ephemerals(&model, "main", &resolver)
        .expect("compile should succeed");

    assert_eq!(
        compiled.output_columns,
        vec!["id".to_string(), "rn".to_string()],
        "the QUALIFY-to-subquery lowering's outer `SELECT *` must not make a \
         statically-known source projection look unknown: sql = {}",
        compiled.sql
    );
}

/// A `smelt.<path>(args).*` struct-spread select item must make the whole
/// projection unresolvable (empty `output_columns`), exactly like a bare
/// `*` (`bare_wildcard_projection_still_yields_empty_output_columns`
/// above): the spread's field count/names are only known once the call is
/// resolved and printed, not from this source select list alone. No
/// downstream item's position may be reinterpreted, and no cast wrap may
/// invent a column name for the un-fanned-out call. The function need not
/// resolve for this check to fire — `derive_projection` runs before any
/// function-body lookup — so an unresolvable `smelt.functions.*` call is
/// enough to exercise it; see `is_struct_spread_call` in
/// `crates/smelt-parser/src/ast.rs`.
#[test]
fn struct_spread_select_item_yields_empty_output_columns() {
    let registry = registry();
    let compiler = registry.get("duckdb");
    let sql = "SELECT id, smelt.functions.parse_event_payload(payload).* FROM events";
    let model = make_model("struct_spread_model", sql);

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert!(
        compiled.output_columns.is_empty(),
        "a struct-spread select item must yield an empty (unknown) \
         output_columns list, not a partial or guessed one: {:?}",
        compiled.output_columns
    );
    assert!(
        !compiled.sql.contains("_smelt_typed"),
        "no cast wrap may be emitted over an unresolvable projection \
         (it would have to invent a column name for the spread): {}",
        compiled.sql
    );
}

/// Load-bearing companion to the test above: without a concrete type
/// anywhere in the select list, `apply_type_casts`'s own `has_concrete`
/// check already suppresses the cast wrap regardless of the
/// `is_struct_spread_call` disjunct in `derive_projection` — so that test
/// alone cannot tell whether the disjunct does anything.
///
/// Here `CAST(id AS BIGINT) AS id` gives the projection a genuinely
/// concrete, named column, so `has_concrete` is true and
/// `apply_type_casts` *would* proceed to wrap the SELECT with CASTs if
/// `derive_projection` treated this select list as statically enumerable.
/// The struct-spread item is deliberately never touched by
/// `synthesize_projection_aliases` either, so `ProjectionColumn::name` is
/// `None` for it; `apply_type_casts` falls back to a synthesized
/// `_smelt_col{i+1}` name for any nameless column when it does wrap — a
/// name that does not correspond to anything the printer actually emits
/// for an un-fanned-out `smelt.<path>(args).*` call. The
/// `is_struct_spread_call` disjunct is what stops `derive_projection`
/// from ever reaching that state: it must make the *whole* projection
/// unresolvable (`columns: None`), so `apply_type_casts` takes its early
/// `Some(columns) = &projection.columns else { return sql unchanged }`
/// exit before `has_concrete` is even evaluated.
#[test]
fn struct_spread_select_item_suppresses_cast_wrap_even_with_a_concrete_sibling_column() {
    let registry = registry();
    let compiler = registry.get("duckdb");
    let sql = "SELECT CAST(id AS BIGINT) AS id, \
               smelt.functions.parse_event_payload(payload).* FROM events";
    let model = make_model("struct_spread_concrete_model", sql);

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert!(
        compiled.output_columns.is_empty(),
        "a struct-spread select item must yield an empty (unknown) \
         output_columns list even when a sibling column has a concrete \
         type: {:?}",
        compiled.output_columns
    );
    assert!(
        !compiled.sql.contains("_smelt_typed"),
        "the presence of a concrete-typed sibling column (`id`) must not \
         make apply_type_casts wrap the SELECT — that would require \
         inventing a `_col2`-style name for the unresolved spread item: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("_col2"),
        "no synthesized positional fallback name may be emitted for the \
         struct-spread item: {}",
        compiled.sql
    );
}

/// Case 3 of the alias-synthesis rule (`docs/specs/multi_backend.md`
/// §"Output-schema type conformance"): a nameless projection item (here,
/// three bare integer literals) gets a real, bound `_smelt_col{n}` alias
/// spliced into the *source* SQL before printing — never an invented name
/// referenced against an inner query that never exposed it. Compiling AND
/// executing on a real DuckDB proves the spliced alias is a name the printed
/// SQL actually binds, not merely a string the cast wrap happens to agree
/// with.
#[test]
fn unaliased_literal_columns_execute_on_duckdb_with_synthesized_aliases() {
    let registry = registry();
    let compiler = registry.get("duckdb");
    let model = make_model("literal_cols", "SELECT id, 1, 2, 3 FROM raw.users");

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    let expected = vec![
        "id".to_string(),
        "_smelt_col2".to_string(),
        "_smelt_col3".to_string(),
        "_smelt_col4".to_string(),
    ];
    assert_eq!(compiled.output_columns, expected, "sql = {}", compiled.sql);

    let conn = duckdb::Connection::open_in_memory().expect("open duckdb");
    conn.execute_batch("CREATE SCHEMA raw; CREATE TABLE raw.users (id INTEGER)")
        .expect("seed raw.users schema");
    conn.execute_batch("INSERT INTO raw.users VALUES (1), (2)")
        .expect("seed raw.users data");

    let mut stmt = conn.prepare(&compiled.sql).unwrap_or_else(|e| {
        panic!(
            "compiled SQL must execute on DuckDB: {e}\nsql = {}",
            compiled.sql
        )
    });
    let mut rows = stmt.query([]).expect("execute compiled sql");
    let mut row_count = 0;
    while rows.next().expect("row iteration").is_some() {
        row_count += 1;
    }
    assert_eq!(row_count, 2, "expected both seeded rows to come back");
    let column_names = rows
        .as_ref()
        .expect("statement must still be available after iteration")
        .column_names();
    assert_eq!(
        column_names, expected,
        "the columns DuckDB actually names for the executed SQL must match \
         the synthesized aliases exactly: sql = {}",
        compiled.sql
    );
}

/// The same model compiled for BigQuery must synthesize the identical
/// `_smelt_col{n}` names — the alias-synthesis rule derives from source
/// position only, so it is dialect-invariant like every other part of the
/// projection.
#[test]
fn unaliased_literal_columns_synthesize_identical_aliases_on_bigquery() {
    let registry = registry();
    let duckdb_compiler = registry.get("duckdb");
    let bigquery_compiler = registry.get("bigquery");
    let sql = "SELECT id, 1, 2, 3 FROM raw.users";

    let duckdb_compiled = duckdb_compiler
        .compile(&make_model("literal_cols_duckdb", sql), "main")
        .expect("duckdb compile should succeed");
    let bigquery_compiled = bigquery_compiler
        .compile(&make_model("literal_cols_bigquery", sql), "main")
        .expect("bigquery compile should succeed");

    let expected = vec![
        "id".to_string(),
        "_smelt_col2".to_string(),
        "_smelt_col3".to_string(),
        "_smelt_col4".to_string(),
    ];
    assert_eq!(duckdb_compiled.output_columns, expected);
    assert_eq!(
        bigquery_compiled.output_columns, expected,
        "bigquery sql = {}",
        bigquery_compiled.sql
    );
}

/// Case 2 of the alias-synthesis rule: a bare (qualified) column reference
/// with no explicit alias keeps its own name — no `AS` is spliced in, since
/// every dialect already agrees on that name.
#[test]
fn bare_qualified_column_ref_keeps_its_own_name_no_alias_spliced() {
    let registry = registry();
    let compiler = registry.get("duckdb");
    let model = make_model("bare_ref_model", "SELECT t.user_id FROM t");

    let compiled = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert_eq!(
        compiled.output_columns,
        vec!["user_id".to_string()],
        "sql = {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("_smelt_col"),
        "a bare column reference must not receive a synthesized alias: {}",
        compiled.sql
    );
}
