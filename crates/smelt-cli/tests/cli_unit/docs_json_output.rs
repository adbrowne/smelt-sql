//! Tests for `smelt docs --json` (catalog JSON) output.
//!
//! Includes:
//!  - Unit smoke test for `CatalogModel` serde shape (direct struct construction).
//!  - Integration test that exercises the real `build_catalog()` pipeline with
//!    a tempdir workspace fixture.
//!  - D-50(i) invariant: every `CatalogColumn` always serializes a `source` key
//!    (never omitted), with `{"type":"unknown"}` for undetermined lineage.
//!  - W8-catalog P3: `--select` filtering — selected models only in map/index,
//!    but edge arrays retain names of ALL deps (including excluded ones).

use std::collections::BTreeSet;

use smelt_cli::docs::{CatalogColumn, CatalogColumnSource, CatalogModel};
use smelt_core::ModelOriginKind;

/// Unit smoke test — verifies `CatalogModel` serde shape directly.
#[test]
fn emitted_model_carries_origin_in_docs_json() {
    let model = CatalogModel {
        name: "cohorts.cohorts.us_west".to_string(),
        description: None,
        owner: None,
        tags: vec![],
        materialization: "view".to_string(),
        refresh: None,
        path: "models/cohorts.gen.sql".to_string(),
        columns: vec![],
        upstream: vec![],
        downstream: vec![],
        incremental: None,
        origin: Some(ModelOriginKind::Generated {
            generator_file: "models/cohorts.gen.sql".to_string(),
            generator_name: "us_west".to_string(),
        }),
        tests_targeting: vec![],
    };

    let json = serde_json::to_string(&model).expect("serialize CatalogModel");
    assert!(
        json.contains("\"origin\""),
        "emitted model JSON must contain 'origin' key; got: {json}"
    );
    assert!(
        json.contains("\"generated\""),
        "origin.type must be 'generated'; got: {json}"
    );
    assert!(
        json.contains("models/cohorts.gen.sql"),
        "origin must include generator_file; got: {json}"
    );
    assert!(
        json.contains("us_west"),
        "origin must include generator_name; got: {json}"
    );
}

// ── Integration test: real pipeline end-to-end ────────────────────────────

/// Helper: write the given `(relative_path, contents)` pairs into a tempdir
/// and create a minimal `smelt.yml`.  Returns the `TempDir` (kept alive).
fn stage_workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    for (rel, contents) in files {
        let path = tmp.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
    }
    let yml = "name: test_proj\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
    tmp
}

/// Integration test: `build_catalog()` emits `origin` for emitted models and
/// no `origin` for hand-authored models when the real pipeline is exercised.
///
/// Uses `build_dependency_graph` so that discovery, generator-file filtering, and
/// provenance annotation are all handled by the same helper the CLI command
/// handler uses — the test never calls `discover_emitted_model_files` or
/// `annotate_emitted_models` directly.
///
/// Plan TDD test 10 (`emitted_model_carries_origin_in_docs_json`) end-to-end variant.
#[test]
fn emitted_model_carries_origin_in_real_docs_catalog_pipeline() {
    // Generator file: emits a single ModelDef named "east".
    let generator = "---\ngenerates: models\n---\n\
        [ModelDef { name: 'east', body: SELECT 1 AS id }]";
    // Hand-authored model: plain SQL.
    let hand_authored = "SELECT 2 AS id";

    let tmp = stage_workspace(&[
        ("models/cohorts.gen.sql", generator),
        ("models/orders.sql", hand_authored),
    ]);
    let project_dir = tmp.path().to_path_buf();

    // `build_dependency_graph` runs the full pipeline: discover SQL files, init the
    // Salsa DB, run the emitted-models generator pipeline, filter generator files
    // from the hand-authored set, build the LogicalGraph, and annotate provenance.
    // The test relies on this helper — it does NOT call discover_emitted_model_files
    // or annotate_emitted_models directly.
    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    // Build catalog and check origin.
    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        None,
    )
    .expect("build catalog");
    let json = serde_json::to_string_pretty(&catalog).expect("serialize catalog");

    // Find the emitted model (key contains "east").
    let (emitted_key, emitted) = catalog
        .models
        .iter()
        .find(|(k, _)| k.contains("east"))
        .unwrap_or_else(|| {
            panic!(
                "expected a model with 'east' in its name; got: {:?}\nfull JSON:\n{json}",
                catalog.models.keys().collect::<Vec<_>>()
            )
        });

    assert!(
        emitted.origin.is_some(),
        "emitted model '{}' must have origin set; full JSON:\n{json}",
        emitted_key
    );
    if let Some(ModelOriginKind::Generated {
        ref generator_file,
        ref generator_name,
    }) = emitted.origin
    {
        assert!(
            generator_file.contains("cohorts.gen.sql"),
            "origin.generator_file must reference cohorts.gen.sql; got: {}",
            generator_file
        );
        assert_eq!(
            generator_name, "east",
            "origin.generator_name must be 'east'; got: {}",
            generator_name
        );
    } else {
        panic!(
            "emitted model origin must be Generated; got: {:?}",
            emitted.origin
        );
    }

    // Find the hand-authored model.
    let hand = catalog.models.get("orders").unwrap_or_else(|| {
        panic!(
            "expected 'orders' model; got: {:?}",
            catalog.models.keys().collect::<Vec<_>>()
        )
    });
    assert!(
        hand.origin.is_none(),
        "hand-authored model 'orders' must NOT have origin; full JSON:\n{json}"
    );
}

