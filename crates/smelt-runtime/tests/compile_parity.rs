//! Compile-parity tests. These exercise `smelt_runtime`'s public compile
//! API (`SqlCompiler::compile_with_sql_and_ephemerals`) end-to-end against
//! small in-memory fixtures, anchoring the Phase 2 invariants documented in
//! `docs/specs/architecture.md` → "Run pipeline parity rule (CLI ↔ UI)":
//!
//! - `smelt.fn.<path>(...)` expands at compile time (fn_bodies wired in).
//! - Ephemeral dependencies are inlined as CTEs.
//! - `smelt.<path>` references resolve to schema-qualified table names.
//! - Type casts apply over `smelt.ref()` columns when upstream schemas are
//!   provided.
//! - `inject_time_filter` composes cleanly with the compile pipeline.
//!
//! The end-to-end CLI-vs-UI run-output parity test lands in Phase 4
//! (`execute_parity.rs`); this file proves the compile API is callable from
//! outside `smelt-runtime` through the `CompilerRegistry` entry point.

use smelt_core::config::{Config, Materialization, Target};
use smelt_runtime::{
    build_source_bound_map, inject_time_filter, CompilerRegistry, EphemeralResolver, FnBodyMap,
    TimeRange, UpstreamSchemas,
};
use smelt_types::{DataType, TypedColumn};
use std::collections::HashMap;
use std::sync::Arc;

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
    }
}

fn test_config() -> Config {
    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target());
    Config {
        name: "compile_parity".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
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

#[test]
fn test_compile_with_function_call() {
    // A model that calls smelt.functions.safe_div(x, y).
    let sql = "SELECT smelt.functions.safe_div(a, b) AS r FROM smelt.models.t";
    let model = make_model("uses_fn", sql);

    let config = test_config();
    let targets = config.targets.clone();
    let mut registry = CompilerRegistry::new(&config, &targets);

    // Wire a function body. `safe_div(num, den)` → `(CASE WHEN den = 0 THEN NULL ELSE num/den END)`.
    let mut fn_bodies: FnBodyMap = HashMap::new();
    fn_bodies.insert(
        "safe_div".to_string(),
        (
            vec![("num".to_string(), None), ("den".to_string(), None)],
            "(CASE WHEN den = 0 THEN NULL ELSE num/den END)".to_string(),
        ),
    );
    registry.set_function_bodies_all(fn_bodies);

    let compiler = registry.get("default");
    let resolver = EphemeralResolver::empty();
    let compiled = compiler
        .compile_with_sql_and_ephemerals(&model, "main", sql, &resolver)
        .expect("compile should succeed");

    // The body must be substituted at the call site — not passed through
    // verbatim. This is the silent-compile-divergence regression test.
    assert!(
        compiled.sql.contains("CASE WHEN"),
        "expected function body to be substituted, got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.functions.safe_div"),
        "expected smelt.functions.safe_div call site to be expanded, got: {}",
        compiled.sql
    );
}

#[test]
fn test_expand_function_calls_reveals_inner_range_bound() {
    // L1: a `RANGE BETWEEN INTERVAL` lookback declared INSIDE a `smelt.define`
    // body must become visible to the bound deriver after expand_function_calls,
    // so a function-encapsulated window does not silently default to a
    // zero-lookback read.
    let sql = "SELECT * FROM smelt.functions.windowed(src => smelt.silver.events_parsed)";
    let model = make_model("uses_window_fn", sql);

    let config = test_config();
    let targets = config.targets.clone();
    let mut registry = CompilerRegistry::new(&config, &targets);
    let mut fn_bodies: FnBodyMap = HashMap::new();
    fn_bodies.insert(
        "windowed".to_string(),
        (
            vec![("src".to_string(), None)],
            "(SELECT ts, LAG(ts) OVER (PARTITION BY device_id ORDER BY ts \
             RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS prev FROM src)"
                .to_string(),
        ),
    );
    registry.set_function_bodies_all(fn_bodies);
    let compiler = registry.get("default");
    let _ = &model;

    let expanded = compiler.expand_function_calls(sql);
    // The inner RANGE bound is now visible…
    assert!(
        expanded.contains("RANGE BETWEEN INTERVAL '1 day' PRECEDING"),
        "expected inner RANGE frame to surface after expansion, got: {expanded}"
    );
    // …and the source ref is intact (not rewritten to a physical name).
    assert!(
        expanded.contains("smelt.silver.events_parsed"),
        "expected the smelt.<path> source ref to survive expansion, got: {expanded}"
    );

    let mut dep_ts: HashMap<String, (Vec<String>, String)> = HashMap::new();
    dep_ts.insert(
        "smelt.silver.events_parsed".to_string(),
        (
            vec!["silver".to_string(), "events_parsed".to_string()],
            "event_date".to_string(),
        ),
    );

    // The deriver picks up the 1-day lookback from the expanded SQL.
    let bounds = build_source_bound_map(&expanded, &dep_ts);
    let b = bounds
        .get("smelt.silver.events_parsed")
        .expect("bound entry for events_parsed");
    assert_eq!(
        b.before_secs, 86400,
        "expected a 1-day lookback derived from inside the function body"
    );

    // Control: WITHOUT expansion the inner bound is invisible (outer SQL only).
    let bounds_outer = build_source_bound_map(sql, &dep_ts);
    let before_outer = bounds_outer
        .get("smelt.silver.events_parsed")
        .map(|b| b.before_secs)
        .unwrap_or(0);
    assert_eq!(
        before_outer, 0,
        "outer SQL alone must NOT see the function-internal bound (this is L1)"
    );
}

