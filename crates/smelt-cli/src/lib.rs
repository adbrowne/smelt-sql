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
    prepend_ephemeral_ctes, resolve_refs_in_sql, CompiledModel, CompilerRegistry,
    EphemeralResolver, SqlCompiler,
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
use std::sync::Arc;

/// Initialize a Salsa database from discovered models and a project directory.
///
/// Loads sources.yml/sources.yaml, registers all model files, and returns a
/// ready-to-query database.
pub fn init_db(project_dir: &Path, models: &[ModelFile]) -> smelt_db::Database {
    use smelt_core::find_config_file;
    use smelt_db::Inputs;

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
    db.set_project_sources_yaml(project_dir.to_path_buf(), Arc::new(sources_yaml));
    db.set_all_project_roots(Arc::new(vec![project_dir.to_path_buf()]));

    let mut file_paths = Vec::with_capacity(models.len());
    for model in models {
        db.set_file_text(model.path.clone(), Arc::new(model.content.clone()));
        db.set_file_project_root(model.path.clone(), project_dir.to_path_buf());
        file_paths.push(model.path.clone());
    }
    db.set_all_files(Arc::new(file_paths));

    db
}
