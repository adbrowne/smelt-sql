//! Function-registry-threaded classification (phase 2,
//! `docs/outcomes/20260815-partition-grain-residue`): the `NotDerivable`
//! lookback-refusal gate (`derive_model_source_bounds`) and the
//! window-function batch-safety check (`analyze_batch_safety`) must both
//! classify off the *expanded* SQL a run actually executes — a lookback
//! hidden inside a `smelt.define` body must be visible exactly as an inline
//! one is (`docs/specs/incremental_shapes.md` §"Functions inside
//! partition-grain bodies").
//!
//! `safety::build_model_graph` is the single call site both
//! `check_bound_derivation` and `check_planner_safety`'s classification read
//! from — these tests exercise it directly rather than through
//! `execute_project`, so they need no backend.

use std::collections::HashMap;

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::metadata::FileMetadata;
use smelt_core::{ModelFile, ModelId, ModelKind};
use smelt_planner::{analyze_batch_safety, derive_model_source_bounds, BatchSafety};
use smelt_runtime::{safety::build_model_graph, FnBodyMap};

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
        name: "classification_expansion".to_string(),
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

/// Build a `ModelFile` the way real discovery would: frontmatter parsed into
/// `metadata` (via `extract_file_metadata`), refs parsed from the AST.
fn make_model(name: &str, sql: &str) -> ModelFile {
    let metadata = match smelt_core::metadata::extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => Some(metadata),
        _ => None,
    };
    let clean = smelt_parser::strip_frontmatter(sql);
    let parse = smelt_parser::parse(&clean);
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
        metadata,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(path),
        address_segments: vec![name.to_string()],
    }
}

fn upstream_sql() -> &'static str {
    "---\n\
     materialization: table\n\
     timeseries:\n\
     \x20 partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n\
     ---\n\
     SELECT event_date FROM raw_orders\n"
}

fn downstream_unexpanded_sql() -> &'static str {
    "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20 partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n\
     ---\n\
     SELECT * FROM smelt.functions.recent_window(source => smelt.orders)\n"
}

fn downstream_hand_inlined_sql() -> &'static str {
    "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20 partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n\
     ---\n\
     SELECT * FROM (SELECT * FROM smelt.orders WHERE event_date >= CURRENT_DATE - INTERVAL '7 day') source\n"
}

fn recent_window_fn_bodies() -> FnBodyMap {
    let mut fn_bodies = FnBodyMap::new();
    fn_bodies.insert(
        "recent_window".to_string(),
        (
            vec![("source".to_string(), None)],
            "(SELECT * FROM source WHERE event_date >= CURRENT_DATE - INTERVAL '7 day')"
                .to_string(),
        ),
    );
    fn_bodies
}

#[test]
fn bound_derivation_sees_define_body_lookback() {
    let config = test_config();

    // Graph A: the unexpanded, function-call form, with `recent_window`'s
    // body wired into `build_model_graph`'s FnBodyMap.
    let models_a = vec![
        make_model("orders", upstream_sql()),
        make_model("downstream", downstream_unexpanded_sql()),
    ];
    let dep_graph_a = DependencyGraph::build(models_a, None).expect("build dep graph a");
    let fn_bodies = recent_window_fn_bodies();
    let selected_a = vec!["orders".to_string(), "downstream".to_string()];
    let model_graph_a = build_model_graph(&selected_a, &dep_graph_a, &config, &fn_bodies);
    let downstream_a = model_graph_a
        .get("downstream")
        .expect("downstream in graph a");
    let bound_a = derive_model_source_bounds(downstream_a, &model_graph_a)
        .ok()
        .and_then(|m| m.get("orders").cloned());

    // Graph B: the hand-inlined form — what execution actually runs — with
    // no function bodies needed (nothing left to expand).
    let models_b = vec![
        make_model("orders", upstream_sql()),
        make_model("downstream", downstream_hand_inlined_sql()),
    ];
    let dep_graph_b = DependencyGraph::build(models_b, None).expect("build dep graph b");
    let empty_fn_bodies = FnBodyMap::new();
    let model_graph_b = build_model_graph(&selected_a, &dep_graph_b, &config, &empty_fn_bodies);
    let downstream_b = model_graph_b
        .get("downstream")
        .expect("downstream in graph b");
    let bound_b = derive_model_source_bounds(downstream_b, &model_graph_b)
        .ok()
        .and_then(|m| m.get("orders").cloned());

    assert!(
        bound_a.is_some(),
        "expected a derivable bound once the define body is expanded, got None"
    );
    assert_eq!(
        bound_a, bound_b,
        "the unexpanded function-call form (graph A, expanded via FnBodyMap) must derive the \
         same bound as the hand-inlined form (graph B) that execution actually runs"
    );
}

