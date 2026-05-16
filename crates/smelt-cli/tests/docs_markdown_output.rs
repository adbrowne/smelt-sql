//! Phase 5 (meta-language-E2): tests for the Markdown rendering of
//! generator-emitted models in `smelt docs generate` output.
//!
//! Includes:
//!  - Unit smoke test for `render_model_page` with a direct `CatalogModel`
//!    that has `origin` set.
//!  - Integration test that exercises the real `build_catalog()` +
//!    `render_model_page()` pipeline with a tempdir workspace fixture.

use smelt_cli::docs::CatalogModel;
use smelt_cli::docs_render::render_model_page;
use smelt_core::ModelOriginKind;

/// Unit smoke test: `render_model_page` surfaces a Source line when `origin`
/// is set on the `CatalogModel`.
#[test]
fn emitted_model_has_source_line_in_markdown() {
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
    };

    let markdown = render_model_page(&model);

    // Must contain a "Source" line naming the generator file and ModelDef name.
    assert!(
        markdown.contains("models/cohorts.gen.sql"),
        "Markdown must reference the generator file; got:\n{markdown}"
    );
    assert!(
        markdown.contains("us_west"),
        "Markdown must reference the ModelDef name; got:\n{markdown}"
    );
    // The "Source" label should appear somewhere in the metadata block.
    assert!(
        markdown.contains("Source") || markdown.contains("source"),
        "Markdown metadata block must include a Source label; got:\n{markdown}"
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

/// Integration test: the Markdown rendering of an emitted model (produced by
/// the real `build_catalog()` + `render_model_page()` pipeline) surfaces a
/// Source line that identifies the generator file and the `ModelDef.name`.
///
/// Uses `build_logical_graph` so that discovery, generator-file filtering, and
/// provenance annotation are all handled by the same helper the CLI command
/// handler uses — the test never calls `discover_emitted_model_files` or
/// `annotate_emitted_models` directly.
#[test]
fn emitted_model_has_source_line_in_real_docs_markdown_pipeline() {
    // Generator file: emits a single ModelDef named "west".
    let generator = "---\ngenerates: models\n---\n\
        [ModelDef { name: 'west', body: SELECT 1 AS id }]";
    let hand_authored = "SELECT 2 AS id";

    let tmp = stage_workspace(&[
        ("models/regions.gen.sql", generator),
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

    // Build catalog.
    let catalog = smelt_cli::docs::build_catalog(&graph, &config, &db).expect("build catalog");

    // Find the emitted model ("regions.west") and render its Markdown page.
    let (_, emitted) = catalog
        .models
        .iter()
        .find(|(k, _)| k.contains("west"))
        .unwrap_or_else(|| {
            panic!(
                "expected a model with 'west' in its name; got: {:?}",
                catalog.models.keys().collect::<Vec<_>>()
            )
        });

    let markdown = render_model_page(emitted);

    // The Markdown must reference the generator file and ModelDef name.
    assert!(
        markdown.contains("regions.gen.sql"),
        "Markdown must reference the generator file (regions.gen.sql); got:\n{markdown}"
    );
    assert!(
        markdown.contains("west"),
        "Markdown must reference the ModelDef name ('west'); got:\n{markdown}"
    );
    assert!(
        markdown.contains("Source") || markdown.contains("source"),
        "Markdown must include a 'Source' label; got:\n{markdown}"
    );
}
