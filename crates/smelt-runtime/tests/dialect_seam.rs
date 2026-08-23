//! The compile path refuses a construct the registry declares unsupported on
//! the target's dialect, rather than emitting SQL the engine rejects.
//!
//! `smelt-dialect`'s `unsupported_emission` suite proves the pure check; this
//! file proves it is actually wired into every `SqlCompiler` print, so a model
//! never reaches a warehouse carrying a construct that warehouse cannot express.

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

fn registry() -> CompilerRegistry {
    let mut targets = HashMap::new();
    targets.insert("duckdb".to_string(), duckdb_target());
    targets.insert("bigquery".to_string(), bigquery_target());
    let config = Config {
        name: "dialect_seam".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: targets.clone(),
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    };
    CompilerRegistry::new(&config, &targets)
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

const FLOOR_DIVIDE_SQL: &str = "SELECT id, val // 2 AS halved FROM events";

#[test]
fn a_model_using_floor_divide_fails_to_compile_for_bigquery() {
    let model = make_model("q", FLOOR_DIVIDE_SQL);
    let err = registry()
        .get("bigquery")
        .compile(&model, "main")
        .expect_err("BigQuery has no `//`; the compiler must refuse before emitting SQL");
    let msg = format!("{err}");
    assert!(msg.contains("//"), "must name the construct: {msg}");
    assert!(
        msg.contains("BigQuery") || msg.contains("bigquery"),
        "must name the backend: {msg}"
    );
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code so the CLI output is greppable: {msg}"
    );
}

#[test]
fn the_same_model_compiles_for_duckdb() {
    let model = make_model("q", FLOOR_DIVIDE_SQL);
    registry()
        .get("duckdb")
        .compile(&model, "main")
        .expect("DuckDB has `//`");
}

/// The refusal is not specific to `compile`: an ephemeral model is inlined as a
/// CTE into its consumer, so it never passes through a consumer's own check.
#[test]
fn an_ephemeral_model_is_refused_too() {
    let err = registry()
        .get("bigquery")
        .build_ephemeral_resolver(
            &[("staged".to_string(), FLOOR_DIVIDE_SQL.to_string())],
            "main",
        )
        .expect_err("an inlined ephemeral CTE reaches the same warehouse");
    assert!(format!("{err}").contains("//"));
}

/// Every occurrence is listed, so a user is not walked through one compile
/// round trip per site.
#[test]
fn all_occurrences_are_named_in_one_error() {
    let model = make_model("q", "SELECT a // b AS x, c // d AS y FROM t");
    let err = registry()
        .get("bigquery")
        .compile(&model, "main")
        .expect_err("two refusals");
    let msg = format!("{err}");
    assert!(msg.contains("2 constructs"), "{msg}");
}

/// Structural: no compile entry point may reach the printer directly, or it
/// would skip the refusal. `print_checked_for` is the sole permitted caller.
#[test]
fn every_compile_path_is_emission_checked() {
    const COMPILE_SRC: &str = include_str!("../src/compile.rs");
    // The two hardwired-DuckDB helpers (`resolve_refs_in_sql` and the
    // function-body expander) are exempt: they take no dialect, return no
    // `Result`, and DuckDB declares nothing unsupported.
    const EXEMPT: usize = 2;
    let direct = COMPILE_SRC.matches("smelt_dialect::print(").count();
    assert_eq!(
        direct,
        EXEMPT + 1,
        "compile.rs calls `smelt_dialect::print` {direct} times; only \
         `print_checked_for` plus the {EXEMPT} hardwired-DuckDB helpers may. A new \
         compile path must print through `print_checked`, or it skips the \
         `UnsupportedOnBackend` refusal."
    );
}
