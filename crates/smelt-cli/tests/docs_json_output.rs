//! Phase 5 (meta-language-E2): tests for the `origin` field in
//! `smelt docs --json` (catalog) output for generator-emitted models.
//!
//! Includes:
//!  - Unit smoke test for `CatalogModel` serde shape (direct struct construction).
//!  - Integration test that exercises the real `build_catalog()` pipeline with
//!    a tempdir workspace fixture.

use smelt_cli::docs::CatalogModel;
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
/// Uses `build_logical_graph` so that discovery, generator-file filtering, and
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

    // `build_logical_graph` runs the full pipeline: discover SQL files, init the
    // Salsa DB, run the emitted-models generator pipeline, filter generator files
    // from the hand-authored set, build the LogicalGraph, and annotate provenance.
    // The test relies on this helper — it does NOT call discover_emitted_model_files
    // or annotate_emitted_models directly.
    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db) = smelt_cli::build_logical_graph(&project_dir, &config, None, &[], "dev")
        .expect("build logical graph");

    // Build catalog and check origin.
    let catalog =
        smelt_cli::docs::build_catalog(&graph, &config, &db, &std::collections::HashMap::new())
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
