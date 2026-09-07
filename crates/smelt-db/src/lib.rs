//! Salsa database for incremental compilation (salsa 0.26 API).
//!
//! This module defines the Salsa inputs and tracked queries that power the
//! LSP and optimizer. Salsa automatically handles incremental recomputation
//! when inputs change.
//!
//! # Architecture: Pure Function Rule
//!
//! All analysis logic must be implemented as **pure functions** that take AST
//! nodes and plain data structures (e.g., `TypeContext`, `ModelSchema`).
//! Salsa tracked functions should be thin wrappers: gather inputs via queries,
//! call the pure function, return the result.
//!
//! **DO**: `fn infer_type(expr: &Expr, ctx: &TypeContext) -> DataType`
//! **DON'T**: Put `db.some_query()` calls inside analysis logic.
//!
//! This keeps the core compiler logic reusable outside Salsa (batch CLI, planner)
//! and independently testable. See `type_inference.rs` and `schema.rs` for examples.
//!
//! # Module layout
//!
//! `lib.rs` is the crate root, so it cannot be a directory module; the bulk
//! of its former contents therefore lives in private sibling modules that are
//! re-exported here, keeping every item's historical `smelt_db::<item>` path.
//!
//! - `salsa_inputs` — Salsa scaffolding: the inputs (`SourceFile`,
//!   `ProjectInput`, `Workspace`, `DeployedSchemaInput`, `LoaderFileInput`),
//!   the `DiagnosticAcc` accumulator, and the `Database` with its path-keyed
//!   registries.
//! - `ids` — the small public ID/location types (`Model`, `RefLocation`,
//!   `SourceLocation`, `Position`, `Range`).
//! - `resolve` — workspace/project resolution: `resolve_ref_leaf`,
//!   `ResolvedRef`, `resolve_ref_path`, `project_sql_address_index`,
//!   `leaf_did_you_mean`, `resolve_source`, `find_project`,
//!   `find_deployed_schema`.
//! - `maintenance_refs` — the ref-resolving inputs the maintenance layer
//!   reads (`ref_timeseries_config`, `ref_source_info`, `model_edges_for`,
//!   `model_source_clamps`, …) plus the `maintenance_plan` /
//!   `maintenance_plan_report` wrappers. Every derivation itself lives in
//!   `smelt-logical`; these are input-gathering wrappers.
//! - `metadata_errors` — `map_metadata_error_to_diagnostic`, the
//!   compiler-enforced exhaustive `MetadataError` → `Diagnostic` match
//!   (`architecture.md` §"Fail-loud discipline").
//! - `meta_lists` — pure classifiers for bare `List<T>` select items.
//! - `diagnostic_mapping` — pure diagnostic-code mappers and the
//!   `cte_ref_outside_test_diagnostics` structural check (no DB access).
//! - `file_check` — the `file_diagnostics` / `check_file_diagnostics`
//!   orchestrator: a thin Salsa wrapper that gathers inputs and calls the
//!   pure checks above.
//! - `plan_query` — the `logical_plan` Salsa query.
//! - `function_graph` — workspace function bodies, the call graph, and
//!   function-call cycle detection.
//! - `diagnostics_types` — `DiagnosticCode`, `DiagnosticData`, `Diagnostic`,
//!   `DiagnosticSeverity`, and the meta-language message builders. Pure data
//!   types; no Salsa dependency.
//! - `queries/` — per-feature Salsa queries (parse / project / functions /
//!   function_diagnostics / schema / check_types / loader). Each submodule
//!   is documented in `queries::mod`.
//! - `backends`, `code_actions`, `config_vars`, `function_body_check`,
//!   `loader` (note: the loader **data module** — parsing/validation pure
//!   helpers), `provenance_validator`, `references`, `schema` (the
//!   **schema data types** — `Column`, `ModelSchema`, …), `type_inference`,
//!   `yaml_edits` — sibling modules.

mod diagnostic_mapping;
mod file_check;
mod file_check_tail;
mod function_graph;
mod ids;
mod maintenance_refs;
mod meta_lists;
mod metadata_errors;
mod plan_query;
mod resolve;
mod salsa_inputs;

pub mod backends;
pub mod code_actions;
pub mod config_vars;
pub mod diagnostics_types;
pub mod function_body_check;
pub mod loader;
pub mod provenance_validator;
pub mod queries;
pub mod references;
pub mod schema;
pub mod type_inference;
pub mod workspace_ingest;
pub mod yaml_edits;

// ---- Re-exports for downstream crates ---------------------------------------
//
// External consumers (smelt-lsp, smelt-cli, smelt-ui, smelt-bench, integration
// tests under crates/*/tests/) import these via the crate root, so the
// historical surface is preserved by re-exporting from the new sibling
// modules.

pub use smelt_core::{
    SeedInfo, SourceColumn, SourceColumnDef, SourceDef, SourceInfo, SourceTableDef, SourcesConfig,
};
pub use smelt_types::{
    parse_type, DataType, ModelOrigin, ModelRefValue, SourceOrigin, SourceRefValue, TypedColumn,
};