// ── D-50(i): source always present ───────────────────────────────────────────

/// Unit test: `CatalogColumn` with `source: Unknown` always serializes a
/// `"source"` key containing `{"type":"unknown"}` — the field is never omitted.
#[test]
fn source_always_present_in_catalog_column_json() {
    let col = CatalogColumn {
        name: "my_col".to_string(),
        data_type: None,
        nullable: None,
        description: None,
        tests: vec![],
        expression: "my_col".to_string(),
        source: CatalogColumnSource::Unknown,
    };

    let json = serde_json::to_string(&col).expect("serialize CatalogColumn");
    assert!(
        json.contains("\"source\""),
        "CatalogColumn JSON must always include 'source' key; got: {json}"
    );
    assert!(
        json.contains("\"type\":\"unknown\""),
        "CatalogColumnSource::Unknown must serialize as {{\"type\":\"unknown\"}}; got: {json}"
    );
}

/// Integration test: after running the full `build_catalog()` pipeline, every
/// column object in the serialized JSON must contain a `"source"` key.
/// D-50(i): `source` is never omitted regardless of how lineage was resolved.
#[test]
fn catalog_column_source_always_present_in_full_pipeline() {
    let sql = "SELECT 1 AS id, 'hello' AS name";

    let tmp = stage_workspace(&[("models/orders.sql", sql)]);
    let project_dir = tmp.path().to_path_buf();

    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        None,
    )
    .expect("build catalog");

    let json = serde_json::to_string_pretty(&catalog).expect("serialize catalog");
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse catalog JSON");

    // Walk every column object; each must have a "source" key.
    let models = value["models"]
        .as_object()
        .expect("catalog.models must be an object");
    assert!(
        !models.is_empty(),
        "catalog must contain at least one model"
    );
    for (model_name, model_val) in models {
        if let Some(cols) = model_val["columns"].as_array() {
            for (i, col) in cols.iter().enumerate() {
                assert!(
                    col.get("source").is_some(),
                    "model '{}' column[{}] is missing 'source' key; full JSON:\n{json}",
                    model_name,
                    i
                );
            }
        }
    }
}

// ── D-50(iii): path is workspace-relative ────────────────────────────────────

/// D-50(iii): `path` in a `CatalogModel` must be workspace-relative (no leading `/`).
#[test]
fn catalog_path_is_workspace_relative() {
    let tmp = stage_workspace(&[("models/orders.sql", "SELECT 1 AS id")]);
    let project_dir = tmp.path().to_path_buf();

    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        None,
    )
    .expect("build catalog");

    let orders = catalog.models.get("orders").unwrap_or_else(|| {
        panic!(
            "expected 'orders' model; got: {:?}",
            catalog.models.keys().collect::<Vec<_>>()
        )
    });
    assert!(
        !orders.path.starts_with('/'),
        "catalog path must be workspace-relative (no leading /); got: {}",
        orders.path
    );
    assert_eq!(
        orders.path, "models/orders.sql",
        "catalog path must be relative to project root"
    );
}

/// D-50(iii): `origin.generator_file` for emitted models must also be workspace-relative.
#[test]
fn catalog_origin_generator_file_is_workspace_relative() {
    let generator = "---\ngenerates: models\n---\n\
        [ModelDef { name: 'east', body: SELECT 1 AS id }]";
    let tmp = stage_workspace(&[("models/cohorts.gen.sql", generator)]);
    let project_dir = tmp.path().to_path_buf();

    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        None,
    )
    .expect("build catalog");

    let (_, emitted) = catalog
        .models
        .iter()
        .find(|(k, _)| k.contains("east"))
        .unwrap_or_else(|| {
            panic!(
                "expected emitted model containing 'east'; got: {:?}",
                catalog.models.keys().collect::<Vec<_>>()
            )
        });

    if let Some(smelt_core::ModelOriginKind::Generated {
        ref generator_file, ..
    }) = emitted.origin
    {
        assert!(
            !generator_file.starts_with('/'),
            "origin.generator_file must be workspace-relative; got: {}",
            generator_file
        );
        assert!(
            generator_file.contains("cohorts.gen.sql"),
            "origin.generator_file must reference cohorts.gen.sql; got: {}",
            generator_file
        );
    } else {
        panic!(
            "emitted model must have Generated origin; got: {:?}",
            emitted.origin
        );
    }
}