#[test]
fn batch_safety_sees_define_body_window() {
    let config = test_config();
    let models = vec![
        make_model("orders", upstream_sql()),
        make_model("downstream", downstream_unexpanded_sql()),
    ];
    let dep_graph = DependencyGraph::build(models, None).expect("build dep graph");
    let fn_bodies = recent_window_fn_bodies();
    let selected = vec!["orders".to_string(), "downstream".to_string()];
    let model_graph = build_model_graph(&selected, &dep_graph, &config, &fn_bodies);
    let downstream = model_graph.get("downstream").expect("downstream in graph");

    let safety = analyze_batch_safety(downstream);
    assert!(
        matches!(safety, BatchSafety::BoundedSafe { .. }),
        "expected BoundedSafe once the define-body lookback is expanded into the graph's SQL, \
         got {safety:?}"
    );
}

#[test]
fn no_fn_bodies_is_identity() {
    // A project with no `smelt.define` bodies at all: an empty FnBodyMap
    // must leave classification unchanged (no expansion work to do).
    let config = test_config();
    let plain_sql = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n\
         ---\n\
         SELECT event_date, COUNT(*) AS n FROM smelt.orders GROUP BY event_date\n";
    let models = vec![
        make_model("orders", upstream_sql()),
        make_model("downstream", plain_sql),
    ];
    let dep_graph = DependencyGraph::build(models, None).expect("build dep graph");
    let selected = vec!["orders".to_string(), "downstream".to_string()];

    let empty_fn_bodies = FnBodyMap::new();
    let model_graph = build_model_graph(&selected, &dep_graph, &config, &empty_fn_bodies);
    let downstream = model_graph.get("downstream").expect("downstream in graph");

    assert_eq!(
        downstream.sql, plain_sql,
        "with no fn bodies, expand_function_calls must be a no-op — the graph's `sql` must be \
         exactly the model's own raw content, unchanged"
    );
    assert!(matches!(
        analyze_batch_safety(downstream),
        BatchSafety::FullyBatchSafe
    ));
}

// ─── Structural gate: every production `ModelInfo { sql: … }` is expanded ──

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Anti-pattern substrings: a `sql:` field assigned straight from a model's
/// raw, unexpanded content. Any of these appearing on (or immediately after)
/// a `sql:` line inside a `ModelInfo { … }` literal means that call site
/// classifies off text a run never actually executes — the exact residue
/// this phase closes.
const RAW_CONTENT_PATTERNS: &[&str] = &[
    "model.content.clone()",
    "model.content.to_string()",
    "model_file.content.clone()",
    "model_file.content.to_string()",
];

fn scan_dir_for_raw_model_info_sql(dir: &std::path::Path, hits: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_for_raw_model_info_sql(&path, hits);
            continue;
        }
        if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let mut in_model_info = false;
            for (i, line) in content.lines().enumerate() {
                if line.contains("ModelInfo {") || line.contains("ModelInfo{") {
                    in_model_info = true;
                }
                if in_model_info && line.trim_start().starts_with("sql:") {
                    if let Some(pat) = RAW_CONTENT_PATTERNS.iter().find(|p| line.contains(**p)) {
                        hits.push(format!("{}:{} — {}", path.display(), i + 1, pat));
                    }
                    in_model_info = false;
                }
                if in_model_info && line.trim_start().starts_with('}') {
                    in_model_info = false;
                }
            }
        }
    }
}

#[test]
fn every_production_model_info_uses_expanded_sql() {
    let root = repo_root();
    let mut hits = Vec::new();
    for crate_name in ["smelt-runtime", "smelt-cli", "smelt-ui"] {
        scan_dir_for_raw_model_info_sql(
            &root.join("crates").join(crate_name).join("src"),
            &mut hits,
        );
    }
    assert!(
        hits.is_empty(),
        "found ModelInfo {{ sql: <raw model content> }} construction — the `sql:` field must be \
         fed by `expand_function_calls`, not a model's raw content, so lookback/window \
         classification sees through `smelt.define` bodies:\n{hits:#?}"
    );
}