pub use diagnostics_types::{
    meta_hof_diagnostic_message, meta_list_diagnostic_message, meta_loader_diagnostic_message,
    meta_map_diagnostic_message, meta_multi_model_diagnostic_message,
    meta_record_diagnostic_message, meta_reflection_diagnostic_message,
    meta_reflection_diagnostic_message_with_table_expr, Diagnostic, DiagnosticCode, DiagnosticData,
    DiagnosticSeverity,
};

pub use function_body_check::{
    check_fragment_context_bindings, check_struct_row_var_binding, check_tier3_return_type,
    declared_return_hover_text, expand_brace_struct_body, extract_function_body_cte_schemas,
    infer_splice_contexts, infer_tableexpr_return_schema, is_tier2_function,
};
pub use schema::{
    Column, ColumnConstraint, ColumnSource, FunctionInput, FunctionOutput, InputConstraint,
    ModelFunctionType, ModelSchema, RefKind, ResolvedSchema, RowExtension, TypedField,
};
pub use type_inference::{
    check_window_in_scalar_contexts, infer_cte_columns, infer_expression_kind,
    infer_expression_type, infer_select_column_types, walk_expression_columns_with_visitor,
    walk_select_columns_with_visitor, TypeContext, WindowInScalarContextInfo,
};

pub use queries::check_types::{
    cannot_infer_type_for_schema, check_expression_types_for_select,
    check_timeseries_granularity_type, check_timeseries_nullability, check_type_diagnostics,
};
pub use queries::function_diagnostics::{
    as_struct_backend_diagnostics_for_file, backends_widening_diagnostics_for_file,
    context_mismatch_diagnostics_for_file, cte_cycle_diagnostics_for_file,
    cte_cycle_diagnostics_for_select, cte_shadow_caller_cte_diagnostics_for_file,
    default_references_parameter_diagnostics_for_file, duplicate_function_diagnostics_for_file,
    extern_fragment_param_diagnostics_for_file, frontmatter_parse_diagnostics_for_file,
    function_backends, function_body_diagnostics_for_file,
    invalid_function_type_ref_diagnostics_for_file, missing_provenance_advisory_for_file,
    provenance_unstable_diagnostics_for_file, smelt_fn_call_diagnostics_for_ast,
    smelt_fn_call_diagnostics_for_file, struct_field_type_unknown_diagnostics_for_file,
    unknown_context_diagnostics_for_file, workspace_function_diagnostics,
};
pub use queries::functions::{
    file_signature_inputs, function_body, function_signature, functions_in_file, resolve_function,
    resolve_function_path, workspace_function_signatures, BodyRange, NameRange,
};
pub use queries::loader::{
    loader_call_diagnostics_for_file, loader_call_diagnostics_for_file_with_content,
    loader_call_diagnostics_for_syntax, loader_file_parsed, loader_resolved_value,
    loader_resolved_value_with_overlay, parse_smelt_type_from_field_annotation,
    smelt_record_declarations, LoaderCallSiteId, LoaderResolvedValue,
};
pub use queries::monotonicity::{gate_nullable_leaf, trace_event_time_checked};
pub use queries::parse::{
    model_path_refs, model_sources, parse_file, parse_model, PathRefLocation,
};
pub use queries::project::{
    all_models, emitted_model_body_analysis, emitted_model_smelt_path, emitted_model_typed_schema,
    emitted_models, evaluate_generator, generator_files, models_all, models_all_with_generators,
    models_with_tag, project_active_backends, project_address_collisions,
    project_emitted_name_collisions, project_paths, project_seeds, project_source_diagnostics,
    project_sources, project_unstable_schema, project_warehouse_tables,
    resolve_seed_or_source_path, smelt_yml_vars_query, sorted_workspace_files, sources_all,
    sources_config, sources_type_errors, sources_with_tag, sources_yaml_error,
    AddressCollisionDiagnostic, EmissionBodyAnalysis, EmittedModelDef, EmittedModelsResult,
    EmittedNameCollisionDiagnostic, EvaluatedGenerator, SourceDiagnostic, SourceTypeError,
    YamlParseError,
};
pub use queries::schema::{
    add_source_info_to_type_context, apply_outer_join_nullability, available_columns,
    build_type_context, columns_of_for_table_expr, columns_to_column_ref_values,
    model_function_type, model_input_constraints, model_schema, resolved_model_schema,
    type_context, typed_model_schema, RefSchemaProvider, SalsaRefSchemaProvider,
    StaticRefSchemaProvider,
};

/// Re-exported so `smelt-lsp`'s hover formatter can consume
/// [`model_source_clamps`]'s return type without a new crate dependency.
pub use smelt_logical::{BoundResult, Offset, Seconds};

// ---- Re-exports from this crate's own split modules -------------------------
//
// `lib.rs` is the crate root and cannot be a directory module, so the bulk of
// its former contents lives in private sibling modules that are re-exported
// here. Every item keeps its historical `smelt_db::<item>` path.

pub(crate) use diagnostic_mapping::*;
pub use file_check::*;
pub use function_graph::*;
pub use ids::*;
pub use maintenance_refs::*;
pub(crate) use meta_lists::*;
pub(crate) use metadata_errors::*;
pub use plan_query::*;
pub use resolve::*;
pub use salsa_inputs::*;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod test_harness;

#[cfg(test)]
mod tests;
