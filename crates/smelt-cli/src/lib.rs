pub mod backend_registry;
pub mod backfill;
pub mod compiler;
pub mod config;
pub mod discovery;
pub mod docs;
pub mod docs_render;
pub mod errors;
pub mod executor;
pub mod explain;
pub mod logical_graph;
pub mod metadata;
pub mod migration;
pub mod physical_graph;
pub mod python;
pub mod seed;
pub mod selector;
pub mod temporal;
pub mod test_compiler;
pub mod test_property;
pub mod test_runner;
pub mod transformer;

pub use backend_registry::BackendRegistry;
pub use backfill::{
    compute_backbuild_plans, compute_batches_for_model, compute_range_run_plans,
    format_plan_summary, BackfillBatch, BackfillOptions, ModelBackfillPlan,
};
pub use compiler::{
    build_fn_body_map, prepend_ephemeral_ctes, resolve_refs_in_sql, CompiledModel,
    CompilerRegistry, EphemeralResolver, FnBodyMap, SqlCompiler,
};
pub use config::{
    find_project_root, BackendType, Config, IncrementalConfig, Materialization, SourcesConfig,
};
pub use discovery::{ModelDiscovery, ModelFile, ModelKind};
pub use errors::CliError;
pub use explain::{build_explain_output, build_physical_explain, ExplainModel, ExplainOutput};
pub use logical_graph::{LogicalGraph, LogicalNode};
pub use metadata::{extract_file_metadata, FileMetadata, MetadataError, ModelMetadata};
pub use physical_graph::{PhysicalGraph, PhysicalGraphBuilder, PhysicalNode, PhysicalStrategy};
pub use python::discover_python_models;
pub use selector::{parse_selector, SelectionMethod, Selector, SelectorParseError};
pub use smelt_core::RefInfo;
pub use temporal::{compute_incremental_windows, IncrementalWindows};
pub use test_compiler::{extract_ctes, CteInfo};
pub use test_runner::TestResult;
pub use transformer::{inject_time_filter, TimeRange, TransformError};

use anyhow::Context as _;
use std::collections::HashMap;
use std::path::Path;

/// Build a [`LogicalGraph`] from a project directory, handling generator-emitted
/// models automatically.
///
/// This function:
/// 1. Discovers all SQL model files under the configured model paths.
/// 2. Initialises a Salsa database from the raw files (including generator files)
///    so that the emitted-models pipeline (`smelt_db::emitted_models`) can run.
/// 3. Discovers generator-emitted virtual `ModelFile`s and their provenance.
/// 4. Filters generator files (`.gen.sql`) and test models out of the regular
///    model set so they don't pollute the graph.
/// 5. Builds a [`LogicalGraph`] from the combined (hand-authored + emitted) models.
/// 6. Annotates the graph nodes with generator provenance via
///    [`LogicalGraph::annotate_emitted_models`].
/// 7. Returns both the annotated graph and the Salsa database (the caller may
///    need the database for further queries, e.g. type inference in `build_catalog`).
pub fn build_logical_graph(
    project_dir: &Path,
    config: &Config,
    sources: Option<&smelt_core::SourcesConfig>,
    seeds: &[smelt_core::SeedInfo],
    default_target: &str,
) -> anyhow::Result<(LogicalGraph, smelt_db::Database)> {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Salsa DB initialised from raw files (includes generator files).
    let db = init_db(project_dir, &sql_models);

    // Emitted-models pipeline: virtual ModelFile entries + provenance map.
    let (emitted_model_files, origins) =
        discover_emitted_model_files(&db, project_dir, &config.paths);

    // Hand-authored models: exclude generator files so they don't collide with
    // their emitted children in the graph, and exclude test models.
    let mut models: Vec<ModelFile> = sql_models
        .into_iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .filter(|m| !m.is_test())
        .collect();
    models.extend(emitted_model_files);

    let mut graph = LogicalGraph::build(models, sources, seeds, config, default_target)
        .with_context(|| "Failed to build logical graph")?;

    graph.annotate_emitted_models(&origins);

    Ok((graph, db))
}

/// Initialize a Salsa database from discovered models and a project directory.
///
/// Loads sources.yml/sources.yaml, registers all model files, and returns a
/// ready-to-query database.
pub fn init_db(project_dir: &Path, models: &[ModelFile]) -> smelt_db::Database {
    use smelt_core::find_config_file;

    let mut db = smelt_db::Database::default();

    // Load sources.yml or sources.yaml
    let sources_yaml = match find_config_file(project_dir, "sources") {
        Ok(Some(path)) => std::fs::read_to_string(&path).unwrap_or_default(),
        Ok(None) => String::new(),
        Err(msg) => {
            tracing::warn!("{}", msg);
            String::new()
        }
    };
    let project = db.set_project_input(project_dir.to_path_buf(), sources_yaml);

    let mut source_files = Vec::with_capacity(models.len());
    for model in models {
        let sf = db.set_source_file(
            model.path.clone(),
            model.content.clone(),
            project_dir.to_path_buf(),
        );
        source_files.push(sf);
    }
    db.set_workspace(source_files, vec![project]);

    db
}

/// Discover emitted-model `ModelFile` objects and their origin provenance from
/// the Salsa generator pipeline.
///
/// Returns a pair:
/// - `Vec<ModelFile>`: one virtual `ModelFile` per surviving emitted model
///   (to be added to the models list before `LogicalGraph::build()`).
/// - `HashMap<String, (String, String)>`: maps the emitted model's smelt-path
///   name (e.g. `"cohorts.us_west"`) to `(generator_file_workspace_relative,
///   generator_def_name)` (e.g. `("models/cohorts.gen.sql", "us_west")`).
///
/// The scan_roots are the paths entries from `smelt.yml` (e.g. `["models"]`).
/// They are used by `smelt_db::emitted_model_smelt_path` to strip the scan-root
/// prefix when computing the smelt path.
///
/// Call `LogicalGraph::annotate_emitted_models(&origins)` after
/// `LogicalGraph::build()` to set provenance on the newly-inserted nodes.
pub fn discover_emitted_model_files(
    db: &smelt_db::Database,
    project_dir: &Path,
    scan_roots: &[String],
) -> (Vec<ModelFile>, HashMap<String, (String, String)>) {
    let workspace = match smelt_db::Workspace::try_get(db) {
        Some(ws) => ws,
        None => return (Vec::new(), HashMap::new()),
    };

    let result = smelt_db::emitted_models(db, workspace);
    let mut model_files = Vec::new();
    let mut origins: HashMap<String, (String, String)> = HashMap::new();

    for emitted in &result.survivors {
        let smelt_name = smelt_db::emitted_model_smelt_path(
            &emitted.generator_file,
            project_dir,
            scan_roots,
            &emitted.name,
        );

        // Workspace-relative generator file path (forward-slash separators).
        let gen_rel = emitted
            .generator_file
            .strip_prefix(project_dir)
            .unwrap_or(&emitted.generator_file)
            .to_string_lossy()
            .replace('\\', "/");

        origins.insert(smelt_name.clone(), (gen_rel, emitted.name.clone()));

        let mf = ModelFile::from_emitted_def(emitted, smelt_name);
        model_files.push(mf);
    }

    (model_files, origins)
}