#[test]
fn test_compile_with_ephemeral_dep() {
    // Downstream model that references an ephemeral upstream.
    // The ephemeral's canonical path is "upstream_ephemeral" (single-segment
    // address_segments), so the correct ref form is `smelt.upstream_ephemeral`.
    // The path_key used for ephemeral matching is `to_path().join("_")` =
    // `"upstream_ephemeral"`, which matches the ephemeral name exactly.
    let downstream_sql = "SELECT * FROM smelt.upstream_ephemeral";
    let downstream = make_model("downstream", downstream_sql);

    let config = test_config();
    let targets = config.targets.clone();
    let registry = CompilerRegistry::new(&config, &targets);
    let compiler = registry.get("default");

    let ephemerals = vec![(
        "upstream_ephemeral".to_string(),
        "SELECT 1 AS x".to_string(),
    )];
    let resolver = compiler.build_ephemeral_resolver(&ephemerals, "main");

    let compiled = compiler
        .compile_with_sql_and_ephemerals(&downstream, "main", downstream_sql, &resolver)
        .expect("compile should succeed");

    // The ephemeral upstream should be inlined as a CTE; the FROM clause
    // should reference the inlined CTE name (`__smelt_*`) rather than a
    // schema-qualified table.
    assert!(
        compiled.sql.contains("WITH") && compiled.sql.contains("__smelt_upstream_ephemeral"),
        "expected ephemeral CTE to be prepended and FROM rewritten, got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("main.upstream_ephemeral"),
        "ephemeral should NOT appear as schema-qualified table, got: {}",
        compiled.sql
    );
}

#[test]
fn test_compile_applies_type_casts() {
    // A model whose projection sums an integer column from an upstream model.
    // With upstream schemas wired, the printer wraps the projection with a
    // CAST to widen aggregate types correctly.
    let sql = "SELECT SUM(amount) AS total FROM smelt.models.events";
    let model = make_model("agg", sql);

    let config = test_config();
    let targets = config.targets.clone();
    let mut registry = CompilerRegistry::new(&config, &targets);

    let upstream = UpstreamSchemas {
        models: {
            let mut m = HashMap::new();
            m.insert(
                "events".to_string(),
                vec![(
                    "amount".to_string(),
                    TypedColumn {
                        data_type: DataType::Integer,
                        nullable: true,
                    },
                )],
            );
            m
        },
        ..Default::default()
    };
    registry.set_upstream_schemas_all(Arc::new(upstream));

    let compiler = registry.get("default");
    let resolver = EphemeralResolver::empty();
    let compiled = compiler
        .compile_with_sql_and_ephemerals(&model, "main", sql, &resolver)
        .expect("compile should succeed");

    // The aggregate output should be wrapped with a CAST — the printer's
    // type-cast wrapper at work. (The exact target type is dialect-specific;
    // we just assert that some CAST appears in the projection.)
    assert!(
        compiled.sql.to_ascii_uppercase().contains("CAST"),
        "expected CAST wrapping in projection, got: {}",
        compiled.sql
    );
}

#[test]
fn test_compile_with_time_filter_injection() {
    // Compose inject_time_filter with the compile pipeline. The injected
    // WHERE survives compilation.
    let sql = "SELECT * FROM smelt.models.events";
    let model = make_model("filtered", sql);
    let range = TimeRange {
        start: "2024-01-15".to_string(),
        end: "2024-01-18".to_string(),
    };
    let filtered = inject_time_filter(sql, "event_time", &range).expect("filter ok");

    let config = test_config();
    let targets = config.targets.clone();
    let registry = CompilerRegistry::new(&config, &targets);
    let compiler = registry.get("default");
    let resolver = EphemeralResolver::empty();
    let compiled = compiler
        .compile_with_sql_and_ephemerals(&model, "main", &filtered, &resolver)
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("event_time >= '2024-01-15'"),
        "expected injected filter to survive compile, got: {}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("event_time < '2024-01-18'"),
        "expected injected upper bound to survive compile, got: {}",
        compiled.sql
    );
}

#[test]
fn test_compile_path_ref_resolution() {
    // A bare smelt.<path> reference resolves to schema.table form.
    let sql = "SELECT * FROM smelt.staging.events";
    let model = make_model("uses_path", sql);

    let config = test_config();
    let targets = config.targets.clone();
    let registry = CompilerRegistry::new(&config, &targets);
    let compiler = registry.get("default");
    let resolver = EphemeralResolver::empty();
    let compiled = compiler
        .compile_with_sql_and_ephemerals(&model, "warehouse", sql, &resolver)
        .expect("compile should succeed");

    // The path-ref resolver should map smelt.staging.events → warehouse.events
    // (last segment becomes the table; address segments are joined with `_`
    // for multi-segment refs — here a single leaf).
    assert!(
        compiled.sql.contains("warehouse.staging_events")
            || compiled.sql.contains("warehouse.events"),
        "expected path-ref resolved to schema.table, got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.staging.events"),
        "smelt.<path> should not survive compile, got: {}",
        compiled.sql
    );
}
