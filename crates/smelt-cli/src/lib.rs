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
pub use explain::{build_explain_output, build_physical_explain, ExplainOutput};
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

use std::path::Path;

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