// ── `materialized_view` is not a storage-axis value ──────────────────────────

/// A model whose frontmatter declares `materialization: materialized_view`
/// cannot end up in the catalog under that storage value: the discovery
/// pipeline treats an unrecognised `materialization:` frontmatter value the
/// same as an absent one (surfaced as a diagnostic by `smelt-db`/the LSP, not
/// a hard failure of catalog *generation*), so the model falls back to the
/// project's `default_materialization`. `smelt run`/`smelt build` (the
/// execution path — see `materialization_parity.rs`) are where this value is
/// hard-rejected with the `refresh: materialized_view` migration hint.
#[test]
fn catalog_pipeline_never_reports_materialized_view_for_retired_value() {
    let tmp = stage_workspace(&[(
        "models/cached_report.sql",
        "---\nmaterialization: materialized_view\n---\nSELECT 1 AS id",
    )]);
    let project_dir = tmp.path().to_path_buf();

    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        None,
    )
    .expect("build catalog");

    let cached_report = catalog
        .models
        .get("cached_report")
        .expect("cached_report model must still be discoverable");
    assert_ne!(
        cached_report.materialization, "materialized_view",
        "retired storage value must never surface in the catalog"
    );
}

/// No `CatalogModel` produced by the real pipeline can ever carry
/// `"materialization":"materialized_view"` — the enum has no such variant, so
/// serialization structurally cannot emit it.
#[test]
fn catalog_json_never_contains_materialized_view_storage_value() {
    let tmp = stage_workspace(&[
        (
            "models/view_model.sql",
            "---\nmaterialization: view\n---\nSELECT 1 AS id",
        ),
        (
            "models/table_model.sql",
            "---\nmaterialization: table\n---\nSELECT 1 AS id",
        ),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        None,
    )
    .expect("build catalog");

    let json = serde_json::to_string(&catalog).expect("serialize catalog");
    assert!(
        !json.contains("materialized_view"),
        "catalog JSON must never contain the retired storage value 'materialized_view'; got: {json}"
    );
}

// ── W8-catalog P3: --select filtering ────────────────────────────────────────

/// W8-catalog P3 TDD gate: when `build_catalog` is called with
/// `selected_names = Some({"b"})` on a 3-model chain `a → b → c`:
///
/// - `catalog.models` contains ONLY `b` (not `a` or `c`)
/// - `catalog.models["b"].upstream` still contains `"a"` (edge retained)
/// - `catalog.models["b"].downstream` still contains `"c"` (edge retained)
/// - `catalog.execution_order` == `["b"]`
/// - `catalog.project.model_count` == 1
#[test]
fn select_filter_retains_edge_names() {
    // Use path-form refs (FROM smelt.<name>) which the DependencyGraph uses
    // to detect edges — NOT smelt.ref() which is a compilation-time function.
    let tmp = stage_workspace(&[
        ("models/a.sql", "SELECT 1 AS id"),
        ("models/b.sql", "SELECT id FROM smelt.a"),
        ("models/c.sql", "SELECT id FROM smelt.b"),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    // Build the FULL (unfiltered) dependency graph — selection only affects
    // what build_catalog puts in the output, not which edges are discovered.
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    let selected = BTreeSet::from(["b".to_string()]);
    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
        &project_dir,
        Some(&selected),
    )
    .expect("build catalog");

    // Only "b" should appear in the models map.
    assert_eq!(
        catalog.models.keys().cloned().collect::<Vec<_>>(),
        vec!["b".to_string()],
        "catalog.models must contain only the selected model 'b'; got: {:?}",
        catalog.models.keys().collect::<Vec<_>>()
    );

    let b = catalog.models.get("b").expect("'b' must be in catalog");

    // Edge arrays retain ALL dep names even when the neighbour is excluded.
    assert!(
        b.upstream.contains(&"a".to_string()),
        "b.upstream must retain 'a' even though 'a' is excluded; got: {:?}",
        b.upstream
    );
    assert!(
        b.downstream.contains(&"c".to_string()),
        "b.downstream must retain 'c' even though 'c' is excluded; got: {:?}",
        b.downstream
    );

    // Execution order must contain only the selected model.
    assert_eq!(
        catalog.execution_order,
        vec!["b".to_string()],
        "execution_order must be [\"b\"]; got: {:?}",
        catalog.execution_order
    );

    // model_count must reflect the selection, not the full graph.
    assert_eq!(
        catalog.project.model_count, 1,
        "project.model_count must be 1 (selected only); got: {}",
        catalog.project.model_count
    );
}
