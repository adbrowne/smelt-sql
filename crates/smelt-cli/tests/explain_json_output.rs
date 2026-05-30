//! Phase 5 (meta-language-E2): tests for the `origin` field in
//! `smelt explain --json` output for generator-emitted models.
//!
//! Includes:
//!  - Unit smoke tests for `ExplainModel` serde shape (direct struct construction).
//!  - Integration test that exercises the real pipeline end-to-end using a
//!    tempdir workspace fixture.

use smelt_cli::explain::ExplainModel;
use smelt_core::{Materialization, ModelOriginKind};

/// A generator-emitted model entry includes `"origin": { "type": "generated",
/// "generator_file": "...", "generator_name": "..." }` in serialized JSON.
///
/// Unit smoke test — verifies `ExplainModel` serde shape directly.
#[test]
fn emitted_model_carries_origin_field() {
    let model = ExplainModel {
        dependencies: vec![],
        materialization: Materialization::View,
        incremental: None,
        tags: vec![],
        owner: None,
        origin: Some(ModelOriginKind::Generated {
            generator_file: "models/cohorts.gen.sql".to_string(),
            generator_name: "us_west".to_string(),
        }),
    };

    let json = serde_json::to_string(&model).expect("serialize ExplainModel");
    assert!(
        json.contains("\"origin\""),
        "emitted model JSON must contain 'origin' key; got: {json}"
    );
    assert!(
        json.contains("\"generated\""),
        "origin.type must be 'generated'; got: {json}"
    );
    assert!(
        json.contains("\"generator_file\""),
        "origin must include generator_file; got: {json}"
    );
    assert!(
        json.contains("models/cohorts.gen.sql"),
        "origin.generator_file must be the generator file path; got: {json}"
    );
    assert!(
        json.contains("\"generator_name\""),
        "origin must include generator_name; got: {json}"
    );
    assert!(
        json.contains("us_west"),
        "origin.generator_name must be the ModelDef name; got: {json}"
    );
}

/// A hand-authored model entry omits the `origin` key entirely.
///
/// Unit smoke test — verifies `ExplainModel` serde shape directly.
#[test]
fn hand_authored_model_omits_origin_field() {
    let model = ExplainModel {
        dependencies: vec![],
        materialization: Materialization::View,
        incremental: None,
        tags: vec![],
        owner: None,
        origin: None,
    };

    let json = serde_json::to_string(&model).expect("serialize ExplainModel");
    assert!(
        !json.contains("\"origin\""),
        "hand-authored model JSON must NOT contain 'origin' key; got: {json}"
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

/// Integration test: `build_explain_output()` emits `origin` for emitted models
/// and no `origin` for hand-authored models when the real pipeline is exercised.
///
/// Uses `build_logical_graph` so that discovery, generator-file filtering, and
/// provenance annotation are all handled by the same helper the CLI command
/// handler uses — the test never calls `discover_emitted_model_files` or
/// `annotate_emitted_models` directly.
///
/// Plan TDD test 3 (`emitted_model_carries_origin_field`) end-to-end variant.
#[test]
fn emitted_model_carries_origin_in_real_explain_pipeline() {
    // Generator file: emits two ModelDefs named "east" and "west".
    let generator = "---\ngenerates: models\n---\n\
        [ModelDef { name: 'east', body: SELECT 1 AS id }, \
         ModelDef { name: 'west', body: SELECT 2 AS id }]";
    // Hand-authored model: plain SQL.
    let hand_authored = "SELECT 3 AS id";

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
    let (graph, _db) = smelt_cli::build_logical_graph(&project_dir, &config, None, &[], "dev")
        .expect("build logical graph");

    // Build explain output and assert on the JSON.
    // This test asserts on emitted-model provenance, not batch-safety, so it
    // needs no function registry.
    let output = smelt_cli::build_explain_output(&graph, &smelt_runtime::FnBodyMap::new())
        .expect("build explain output");
    let json = serde_json::to_string_pretty(&output).expect("serialize ExplainOutput");

    // --- Assert emitted models have origin ---
    // "cohorts.east" and "cohorts.west" are the smelt paths for the two emissions.
    for emitted_name in &["cohorts.east", "cohorts.west"] {
        let model = output.models.get(*emitted_name).unwrap_or_else(|| {
            panic!(
                "expected model '{}' in explain output; got models: {:?}",
                emitted_name,
                output.models.keys().collect::<Vec<_>>()
            )
        });
        assert!(
            model.origin.is_some(),
            "emitted model '{}' must have origin set; full JSON:\n{json}",
            emitted_name
        );
        if let Some(ModelOriginKind::Generated {
            ref generator_file,
            ref generator_name,
        }) = model.origin
        {
            assert!(
                generator_file.contains("cohorts.gen.sql"),
                "'{}' origin.generator_file must reference cohorts.gen.sql; got: {}",
                emitted_name,
                generator_file
            );
            assert!(
                *emitted_name == format!("cohorts.{generator_name}"),
                "'{}' origin.generator_name must match the ModelDef name; got: {}",
                emitted_name,
                generator_name
            );
        } else {
            panic!(
                "emitted model '{}' origin must be Generated, got: {:?}",
                emitted_name, model.origin
            );
        }
        // The JSON representation must contain the 'origin' key.
        assert!(
            json.contains("\"origin\""),
            "JSON output must contain 'origin' key for emitted models; got:\n{json}"
        );
        assert!(
            json.contains("\"generated\""),
            "JSON output must contain 'generated' type; got:\n{json}"
        );
    }

    // --- Assert hand-authored model has no origin ---
    let hand_authored_model = output.models.get("orders").unwrap_or_else(|| {
        panic!(
            "expected 'orders' model in explain output; got: {:?}",
            output.models.keys().collect::<Vec<_>>()
        )
    });
    assert!(
        hand_authored_model.origin.is_none(),
        "hand-authored model 'orders' must NOT have origin; full JSON:\n{json}"
    );
}
