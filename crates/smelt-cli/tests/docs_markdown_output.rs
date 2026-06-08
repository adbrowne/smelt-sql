//! Tests for the Markdown rendering of catalog model pages.
//!
//! Covers:
//!  - Generator-emitted models (Source line in metadata block).
//!  - Test-model targeting section ("## Tests" on the targeted model's page).
//!  - BUG-048 regression: per-model Markdown page must list test models that target it.
//!
//! Includes:
//!  - Unit smoke test for `render_model_page` with a direct `CatalogModel`
//!    that has `origin` set.
//!  - Integration test that exercises the real `build_catalog()` +
//!    `render_model_page()` pipeline with a tempdir workspace fixture.

use smelt_cli::docs::CatalogModel;
use smelt_cli::docs_render::render_model_page;
use smelt_core::ModelOriginKind;

// ── BUG-048 regression ────────────────────────────────────────────────────
// Spec §Surface: `models/<name>.md` must contain a "Tests" section listing
// every `materialization: test` model with `test.model: <this model>` in its
// frontmatter.  Previously, test models were filtered from the pipeline before
// `build_catalog()` was called, so the section was never rendered.

/// Integration test (BUG-048): a model page renders a "## Tests" section
/// listing the test model that targets it when the full catalog pipeline is
/// exercised (the path the `smelt docs generate` command takes).
#[test]
fn model_page_lists_targeting_test_models() {
    let model_sql = "SELECT 1 AS id";
    // A test model that targets "orders".
    let test_sql = "---\nmaterialization: test\ntest:\n  model: orders\n  expect:\n    - id: 1\n---\nSELECT id FROM orders";

    let tmp = stage_workspace(&[
        ("models/orders.sql", model_sql),
        ("models/orders_test.sql", test_sql),
    ]);
    let project_dir = tmp.path().to_path_buf();

    // Discover ALL models (including test models) and partition manually,
    // mirroring what `commands/docs.rs::generate()` does.
    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let discovery = smelt_cli::ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover models");

    // Collect test-target mapping before filtering.
    let mut test_targets: std::collections::HashMap<String, Vec<smelt_cli::docs::TestRef>> =
        std::collections::HashMap::new();
    for m in &sql_models {
        if m.is_test() {
            if let Some(tc) = m.test_config() {
                test_targets
                    .entry(tc.model.clone())
                    .or_default()
                    .push(smelt_cli::docs::TestRef {
                        name: m.name.clone(),
                        path: m.path.display().to_string(),
                    });
            }
        }
    }

    // Build catalog via the non-test model path (same as the CLI command).
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");
    let catalog = smelt_cli::docs::build_catalog(&graph, &config, &db, &origins, &test_targets)
        .expect("build catalog");

    // "orders" should be in the catalog.
    let orders = catalog
        .models
        .get("orders")
        .expect("'orders' must be in catalog");

    // The tests_targeting field must include the test model.
    assert!(
        !orders.tests_targeting.is_empty(),
        "orders.tests_targeting must be non-empty; got: {:?}",
        orders.tests_targeting
    );
    assert!(
        orders
            .tests_targeting
            .iter()
            .any(|t| t.name == "orders_test"),
        "tests_targeting must include 'orders_test'; got: {:?}",
        orders.tests_targeting
    );

    // The rendered Markdown page must contain a "## Tests" section.
    let markdown = smelt_cli::docs_render::render_model_page(orders);
    assert!(
        markdown.contains("## Tests"),
        "Markdown page for 'orders' must contain '## Tests' section; got:\n{markdown}"
    );
    assert!(
        markdown.contains("orders_test"),
        "Markdown 'Tests' section must mention 'orders_test'; got:\n{markdown}"
    );
}

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
        tests_targeting: vec![],
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
/// Uses `build_dependency_graph` so that discovery, generator-file filtering, and
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

    // `build_dependency_graph` runs the full pipeline: discover SQL files, init the
    // Salsa DB, run the emitted-models generator pipeline, filter generator files
    // from the hand-authored set, build the LogicalGraph, and annotate provenance.
    // The test relies on this helper — it does NOT call discover_emitted_model_files
    // or annotate_emitted_models directly.
    let config = smelt_cli::Config::load(&project_dir).expect("load config");
    let (graph, db, origins) =
        smelt_cli::build_dependency_graph_with_origins(&project_dir, &config, None, &[], "dev")
            .expect("build logical graph");

    // Build catalog.
    let catalog = smelt_cli::docs::build_catalog(
        &graph,
        &config,
        &db,
        &origins,
        &std::collections::HashMap::new(),
    )
    .expect("build catalog");

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
