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
//!   `yaml_edits` — pre-existing sibling modules; unchanged by this split.
//! - This file (`lib.rs`) keeps the Salsa scaffolding (inputs, `Database`,
//!   accumulator), the public ID types (`Model`, `RefLocation`, …), the
//!   workspace-level `resolve_ref` / `resolve_ref_path` / `resolve_source`
//!   queries, and the `file_diagnostics` / `check_file_diagnostics`
//!   orchestrator.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use line_index::{LineCol as LILineCol, LineIndex as LI};
use salsa::{Accumulator, Setter};
use smelt_core::metadata::{extract_file_metadata, FileMetadata, MetadataError, MixedKind};
use smelt_parser::{self, File as AstFile};

/// True if a top-level SELECT-item expression evaluates to a bare, unconsumed
/// `List<T>` — used to emit `MetaListInScalarPosition` (`meta_language.md`
/// §Semantics "Lists and spread" rule 10). List-yielding shapes:
///   - a bare list literal (`[1, 2, 3]`);
///   - a top-level `map` / `filter` HOF call (each produces a `List<U>`);
///   - a pipe whose outermost call is `map` / `filter` (`xs |> map(…)`).
///
/// `reduce` collapses a list to a scalar, so a `reduce(...)` item is consumed
/// and not list-yielding. A spread (`...xs`) is a `LIST_SPREAD` node, not a
/// select-item expression, so it never reaches this check.
///
/// A `smelt.config.load_yaml` / `load_json` call whose schema is `List<…>` or
/// `Map<Text, …>` yields a collection value (`List<record>` / `Map<Text,
/// record>`); left bare in a select item it is likewise unconsumed
/// (`meta_config_loading.md` — the loader is governed by the same
/// lists-must-be-consumed rule). A record-schema loader returns a single record,
/// not a collection, and is not flagged here.
/// Map a `MetadataError` to a `Diagnostic`, or `None` when the variant is
/// handled by a dedicated arm elsewhere in `check_file_diagnostics`.
///
/// **This match must remain exhaustive.** Every variant of `MetadataError` is
/// listed explicitly so the compiler refuses to compile when a new variant is
/// added without a corresponding handler. `None` arms are intentional: they
/// document that the variant is handled somewhere else (annotated inline).
/// This is the compiler-enforced gate for the fail-loud discipline —
/// `MetadataError` variant exhaustiveness rule (architecture.md §11).
fn map_metadata_error_to_diagnostic(err: &MetadataError) -> Option<Diagnostic> {
    match err {
        MetadataError::MalformedDelimiter(line) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "malformed multi-model section delimiter at line {line}: SQL content must be \
                 inside a '--- name: model_name ---' section; found non-section content before \
                 the first delimiter"
            ),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::MalformedSectionDelimiter),
            data: None,
        }),
        MetadataError::UnclosedFrontmatter(_line) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "frontmatter not closed: missing closing '---'".to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::UnclosedFrontmatter),
            data: None,
        }),
        MetadataError::MissingModelName(section) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("multi-model section {section} is missing a model name"),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::MalformedSectionDelimiter),
            data: None,
        }),
        MetadataError::YamlParseError(e) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("YAML parse error in frontmatter: {e}"),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::YamlParseError),
            data: None,
        }),
        // Raised at extraction time (`fold_top_level_safety_overrides`), like
        // `YamlParseError` above — reuses its `DiagnosticCode` rather than
        // adding a new catalogue entry for this structural conflict error.
        MetadataError::SafetyOverridesDoubleDeclared => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::YamlParseError),
            data: None,
        }),
        // Handled by dedicated arms in check_file_diagnostics (with precise span
        // anchoring and early returns):
        MetadataError::GeneratesUnknownValue { .. } => None,
        MetadataError::GeneratesMixedWithBareModel { .. } => None,
        // These variants only arise from validate_timeseries on the Ok(Single)
        // path — they are never returned by extract_file_metadata itself:
        MetadataError::TimeseriesRequiredForPartitionGrain => None,
        MetadataError::MalformedTimeseries { .. } => None,
        MetadataError::PlausibleContractOnSkeletonColumn { .. } => None,
        MetadataError::KeyedForbidsTimeseries => None,
        MetadataError::PartitionGrainRequiresRefreshIncremental => None,
        MetadataError::KeyedForbidsSafetyOverrides => None,
        MetadataError::MaterializedViewForbidsTimeseries => None,
        MetadataError::MaterializedViewForbidsPartitionGrain => None,
        MetadataError::MalformedFunctionalDependency { .. } => None,
        MetadataError::MalformedBoundedDomain { .. } => None,
        MetadataError::GrainRequiredForIncremental => None,
        MetadataError::GrainRequiresIncremental => None,
        MetadataError::GrainAssertionMismatch { .. } => None,
        // Never returned by extract_file_metadata/validate_timeseries — made
        // by `maintenance_plan_diagnostics` (needs the write-pattern
        // registry + backend capabilities) and folded into
        // `Maintenance*` diagnostics in `check_file_diagnostics` below,
        // exactly like `KeyedForbidsTimeseries` above.
        MetadataError::MaintenanceWritePatternUnavailable { .. } => None,
        MetadataError::MaintenanceWriteAddressingRefused { .. } => None,
        // Handled by a dedicated arm in check_file_diagnostics: `UnknownColumnTestKind`
        // is raised by the pure `validate_column_tests` on the `Ok(Single)` path;
        // `ColumnTestOnUnknownColumn` needs `typed_model_schema` (Salsa), which this
        // pure mapper does not have.
        MetadataError::UnknownColumnTestKind { .. } => None,
        MetadataError::ColumnTestOnUnknownColumn { .. } => None,
        // Raised by `extract_single_model`'s strict `contract:` pre-validation
        // (a pure format check, no Salsa data needed) — handled here like
        // `YamlParseError`, sharing its `ContractFrozenHorizonInvalid`
        // diagnostic code with the distinct grain-admissibility check made by
        // `smelt_logical::contract::frozen_horizon::validate_frozen_horizon`
        // (a dedicated arm further down in `check_file_diagnostics`, since
        // that check needs the parsed `ModelMetadata.grain`).
        MetadataError::ContractFrozenHorizonInvalid { .. } => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::ContractFrozenHorizonInvalid),
            data: None,
        }),
        // Raised by `extract_single_model`'s strict `contract:` pre-validation,
        // the same site and pattern as `ContractFrozenHorizonInvalid` above —
        // disambiguated by `smelt_core::metadata`'s own field-level check
        // rather than by this mapper.
        MetadataError::ContractDeferralInvalid { .. } => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::ContractDeferralInvalid),
            data: None,
        }),
        // Raised by `extract_single_model`'s strict `contract:` pre-validation,
        // the same site and pattern as `ContractFrozenHorizonInvalid`/
        // `ContractDeferralInvalid` above — disambiguated by
        // `smelt_core::metadata`'s own field-level check rather than by this
        // mapper.
        MetadataError::ContractRetainDepartedInvalid { .. } => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::ContractRetainDepartedInvalid),
            data: None,
        }),
    }
}

fn select_item_yields_bare_list(expr: &smelt_parser::ast::Expr) -> bool {
    // Case 1: a bare list literal directly in the select item.
    if expr.as_array_literal().is_some() {
        return true;
    }
    // Case 1b: a bare collection-valued loader call (`load_yaml` / `load_json`
    // with a `List<…>` / `Map<…>` schema argument).
    if loader_call_yields_collection(expr) {
        return true;
    }
    // Case 2a: a top-level `map` / `filter` HOF call.
    if let Some(call) = expr.as_function_call() {
        if hof_call_is_list_yielding(&call) {
            return true;
        }
    }
    // Case 2b: a pipe whose outermost RHS call is `map` / `filter`.
    let node = expr.syntax();
    let pipe = node
        .children()
        .find_map(smelt_parser::ast::PipeExpr::cast)
        .or_else(|| smelt_parser::ast::PipeExpr::cast(node.clone()));
    if let Some(pipe) = pipe {
        if let Some(rhs) = pipe.rhs() {
            if let Some(call) = rhs.as_function_call() {
                if hof_call_is_list_yielding(&call) {
                    return true;
                }
            }
        }
    }
    false
}

/// True if `expr` is a `smelt.config.load_yaml` / `load_json` call whose schema
/// argument (the second positional argument) is a `List<…>` or `Map<…>` type —
/// i.e. the loader's value is a collection that must be consumed before it
/// reaches a Data-World scalar position. A record-schema loader (`{…}` or a
/// named record) returns a single record and is excluded.
fn loader_call_yields_collection(expr: &smelt_parser::ast::Expr) -> bool {
    let Some(call) = expr.as_smelt_path_call() else {
        return false;
    };
    let segs = call.segments();
    if segs.len() != 2 || segs[0].to_lowercase() != "config" {
        return false;
    }
    let loader = segs[1].to_lowercase();
    if loader != "load_yaml" && loader != "load_json" {
        return false;
    }
    let Some(schema_arg) = call
        .arg_list()
        .and_then(|a| a.positional_args().into_iter().nth(1))
    else {
        return false;
    };
    let schema_text = schema_arg.syntax().text().to_string();
    let trimmed = schema_text.trim();
    trimmed.starts_with("List<") || trimmed.starts_with("Map<")
}

/// True if a function call is a `map` or `filter` HOF (the list-yielding HOFs).
/// `filter` lexes as a keyword (`FILTER_KW`), so `name()` may be `None`; fall
/// back to the call's first token text (mirrors `hof.rs`).
fn hof_call_is_list_yielding(call: &smelt_parser::ast::FunctionCall) -> bool {
    let name = call.name().map(|n| n.to_lowercase()).or_else(|| {
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .map(|t| t.text().to_lowercase())
            .find(|t| t == "map" || t == "filter")
    });
    matches!(name.as_deref(), Some("map") | Some("filter"))
}

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

// ============================================================================
// Salsa inputs
// ============================================================================

/// Re-exported so `smelt-lsp`'s hover formatter can consume
/// [`model_source_clamps`]'s return type without a new crate dependency.
pub use smelt_logical::{BoundResult, Offset, Seconds};

/// A source file tracked by Salsa. Holds the file's current text and the
/// project root it belongs to. Looked up by path via the Database's
/// internal registry (`Database::source_file`).
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub path: PathBuf,
    #[returns(ref)]
    pub text: String,
    #[returns(ref)]
    pub project_root: PathBuf,
}

/// A project-level input storing the raw `sources.yml` text for the project.
/// Looked up by project root path via `Database::project_input`.
#[salsa::input]
pub struct ProjectInput {
    #[returns(ref)]
    pub root: PathBuf,
    #[returns(ref)]
    pub sources_yaml: String,
    /// Raw text of the workspace's `smelt.yml` file. Empty string when not
    /// loaded (treated as no unstable flags set). Updated by the LSP whenever
    /// the file changes on disk, keeping Salsa's change detection valid.
    #[returns(ref)]
    pub smelt_yml_text: String,
}

/// Workspace-level singleton input tracking the full set of files and projects.
/// Queries that need to enumerate everything (e.g. `all_models`, `resolve_ref`)
/// read from this singleton so they're invalidated when the file set changes.
#[salsa::input(singleton)]
pub struct Workspace {
    #[returns(ref)]
    pub files: Vec<SourceFile>,
    #[returns(ref)]
    pub projects: Vec<ProjectInput>,
    /// All registered loader-file inputs. Kept here so that Salsa-tracked queries
    /// (which receive `&dyn salsa::Database`, not the concrete `Database`) can
    /// enumerate registered loader files without downcasting.
    #[returns(ref)]
    pub loader_files: Vec<LoaderFileInput>,
    /// The active build target (e.g. `"prod"`, `"staging"`). When `Some`, the
    /// loader resolution path dispatches to `loader_resolved_value_with_overlay`
    /// using `<basename>.<target>.<ext>` overlay files. `None` means base-only
    /// resolution. Set from the `smelt.yml` `target:` field (default) or via
    /// the `--target` CLI flag (override). Both CLI and LSP read the same
    /// config-derived default to keep discovery symmetric.
    pub active_target: Option<Arc<str>>,
    /// All registered deployed-schema snapshot inputs. Kept here so that
    /// Salsa-tracked queries (which receive `&dyn salsa::Database`, not the
    /// concrete `Database`) can enumerate registered snapshots without
    /// downcasting — mirrors `loader_files` above.
    #[returns(ref)]
    pub deployed_schemas: Vec<DeployedSchemaInput>,
}

// ============================================================================
// Deployed-schema snapshot inputs
// ============================================================================

/// A model's previously-deployed schema snapshot, registered as a Salsa
/// world-fact input (`docs/specs/definition_deltas.md` §"Detection": "the
/// deployed-schema snapshot is a world fact both the LSP and the CLI
/// register at workspace load"). One input per `(project_root, model)` pair
/// — project-scoped, per the project-isolation rule, since two projects in
/// the same workspace folder may each maintain a model of the same name.
#[salsa::input]
pub struct DeployedSchemaInput {
    #[returns(ref)]
    pub model: Arc<str>,
    #[returns(ref)]
    pub project_root: PathBuf,
    /// The snapshot's deployed output column names.
    #[returns(ref)]
    pub columns: Vec<Arc<str>>,
    /// The model's source SQL text at the time this schema was deployed —
    /// `None` for a snapshot written before `DeployedSchema::model_sql`
    /// existed. Consulted only by the skeleton-clause-changed check
    /// (`smelt_logical::maintenance::derive::skeleton_clause_changed`).
    #[returns(ref)]
    pub model_sql: Option<Arc<str>>,
    /// The model's declared `timeseries.partition_column` at the time this
    /// schema was deployed — `None` for a snapshot written before
    /// `DeployedSchema::partition_column` existed, or for a model with no
    /// partition grain. Consulted only by the partition-column-rename
    /// refusal check
    /// (`smelt_logical::maintenance::derive::partition_column_changed`).
    #[returns(ref)]
    pub partition_column: Option<Arc<str>>,
}

// ============================================================================
// Loader file inputs
// ============================================================================

/// A per-loader-call file path registered as a Salsa input.
///
/// One `LoaderFileInput` is created per unique loader-target path encountered
/// in the workspace. The LSP (and CLI build orchestration) creates and updates
/// these inputs when the corresponding files change on disk.
#[salsa::input]
pub struct LoaderFileInput {
    /// Workspace-relative path with `/` separators (e.g. `"configs/cohorts.yaml"`).
    #[returns(ref)]
    pub path: Arc<str>,
    /// Raw text of the file. Empty string when the file has not been loaded yet
    /// (callers should always set this before running queries).
    #[returns(ref)]
    pub text: Arc<str>,
    /// Whether the file currently exists in the workspace.
    pub exists: bool,
}

// ============================================================================
// Diagnostics accumulator
// ============================================================================

#[salsa::accumulator]
pub struct DiagnosticAcc(pub Diagnostic);

// ============================================================================
// Database
// ============================================================================

/// The Salsa database for smelt. The `files` and `projects` maps provide a
/// path-keyed registry for looking up input structs — Salsa itself doesn't
/// do this automatically.
#[salsa::db]
#[derive(Clone, Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
    files: Arc<RwLock<HashMap<PathBuf, SourceFile>>>,
    projects: Arc<RwLock<HashMap<PathBuf, ProjectInput>>>,
    /// Per-loader-file inputs keyed by workspace-relative path.
    loader_files: Arc<RwLock<HashMap<String, LoaderFileInput>>>,
    /// Per-deployed-schema inputs keyed by `(project_root, model)`.
    deployed_schemas: Arc<RwLock<HashMap<(PathBuf, String), DeployedSchemaInput>>>,
}

#[salsa::db]
impl salsa::Database for Database {}

/// Read/write a `Database` registry lock, recovering from poisoning instead of
/// panicking. The lock is only poisoned by a panic while the guard is held,
/// which cannot happen in the single-threaded Salsa mutation context these
/// registries are used in; recovering keeps the registry readable rather than
/// cascading a second panic if that invariant is ever violated.
fn read_registry<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_registry<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Database {
    /// Create or retrieve the `SourceFile` input for `path`, seeding its fields.
    /// If a `SourceFile` already exists, its text/project_root are updated to
    /// the provided values.
    pub fn set_source_file(
        &mut self,
        path: PathBuf,
        text: String,
        project_root: PathBuf,
    ) -> SourceFile {
        let existing = read_registry(&self.files).get(&path).copied();
        match existing {
            Some(file) => {
                file.set_text(self).to(text);
                file.set_project_root(self).to(project_root);
                file
            }
            None => {
                let file = SourceFile::new(self, path.clone(), text, project_root);
                write_registry(&self.files).insert(path, file);
                file
            }
        }
    }

    /// Create or retrieve the `ProjectInput` for `root`, seeding its yaml.
    ///
    /// Also reads `smelt.yml` from `root` (if present) and stores it in
    /// `smelt_yml_text` so that `project_unstable_schema` is Salsa-tracked
    /// without further call-site changes. The LSP can call
    /// `set_project_smelt_yml` later to propagate in-editor edits.
    pub fn set_project_input(&mut self, root: PathBuf, sources_yaml: String) -> ProjectInput {
        let smelt_yml_text = std::fs::read_to_string(root.join("smelt.yml")).unwrap_or_default();
        let existing = read_registry(&self.projects).get(&root).copied();
        match existing {
            Some(project) => {
                project.set_sources_yaml(self).to(sources_yaml);
                project.set_smelt_yml_text(self).to(smelt_yml_text);
                project
            }
            None => {
                let project = ProjectInput::new(self, root.clone(), sources_yaml, smelt_yml_text);
                write_registry(&self.projects).insert(root, project);
                project
            }
        }
    }

    /// Update the `smelt.yml` text for an already-registered project. Called by
    /// the LSP whenever the file changes on disk; Salsa propagates the
    /// invalidation through `project_unstable_schema` and any query that reads it.
    pub fn set_project_smelt_yml(&mut self, root: &Path, smelt_yml_text: String) {
        let project = read_registry(&self.projects).get(root).copied();
        if let Some(project) = project {
            project.set_smelt_yml_text(self).to(smelt_yml_text);
        }
    }

    /// Look up an already-registered `SourceFile` by path.
    pub fn source_file(&self, path: &Path) -> Option<SourceFile> {
        read_registry(&self.files).get(path).copied()
    }

    /// Look up an already-registered `ProjectInput` by root path.
    pub fn project_input(&self, root: &Path) -> Option<ProjectInput> {
        read_registry(&self.projects).get(root).copied()
    }

    /// Set (or create) the workspace singleton with the given file and project lists.
    ///
    /// Preserves the existing `loader_files` and `active_target` if the workspace
    /// already exists; `set_loader_file` and `set_active_target` manage those fields.
    pub fn set_workspace(&mut self, files: Vec<SourceFile>, projects: Vec<ProjectInput>) {
        match Workspace::try_get(self) {
            Some(ws) => {
                ws.set_files(self).to(files);
                ws.set_projects(self).to(projects);
                // loader_files and active_target are preserved; managed by
                // set_loader_file and set_active_target respectively.
            }
            None => {
                Workspace::new(self, files, projects, Vec::new(), None, Vec::new());
            }
        }
    }

    /// Convenience accessor: the workspace singleton, creating it empty if missing.
    pub fn workspace(&mut self) -> Workspace {
        match Workspace::try_get(self) {
            Some(ws) => ws,
            None => Workspace::new(self, Vec::new(), Vec::new(), Vec::new(), None, Vec::new()),
        }
    }

    /// Set the active build target on the workspace singleton.
    ///
    /// Changing the target causes Salsa to re-evaluate any tracked query that reads
    /// `workspace.active_target(db)`. The loader dispatch reads this value and selects
    /// `loader_resolved_value_with_overlay` when a matching `<basename>.<target>.<ext>`
    /// overlay file exists, falling back to the base file when no overlay is present.
    pub fn set_active_target(&mut self, target: Option<Arc<str>>) {
        if let Some(ws) = Workspace::try_get(self) {
            ws.set_active_target(self).to(target);
        } else {
            Workspace::new(self, Vec::new(), Vec::new(), Vec::new(), target, Vec::new());
        }
    }

    /// Create or update the `LoaderFileInput` for a workspace-relative path.
    ///
    /// Called by the LSP / CLI build orchestration whenever a loader-target file
    /// is discovered (during workspace load) or edited (during an LSP edit
    /// session). Salsa propagates invalidations to `loader_file_parsed` and
    /// `loader_resolved_value` automatically.
    pub fn set_loader_file(
        &mut self,
        path: Arc<str>,
        text: Arc<str>,
        exists: bool,
    ) -> LoaderFileInput {
        let path_str = path.to_string();
        let existing = read_registry(&self.loader_files).get(&path_str).copied();
        match existing {
            Some(input) => {
                input.set_path(self).to(path);
                input.set_text(self).to(text);
                input.set_exists(self).to(exists);
                // The input is already in workspace.loader_files; no structural change needed.
                input
            }
            None => {
                let input = LoaderFileInput::new(self, path, text, exists);
                write_registry(&self.loader_files).insert(path_str, input);
                // Register the new input into the workspace singleton so that
                // Salsa-tracked queries (which receive `&dyn salsa::Database`)
                // can enumerate loader files without downcasting.
                if let Some(ws) = Workspace::try_get(self) {
                    let mut current = ws.loader_files(self).to_vec();
                    current.push(input);
                    ws.set_loader_files(self).to(current);
                }
                input
            }
        }
    }

    /// Look up an already-registered `LoaderFileInput` by workspace-relative path.
    pub fn loader_file(&self, path: &str) -> Option<LoaderFileInput> {
        read_registry(&self.loader_files).get(path).copied()
    }

    /// Create or update the `DeployedSchemaInput` for `(project_root, model)`.
    ///
    /// Called by [`workspace_ingest::register_deployed_schemas_from_disk`]
    /// (CLI `init_db` / LSP `initialize`, workspace-loading-parity rule)
    /// whenever a `.smelt/targets/<target>/schemas/<model>.json` snapshot is
    /// discovered on disk. Salsa propagates invalidations to `maintenance_plan`
    /// / `check_file_diagnostics` automatically — re-setting an already-
    /// registered input's fields (a re-run `register_deployed_schemas_from_disk`
    /// after a fresh deploy) re-invalidates within the same `Database`.
    pub fn set_deployed_schema(
        &mut self,
        model: Arc<str>,
        project_root: PathBuf,
        columns: Vec<Arc<str>>,
        model_sql: Option<Arc<str>>,
        partition_column: Option<Arc<str>>,
    ) -> DeployedSchemaInput {
        let key = (project_root.clone(), model.to_string());
        let existing = read_registry(&self.deployed_schemas).get(&key).copied();
        match existing {
            Some(input) => {
                input.set_columns(self).to(columns);
                input.set_model_sql(self).to(model_sql);
                input.set_partition_column(self).to(partition_column);
                input
            }
            None => {
                let input = DeployedSchemaInput::new(
                    self,
                    model,
                    project_root,
                    columns,
                    model_sql,
                    partition_column,
                );
                write_registry(&self.deployed_schemas).insert(key, input);
                if let Some(ws) = Workspace::try_get(self) {
                    let mut current = ws.deployed_schemas(self).to_vec();
                    current.push(input);
                    ws.set_deployed_schemas(self).to(current);
                }
                input
            }
        }
    }

    /// Look up an already-registered `DeployedSchemaInput` by
    /// `(project_root, model)`.
    pub fn deployed_schema(&self, project_root: &Path, model: &str) -> Option<DeployedSchemaInput> {
        read_registry(&self.deployed_schemas)
            .get(&(project_root.to_path_buf(), model.to_string()))
            .copied()
    }
}

// ============================================================================
// Public data types
// ============================================================================

/// Represents a model (SQL file in models/ directory)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub name: String,
    pub path: PathBuf,
    /// Physical file path on disk (same as `path` for single-model files,
    /// differs for multi-model files where `path` is a virtual key).
    pub source_path: PathBuf,
}

/// Reference location with position information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLocation {
    pub name: String,
    pub range: rowan::TextRange,
}

/// Source location with position information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub source_name: String,
    pub table_name: String,
    pub qualified_name: String,
    pub range: rowan::TextRange,
}

/// Position in a file (line, column)
pub type Position = smelt_parser::ast::Position;

/// Range in a file (start, end)
pub type Range = smelt_parser::ast::Range;

// ============================================================================
// Semantic queries
// ============================================================================

/// Leaf-only model resolution used by the schema-inference subsystem
/// (`RowExtension.ref_name`, `InputConstraint.ref_name`) and the LSP's
/// column-goto-definition. Architecture Invariant 9 keeps leaf-only
/// resolution out of the value-ref path (`resolve_ref_path` is the
/// canonical path resolver for `smelt.<path>` refs in SQL bodies and
/// CLI argument resolution). The schema layer's column-origin tracking
/// still uses leaf names today; migrating it to canonical paths is a
/// separate refactor (tracked under architecture.md Known Divergences).
///
/// Project-scoped per `docs/specs/architecture.md` → "Project isolation
/// rule": a workspace folder may contain multiple smelt projects, and each
/// project is a closed resolution scope. Without filtering, a same-named
/// model in another project leaks into this project's name lookups.
///
/// Callers thread the project through from the file under analysis:
/// `source_file.project_root(db)` → `find_project(workspace, root)`.
pub fn resolve_ref_leaf(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: ProjectInput,
    model_name: String,
) -> Option<SourceFile> {
    let project_root = project.root(db);
    for file in workspace.files(db).iter().copied() {
        if file.project_root(db) != project_root {
            continue;
        }
        if let Some(model) = parse_model(db, file) {
            if model.name == model_name {
                return Some(file);
            }
        }
    }
    None
}

/// Result of resolving a `smelt.<path>` ref against the workspace.
///
/// Phase 2a unifies model / seed / source / function / test resolution
/// behind a single entry point — [`resolve_ref_path`]. Callers dispatch
/// on `kind` to decide what to do; `source_file` is populated for
/// `Model`, `Function`, and `Test` kinds (the entity lives in a
/// `.sql` file tracked by Salsa).
#[derive(Clone)]
pub struct ResolvedRef {
    pub kind: RefKind,
    /// The Salsa-tracked file backing the entity. Populated for
    /// `Model` / `Function` / `Test`. `None` for seeds and sources
    /// (which live outside the SQL file index).
    pub source_file: Option<SourceFile>,
    /// The path tuple used to perform the lookup, for round-tripping
    /// into diagnostics.
    pub path: Vec<String>,
}

impl std::fmt::Debug for ResolvedRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRef")
            .field("kind", &self.kind)
            .field("source_file", &self.source_file.is_some())
            .field("path", &self.path)
            .finish()
    }
}

/// Resolve a path tuple (`["models", "users"]`,
/// `["seeds", "raw", "users"]`, …) against the workspace.
///
/// Per architecture Surface §"Resolution: smelt.<path> is the universal
/// addressing scheme":
/// - `.sql` file with a bare SELECT → `Model`
/// - `.sql` file declaring `smelt.define` → `Function`
/// - `.sql` file containing `smelt.test` declarations → `Test`
/// - `.csv` under a project's `paths` → `Seed`
/// - `.yml` declaring an external table → `Source`
///
/// The tuple is matched against each workspace `SourceFile`'s path,
/// falling back to seed/source registries for non-SQL kinds. Kind
/// dispatch is by file format/content, never by directory name.
pub fn resolve_ref_path(
    db: &dyn salsa::Database,
    workspace: Workspace,
    path: Vec<String>,
) -> Option<ResolvedRef> {
    if path.is_empty() {
        return None;
    }

    // Try every project root in the workspace; the first match wins.
    for project in workspace.projects(db).iter().copied() {
        let project_root = project.root(db).clone();
        // Fetch the project's scan-root list once (cached via Salsa) and pass
        // it into `file_path_tuple` for every workspace file. Without this
        // hoist, each iteration of the file loop below would re-parse
        // `smelt.yml` from disk inside `file_path_tuple`, which scaled the
        // resolver to O(workspace_files * config_load_cost) per call.
        let scan_roots = project_paths(db, project);

        // Seeds: match by address_segments (Phase 2 — no "seeds" prefix required).
        // address_segments is the scan-root-stripped path tuple, so
        // `smelt.data.users` matches a seed at `seeds/data/users.csv` under
        // `paths: ["seeds"]` with address_segments = ["data", "users"].
        for seed in project_seeds(db, project).iter() {
            if seed.address_segments == path.as_slice() {
                return Some(ResolvedRef {
                    kind: RefKind::Seed,
                    source_file: None,
                    path,
                });
            }
        }

        // Sources: Phase 6 per-entity YAML files. Each source has an
        // `address_segments` tuple (scan-root-stripped path to stem).
        // `smelt.sources.raw.users` → path = ["sources", "raw", "users"]
        // which matches the `.yml` at `models/sources/raw/users.yml`.
        for source in project_sources(db, project).iter() {
            if source.address_segments == path.as_slice() {
                return Some(ResolvedRef {
                    kind: RefKind::Source,
                    source_file: None,
                    path,
                });
            }
        }

        // Legacy sources: project-level aggregate `sources.yml`. Used as a
        // fallback for any projects not yet migrated to per-entity YAMLs.
        // Kept until Phase 6 migration is complete across all callers.
        if project_sources(db, project).is_empty() && path.len() >= 3 && path[0] == "sources" {
            let source_name = &path[path.len() - 2];
            let table_name = &path[path.len() - 1];
            if resolve_source(db, project, source_name.clone(), table_name.clone()).is_some() {
                return Some(ResolvedRef {
                    kind: RefKind::Source,
                    source_file: None,
                    path,
                });
            }
        }

        // SQL files: O(1) lookup in the per-project address index instead of
        // rescanning every workspace file and recomputing its path tuple.
        // The index (`project_sql_address_index`) is a workspace-keyed tracked
        // query, so the scan runs once per revision rather than once per ref —
        // collapsing cold ref resolution from O(files × refs) to O(refs).
        if let Some((kind, file)) = project_sql_address_index(db, workspace, project).get(&path) {
            return Some(ResolvedRef {
                kind: *kind,
                source_file: Some(*file),
                path,
            });
        }

        // Generator-emitted models: check the W3 emission survivors for a path
        // match. Emitted models are not registered as SourceFile inputs, so they
        // are not found in the SQL-files walk above. The smelt path of an emitted
        // model is `<dir_dots>.<file_stem>.<ModelDef.name>` (from
        // `emitted_model_smelt_path`), and the dot-separated components equal the
        // `path` Vec we are resolving.
        let emitted = crate::queries::project::emitted_models(db, workspace);
        for emitted_model in &emitted.survivors {
            if !emitted_model.generator_file.starts_with(&project_root) {
                continue;
            }
            let smelt_name = crate::queries::project::emitted_model_smelt_path(
                &emitted_model.generator_file,
                &project_root,
                scan_roots.as_slice(),
                &emitted_model.name,
            );
            let emitted_path: Vec<String> = smelt_name.split('.').map(|s| s.to_string()).collect();
            if emitted_path == path {
                // Return a ResolvedRef pointing at the generator file; the
                // goto-def handler will navigate to the ModelDef.name span within it.
                // Look up the generator file's SourceFile handle from workspace files.
                let gen_file = workspace
                    .files(db)
                    .iter()
                    .copied()
                    .find(|f| f.path(db) == &emitted_model.generator_file);
                return Some(ResolvedRef {
                    kind: RefKind::Model,
                    source_file: gen_file,
                    path,
                });
            }
        }
    }

    None
}

/// Compute the path tuple for a SQL file relative to its project root,
/// stripping the matching `config.paths` scan-root prefix if one applies.
///
/// Algorithm:
/// 1. Strip `project_root` to get `rel`.
/// 2. Try each scan root from `config.paths`: if `rel` starts with the
///    scan root, use the remainder as the parent path.
/// 3. If no scan root matches (e.g. `functions/` with `paths: ["models"]`),
///    fall back to using `rel.parent()` (original behaviour).
/// 4. Build tuple from parent segments + leaf name (model name override as today).
///
/// Returns `None` if the file is not a descendant of the project root.
fn file_path_tuple(
    project_root: &Path,
    file_path: &Path,
    file: SourceFile,
    db: &dyn salsa::Database,
    scan_roots: &[String],
) -> Option<Vec<String>> {
    let rel = file_path.strip_prefix(project_root).ok()?;

    // Try each scan root. Use the first one that `rel` is under.
    let effective_rel = scan_roots
        .iter()
        .find_map(|sr| rel.strip_prefix(sr.as_str()).ok())
        .unwrap_or(rel);

    let parent = effective_rel.parent()?;
    let mut tuple: Vec<String> = parent
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    // Leaf segment: prefer the parsed model name (so multi-model files
    // expose their declared `name:` rather than the filename), falling
    // back to the file stem for non-model SQL files (functions, tests).
    let leaf = parse_model(db, file).map(|m| m.name.clone()).or_else(|| {
        file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })?;
    tuple.push(leaf);
    Some(tuple)
}

/// Determine the kind of a SQL file by its content. Model, function,
/// and test all live in `.sql` files; the dispatch is on
/// content/frontmatter, not filename.
fn sql_file_kind(db: &dyn salsa::Database, file: SourceFile) -> RefKind {
    // 1. `smelt.define` → Function; `smelt.test` → Test. Both dispatch on
    //    the parsed AST (already cached by Salsa).
    let parse = parse_file(db, file);
    if let Some(ast) = AstFile::cast(parse.syntax()) {
        if ast.defines().next().is_some() {
            return RefKind::Function;
        }
        if ast.tests().next().is_some() {
            return RefKind::Test;
        }
        if ast.checks().next().is_some() {
            return RefKind::Check;
        }
    }
    // 2. Default: Model.
    RefKind::Model
}

/// One-pass index from a project's SQL-file path tuples to their
/// `(RefKind, SourceFile)`, keyed on the [`Workspace`] + [`ProjectInput`].
///
/// `resolve_ref_path` previously rescanned **every** workspace file (computing
/// `file_path_tuple` for each) on every call, making a cold diagnostics pass
/// O(files × refs × files) — the dominant `std::path` cost in the Initial Load
/// benchmark. Hoisting that scan into one workspace-keyed query collapses the
/// per-ref cost to an O(1) `HashMap` lookup; the scan runs once per revision and
/// is shared by every resolver call. This mirrors `workspace_function_signatures`.
///
/// First-writer-wins on tuple collisions, preserving the original loop's
/// "first matching file in `workspace.files` order wins" semantics.
#[salsa::tracked]
pub fn project_sql_address_index(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: ProjectInput,
) -> Arc<HashMap<Vec<String>, (RefKind, SourceFile)>> {
    let project_root = project.root(db).clone();
    let scan_roots = project_paths(db, project);
    let mut map: HashMap<Vec<String>, (RefKind, SourceFile)> = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        let file_path = file.path(db);
        // Mirror the resolver's file filter: SQL models, Python models (whose
        // content is generated SQL), virtual `*.sql::model` split paths, and
        // virtual `*.py::name` paths for Python-emitted models.
        // Note: Path::extension() on "py_source.py::py_source" returns
        // "py::py_source" (everything after the last dot), not "py", so the
        // .py:: check is required to catch Python virtual paths.
        let path_str = file_path.to_str().unwrap_or("");
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "sql"
            && ext != "py"
            && !path_str.contains(".sql::")
            && !path_str.contains(".py::")
        {
            continue;
        }
        let Some(tuple) = file_path_tuple(&project_root, file_path, file, db, &scan_roots) else {
            continue;
        };
        map.entry(tuple)
            .or_insert_with(|| (sql_file_kind(db, file), file));
    }
    Arc::new(map)
}

/// Find every canonical `smelt.<path>` address in `workspace` whose leaf
/// segment equals `leaf`, scoped to `project` per the Project Isolation Rule.
///
/// Returns the canonical paths (with the `smelt.` prefix) sorted
/// alphabetically. The result is used by the `UndefinedModelRef` diagnostic to
/// generate a "did you mean …?" hint.
///
/// This is a **pure function**: it receives all necessary data as parameters
/// and performs no Salsa query calls internally — Salsa queries are called by
/// the callers that gather the inputs before passing them in.
///
/// # Project Isolation Rule
/// When `project` is `Some`, only files belonging to that project are
/// considered. Two projects in the same workspace folder do not share leaf-match
/// candidates.
pub fn leaf_did_you_mean(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    leaf: &str,
) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();

    for file in workspace.files(db).iter().copied() {
        // Project isolation: skip files from other projects.
        if let Some(p) = project {
            if file.project_root(db) != p.root(db) {
                continue;
            }
        }

        // Only SQL or Python-emitted model files can be models.
        let file_path = file.path(db);
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let path_str = file_path.to_str().unwrap_or("");
        if ext != "sql"
            && ext != "py"
            && !path_str.contains(".sql::")
            && !path_str.contains(".py::")
        {
            continue;
        }

        // Get the leaf segment: parse_model gives us the declared model name;
        // fall back to file stem for non-model files.
        let file_leaf = parse_model(db, file).map(|m| m.name.clone()).or_else(|| {
            file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        });

        if file_leaf.as_deref() != Some(leaf) {
            continue;
        }

        // Compute the canonical path for this file using the project's scan roots.
        // We need to determine which project this file belongs to in order to
        // get the correct scan roots.
        let project_for_file = project.or_else(|| {
            let file_root = file.project_root(db).clone();
            workspace
                .projects(db)
                .iter()
                .copied()
                .find(|p| p.root(db) == &file_root)
        });

        let tuple = if let Some(p) = project_for_file {
            let scan_roots = project_paths(db, p);
            file_path_tuple(p.root(db), file_path, file, db, &scan_roots)
        } else {
            None
        };

        if let Some(t) = tuple {
            candidates.push(format!("smelt.{}", t.join(".")));
        }
    }

    // Sort alphabetically for deterministic output.
    candidates.sort();
    candidates
}

#[salsa::tracked]
pub fn resolve_source(
    db: &dyn salsa::Database,
    project: ProjectInput,
    source_name: String,
    table_name: String,
) -> Option<SourceTableDef> {
    let config = sources_config(db, project);
    let source = config.sources.iter().find(|s| s.name == source_name)?;
    source.tables.iter().find(|t| t.name == table_name).cloned()
}

// ============================================================================
// Diagnostics (accumulator-based orchestrator)
// ============================================================================

/// Top-level file diagnostics. Internally dispatches to the parse/type checkers,
/// which push into `DiagnosticAcc`. Returns the accumulated diagnostics.
pub fn file_diagnostics(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    check_file_diagnostics::accumulated::<DiagnosticAcc>(db, workspace, file)
        .into_iter()
        .map(|d| d.0.clone())
        .collect()
}

/// Pure structural check: walk all `SMELT_PATH_REF` nodes in `syntax`
/// that carry a `#`-suffix `CTE_SEGMENT` child.  For each such node, check
/// whether any ancestor is a `SMELT_TEST` node.  If NOT, emit a
/// `CteRefOutsideTest` diagnostic anchored at the `#` token.
///
/// This is a Salsa-purity-compliant analysis function (no DB access).  The
/// thin Salsa wrapper in `check_file_diagnostics` calls it after gathering the
/// parse input.
fn cte_ref_outside_test_diagnostics(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
) -> Vec<Diagnostic> {
    use smelt_parser::ast::SmeltPathRef;
    use smelt_parser::SyntaxKind::{SMELT_PATH_REF, SMELT_TEST};

    let mut diags = Vec::new();
    for node in syntax.descendants().filter(|n| n.kind() == SMELT_PATH_REF) {
        if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
            if let Some(hash_range) = path_ref.hash_range() {
                // Emit unless there is a SMELT_TEST ancestor.
                let inside_test = node.ancestors().any(|a| a.kind() == SMELT_TEST);
                if !inside_test {
                    diags.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "CTE references using `#` are only valid inside a `smelt.test` body; \
                                  remove the `#<cte>` suffix or move this reference inside a `smelt.test` declaration"
                            .to_string(),
                        range: hash_range,
                        code: Some(DiagnosticCode::CteRefOutsideTest),
                        data: None,
                    });
                }
            }
        }
    }
    diags
}

/// Lowercase display of a `Granularity` for diagnostic messages (matches the
/// wire/frontmatter spelling, e.g. `granularity: day`).
fn granularity_lower(g: smelt_core::Granularity) -> &'static str {
    use smelt_core::Granularity as G;
    match g {
        G::Hour => "hour",
        G::Day => "day",
        G::Week => "week",
        G::Month => "month",
        G::Quarter => "quarter",
        G::Year => "year",
    }
}

/// Map a planner-rule diagnostic code onto smelt-db's diagnostic-code
/// catalogue. The 1:1 mapping is the seam the Diagnostic-parity rule relies on
/// (`architecture.md` §"Planner scope").
fn rule_diagnostic_code(code: smelt_logical::RuleDiagnosticCode) -> DiagnosticCode {
    use smelt_logical::RuleDiagnosticCode as R;
    match code {
        R::KeyedRequiresGroupBy => DiagnosticCode::KeyedRequiresGroupBy,
        R::KeyedUnknownCombiner => DiagnosticCode::KeyedUnknownCombiner,
        R::KeyedGroupByContainsPartitionColumn => {
            DiagnosticCode::KeyedGroupByContainsPartitionColumn
        }
        R::KeyedForbidsWindowFunctions => DiagnosticCode::KeyedForbidsWindowFunctions,
        R::KeyedForbidsNondeterministic => DiagnosticCode::KeyedForbidsNondeterministic,
        R::KeyedSnapshotPostureUnsupported => DiagnosticCode::KeyedSnapshotPostureUnsupported,
        R::KeyedSnapshotSourceUnsupportedColumn => {
            DiagnosticCode::KeyedSnapshotSourceUnsupportedColumn
        }
        R::KeyedMultipleDrivingSources => DiagnosticCode::KeyedMultipleDrivingSources,
        R::KeyedSqlNotParseable => DiagnosticCode::KeyedSqlNotParseable,
        R::KeyedOnceWriteUnproven => DiagnosticCode::KeyedOnceWriteUnproven,
        R::KeyedStateColumnCollision => DiagnosticCode::KeyedStateColumnCollision,
        R::PartitionGrainNotSafe => DiagnosticCode::PartitionGrainNotSafe,
        R::EventTimeColumnNotVisibleAtOuterSelect => {
            DiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect
        }
        R::PartitionGrainForbidsMetrics => DiagnosticCode::PartitionGrainForbidsMetrics,
    }
}

/// Remap a parse error message to a more specific diagnostic code when the
/// error originated from the pipe-stage parser.
///
/// The pipe-stage parser emits errors via `Parser::error()`, which stores them
/// as parse errors with the message text. This function inspects the message to
/// promote those errors to their proper diagnostic codes so consumers can
/// distinguish pipe-specific errors from generic syntax errors.
///
/// Mapping rules:
/// - `"pipe operator '<kw>' is not supported — …"` → `PipeOperatorUnsupported`
/// - `"unknown pipe operator '<kw>'"` → `PipeUnknownOperator`
/// - `"malformed '<kw>' pipe stage"` → `PipeStageMalformed`
/// - `"unexpected content after model body"` → `TrailingTopLevelContent`
/// - anything else → `ParseError` (unchanged)
fn remap_pipe_parse_error_code(message: &str) -> DiagnosticCode {
    if message.starts_with("pipe operator '") && message.contains("is not supported") {
        DiagnosticCode::PipeOperatorUnsupported
    } else if message.starts_with("unknown pipe operator '") {
        DiagnosticCode::PipeUnknownOperator
    } else if message.starts_with("malformed '") && message.contains("pipe stage") {
        DiagnosticCode::PipeStageMalformed
    } else if message == "unexpected content after model body" {
        DiagnosticCode::TrailingTopLevelContent
    } else {
        DiagnosticCode::ParseError
    }
}

/// Resolve a `smelt.<path>` ref string to its definition's frontmatter
/// `timeseries:` block, when it resolves to a model that declares one. This
/// reconstructs (project-scoped) the `smelt.<path> → timeseries` lookup the
/// runtime builds from the model graph, so the keyed classifier sees the
/// same driving sources in the editor as it does at build time.
fn ref_timeseries_config(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    ref_str: &str,
) -> Option<smelt_core::config::TimeseriesConfig> {
    let segments: Vec<String> = ref_str
        .strip_prefix("smelt.")?
        .split('.')
        .map(|s| s.to_string())
        .collect();
    let leaf = segments.last()?.clone();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    // Per-entity source YAML (`RefKind::Source`) has no `source_file` — its
    // `timeseries:` block lives on the `SourceInfo` the project's source scan
    // already parsed, not on a frontmatter-bearing model file. Look it up by
    // `address_segments` before falling through to the model-file path below
    // (which only applies to `RefKind::Model`/generator refs).
    if resolved.kind == RefKind::Source {
        let project = project?;
        return project_sources(db, project)
            .iter()
            .find(|s| s.address_segments == segments)
            .and_then(|s| s.timeseries.clone());
    }
    let file = resolved.source_file?;
    let text = file.text(db);
    match extract_file_metadata(text) {
        // Hand-authored single model: the `timeseries:` is its own frontmatter.
        Ok(FileMetadata::Single { metadata, .. }) => metadata.timeseries.clone(),
        // Multi-model file: match the addressed section by name.
        Ok(FileMetadata::Multi { models }) => models
            .iter()
            .find(|s| s.metadata.name.as_deref() == Some(leaf.as_str()))
            .and_then(|s| s.metadata.timeseries.clone()),
        // Generator-emitted model: `timeseries:` is inherited onto the emitted
        // model (carried on the `EmittedModelDef`), not on the generator file's
        // own frontmatter — mirror the runtime, which reads it from the graph.
        Ok(FileMetadata::Generator { .. }) => emitted_models(db, workspace)
            .survivors
            .iter()
            .find(|e| e.name == leaf)
            .and_then(|e| e.timeseries_config.clone()),
        _ => None,
    }
}

/// Resolve `ref_str` to its [`smelt_core::SourceInfo`] when it addresses a
/// declared source — `None` when the ref doesn't resolve, or resolves to
/// something other than a source (a model, seed, function). Sibling of
/// [`ref_timeseries_config`], reused by [`maintenance_plan`] to build the
/// [`smelt_logical::maintenance::SourceFacts`] the plan derivation reads.
fn ref_source_info(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    ref_str: &str,
) -> Option<smelt_core::SourceInfo> {
    let segments: Vec<String> = ref_str
        .strip_prefix("smelt.")?
        .split('.')
        .map(|s| s.to_string())
        .collect();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Source {
        return None;
    }
    let project = project?;
    project_sources(db, project)
        .iter()
        .find(|s| s.address_segments == segments)
        .cloned()
}

/// Resolve `ref_str` to a locality-admitted composed model's own output as
/// a [`smelt_logical::maintenance::SourceFacts`] candidate driving source
/// (`incremental_shapes.md` §"Key temporal locality (the time-partitioned
/// output)" — "The output as a clocked source": "a downstream keyed model
/// may take it as its clocked driving source"). `None` when the ref does
/// not resolve to a maintained `grain: key` model whose own `timeseries:`
/// block cleared the locality gate — a declared source, a `full`/view
/// model, a `grain: partition` model (already visible to downstream
/// pushdown via `smelt-logical`'s own model-graph registry, not this
/// path), or a keyed model whose own locality gate refused all resolve to
/// `None` here, so the caller's driving-source resolution falls back to
/// whatever declared sources it has.
///
/// Recurses one level into [`maintenance_plan_report`] over the upstream's
/// own file to read its already-derived
/// [`smelt_logical::maintenance::KeyLocality`] verdict — this never
/// re-implements the locality gate itself (`CLAUDE.md` §"Maintenance-plan
/// purity"): it calls the same pure entry point
/// ([`smelt_logical::maintenance::locality::establish_locality`], reached
/// via [`crate::queries::maintenance::derive_model_maintenance_plan`]) the
/// upstream's own plan derivation already calls, and reads its result
/// rather than deriving a second one. Terminates because the model graph
/// is acyclic (a `smelt.ref()` cycle is rejected elsewhere in workspace
/// loading); a long composed chain recurses one frame per hop, which is
/// how the clock is meant to propagate through the DAG.
/// Returns the candidate [`SourceFacts`](smelt_logical::maintenance::SourceFacts)
/// alongside the upstream's own declared `timeseries.granularity` — a
/// downstream keyed model's locality gate needs both: the source-shape
/// candidate for [`smelt_logical::maintenance::locality::
/// resolve_driving_source`], and the granularity for the gate's
/// granularity-equality structural precondition (mirroring
/// [`crate::queries::maintenance::single_clocked_source_granularity`]'s
/// role for declared sources).
/// `model_scan_bounds`/`project_scan_bounds` are the DOWNSTREAM (referencing)
/// model's own `maintenance.scan_bounds` declarations — the same two configs
/// [`crate::queries::maintenance::build_source_facts`] already threads for a
/// declared `sources:` entry — consulted here so a model-edge candidate can
/// be granted `allow_full_scan` too (keyed by its bare, `smelt.`-stripped
/// name, exactly like a declared source's `per_source` key): before this,
/// there was no way to declare the K8 escape hatch for an upstream
/// maintained-model source at all, which phase 19
/// (`docs/outcomes/20260815-definition-delta-migrate`) newly needs — an
/// `UpstreamMutation` cell is now genuinely derivable for one of these
/// candidates too (an `AppendOnly` composed source in a value-sensitive
/// aggregate column group), not only for a declared `sources:` entry.
fn ref_model_source_facts(
    db: &dyn salsa::Database,
    workspace: Workspace,
    ref_str: &str,
    model_scan_bounds: Option<&smelt_core::config::ScanBoundsConfig>,
    project_scan_bounds: Option<&smelt_core::config::ScanBoundsConfig>,
) -> Option<(
    smelt_logical::maintenance::SourceFacts,
    smelt_core::config::Granularity,
)> {
    let stripped = ref_str.strip_prefix("smelt.")?;
    let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Model {
        return None;
    }
    let file = resolved.source_file?;
    let result = maintenance_plan_report(db, workspace, file)?;
    let locality = result.plan.key_locality.as_ref()?;
    let granularity = ref_timeseries_config(
        db,
        workspace,
        find_project(db, workspace, file.project_root(db)),
        ref_str,
    )?
    .granularity;
    let (allow_full_scan, _require, _on_violation) =
        crate::queries::maintenance::effective_scan_bounds(
            stripped,
            model_scan_bounds,
            project_scan_bounds,
        );
    Some((
        smelt_logical::maintenance::SourceFacts {
            name: stripped.to_string(),
            // A composed maintained output's rows, once written by a run,
            // are not retroactively mutated by a *later* run touching a
            // different slice — the same append-only posture a declared
            // `timeseries:` source with no explicit
            // `mutation_profile: mutable` gets by default
            // (`crate::queries::maintenance::source_facts`).
            mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
            partition_col: Some(locality.slice.partition_column().to_string()),
            unique_key: Vec::new(),
            allow_full_scan,
        },
        granularity,
    ))
}

/// Resolve `ref_str` to an upstream **maintained-model edge**
/// (`incremental_models.md` §"Upstream model edges") when it addresses another
/// maintained (non-`full`, non-view) model in this project — `None` when the
/// ref doesn't resolve, resolves to a source/seed/function, or resolves to a
/// `full`-mode or view model (which delivers no incremental delta and so
/// contributes neither a creation cell nor a refusal). Sibling of
/// [`ref_source_info`]; reused by [`maintenance_plan_report`] to assemble the
/// [`smelt_logical::maintenance::derive::ModelEdge`]s the plan derivation
/// reads. `clock_col` is the upstream's own validated
/// `timeseries.partition_column`, or `None` when it declares none — the
/// derivation records that as a `MaintenanceReachNotDerivable` refusal.
/// Extract the addressed section's own SQL body (frontmatter stripped) and
/// [`smelt_core::metadata::ModelMetadata`] from a model file's full `text`,
/// for either a single-model file (`leaf` unused) or a multi-model file
/// (matched by declared `name:`). `None` for a generator file — its
/// maintenance metadata lives on the emitted model, not the generator
/// file's own frontmatter (not exercised by any current maintained-upstream
/// fixture; resolving it is deferred), or a file with no frontmatter.
fn resolved_model_sql_and_meta(
    text: &str,
    leaf: &str,
) -> Option<(String, smelt_core::metadata::ModelMetadata)> {
    match extract_file_metadata(text) {
        Ok(FileMetadata::Single {
            metadata,
            sql_offset,
        }) => Some((text[sql_offset..].to_string(), *metadata)),
        Ok(FileMetadata::Multi { models }) => {
            let section = models
                .into_iter()
                .find(|s| s.metadata.name.as_deref() == Some(leaf))?;
            Some((
                text[section.sql_range.clone()].to_string(),
                section.metadata,
            ))
        }
        _ => None,
    }
}

/// This model's own `smelt.sources.*` refs as [`output_delta::SourceFacts`]
/// — the per-model input the output-delta walk reads. Mirrors
/// `smelt-runtime::propagation::model_output_delta_sources`'s declared-source
/// collection over a `ModelFile`'s own `refs`, rebuilt here from `sql` text
/// via [`smelt_logical::collect_path_refs`] since the Salsa side has no
/// eagerly-loaded `ModelFile::refs` at this call site.
fn model_own_source_facts(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    sql: &str,
) -> Vec<smelt_logical::analysis::output_delta::SourceFacts> {
    let mut sources = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for r in smelt_logical::collect_path_refs(sql) {
        let Some(stripped) = r.strip_prefix("smelt.") else {
            continue;
        };
        let Some(bare) = stripped.strip_prefix("sources.") else {
            continue;
        };
        if !seen.insert(bare.to_string()) {
            continue;
        }
        if let Some(info) = ref_source_info(db, workspace, project, &r) {
            sources.push(
                smelt_logical::analysis::output_delta::SourceFacts::from_source_info(bare, &info),
            );
        }
    }
    sources
}

/// Assemble the per-model [`smelt_logical::analysis::output_delta::
/// ModelDeltaInput`] records for the cross-model output-delta fold
/// (`derive_workspace_output_deltas`), scoped to every model transitively
/// reachable from `file`'s own refs — mirrors `smelt-runtime::propagation::
/// workspace_output_delta_verdicts`'s per-model input shape, but built by
/// walking refs rather than over an eagerly-loaded `&[ModelFile]` (`smelt-db`
/// has no such list at this call site). `address` is the ref's own
/// `smelt.`-stripped path, lowercased — the SAME key
/// [`smelt_logical::analysis::output_delta::derive_workspace_output_deltas`]
/// inserts into its verdict map, so a model-reference leaf inside any
/// reached model's own SQL resolves against it. Deduplicated by that address
/// (not by `SourceFile`), which is what makes a cyclic model-ref graph
/// terminate: each distinct address is queued at most once, so the walk is
/// bounded by the number of distinct reachable addresses regardless of how
/// many cycles connect them — never a per-model-reference recursive Salsa
/// query (`CLAUDE.md` §"Salsa purity rule"), which could not terminate over
/// a cycle.
fn model_delta_inputs(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<smelt_logical::analysis::output_delta::ModelDeltaInput> {
    let mut inputs = Vec::new();
    let mut visited: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier: Vec<SourceFile> = vec![file];
    while let Some(f) = frontier.pop() {
        let text = f.text(db);
        for r in smelt_logical::collect_path_refs(text) {
            let Some(stripped) = r.strip_prefix("smelt.") else {
                continue;
            };
            let address = stripped.to_ascii_lowercase();
            if !visited.insert(address.clone()) {
                continue;
            }
            let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
            let Some(leaf) = segments.last().cloned() else {
                continue;
            };
            let Some(resolved) = resolve_ref_path(db, workspace, segments) else {
                continue;
            };
            if resolved.kind != RefKind::Model {
                continue;
            }
            let Some(model_file) = resolved.source_file else {
                continue;
            };
            let model_text = model_file.text(db);
            let Some((sql, _meta)) = resolved_model_sql_and_meta(model_text, &leaf) else {
                continue;
            };
            let project = find_project(db, workspace, model_file.project_root(db));
            let sources = model_own_source_facts(db, workspace, project, &sql);
            inputs.push(smelt_logical::analysis::output_delta::ModelDeltaInput {
                address,
                sql,
                ctx: smelt_logical::analysis::join_shape::JoinContext::new(),
                sources,
            });
            frontier.push(model_file);
        }
    }
    inputs
}

/// Every upstream maintained-model edge for `file` (`incremental_models.md`
/// §"Upstream model edges"): the model refs `file`'s own SQL makes that
/// resolve to another maintained model in this project, each carrying that
/// upstream's own validated clock and derived output-delta shape
/// (`ModelEdge::output_shape`). Its own entry point (not only inlined within
/// [`maintenance_plan_report`]) so `smelt explain`'s plan report and a
/// direct caller — a test pinning the Salsa-side derivation itself — read
/// the SAME edges rather than two independently-assembled lists. `file`
/// with no frontmatter or no `Single`-model metadata contributes no edges.
pub fn model_edges_for(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<smelt_logical::maintenance::derive::ModelEdge> {
    let text = file.text(db);
    let Ok(FileMetadata::Single { sql_offset, .. }) = extract_file_metadata(text) else {
        return Vec::new();
    };
    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    // The cross-model output-delta verdict map is folded ONCE per call (not
    // once per ref) over every model transitively reachable from `file`'s
    // own refs, then threaded into every `ref_model_edge` call so a
    // model-reference leaf inside any upstream's own SQL resolves against
    // it (`docs/outcomes/20260809-output-delta-typing/outcome.md` phase 9).
    let model_verdicts = smelt_logical::analysis::output_delta::derive_workspace_output_deltas(
        &model_delta_inputs(db, workspace, file),
    );
    refs.iter()
        .filter_map(|r| ref_model_edge(db, workspace, r, &model_verdicts))
        .collect()
}

fn ref_model_edge(
    db: &dyn salsa::Database,
    workspace: Workspace,
    ref_str: &str,
    model_verdicts: &std::collections::BTreeMap<
        String,
        smelt_logical::analysis::output_delta::OutputDeltaFacts,
    >,
) -> Option<smelt_logical::maintenance::derive::ModelEdge> {
    let stripped = ref_str.strip_prefix("smelt.")?;
    let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
    let leaf = segments.last()?.clone();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Model {
        return None;
    }
    let file = resolved.source_file?;
    let text = file.text(db);
    // Extract the addressed model's own `refresh:`/`timeseries:` plus its
    // own SQL body — the latter feeds `output_shape` below.
    let (sql, meta) = resolved_model_sql_and_meta(text, &leaf)?;
    // Only a maintained (`refresh: incremental`) upstream delivers an
    // incremental delta to receive; a `full`-mode or view upstream is
    // excluded (no creation cell, no refusal).
    if meta.refresh != Some(smelt_core::config::RefreshStrategy::Incremental) {
        return None;
    }
    let clock_col = meta.timeseries.as_ref().map(|t| t.partition_column.clone());
    // Sibling spellings of `clock_col` within the upstream's own SQL
    // (`ModelEdge::clock_col_aliases`'s doc comment) — derived from the same
    // `text` the metadata above was extracted from.
    let clock_col_aliases = clock_col
        .as_deref()
        .map(|c| smelt_logical::analysis::source_bounds::defining_expr_siblings(text, c))
        .unwrap_or_default();
    // The upstream's own declared top-level `unique_key:` (`models.md`
    // §"The Relation Contract"), threaded through so a downstream's P1
    // skeleton-source-closure proof over this edge can prove the join
    // one-to-one (T3, `docs/plans/20260715-composed-axes-conditional-
    // maintenance.md` Phase E3) — `ModelEdge::unique_key`'s doc comment.
    let unique_key = meta.unique_key.clone().unwrap_or_default();
    // The upstream's own derived output-delta shape (`ModelEdge::
    // output_shape`'s doc comment): the meet across whatever per-column-group
    // verdicts this upstream's own SQL derives — the SAME per-workspace fold
    // `smelt-runtime::propagation::upstream_output_delta_groups` computes,
    // never re-implemented differently here. `None` when the upstream
    // contributes no groups at all (e.g. an unclassifiable `SELECT *`
    // projection) rather than an optimistic guess.
    let project = find_project(db, workspace, file.project_root(db));
    let sources = model_own_source_facts(db, workspace, project, &sql);
    let declared_unique_key = meta.unique_key.clone().unwrap_or_default();
    let partition_col = meta.timeseries.as_ref().map(|t| t.partition_column.clone());
    let output_shape = own_output_delta_shape(
        &sql,
        &declared_unique_key,
        partition_col.as_deref(),
        &sources,
        model_verdicts,
    );
    Some(smelt_logical::maintenance::derive::ModelEdge {
        name: stripped.to_string(),
        clock_col,
        clock_col_aliases,
        unique_key,
        output_shape,
    })
}

/// A model's own derived output-delta shape: the meet across whatever
/// per-column-group verdicts its own SQL derives, given the cross-model
/// verdict map its own model-references should resolve against. Pure
/// (Salsa purity rule) — extracted out of [`ref_model_edge`] so
/// [`model_output_delta_for`] computes a model's own shape through the SAME
/// derivation a downstream's edge view of that model already uses; the two
/// call sites differ only in which model's SQL/sources/verdict-map they
/// pass in, never in what this function does with them.
fn own_output_delta_shape(
    sql: &str,
    unique_key: &[String],
    partition_col: Option<&str>,
    sources: &[smelt_logical::analysis::output_delta::SourceFacts],
    model_verdicts: &std::collections::BTreeMap<
        String,
        smelt_logical::analysis::output_delta::OutputDeltaFacts,
    >,
) -> Option<smelt_logical::analysis::output_delta::OutputDelta> {
    let skeleton =
        smelt_logical::maintenance::skeleton::skeleton_columns(sql, unique_key, partition_col);
    smelt_logical::analysis::output_delta::derive_output_delta_with_model_verdicts(
        sql,
        &smelt_logical::analysis::join_shape::JoinContext::new(),
        sources,
        &skeleton,
        model_verdicts,
    )
    .into_iter()
    .map(|(_, shape)| shape)
    .reduce(smelt_logical::analysis::output_delta::OutputDelta::meet)
}

/// This model's own emitted output-delta shape (`incremental_models.md`
/// §Surface "CLI" headline — the delta signature `smelt explain` prints
/// first): the SAME derivation [`ref_model_edge`] applies when some
/// downstream reports this model as an upstream edge, single-owned via
/// [`own_output_delta_shape`] so `smelt explain`'s own-model headline and a
/// downstream's edge view of this same model can never disagree
/// (`docs/outcomes/20260904-delta-signature-front-door/outcome.md` phase
/// 1). `None` for a generator/multi-model file (only a `Single`-model file
/// has one address to report a shape for), a file with no frontmatter, or a
/// model whose own SQL yields no output column groups.
pub fn model_output_delta_for(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_logical::analysis::output_delta::OutputDelta> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return None;
    };
    let sql = &text[sql_offset..];
    let unique_key = metadata.unique_key.clone().unwrap_or_default();
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let project = find_project(db, workspace, file.project_root(db));
    let sources = model_own_source_facts(db, workspace, project, sql);
    let model_verdicts = smelt_logical::analysis::output_delta::derive_workspace_output_deltas(
        &model_delta_inputs(db, workspace, file),
    );
    own_output_delta_shape(
        sql,
        &unique_key,
        partition_col.as_deref(),
        &sources,
        &model_verdicts,
    )
}

/// Per-source clamp observability (`docs/specs/incremental_shapes.md`
/// §"Observing the per-source clamp"): `file`'s own [`BoundResult`] per
/// `smelt.<path>` source it references, for editor hover. Thin Salsa
/// wrapper (Salsa purity rule) over the pure
/// `smelt_logical::analysis::source_bounds::derive_model_bounds`: resolves
/// each of `file`'s own refs to the upstream's declared
/// `timeseries.partition_column` (+ sibling spellings), mirroring
/// [`ref_model_edge`]'s pattern, builds the `BoundContext`, and calls the
/// pure derivation over `file`'s own SQL. Returns an empty map when `file`'s
/// own model is not itself partition-grain (no `timeseries:` declared) or
/// references no bounded sources — hover has nothing to show either way.
pub fn model_source_clamps(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> std::collections::BTreeMap<String, smelt_logical::BoundResult> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return Default::default();
    };
    if metadata.timeseries.is_none() {
        return Default::default();
    }
    let sql = &text[sql_offset..];
    let mut ctx = smelt_logical::BoundContext::new();
    for r in smelt_logical::collect_path_refs(sql) {
        let Some(stripped) = r.strip_prefix("smelt.") else {
            continue;
        };
        let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
        let Some(leaf) = segments.last().cloned() else {
            continue;
        };
        let Some(resolved) = resolve_ref_path(db, workspace, segments.clone()) else {
            continue;
        };
        if resolved.kind != RefKind::Model {
            continue;
        }
        let Some(upstream_file) = resolved.source_file else {
            continue;
        };
        let upstream_text = upstream_file.text(db);
        let Some((upstream_sql, upstream_meta)) = resolved_model_sql_and_meta(upstream_text, &leaf)
        else {
            continue;
        };
        let Some(ts) = upstream_meta.timeseries.as_ref() else {
            continue;
        };
        ctx.add_source(stripped, &ts.partition_column);
        let aliases = smelt_logical::analysis::source_bounds::defining_expr_siblings(
            &upstream_sql,
            &ts.partition_column,
        );
        ctx.add_source_partition_col_aliases(stripped, aliases);
    }
    if ctx.source_partition_cols.is_empty() {
        return Default::default();
    }
    smelt_logical::analysis::source_bounds::derive_model_bounds(sql, &ctx)
        .into_iter()
        .collect()
}

/// Thin Salsa wrapper around
/// `smelt_logical::maintenance::derive::derive_maintenance_plan`
/// (`incremental_models.md` §Surface "The plan (derived, reported)"): gathers
/// `file`'s referenced sources and declared `maintenance:`/`grain:`
/// frontmatter, then calls
/// [`crate::queries::maintenance::maintenance_plan_diagnostics`] (pure) to
/// derive the plan and map its admission refusals onto a Salsa-safe
/// return shape. Returns the default (empty) result for a model with no
/// maintenance plan (not `refresh: incremental`, or no frontmatter at all).
#[salsa::tracked]
pub fn maintenance_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<crate::queries::maintenance::MaintenancePlanDiagnostics> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return Arc::new(Default::default());
    };
    let resolved_grain = metadata.resolved_grain();
    if metadata.refresh != Some(smelt_core::config::RefreshStrategy::Incremental)
        || resolved_grain.is_none()
    {
        return Arc::new(Default::default());
    }
    let path = file.path(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let info = ref_source_info(db, workspace, project, r)?;
            // `SourceFacts::name` is the *bare* source name — the address
            // with the leading `sources` breadcrumb stripped
            // (`crate::maintenance::grouping` resolves a FROM alias's
            // `smelt.<path>` the same way, stripping `sources.` before
            // matching against `SourceFacts.name`; see
            // `maintenance_plan_admission.rs`'s fixtures, which name
            // sources bare — e.g. `"payments"` for `FROM
            // smelt.sources.payments`). Keeping this stripping in one place
            // (here) keeps the trigger/`scan_bounds.per_source` keys and the
            // grouping-derived `mutation_sensitivity` keys in agreement.
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let project_scan_bounds = project
        .and_then(|p| (*crate::queries::project::project_maintenance_config(db, p)).clone())
        .and_then(|m| m.scan_bounds);

    let table = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Mirrors `maintenance_plan_report`'s own composed-driving-source
    // wiring below: a `grain: key` model's driving source may be another
    // maintained model's locality-admitted composed output, not just a
    // declared `sources:` entry.
    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let extra_model_sources: Vec<(
        smelt_logical::maintenance::SourceFacts,
        smelt_core::config::Granularity,
    )> = if resolved_grain == Some(smelt_core::config::Grain::Key) {
        refs.iter()
            .filter_map(|r| {
                ref_model_source_facts(
                    db,
                    workspace,
                    r,
                    model_scan_bounds,
                    project_scan_bounds.as_ref(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    // `maintenance.cells[].write` pins are validated against every one of
    // the project's declared target backends (`write_pin_diagnostics`'s own
    // doc comment) — reuses the same `project_active_backends` query the
    // `smelt.as_struct()` backend check already threads through
    // `file_diagnostics` (`as_struct_backend_diagnostics_for_file`).
    let active_backends = project
        .and_then(|p| project_active_backends(db, p))
        .unwrap_or_default();

    // `state.warehouse_tables` (`docs/specs/state.md` §"Opting out of
    // warehouse bookkeeping") — the other availability-resolution input,
    // threaded alongside `active_backends` above. Absent/unparseable config
    // resolves to the default posture (`Allowed`), same as an absent
    // `state:` block.
    let warehouse_tables = project
        .and_then(|p| project_warehouse_tables(db, p))
        .unwrap_or_default();

    // The deployed-schema snapshot (`docs/specs/definition_deltas.md`
    // §"Detection"): a Salsa world-fact input the CLI and LSP both register
    // at workspace load (`workspace_ingest::register_deployed_schemas_from_disk`).
    // `deployed_column_names` now threads the snapshot's real column names —
    // a non-skeleton `Trigger::ColumnAdded` cell that cannot be backfilled in
    // place reports `MaintenanceColumnAddNotBackfillable` as a Warning rather
    // than blocking the plan (`definition_deltas.md` §"Detection" posture
    // rules 1-3), matching what `smelt-runtime`'s own run gate already
    // admits. A model declaring `schema_evolution: strategy: full_refresh`
    // derives no definition-change trigger at all (rule 3): the runtime
    // rebuilds the whole table, so there is no in-place backfill obligation
    // to report ahead of time — implemented here, at fact assembly, rather
    // than as a new branch inside the pure derivation.
    let deployed_schema = find_deployed_schema(db, workspace, &project_root, &table);
    let deployed_model_sql: Option<String> = deployed_schema.and_then(|s| {
        s.model_sql(db)
            .as_ref()
            .map(|sql: &Arc<str>| sql.to_string())
    });
    let deployed_partition_column: Option<String> = deployed_schema.and_then(|s| {
        s.partition_column(db)
            .as_ref()
            .map(|col: &Arc<str>| col.to_string())
    });
    let full_refresh_schema_evolution = metadata.schema_evolution.as_ref().is_some_and(|se| {
        se.strategy == smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh
    });
    let deployed_column_names: Vec<String> = if full_refresh_schema_evolution {
        Vec::new()
    } else {
        deployed_schema
            .map(|s| s.columns(db).iter().map(|c| c.to_string()).collect())
            .unwrap_or_default()
    };

    Arc::new(crate::queries::maintenance::maintenance_plan_diagnostics(
        sql_body,
        &table,
        &metadata,
        &source_refs,
        project_scan_bounds.as_ref(),
        &extra_model_sources,
        &active_backends,
        warehouse_tables,
        &deployed_column_names,
        deployed_model_sql.as_deref(),
        deployed_partition_column.as_deref(),
    ))
}

/// Plain (non-Salsa-tracked) counterpart of [`maintenance_plan`] that returns
/// the *full* derived plan — cells, clamps, locality verdicts — rather than
/// the Salsa-safe refusals-only projection. Used by `smelt explain <model>`
/// (`incremental_models.md` §Surface "CLI"), a one-shot CLI report that has no
/// need for Salsa's incremental caching and cannot use the tracked query
/// because [`smelt_logical::maintenance::MaintenancePlan`] does not implement
/// `PartialEq`/`Eq` (the Salsa tracked-return-value requirement the
/// refusals-only [`crate::queries::maintenance::MaintenancePlanDiagnostics`]
/// projection exists to satisfy instead).
///
/// Mirrors the exact input-assembly `maintenance_plan` performs above, but
/// calls [`crate::queries::maintenance::derive_model_maintenance_plan`]
/// directly. Still a Salsa-purity-respecting function: it only assembles
/// inputs from Salsa accessors and calls pure derivation code — it never
/// re-implements admission, locality, or ledger logic. Returns `None` for a
/// model with no maintenance plan (not `refresh: incremental`, or no
/// shape-defining fact declared and no `grain:` to resolve).
pub fn maintenance_plan_report(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<crate::queries::maintenance::MaintenancePlanResult> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return None;
    };
    let resolved_grain = metadata.resolved_grain();
    if metadata.refresh != Some(smelt_core::config::RefreshStrategy::Incremental)
        || resolved_grain.is_none()
    {
        return None;
    }
    let path = file.path(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let info = ref_source_info(db, workspace, project, r)?;
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    // Upstream maintained-model edges (`incremental_models.md` §"Upstream model
    // edges"): the model refs that resolve to another maintained model in
    // this project, each carrying that upstream's own validated clock and
    // derived output-delta shape.
    let model_edges = model_edges_for(db, workspace, file);

    let project_scan_bounds = project
        .and_then(|p| (*crate::queries::project::project_maintenance_config(db, p)).clone())
        .and_then(|m| m.scan_bounds);

    let table = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (mut sources, _scan_bounds_warnings) = crate::queries::maintenance::build_source_facts(
        &source_refs,
        model_scan_bounds,
        project_scan_bounds.as_ref(),
    );
    // A `grain: key` model's driving source may itself be another
    // maintained model's locality-admitted composed output, not just a
    // declared `sources:` entry — `resolve_driving_source` (consulted
    // below via `derive_model_maintenance_plan`) is already agnostic to
    // provenance, so publish every referenced upstream model that clears
    // the locality gate into the same `SourceFacts` candidate list a
    // declared source populates (`incremental_shapes.md` §"Key temporal
    // locality (the time-partitioned output)" — "The output as a clocked
    // source"). Scoped to `grain: key` models only — a `grain: partition`
    // downstream's pushdown against a composed upstream is already derived
    // through `smelt-logical`'s own model-graph registry, not this path.
    let mut model_source_granularities: Vec<smelt_core::config::Granularity> = Vec::new();
    if resolved_grain == Some(smelt_core::config::Grain::Key) {
        for r in &refs {
            if let Some((facts, granularity)) = ref_model_source_facts(
                db,
                workspace,
                r,
                model_scan_bounds,
                project_scan_bounds.as_ref(),
            ) {
                if !sources.iter().any(|s| s.name == facts.name) {
                    sources.push(facts);
                    model_source_granularities.push(granularity);
                }
            }
        }
    }
    let key_recurrences = crate::queries::maintenance::build_key_recurrences(&source_refs);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();

    // The locality gate's granularity-equality structural precondition
    // needs the driving source's granularity regardless of whether it is a
    // declared source or a composed upstream model's own output — combine
    // both candidate pools and pass the union through the single shared
    // "exactly one clocked candidate, else undecided" rule
    // (`smelt_logical::maintenance::locality::single_clocked_granularity`),
    // the same rule `single_clocked_source_granularity` applies over
    // declared sources alone.
    let mut clocked_granularities: Vec<smelt_core::config::Granularity> = source_refs
        .iter()
        .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
        .map(|t| t.granularity)
        .collect();
    clocked_granularities.extend(model_source_granularities);
    let driving_source_granularity =
        smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities);
    let source_referential_integrity =
        crate::queries::maintenance::build_source_referential_integrity(&source_refs);
    // The deployed-schema snapshot world-fact — see `maintenance_plan`'s own
    // call site for the full rationale: `deployed_column_names` threads the
    // snapshot's real column names (gated to empty under `schema_evolution:
    // strategy: full_refresh`, rule 3), and `model_sql` feeds the
    // skeleton-clause check; `smelt explain`'s report path reads the same
    // registered Salsa input `maintenance_plan` does.
    let deployed_schema = find_deployed_schema(db, workspace, &project_root, &table);
    let deployed_model_sql: Option<String> = deployed_schema.and_then(|s| {
        s.model_sql(db)
            .as_ref()
            .map(|sql: &Arc<str>| sql.to_string())
    });
    let deployed_partition_column: Option<String> = deployed_schema.and_then(|s| {
        s.partition_column(db)
            .as_ref()
            .map(|col: &Arc<str>| col.to_string())
    });
    let full_refresh_schema_evolution = metadata.schema_evolution.as_ref().is_some_and(|se| {
        se.strategy == smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh
    });
    let deployed_column_names: Vec<String> = if full_refresh_schema_evolution {
        Vec::new()
    } else {
        deployed_schema
            .map(|s| s.columns(db).iter().map(|c| c.to_string()).collect())
            .unwrap_or_default()
    };
    let mut result = crate::queries::maintenance::derive_model_maintenance_plan_with_edges(
        sql_body,
        &table,
        &metadata,
        &sources,
        &explicitly_mutable,
        &model_edges,
        driving_source_granularity,
        &key_recurrences,
        &deployed_column_names,
        &source_referential_integrity,
        deployed_model_sql.as_deref(),
        deployed_partition_column.as_deref(),
    )?;

    // Decomposed-state summary (`docs/outcomes/20260809-rung2-state-shapes`
    // row 9): only a `grain: key` model can carry state-bearing columns
    // (`rules::cumulative::classify_cumulative` is the keyed classifier),
    // and only when it actually admits — an unadmitted model contributes an
    // empty summary rather than a guess. `classify_cumulative` is the single
    // owner of which spellings are state-bearing; this call derives nothing
    // beyond assembling its inputs from the resolved `SourceInfo`s already
    // gathered above, per the Salsa purity rule.
    if metadata.is_keyed() {
        let mut source_timeseries: smelt_logical::SourceTimeseriesMap = HashMap::new();
        for r in &refs {
            if let Some(ts) = ref_timeseries_config(db, workspace, project, r) {
                source_timeseries.insert(r.clone(), ts);
            }
        }
        if let Ok(classification) = smelt_logical::classify_cumulative(
            sql_body,
            &refs,
            &source_timeseries,
            metadata.timeseries.is_some(),
            &metadata.functional_dependencies,
        ) {
            result.state_columns = smelt_logical::state_column_summary(&classification);
            result.execution_postures = Some(classification.execution_postures());
            result.is_snapshot_reconcile = Some(classification.is_snapshot_reconcile());
        }
    }

    Some(result)
}

#[salsa::tracked]
pub fn check_file_diagnostics(db: &dyn salsa::Database, workspace: Workspace, file: SourceFile) {
    let path = file.path(db);
    let text = file.text(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    // Phase 7: seed CSV without a sibling sidecar YAML emits a workspace
    // warning. We check the file extension first so non-CSV files skip the
    // disk check entirely.
    if path.extension().is_some_and(|e| e == "csv") {
        let sidecar_path = path.with_extension("yml");
        if !sidecar_path.exists() {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "Seed schema is inferred and may drift if the CSV changes — pin it"
                    .to_string(),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::MissingSeedSidecar),
                data: Some(DiagnosticData::MissingSeedSidecar {
                    csv_path: path.clone(),
                    sidecar_path: sidecar_path.clone(),
                }),
            })
            .accumulate(db);
        }
        // CSV files have no SQL content — skip all SQL-level checks.
        return;
    }

    // Generator-file frontmatter diagnostics: bridge MetadataError variants
    // that arise from `generates:` key validation into standard diagnostics.
    // These must run before parse errors so that callers see the frontmatter
    // error rather than a confusing "Expected SELECT statement" parse error.
    match extract_file_metadata(text) {
        Err(MetadataError::GeneratesUnknownValue { value, value_span }) => {
            // Anchor at the YAML value token (1-based line/col → 0-based).
            let diag_line = value_span.line.saturating_sub(1) as u32;
            let diag_col = value_span.column.saturating_sub(1) as u32;
            let li = LI::new(text);
            let start_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col,
                })
                .unwrap_or_default();
            let end_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col + value.len() as u32,
                })
                .unwrap_or(start_ts);
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("generates must be `models`; found {}", value),
                range: rowan::TextRange::new(start_ts, end_ts),
                code: Some(DiagnosticCode::GeneratesUnknownValue),
                data: None,
            })
            .accumulate(db);
            // File does not parse as SQL; no further checks make sense.
            return;
        }
        Err(MetadataError::GeneratesMixedWithBareModel { offending, span }) => {
            // Anchor at the offending key / delimiter (1-based → 0-based).
            let diag_line = span.line.saturating_sub(1) as u32;
            let diag_col = span.column.saturating_sub(1) as u32;
            let key_len = match &offending {
                MixedKind::NameField => "name:".len() as u32,
                MixedKind::SectionDelimiter => "--- name:".len() as u32,
            };
            let li = LI::new(text);
            let start_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col,
                })
                .unwrap_or_default();
            let end_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col + key_len,
                })
                .unwrap_or(start_ts);
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: "generates: models cannot coexist with bare-model identity (name field or section delimiter)".to_string(),
                range: rowan::TextRange::new(start_ts, end_ts),
                code: Some(DiagnosticCode::GeneratesMixedWithBareModel),
                data: None,
            })
            .accumulate(db);
            // File cannot be used further; bail.
            return;
        }
        Err(e) => {
            // All MetadataError variants not handled by the Generates* arms above
            // go through the exhaustive mapper. The compiler enforces that every
            // new MetadataError variant is explicitly listed there.
            if let Some(diag) = map_metadata_error_to_diagnostic(&e) {
                DiagnosticAcc(diag).accumulate(db);
            }
            // A structural metadata error means the file's model shape is unknown;
            // skip all subsequent semantic checks (refs, types, timeseries, etc.)
            // to avoid cascading noise from a file the parser couldn't classify.
            return;
        }
        Ok(FileMetadata::Generator { .. }) => {
            // Check whether the parsed generator body starts with a bare SQL
            // statement. The parse_file query routes generator files through the
            // meta-expression parser which produces a SELECT_STMT node when it
            // encounters SELECT/WITH/VALUES as the first body token.
            let parse = parse_file(db, file);
            let syntax = parse.syntax();
            // A bare SELECT is only a problem when it is a *direct* child of the
            // FILE root — that is, when the generator body itself is a top-level
            // SELECT/WITH/VALUES statement (the hand-authored model shape).
            // SELECT_STMT nodes nested inside record-literal field values (e.g.
            // `ModelDef { body: SELECT * FROM t }`) are valid TableExpr values and
            // must NOT trigger this diagnostic.
            let has_bare_sql = syntax
                .children()
                .any(|n| n.kind() == smelt_parser::SyntaxKind::SELECT_STMT);
            if has_bare_sql {
                // Find the SELECT_STMT direct child to anchor the diagnostic.
                let select_node = syntax
                    .children()
                    .find(|n| n.kind() == smelt_parser::SyntaxKind::SELECT_STMT);
                let bare_range = select_node
                    .and_then(|n| n.first_token())
                    .map(|t| t.text_range())
                    .unwrap_or(rowan::TextRange::empty(rowan::TextSize::from(0)));
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape".to_string(),
                    range: bare_range,
                    code: Some(DiagnosticCode::GenerateFileBareSelectForbidden),
                    data: None,
                })
                .accumulate(db);
            }

            // Surface diagnostics from the W2 (evaluate_generator) and W3
            // (emitted_models) pipeline for this generator file.
            //
            // W2 diagnostics include: GenerateFileBodyTypeError,
            // ModelDefDuplicateName, ModelDefInvalidName,
            // ModelDefInvalidMaterialization, GeneratorBodyForbidsModelReflection.
            //
            // W3 diagnostics include: ModelDefHandAuthoredCollision and
            // cross-generator collisions anchored at this file.
            let gen_file_path = file.path(db).to_path_buf();
            let evaluated = evaluate_generator(db, workspace, file);
            for diag in &evaluated.diagnostics {
                DiagnosticAcc(diag.clone()).accumulate(db);
            }
            // W3 collision diagnostics: each `DiscardedEmission` pairs the
            // dropped emission with its collision diagnostic in a single
            // struct, so there is no risk of the two drifting out of step
            // (`DiscardedEmission` in `crates/smelt-db/src/queries/project.rs`).
            // We emit only those where the discarded emission's
            // `generator_file` matches the current file.
            let emitted_result = emitted_models(db, workspace);
            for item in emitted_result.discarded.iter() {
                if item.emission.generator_file == gen_file_path {
                    DiagnosticAcc(item.diagnostic.clone()).accumulate(db);
                }
            }

            // W4 body diagnostics: for each surviving emission from this
            // generator file, run `emitted_model_body_analysis` and surface
            // any SQL-level diagnostics (UndeclaredColumn, ParseError,
            // CteCycle, etc.) anchored inside the generator file body.
            // Discarded emissions are naturally skipped because they are not
            // in `survivors` — their bodies are never analysed.
            for survivor in emitted_result.survivors.iter() {
                if survivor.generator_file != gen_file_path {
                    continue;
                }
                let analysis =
                    emitted_model_body_analysis(db, workspace, file, survivor.name.clone());
                for diag in analysis.diagnostics.iter() {
                    DiagnosticAcc(diag.clone()).accumulate(db);
                }
            }

            // Loader call diagnostics for generator files: smelt.config.load_yaml /
            // load_json calls in the generator body must also be validated (path
            // literals, schema arguments, content, and per-target overlay validation).
            // These diagnostics are emitted here (before the early return) so that
            // generator files surface the same loader-call diagnostics as regular
            // model files.  BUG-014 P4: this is the seam that surfaces overlay
            // validation errors (`ConfigLoaderUnknownField` etc.) for generator files.
            for diag in
                crate::queries::loader::loader_call_diagnostics_for_file(db, workspace, file)
            {
                DiagnosticAcc(diag).accumulate(db);
            }

            // Generator files are not SQL models; skip the model-validity check
            // and all SQL-only diagnostics.
            return;
        }
        _ => {
            // Non-generator file: continue with the standard parse-error pipeline.
        }
    }

    // Model frontmatter diagnostics via the unified catalogue (U3).
    // Skips smelt.define / smelt.extern function files — their frontmatter is
    // handled (with the correct DeclarationKind) by
    // frontmatter_parse_diagnostics_for_file. Only pure SQL model files reach
    // this block. Calls parse_frontmatter(text, Model/Check) to surface unknown-key
    // errors and inapplicable-key warnings. Also tries to deserialize
    // ModelMetadata from the validated map to catch nested sub-field failures
    // (e.g. a bad timeseries.granularity value) that would previously be swallowed.
    let (is_function_file, is_check_file) = {
        let p = parse_file(db, file);
        let ast_opt = AstFile::cast(p.syntax());
        let is_fn = ast_opt
            .as_ref()
            .map(|ast| ast.defines().next().is_some() || ast.externs().next().is_some())
            .unwrap_or(false);
        let is_chk = ast_opt
            .as_ref()
            .map(|ast| ast.checks().next().is_some())
            .unwrap_or(false);
        (is_fn, is_chk)
    };
    if !is_function_file {
        if let Some(yaml_text) = smelt_core::frontmatter_yaml_text(text) {
            use smelt_core::{FrontmatterSeverity, ModelMetadata};
            let decl_kind = if is_check_file {
                smelt_core::DeclarationKind::Check
            } else {
                smelt_core::DeclarationKind::Model
            };
            let (validated_map, fm_diags) = smelt_core::parse_frontmatter(&yaml_text, decl_kind);

            // Emit catalogue diagnostics (unknown key → Error, inapplicable → Warning).
            for fm_diag in &fm_diags {
                let severity = match fm_diag.severity {
                    FrontmatterSeverity::Error => DiagnosticSeverity::Error,
                    FrontmatterSeverity::Warning => DiagnosticSeverity::Warning,
                };
                DiagnosticAcc(Diagnostic {
                    severity,
                    message: fm_diag.message.clone(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::FrontmatterParseError),
                    data: None,
                })
                .accumulate(db);
            }

            // Try to deserialize ModelMetadata from the validated map to catch
            // nested sub-field failures (e.g. timeseries.granularity: fortnight).
            // A failure here means a nested field is malformed — surface as
            // MalformedTimeseries.
            if !validated_map.is_empty() {
                if let Err(serde_err) = serde_yaml::from_value::<ModelMetadata>(
                    serde_yaml::Value::Mapping(validated_map),
                ) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("MalformedTimeseries: {serde_err}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::MalformedTimeseries),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Timeseries / incremental frontmatter validation.
    // Runs on every non-CSV, non-generator file that has Single frontmatter.
    // Calls the pure `validate_timeseries` function from smelt-core and maps
    // its errors into DiagnosticAcc entries so they surface through
    // `file_diagnostics`.
    if let Ok(FileMetadata::Single {
        ref metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    {
        let sql_body = &text[sql_offset..];
        if let Err(ts_err) = smelt_core::metadata::validate_timeseries(metadata, sql_body) {
            let maybe_diag = match &ts_err {
                smelt_core::metadata::MetadataError::TimeseriesRequiredForPartitionGrain => Some((
                    ts_err.to_string(),
                    DiagnosticCode::TimeseriesRequiredForPartitionGrain,
                )),
                smelt_core::metadata::MetadataError::MalformedTimeseries { .. } => {
                    Some((ts_err.to_string(), DiagnosticCode::MalformedTimeseries))
                }
                smelt_core::metadata::MetadataError::PlausibleContractOnSkeletonColumn {
                    ..
                } => Some((
                    ts_err.to_string(),
                    DiagnosticCode::PlausibleContractOnSkeletonColumn,
                )),
                // `validate_timeseries` no longer raises this — whether
                // keyed+timeseries: is admitted is decided by the locality
                // gate in plan derivation
                // (`smelt_logical::maintenance::locality::establish_locality`),
                // which surfaces its own `KeyedForbidsTimeseries` diagnostic
                // from the maintenance-plan fold-in below. The arm is kept
                // (rather than folded into the `_ => None` wildcard) so the
                // `MetadataError` variant's diagnostic mapping stays
                // documented at its point of historical use.
                smelt_core::metadata::MetadataError::KeyedForbidsTimeseries => None,
                // `batched:` without `refresh: batched` maps to the generic
                // YamlParseError code — no dedicated code exists yet. This
                // is also the only remaining way a `grain: key` model can
                // still carry an internally-folded `batched` block — the
                // literal sub-block is refused before a `ModelMetadata`
                // exists, so the dedicated `KeyedForbidsPartitionGrain` code was
                // retired outright (`docs/specs/diagnostics.md` §"Keyed
                // refresh mode").
                smelt_core::metadata::MetadataError::PartitionGrainRequiresRefreshIncremental => {
                    Some((ts_err.to_string(), DiagnosticCode::YamlParseError))
                }
                smelt_core::metadata::MetadataError::KeyedForbidsSafetyOverrides => Some((
                    ts_err.to_string(),
                    DiagnosticCode::KeyedForbidsSafetyOverrides,
                )),
                smelt_core::metadata::MetadataError::MaterializedViewForbidsTimeseries => Some((
                    ts_err.to_string(),
                    DiagnosticCode::MaterializedViewForbidsTimeseries,
                )),
                smelt_core::metadata::MetadataError::MaterializedViewForbidsPartitionGrain => {
                    Some((
                        ts_err.to_string(),
                        DiagnosticCode::MaterializedViewForbidsPartitionGrain,
                    ))
                }
                smelt_core::metadata::MetadataError::GrainRequiredForIncremental => Some((
                    ts_err.to_string(),
                    DiagnosticCode::GrainRequiredForIncremental,
                )),
                smelt_core::metadata::MetadataError::GrainRequiresIncremental => {
                    Some((ts_err.to_string(), DiagnosticCode::GrainRequiresIncremental))
                }
                smelt_core::metadata::MetadataError::GrainAssertionMismatch { .. } => {
                    Some((ts_err.to_string(), DiagnosticCode::GrainAssertionMismatch))
                }
                // Other MetadataError variants are already handled by the generates-key
                // block above or by serde_yaml at parse time; skip them here.
                _ => None,
            };
            if let Some((message, code)) = maybe_diag {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message,
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(code),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Functional-dependency (`key -> determines`) declaration structural
        // validation (DC2, `model_properties.md` §"Model-scoped declarations").
        if let Err(fd_err) =
            smelt_core::metadata::validate_functional_dependencies(metadata, sql_body)
        {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: fd_err.to_string(),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::MalformedFunctionalDependency),
                data: None,
            })
            .accumulate(db);
        }

        // Bounded-domain / space-budget declaration structural validation
        // (DC3, `model_properties.md` §"Model-scoped declarations").
        if let Err(bd_err) = smelt_core::metadata::validate_bounded_domains(metadata, sql_body) {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: bd_err.to_string(),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::MalformedBoundedDomain),
                data: None,
            })
            .accumulate(db);
        }

        // Contract-lattice `frozen_horizon` grain-admissibility check
        // (`docs/specs/incremental_models.md` §"Contract relaxations
        // (`contract:`)"). Format validity was already checked at
        // frontmatter-parse time (`MetadataError::ContractFrozenHorizonInvalid`,
        // handled above); this pure `smelt-logical` validator only checks that
        // the declaration sits on a partition-grain model, sharing the same
        // diagnostic code (single-owner rule: the oracle/validator, not this
        // Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if contract.frozen_horizon.is_some() {
                let grain = metadata.grain.unwrap_or(smelt_core::config::Grain::Key);
                if let Err(why) = smelt_logical::validate_frozen_horizon(grain) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("ContractFrozenHorizonInvalid: {why}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::ContractFrozenHorizonInvalid),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }

        // Contract-lattice `frozen_horizon` driving-source posture check
        // (`docs/specs/incremental_models.md` §"The contract lattice":
        // declaring `frozen_horizon` on a model whose driving source has any
        // other *declared* mutation profile is refused, since the late-
        // arrival probe's row-count comparison is blind under any posture
        // other than `append_only`). Resolves the model's driving relation
        // from the FROM clause's first entry, the same parse pattern
        // `smelt_logical::maintenance::locality::resolve_driving_source`
        // uses, and shares the same diagnostic code (single-owner rule: the
        // oracle/validator, not this Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if contract.frozen_horizon.is_some() {
                let driving_source =
                    smelt_parser::File::cast(smelt_parser::parse(sql_body).syntax())
                        .and_then(|f| f.select_stmt())
                        .and_then(|s| s.from_clause())
                        .map(|fc| {
                            smelt_logical::analysis::source_bounds::from_clause_alias_sources(&fc)
                        })
                        .and_then(|sources| sources.into_iter().next());
                if let Some((_, source_name)) = driving_source {
                    let profile =
                        ref_source_info(db, workspace, project, &format!("smelt.{source_name}"))
                            .and_then(|info| info.mutation_profile.map(|m| m.kind));
                    if let Err(why) =
                        smelt_logical::validate_frozen_horizon_posture(&source_name, profile)
                    {
                        DiagnosticAcc(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!("ContractFrozenHorizonInvalid: {why}"),
                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                            code: Some(DiagnosticCode::ContractFrozenHorizonInvalid),
                            data: None,
                        })
                        .accumulate(db);
                    }
                }
            }
        }

        // Contract-lattice `deferral` clock-admissibility check
        // (`docs/specs/incremental_models.md` §"Contract relaxations
        // (`contract:`)"). Format validity was already checked at
        // frontmatter-parse time (`MetadataError::ContractDeferralInvalid`,
        // handled above); this check resolves whether the declaration has an
        // interval-representable clock to measure lag against — the model's
        // own `timeseries:` clock for a model-level `deferral`, or the
        // resolved source behind a `cells[].on` trigger for a cell-level
        // one — and shares the same diagnostic code (single-owner rule: the
        // oracle/validator, not this Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if contract.deferral.is_some() {
                let model_name = metadata.name.as_deref().unwrap_or("<unnamed>");
                if let Err(why) = smelt_logical::validate_deferral(
                    metadata.timeseries.is_some(),
                    &format!("model '{model_name}'"),
                ) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("ContractDeferralInvalid: {why}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::ContractDeferralInvalid),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
            if contract.cells.iter().any(|c| c.deferral.is_some()) {
                let refs = smelt_logical::collect_path_refs(sql_body);
                for cell in contract.cells.iter().filter(|c| c.deferral.is_some()) {
                    let has_clock = cell.on != "backfill"
                        && refs
                            .iter()
                            .filter_map(|r| {
                                let stripped = r.strip_prefix("smelt.")?;
                                let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
                                if bare != cell.on {
                                    return None;
                                }
                                ref_source_info(db, workspace, project, r)
                            })
                            .next()
                            .is_some_and(|info| {
                                info.timeseries.is_some()
                                    && info.mutation_profile.as_ref().is_some_and(|m| {
                                        m.kind == smelt_core::sources::MutationProfile::AppendOnly
                                    })
                            });
                    if let Err(why) = smelt_logical::validate_deferral(
                        has_clock,
                        &format!("cell on '{}'", cell.on),
                    ) {
                        DiagnosticAcc(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!("ContractDeferralInvalid: {why}"),
                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                            code: Some(DiagnosticCode::ContractDeferralInvalid),
                            data: None,
                        })
                        .accumulate(db);
                    }
                }
            }
        }

        // Contract-lattice `retain_departed` posture-admissibility +
        // tombstone-column check (`docs/specs/incremental_models.md`
        // §"Contract relaxations (`contract:`)"). Format validity was
        // already checked at frontmatter-parse time
        // (`MetadataError::ContractRetainDepartedInvalid`, handled above);
        // this pure `smelt-logical` validator checks that the declaration
        // sits on a keyed shape consuming a mutable snapshot, and that a
        // declared tombstone column exists in the model's inferred output —
        // sharing the same diagnostic code (single-owner rule: the
        // oracle/validator, not this Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if let Some(retain_departed) = &contract.retain_departed {
                let model_name = metadata.name.as_deref().unwrap_or("<unnamed>");
                let grain = metadata.grain.unwrap_or(smelt_core::config::Grain::Key);
                let refs = smelt_logical::collect_path_refs(sql_body);
                let consumes_mutable_snapshot = refs.iter().any(|r| {
                    ref_source_info(db, workspace, project, r).is_some_and(|info| {
                        info.mutation_profile.as_ref().is_some_and(|m| {
                            m.kind == smelt_core::sources::MutationProfile::Mutable
                        })
                    })
                });
                let tombstone_column = match retain_departed {
                    smelt_core::config::RetainDeparted::Bool(_) => None,
                    smelt_core::config::RetainDeparted::Tombstone { tombstone } => {
                        Some(tombstone.as_str())
                    }
                };
                let typed_schema = typed_model_schema(db, workspace, file);
                let output_columns: Vec<String> = typed_schema
                    .columns
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                if let Err(why) = smelt_logical::validate_retain_departed(
                    grain,
                    consumes_mutable_snapshot,
                    tombstone_column,
                    &output_columns,
                    model_name,
                ) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("ContractRetainDepartedInvalid: {why}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::ContractRetainDepartedInvalid),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }

        // Declarative column test validation (`docs/specs/data_tests.md`
        // §"Fail-loud validation"). Two checks, run only when at least one
        // column declares a non-empty `tests` list:
        //   1. `UnknownColumnTestKind` — pure, from `validate_column_tests`.
        //   2. `ColumnTestOnUnknownColumn` — needs the inferred output
        //      schema, so it is made here (not in `smelt-core`) via
        //      `typed_model_schema`.
        if metadata.columns.values().any(|c| !c.tests.is_empty()) {
            if let Err(kind_err) = smelt_core::metadata::validate_column_tests(metadata) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: kind_err.to_string(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::UnknownColumnTestKind),
                    data: None,
                })
                .accumulate(db);
            }

            let model_name = metadata.name.as_deref().unwrap_or("<unnamed>");
            let typed_schema = typed_model_schema(db, workspace, file);
            let schema_columns: Vec<String> = typed_schema
                .columns
                .iter()
                .map(|c| c.name.clone())
                .collect();
            if let Err(col_err) = smelt_core::metadata::validate_column_tests_against_schema(
                metadata,
                model_name,
                &schema_columns,
            ) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: col_err.to_string(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::ColumnTestOnUnknownColumn),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Timeseries schema invariants (D-52 rules 7 and 8).
        if let Some(ts) = metadata.timeseries.as_ref() {
            let typed_schema = typed_model_schema(db, workspace, file);
            // Rule 7: partition_column and event_time_column must be NOT NULL.
            for diag in queries::check_types::check_timeseries_nullability(ts, &typed_schema) {
                DiagnosticAcc(diag).accumulate(db);
            }
            // Rule 8: sub-day granularity (hour) requires a timestamp-resolution
            // partition_column type (not DATE).
            for diag in queries::check_types::check_timeseries_granularity_type(ts, &typed_schema) {
                DiagnosticAcc(diag).accumulate(db);
            }
        }

        // State posture widening check (D-47): a model may narrow the project's
        // state.mode but not widen it.
        if let Some(model_state) = metadata.state.as_ref() {
            let project_mode = project
                .map(|p| crate::queries::project::project_state_mode(db, p))
                .unwrap_or_default();
            if !project_mode.can_narrow_to(&model_state.mode) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "model declares state.mode {} but project posture is {}; \
                         models may narrow but not widen the project posture",
                        model_state.mode.as_str(),
                        project_mode.as_str(),
                    ),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::StateModeWidening),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Built-in planner-rule diagnostics (keyed classifier, incremental
        // batch-safety) surfaced through the uniform rule → diagnostics
        // interface. The checks live in `smelt-planner` (analysis-pure); this
        // query only gathers inputs and aggregates, so the editor and the build
        // reach an identical verdict (architecture.md §"Diagnostic parity rule"
        // + §"Planner scope"). Anchored at the model SQL body start.
        // Route keyed detection through is_keyed() (`refresh: incremental` +
        // `grain: key`) and partition-grain detection through
        // is_partition_grain() (`refresh: incremental` + `grain: partition`
        // — the opt-in, independent of whether the optional `batched:` block
        // is present) so both reach the classifier. The strings below are the
        // classifier's internal keys for each rule, not user surface values.
        let materialization = if metadata.is_keyed() {
            "cumulative_aggregate"
        } else if metadata.is_partition_grain() {
            "incremental"
        } else {
            ""
        };
        if !materialization.is_empty() {
            let stripped = smelt_parser::strip_frontmatter(text);
            let refs = smelt_logical::collect_path_refs(&stripped);
            // The keyed classifier resolves its driving source by looking
            // each ref up in this map. The incremental rule's UNION-ALL
            // injectability check (`rule_diagnostics::check_union_all_injectable`)
            // also needs it — it builds the same per-ref `BoundContext` the
            // pushdown-scoping walk (`rules::incremental::derive_model_source_bounds`)
            // builds from `RuleContext.refs`/`source_timeseries`, so both rules
            // populate this map for every ref regardless of materialization.
            let mut source_timeseries: smelt_logical::SourceTimeseriesMap = HashMap::new();
            for r in &refs {
                if let Some(ts) = ref_timeseries_config(db, workspace, project, r) {
                    source_timeseries.insert(r.clone(), ts);
                }
            }
            let model_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // The opt-in is `refresh: batched`, not the presence of the optional
            // `batched:` block — default to an empty config when the block is
            // absent so a bare `refresh: batched` model still reaches the rule.
            let default_batched_config = smelt_core::config::PartitionGrainConfig::default();
            let plausible_columns: std::collections::BTreeSet<String> = metadata
                .columns
                .iter()
                .filter(|(_, c)| c.contract == Some(smelt_core::metadata::Contract::Plausible))
                .map(|(name, _)| name.clone())
                .collect();
            let ctx = smelt_logical::RuleContext {
                model_name: &model_name,
                materialization,
                sql: &stripped,
                refs: &refs,
                source_timeseries: &source_timeseries,
                timeseries_config: metadata.timeseries.as_ref(),
                incremental_config: if materialization == "incremental" {
                    Some(metadata.batched.as_ref().unwrap_or(&default_batched_config))
                } else {
                    None
                },
                declared_functional_dependencies: &metadata.functional_dependencies,
                plausible_columns: &plausible_columns,
            };
            let body_start = rowan::TextSize::from(sql_offset as u32);
            for rd in smelt_logical::detect_builtin_rules(&ctx) {
                DiagnosticAcc(Diagnostic {
                    severity: match rd.severity {
                        smelt_logical::RuleSeverity::Error => DiagnosticSeverity::Error,
                        smelt_logical::RuleSeverity::Warning => DiagnosticSeverity::Warning,
                    },
                    message: rd.message,
                    range: rowan::TextRange::empty(body_start),
                    code: Some(rule_diagnostic_code(rd.code)),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Maintenance-plan diagnostics (`incremental_models.md` §Diagnostics):
        // fold the derived plan's admission refusals and the
        // `maintenance.cells[]` column-group-span check onto the
        // `Maintenance*` codes. `maintenance_plan` is the thin Salsa query —
        // this block only maps its (already-derived) result onto
        // diagnostics, never re-derives the plan itself.
        let plan_diags = maintenance_plan(db, workspace, file);
        let body_start = rowan::TextSize::from(sql_offset as u32);
        for refusal in &plan_diags.refusals {
            let (severity, code, message) = match refusal {
                crate::queries::maintenance::MaintenanceRefusal::ScanUnbounded { source, why } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::MaintenanceScanUnbounded,
                    format!("maintenance scan over '{source}' cannot be partition-bounded: {why}"),
                ),
                crate::queries::maintenance::MaintenanceRefusal::NoAdmissibleTechnique {
                    trigger,
                    why,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::MaintenanceNoAdmissibleTechnique,
                    format!("no maintenance technique admits trigger {trigger}: {why}"),
                ),
                crate::queries::maintenance::MaintenanceRefusal::LocalityNotEstablished {
                    message,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::KeyedForbidsTimeseries,
                    message.clone(),
                ),
                crate::queries::maintenance::MaintenanceRefusal::KeyedRecurrenceDeclarationMismatch {
                    message,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::KeyedRecurrenceDeclarationMismatch,
                    message.clone(),
                ),
                crate::queries::maintenance::MaintenanceRefusal::IdentityNotDerivable {
                    message,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::GrainAssertionMismatch,
                    message.clone(),
                ),
                crate::queries::maintenance::MaintenanceRefusal::SkeletonChanged { column } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::MaintenanceSkeletonChanged,
                    format!(
                        "column '{column}' occupies a row-membership/identity (skeleton) \
                         position — a grain change, never a column backfill (EX-39, \
                         docs/specs/incremental_models.md §\"The definition-change trigger\")",
                    ),
                ),
                crate::queries::maintenance::MaintenanceRefusal::SkeletonClauseChanged {
                    reason,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::MaintenanceSkeletonChanged,
                    format!(
                        "the model's skeleton clause changed against its deployed schema \
                         snapshot: {reason} — a grain change, never a column backfill (EX-39, \
                         docs/specs/incremental_models.md §\"The definition-change trigger\")",
                    ),
                ),
                crate::queries::maintenance::MaintenanceRefusal::PartitionColumnChanged {
                    from,
                    to,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::MaintenancePartitionColumnChanged,
                    format!(
                        "declared timeseries.partition_column changed from '{from}' to '{to}' \
                         since this model was last deployed — the recorded address every \
                         partition-grain maintenance write targets no longer matches; this is a \
                         pre-execution refusal that no run flag bypasses (the analyzer gate \
                         blocks on any Error-severity diagnostic unconditionally), so delete the \
                         model's recorded snapshot (.smelt/targets/<target>/schemas/<model>.json) \
                         and re-run `smelt run` to re-address the table under the new column",
                    ),
                ),
                crate::queries::maintenance::MaintenanceRefusal::UnsupportedGrain {
                    grain,
                    tracking_plan,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::MaintenanceUnsupportedGrain,
                    format!(
                        "grain: {grain} is not yet supported by maintenance-plan derivation \
                         (tracked in {tracking_plan}); declare a supported grain \
                         (partition or key) or use refresh: full",
                    ),
                ),
                crate::queries::maintenance::MaintenanceRefusal::DefinitionChangeNotBackfillable {
                    columns,
                    why,
                } => (
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::MaintenanceColumnAddNotBackfillable,
                    format!(
                        "added column(s) {} cannot be backfilled in place: {why} — the run will \
                         ALTER them in and leave historical rows NULL until `smelt migrate` \
                         backfills them",
                        columns.join(", "),
                    ),
                ),
                crate::queries::maintenance::MaintenanceRefusal::KeyedRetractableContribution {
                    source,
                    columns,
                    why,
                } => (
                    DiagnosticSeverity::Error,
                    DiagnosticCode::KeyedRetractableContribution,
                    format!(
                        "enrichment join against '{source}' feeds a retractable contribution to \
                         column(s) {}: {why} — use `refresh: materialized_view`, or compose the \
                         enrichment as a separate model",
                        columns.join(", "),
                    ),
                ),
            };
            DiagnosticAcc(Diagnostic {
                severity,
                message,
                range: rowan::TextRange::empty(body_start),
                code: Some(code),
                data: None,
            })
            .accumulate(db);
        }
        for violation in &plan_diags.cell_column_group_violations {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: violation.clone(),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique),
                data: None,
            })
            .accumulate(db);
        }
        for source in &plan_diags.scan_bounds_warnings {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "maintenance scan over '{source}' cannot be partition-bounded — admitted \
                     under scan_bounds.on_violation: warn"
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceScanUnbounded),
                data: None,
            })
            .accumulate(db);
        }
        for write_refusal in &plan_diags.write_pin_refusals {
            let (code, message) = match write_refusal {
                crate::queries::maintenance::WritePinDiagnostic::PatternUnavailable {
                    pattern,
                    backend,
                } => (
                    DiagnosticCode::MaintenanceWritePatternUnavailable,
                    format!(
                        "MaintenanceWritePatternUnavailable: write pattern '{pattern}' is \
                         unrecognised, or backend '{backend}' cannot provide it"
                    ),
                ),
                crate::queries::maintenance::WritePinDiagnostic::AddressingRefused {
                    cell,
                    pattern,
                    why,
                } => (
                    DiagnosticCode::MaintenanceWriteAddressingRefused,
                    format!(
                        "MaintenanceWriteAddressingRefused: write pattern '{pattern}' cannot \
                         uphold the equivalence invariant for cell {cell} — {why}"
                    ),
                ),
            };
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message,
                range: rowan::TextRange::empty(body_start),
                code: Some(code),
                data: None,
            })
            .accumulate(db);
        }
        for downgrade in &plan_diags.state_downgrades {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "MaintenanceStateDowngraded: cell {} downgraded from {} to its \
                     recompute-family equivalent — {}",
                    downgrade.cell, downgrade.original_technique, downgrade.reason
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceStateDowngraded),
                data: None,
            })
            .accumulate(db);
        }
        for refusal in &plan_diags.contract_state_refusals {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "DeclaredContractRequiresState: {} requires the {}, which is unavailable \
                     on backend '{}'",
                    refusal.declaration, refusal.missing_structure, refusal.backend
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::DeclaredContractRequiresState),
                data: None,
            })
            .accumulate(db);
        }
        if let Some(mismatch) = &plan_diags.granularity_mismatch {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "declared timeseries.granularity ({}) is contradicted by the model's own \
                     partition-column grouping, which derives to {}",
                    granularity_lower(mismatch.declared),
                    granularity_lower(mismatch.actual),
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceGranularityMismatch),
                data: None,
            })
            .accumulate(db);
        }
    }

    // Parse errors
    let parse = parse_file(db, file);
    for error in parse.errors.iter() {
        let range = error.range;
        // Remap pipe-operator parse errors to their proper diagnostic codes so
        // consumers can distinguish them from generic syntax errors.
        let code = remap_pipe_parse_error_code(&error.message);
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            range,
            code: Some(code),
            data: None,
        })
        .accumulate(db);
    }

    // Duplicate-function diagnostics (Phase 3): emitted at the second
    // `smelt.define` declaration's name span; workspace-wide check.
    for diag in duplicate_function_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Invalid-type-ref diagnostics (Phase 4): emitted at each malformed
    // `Expr<T>` / unsupported-sort annotation on parameters or return types.
    for diag in invalid_function_type_ref_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Struct-field-unknown diagnostics (hardening Phase 3): emitted at each
    // struct field whose type text is not a recognised concrete DataType.
    for diag in struct_field_type_unknown_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // BUG-003: Semantics #9 — default must not reference sibling parameters.
    for diag in default_references_parameter_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Unknown-context diagnostics (Phase 19): emitted when `Expr<T, ctx>`
    // context name doesn't resolve to any parameter in the same function.
    for diag in unknown_context_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // CTE cycle diagnostics (Phase 20): emitted when a function body's WITH
    // clause contains a cyclic CTE reference.
    for diag in cte_cycle_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // BUG-007: CTE-collision diagnostics — emitted when a model's top-level
    // CTE name collides with a CTE declared in the body of a directly-called
    // transparent function (CteShadowsCallerCte, Error).
    for diag in cte_shadow_caller_cte_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Context mismatch diagnostics (Phase 20): emitted when an explicit
    // Expr<T, ctx> annotation disagrees with the inferred splice-point context.
    for diag in context_mismatch_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Function body diagnostics (Phase 5): duplicate param names, unknown
    // identifiers inside a body, and body-level type mismatches. Emitted
    // regardless of whether the file contains a SELECT statement — pure
    // function files (functions/*.sql with no model) still surface body
    // diagnostics.
    for diag in function_body_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Call-site expansion diagnostics (Phase 6): unknown/missing/type-
    // mismatched args on `smelt.fn.*` calls, plus any body-cascaded errors
    // re-anchored to the call site. Runs before the `parse_model.is_none()`
    // early-return so call sites in non-model files also surface.
    for diag in smelt_fn_call_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 11 — backends widening / malformed frontmatter.
    for diag in backends_widening_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 43 — frontmatter parse-error / unknown-key diagnostics.
    // Fires unconditionally so workspaces with `unstable_schema: true` still
    // surface malformed YAML and unknown-key warnings on `smelt.define` /
    // `smelt.extern` declarations.
    for diag in frontmatter_parse_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 31 — provenance: unstable-schema gate.
    let unstable_schema = project
        .map(|p| project_unstable_schema(db, p))
        .unwrap_or(false);
    for diag in provenance_unstable_diagnostics_for_file(db, file, unstable_schema) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 51 — provenance/joins validator (only when unstable_schema: true).
    if unstable_schema {
        for diag in provenance_validator::provenance_validator_diagnostics_for_file(db, file) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase 52 — extern fragment-param rejection (fires unconditionally).
    for diag in extern_fragment_param_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 52 — missing-provenance pushdown advisory (Hint severity,
    // only when unstable_schema: true).
    if unstable_schema {
        for diag in missing_provenance_advisory_for_file(db, workspace, file) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase 38 / Phase 42 — smelt.as_struct() backend-capability gate.
    // Functions with explicit `backends:` are checked against that set;
    // functions without (default `BackendSet::All`) are checked against
    // the workspace's active backends from `smelt.yml`.
    let active_backends = project.and_then(|p| project_active_backends(db, p));
    for diag in as_struct_backend_diagnostics_for_file(db, file, active_backends.as_deref()) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 41 — transparent-function call-graph cycle pre-pass.
    for diag in function_call_cycle_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase B (meta-language) — smelt.config.var diagnostic wiring.
    //
    // Walk all SMELT_PATH_CALL nodes in the file for `smelt.config.var(...)` calls.
    // Emits: ConfigVarNameNotLiteral, ConfigVarNotFound, ConfigVarNullCoercion.
    // Requires the project-level vars map so this lives in check_file_diagnostics
    // (not check_type_diagnostics) where the project context is available.
    {
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        let vars_map = project
            .map(|p| smelt_yml_vars_query(db, p))
            .unwrap_or_default();
        for diag in type_inference::check_config_var_call_diagnostics(&syntax, &vars_map) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase E1 Phase 5: smelt.config.load_yaml / load_json / load_toml call diagnostics.
    //
    // Validates path literals, schema arguments, and file existence.
    // Runs unconditionally (before the early return on parse failure) so that
    // config-var files also surface loader diagnostics.
    {
        for diag in loader_call_diagnostics_for_file(db, workspace, file) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase B (meta-language) Phase 3: smelt.define name-shadowing.
    //
    // Check each smelt.define declaration for names that shadow built-in
    // HOFs (map, filter, reduce) or reducers (comma_sep, and_all, …).
    // Fires unconditionally so function-only files (functions/*.sql with no
    // SELECT statement) also surface these diagnostics before the early return.
    // Emits HofNameShadowed or ReducerNameShadowed at the name token.
    {
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        if let Some(ast) = AstFile::cast(syntax) {
            for define in ast.defines() {
                for diag in type_inference::check_define_name_shadowing(&define) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }
    }

    // Phase E2 — ModelDefOutsideGeneratorFile: scan for ModelDef record literals
    // in non-generator files. A `ModelDef { name: '…', body: … }` construct is
    // only valid inside a generator file (generates: models); using it in a
    // regular SQL model file is an error.
    {
        use smelt_parser::ast::RecordLiteral;
        use smelt_parser::SyntaxKind::{IDENT, RECORD_LITERAL};
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        let mut ctx = type_inference::TypeContext::new();
        ctx.is_inside_generator_file = false; // non-generator file
        for node in syntax.descendants().filter(|n| n.kind() == RECORD_LITERAL) {
            if let Some(lit) = RecordLiteral::cast(node) {
                // Only check record literals whose leading token is the identifier
                // "ModelDef". In the CST, a named record literal `TypeName { … }`
                // has the type-name IDENT as its first token.
                let leading_name = lit
                    .syntax()
                    .children_with_tokens()
                    .find_map(|e| {
                        let tok = e.into_token()?;
                        if tok.kind() == IDENT {
                            Some(tok.text().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                if leading_name != "ModelDef" {
                    continue;
                }
                let result = type_inference::infer_model_def_literal(&lit, &ctx);
                for sentinel in result.sentinels {
                    if sentinel.code == DiagnosticCode::ModelDefOutsideGeneratorFile {
                        let range = sentinel.span;
                        DiagnosticAcc(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: sentinel.message,
                            range,
                            code: Some(sentinel.code),
                            data: None,
                        })
                        .accumulate(db);
                    }
                }
            }
        }
    }

    // VALUES / CTE alias-column arity checks.
    //
    // Walk all TABLE_REF and CTE nodes in the file's CST and emit:
    //   - `AliasColumnArityMismatch` when the alias column list length does not
    //     match the underlying relation's column count.
    //   - `EmptyValuesClause` when a VALUES derived table has zero rows.
    //
    // These are pure structural checks that do not require schema resolution.
    // They run unconditionally (before the `parse_model.is_none()` early-return)
    // so that function files also surface them.
    {
        use smelt_parser::ast::{Cte, TableRef};
        use smelt_parser::SyntaxKind::{CTE, TABLE_REF};
        let parse = parse_file(db, file);
        let syntax = parse.syntax();

        // VALUES derived-table checks: scan all TABLE_REF nodes.
        for node in syntax.descendants().filter(|n| n.kind() == TABLE_REF) {
            if let Some(tr) = TableRef::cast(node) {
                for diag in type_inference::check_table_ref_values_arity(&tr) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }

        // CTE alias-list checks: scan all CTE nodes.
        for node in syntax.descendants().filter(|n| n.kind() == CTE) {
            if let Some(cte) = Cte::cast(node) {
                for diag in type_inference::check_cte_alias_arity(&cte) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }
    }

    // Phase 4 (testing): `#` CTE-reference outside smelt.test.
    //
    // A `smelt.<path>#<cte>` reference is only valid inside a `smelt.test`
    // body.  Walk all SMELT_PATH_REF nodes in the CST and emit
    // `CteRefOutsideTest` for any that carry a `#` suffix but are not
    // inside a SMELT_TEST ancestor.  This is a pure structural check that
    // runs unconditionally (no early-return) so model files, function files,
    // check files, and test files all surface it correctly.
    {
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        for diag in cte_ref_outside_test_diagnostics(&syntax) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // `smelt.check` structural validation: PASSING and EXPECT clauses are
    // test-only surface. A check body is a failing-rows query against real
    // built data; it has no mock tables and no expected output rows. Emit
    // `CheckHasTestClause` anchored at the offending clause keyword range.
    {
        let parse = parse_file(db, file);
        if let Some(ast) = AstFile::cast(parse.syntax()) {
            for check in ast.checks() {
                for passing in check.passing_clauses() {
                    let range = passing.syntax().text_range();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "PASSING clauses are not valid on smelt.check — \
                                  only smelt.test declarations accept mock table data"
                            .to_string(),
                        range,
                        code: Some(DiagnosticCode::CheckHasTestClause),
                        data: None,
                    })
                    .accumulate(db);
                }
                if let Some(expect) = check.expect_clause() {
                    let range = expect.syntax().text_range();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "EXPECT clause is not valid on smelt.check — \
                                  only smelt.test declarations assert against expected output rows"
                            .to_string(),
                        range,
                        code: Some(DiagnosticCode::CheckHasTestClause),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Check if model is valid
    if parse_model(db, file).is_none() {
        let path_str = path.to_str().unwrap_or("");
        let is_virtual_submodel = path_str.contains("::");
        if !is_virtual_submodel && path_str.contains("models/") {
            // Files that contain only `smelt.test` or `smelt.check` declarations are
            // valid — they have no SELECT body but they are not broken models.
            // Suppress the "does not contain a valid SQL query" warning for such files.
            let parse = parse_file(db, file);
            let has_smelt_tests_or_checks = AstFile::cast(parse.syntax())
                .map(|ast| ast.tests().next().is_some() || ast.checks().next().is_some())
                .unwrap_or(false);

            if !has_smelt_tests_or_checks {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: "File does not contain a valid SQL query".to_string(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::InvalidModel),
                    data: None,
                })
                .accumulate(db);
            }
        }
        return;
    }

    // Unified path-form refs. Resolve through the path-tuple
    // resolver and either (a) flag undefined paths or (b) flag a
    // kind-mismatch when a `smelt.tests.*` path appears in a FROM
    // position (architecture Surface §"Resolution").
    let path_refs = model_path_refs(db, file);
    for path_ref_loc in path_refs.iter() {
        match resolve_ref_path(db, workspace, path_ref_loc.path.clone()) {
            Some(resolved) => {
                if resolved.kind == RefKind::Test && path_ref_loc.in_table_expr_position {
                    let leaf = path_ref_loc.path.last().cloned().unwrap_or_default();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Cannot reference test '{leaf}' in a FROM position — \
                             smelt.tests.* paths are not valid as TableExpr values"
                        ),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::KindMismatch),
                        data: None,
                    })
                    .accumulate(db);
                }
                if resolved.kind == RefKind::Check && path_ref_loc.in_table_expr_position {
                    let leaf = path_ref_loc.path.last().cloned().unwrap_or_default();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Cannot reference check '{leaf}' in a FROM position — \
                             smelt.check files produce no DB object and cannot be used as TableExpr values"
                        ),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::KindMismatch),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
            None => {
                let path_str = format!("smelt.{}", path_ref_loc.path.join("."));
                // Emit the right diagnostic code based on the path namespace so
                // code-action providers can offer the correct quickfix:
                //   smelt.sources.* → UndefinedSource (offer "Add table to YAML")
                //   smelt.models.*  → UndefinedModelRef (offer "Create model")
                //   anything else   → UndefinedModelRef (generic fallback)
                let is_source_path =
                    path_ref_loc.path.first().map(|s| s.as_str()) == Some("sources");
                if is_source_path && path_ref_loc.path.len() >= 3 {
                    let source_name = path_ref_loc.path[path_ref_loc.path.len() - 2].clone();
                    let table_name = path_ref_loc.path[path_ref_loc.path.len() - 1].clone();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined source: '{}.{}'", source_name, table_name),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::UndefinedSource),
                        data: Some(DiagnosticData::UndefinedSource {
                            source_name,
                            table_name,
                        }),
                    })
                    .accumulate(db);
                } else {
                    // Compute a "did you mean" hint by scanning for models
                    // whose leaf segment matches the last segment of the
                    // unresolved path. This helps users find the full canonical
                    // address when they used only a leaf or partial path.
                    let leaf = path_ref_loc.path.last().map(|s| s.as_str()).unwrap_or("");
                    let hint = if !leaf.is_empty() {
                        let candidates = leaf_did_you_mean(db, workspace, project, leaf);
                        match candidates.as_slice() {
                            [] => String::new(),
                            [single] => format!(" did you mean '{single}'?"),
                            many => {
                                let list = many
                                    .iter()
                                    .map(|s| format!("'{s}'"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!(" did you mean one of {list}?")
                            }
                        }
                    } else {
                        String::new()
                    };
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined ref: {path_str}{hint}"),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::UndefinedModelRef),
                        data: Some(DiagnosticData::UndefinedRef {
                            model_name: path_ref_loc.path.last().cloned().unwrap_or_default(),
                        }),
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Undefined sources
    let sources = model_sources(db, file);
    for source_loc in sources.iter() {
        let resolved = if let Some(p) = project {
            resolve_source(
                db,
                p,
                source_loc.source_name.clone(),
                source_loc.table_name.clone(),
            )
        } else {
            None
        };
        if resolved.is_none() {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined source: '{}'", source_loc.qualified_name),
                range: source_loc.range,
                code: Some(DiagnosticCode::UndefinedSource),
                data: Some(DiagnosticData::UndefinedSource {
                    source_name: source_loc.source_name.clone(),
                    table_name: source_loc.table_name.clone(),
                }),
            })
            .accumulate(db);
        }
    }

    // BUG-078: checked whenever the project carries aggregate `sources.yml`
    // text — NOT gated on `sources` (legacy `smelt.source()` call sites, which
    // are always empty since the per-entity migration made `smelt.source()` a
    // parse error). Gating here made a YAML-broken aggregate file silently
    // fall back to `SourcesConfig::default()` with no diagnostic.
    if let Some(p) = project {
        if let Some(yaml_error) = sources_yaml_error(db, p) {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("sources.yml parse error: {}", yaml_error.message),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::YamlParseError),
                data: None,
            })
            .accumulate(db);
        }
    }

    if !sources.is_empty() {
        if let Some(p) = project {
            let type_errors = sources_type_errors(db, p);
            for error in type_errors.iter() {
                let source_qualified = format!("{}.{}", error.source_name, error.table_name);
                if sources.iter().any(|s| s.qualified_name == source_qualified) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                            "Unknown type '{}' for column '{}' in source '{}'. Type information unavailable.",
                            error.invalid_type, error.column_name, source_qualified
                        ),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::SourceTypeError),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Unsupported constructs + malformed sources + CAST / unknown fn / ambiguous column
    queries::check_types::check_unsupported_constructs(&parse.syntax(), db);

    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        // Phase 4: smelt.source() is a parse error so there are no SourceCall
        // nodes to validate. The malformed-source check is superseded by the
        // parser rejection.

        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        queries::check_types::check_expression_types(&expr, db);
                    }
                    // The `_smelt_` alias prefix is reserved for smelt's own
                    // generated identifiers (`multi_backend.md` §"Output-schema
                    // type conformance") — most visibly the synthesized
                    // `_smelt_col{n}` alias bound to a nameless projection
                    // item. Emitted here (the analyzer) rather than only at
                    // build time so the LSP and the CLI build path agree
                    // (`architecture.md` §"Diagnostic parity rule").
                    if let Some(alias) = item.alias() {
                        if alias.starts_with("_smelt_") {
                            let range = item.alias_range().unwrap_or_else(|| item.range());
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                message: format!(
                                    "column alias `{alias}` uses the reserved `_smelt_` prefix; \
                                     smelt uses this prefix for its own generated identifiers \
                                     (e.g. the synthesized name for an unaliased expression \
                                     column) — choose a different alias"
                                ),
                                range,
                                code: Some(DiagnosticCode::ReservedProjectionAliasPrefix),
                                data: None,
                            })
                            .accumulate(db);
                        }
                    }
                }
            }

            // Phase 14 (§16 #24): reject window-kind expressions in WHERE
            // and GROUP BY positions. Kind synthesis is independent of any
            // column-schema lookups (column refs are always Scalar), so
            // the check runs on a fresh empty `TypeContext`.
            let kind_ctx = type_inference::TypeContext::new();
            for info in type_inference::check_window_in_scalar_contexts(&select_stmt, &kind_ctx) {
                let range = info.range;
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Window function `{}` is not allowed in {} (only scalar / aggregate \
                         expressions are permitted here)",
                        info.expression_text, info.clause
                    ),
                    range,
                    code: Some(DiagnosticCode::WindowInScalarContext),
                    data: None,
                })
                .accumulate(db);
            }

            // Phase A (meta-language) Phase 3: list + spread diagnostics.
            //
            // 1. Walk LIST_SPREAD nodes in the SELECT list.
            //    Handles: MetaSpreadOnNonList, MetaListHeterogeneous (for inline
            //    spread-of-literal), MetaListEmptyTypeUnknown.
            //    GroupBy / OrderBy / function args / IN-list / VALUES remain
            //    deferred — the parser DOES emit LIST_SPREAD there, but the
            //    orchestrator does not yet walk those positions.
            //
            // 2. Walk SELECT_ITEM expressions for bare list literals
            //    (`SELECT [1, 'x'] FROM t`).
            //    Handles: MetaListHeterogeneous and MetaListEmptyTypeUnknown
            //    for non-spread list literals appearing directly in the
            //    SELECT list.
            //
            // 3. Detect spreads in forbidden positions (WHERE, etc.).
            //    Handles: MetaSpreadInForbiddenPosition.
            //
            // All three checks use an empty TypeContext (no column schema
            // available at this point) — consistent with the window-function
            // check above.
            // Ranges of meta diagnostics already emitted for this select
            // statement. A `List<T>`-in-scalar-position check (below) is
            // suppressed for any select item that already carries another meta
            // error (drop-on-error: a single malformed item does not avalanche).
            let mut flagged_meta_ranges: Vec<rowan::TextRange> = Vec::new();

            let spread_result = type_inference::check_select_list_spreads(&select_stmt, &kind_ctx);
            for diag in spread_result.diagnostics {
                flagged_meta_ranges.push(diag.range);
                DiagnosticAcc(diag).accumulate(db);
            }

            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        if let Some(arr) = expr.as_array_literal() {
                            let elements = arr.elements();
                            // Use the expression's span for the diagnostic anchor.
                            let span = expr.syntax().text_range();
                            for diag in type_inference::list_literal_sentinels_to_diagnostics(
                                &elements, &kind_ctx, span,
                            ) {
                                flagged_meta_ranges.push(diag.range);
                                DiagnosticAcc(diag).accumulate(db);
                            }
                        }
                    }
                }
            }

            let forbidden_diags =
                type_inference::check_forbidden_position_spreads(&select_stmt, &kind_ctx);
            for diag in forbidden_diags {
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase B (meta-language) Phase 3: HOF + lambda + pipe diagnostics.
            //
            // Walks every LAMBDA, FUNCTION_CALL (HOF), and PIPE_EXPR descendant.
            // Covers: LambdaInForbiddenPosition, LambdaArityMismatch, LambdaZeroParameters,
            //   LambdaDuplicateParameter, LambdaResultTypeMismatch, HofExpectsLambda,
            //   HofExpectsReducer, PipeRhsNotCall, PipeInDataPosition,
            //   ReducerInputTypeMismatch, ReducerEmptyNoIdentity.
            // Also covers Phase F REDUCER_CALL nodes (parameterised reducers):
            //   ReducerArityMismatch, ReducerArgTypeMismatch, ReducerArgNotCompileTime,
            //   ReducerNamedArgument.
            // Uses an empty TypeContext (consistent with spread/window checks above).
            let hof_diags =
                type_inference::check_hof_position_diagnostics(&select_stmt, &kind_ctx, text);
            for diag in hof_diags {
                flagged_meta_ranges.push(diag.range);
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase F (meta-language) — Ternary expression diagnostics.
            //
            // Walks every TERNARY_EXPR descendant and bare THEN_KW tokens.
            // Covers: TernaryConditionNotBoolean, TernaryBranchTypeMismatch,
            //   TernaryDanglingElse, TernaryDanglingThen.
            // Uses an empty TypeContext (consistent with HOF checks above).
            {
                let ternary_diags =
                    type_inference::check_ternary_expr_diagnostics(&select_stmt, &kind_ctx);
                for diag in ternary_diags {
                    flagged_meta_ranges.push(diag.range);
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // BUG-017: cross-family binary arithmetic → TypeMismatch.
            //
            // Walks every BINARY_EXPR and emits exactly one TypeMismatch Error
            // at the operator span when a numeric/string/boolean/temporal
            // cross-family pair is detected (spec §1 and §14).
            // Uses an empty TypeContext — literal operands (`42 + '3'`)
            // resolve without column context; column-typed operands resolve
            // if a full ctx is available later in check_type_diagnostics.
            {
                let xfamily_diags = type_inference::check_crossfamily_arithmetic_diagnostics(
                    &select_stmt,
                    &kind_ctx,
                );
                for diag in xfamily_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §15 — decimal precision overflow → `DecimalPrecisionOverflow`.
            //
            // Walks every `+`, `-`, `*`, `%` BINARY_EXPR and emits exactly one
            // `DecimalPrecisionOverflow` Error at the operator span when the
            // Spark-style growth formula yields `p' > 38`. Division is excluded
            // (handled below). The result type in such expressions is already
            // `DataType::Unknown` as computed by `promote_numeric_operands_for_op`.
            {
                let overflow_diags = type_inference::check_decimal_precision_overflow_diagnostics(
                    &select_stmt,
                    &kind_ctx,
                );
                for diag in overflow_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §15 — division rejection → `TypeMismatch`.
            //
            // `Decimal / T` for any numeric `T` is not in the portable surface.
            // Emits one `TypeMismatch` Error at the `/` operator span directing
            // the user to cast to Double. The inferred result type is already
            // `DataType::Unknown` (set by `promote_numeric_operands_for_op`).
            {
                let div_diags =
                    type_inference::check_decimal_division_diagnostics(&select_stmt, &kind_ctx);
                for diag in div_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §17 — non-portable collation → `NonPortableCollation`.
            //
            // Walks every COLLATE_EXPR in the SELECT statement. For any
            // non-binary collation name the diagnostic fires at the COLLATE
            // clause span and the expression type degrades to Unknown
            // (handled in `infer_expression_type` via
            // `infer_collate_expr_type`). Binary collations (COLLATE "C",
            // COLLATE BINARY, COLLATE UTF8_BINARY, COLLATE POSIX) are
            // silent no-ops.
            {
                let collation_diags =
                    type_inference::check_collation_diagnostics(&select_stmt, &kind_ctx);
                for diag in collation_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §16 — mixed naive/tz-aware Timestamp in set operations, CASE
            // branches, and arithmetic → TypeMismatch.
            //
            // These three checks need the full per-file TypeContext (column types
            // from upstream models) so that column references such as `ts_col` and
            // `tstz_col` resolve to their inferred DataType. They cannot run on the
            // empty `kind_ctx` used for shape checks above. `type_context` is a
            // Salsa query that builds the column-schema context for this file; it
            // is safe to call from within a Salsa tracked function.
            //
            // Only run for model files that have at least one data reference
            // (the model_path filter is already satisfied by the outer `if let
            // Some(select_stmt)` guard and the `models/` path check earlier).
            {
                let tz_ctx = type_context(db, workspace, file);

                // Set-operations (UNION/INTERSECT/EXCEPT)
                let setop_diags =
                    type_inference::check_mixed_tz_setop_diagnostics(&select_stmt, &tz_ctx);
                for diag in setop_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // CASE branches
                let case_diags =
                    type_inference::check_mixed_tz_case_diagnostics(&select_stmt, &tz_ctx);
                for diag in case_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // Arithmetic operators (-, +, *, /, %)
                let mixed_tz_arith_diags =
                    type_inference::check_mixed_tz_arithmetic_diagnostics(&select_stmt, &tz_ctx);
                for diag in mixed_tz_arith_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // VALUES-clause columns (§16 strict temporal mixing rule)
                let values_temporal_diags =
                    type_inference::check_mixed_temporal_values_diagnostics(&select_stmt, &tz_ctx);
                for diag in values_temporal_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Meta-language (P6) — `MetaListInScalarPosition`.
            //
            // A `List<T>`-typed expression that reaches a Data-World scalar /
            // SELECT-item position without being consumed (by a spread, a HOF,
            // a reducer, a record, a map, or a generator) cannot materialise as
            // a scalar value — there is no implicit auto-spread
            // (`meta_language.md` §Semantics "Lists and spread" rule 10). A bare
            // list literal (`SELECT [1, 2, 3]`), or a bare `map`/`filter` /
            // pipe-to-`map`/`filter` result (`SELECT xs |> map(fn c => …)`),
            // left in a select item is unconsumed. `reduce` collapses a list to
            // a scalar, so a `reduce(...)` select item is consumed and clean.
            //
            // This is a select-shape check that runs for every model, including
            // a model with no FROM clause — `check_type_diagnostics`
            // early-returns when a model has no data refs, so the check lives
            // here (the meta walk runs regardless of FROM). Suppressed for any
            // item already carrying another meta diagnostic (drop-on-error).
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    let Some(expr) = item.expression() else {
                        continue;
                    };
                    if !select_item_yields_bare_list(&expr) {
                        continue;
                    }
                    let span = expr.syntax().text_range();
                    if flagged_meta_ranges
                        .iter()
                        .any(|r| r.intersect(span).is_some())
                    {
                        continue;
                    }
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "a List<T> cannot be used as a scalar value here; consume it \
                                  with a spread (`...xs`), a reducer (`reduce(xs, …)`), or a HOF \
                                  before splicing"
                            .to_string(),
                        range: span,
                        code: Some(DiagnosticCode::MetaListInScalarPosition),
                        data: None,
                    })
                    .accumulate(db);
                }
            }

            // Phase C (meta-language) — smelt.columns_of diagnostic wiring.
            //
            // Walks every SMELT_PATH_CALL for `smelt.columns_of(...)` in the
            // select statement. Emits:
            //   - ColumnsOfNamedArgument: named argument passed to columns_of
            //   - ColumnsOfRequiresTableExpr: non-TableExpr positional arg
            // Uses the same empty TypeContext as HOF checks (no column schema
            // available at this stage in the orchestrator).
            {
                let cols_of_diags =
                    type_inference::check_columns_of_diagnostics(&select_stmt, &kind_ctx);
                for diag in cols_of_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase C (meta-language) — ColumnsOfUnresolvableSchema wiring.
            //
            // For each `smelt.columns_of(smelt.models.<name>)` (or
            // `smelt.columns_of(<name>)` where `<name>` is a bare identifier that
            // resolves via the workspace) call in the select statement, attempt to
            // resolve the model schema via `columns_of_for_table_expr`. When the
            // schema cannot be resolved (the model does not exist or has an unknown
            // schema), emit exactly one `ColumnsOfUnresolvableSchema` diagnostic
            // anchored at the full `smelt.columns_of(...)` call span.
            //
            // This implements the drop-on-error recovery policy (same as
            // `MetaSpreadInForbiddenPosition`): the call-site gets one diagnostic
            // and no cascading errors from the surrounding expression.
            {
                use smelt_parser::ast::SmeltPathCall;
                use smelt_parser::SyntaxKind::SMELT_PATH_CALL;
                for node in select_stmt.syntax().descendants() {
                    if node.kind() != SMELT_PATH_CALL {
                        continue;
                    }
                    let call = match SmeltPathCall::cast(node.clone()) {
                        Some(c) => c,
                        None => continue,
                    };
                    let segs = call.segments();
                    if segs.len() != 1 || segs[0].to_lowercase() != "columns_of" {
                        continue;
                    }
                    let arg_list = match call.arg_list() {
                        Some(al) => al,
                        None => continue,
                    };
                    // Only check positional args (named args are caught by
                    // ColumnsOfNamedArgument above).
                    for pos_arg in arg_list.positional_args() {
                        // Extract the model name from the positional argument:
                        // - smelt path ref: e.g. `smelt.models.orders` → last segment
                        // - bare identifier: e.g. `orders`
                        let model_name: Option<String> = {
                            // Try smelt path ref child.
                            let path_ref_name = pos_arg
                                .syntax()
                                .children()
                                .find_map(smelt_parser::ast::SmeltPathRef::cast)
                                .and_then(|r| r.segments().last().cloned());
                            if let Some(n) = path_ref_name {
                                Some(n)
                            } else {
                                // Try direct SmeltPathRef cast.
                                smelt_parser::ast::SmeltPathRef::cast(pos_arg.syntax().clone())
                                    .and_then(|r| r.segments().last().cloned())
                                    .or_else(|| {
                                        // Bare identifier: must start with a letter or
                                        // underscore (not a numeric literal like `42`).
                                        let arg_text = pos_arg.text().trim().to_string();
                                        let is_bare = !arg_text.is_empty()
                                            && arg_text
                                                .chars()
                                                .next()
                                                .is_some_and(|c| c.is_alphabetic() || c == '_')
                                            && arg_text
                                                .chars()
                                                .all(|c| c.is_alphanumeric() || c == '_');
                                        if is_bare {
                                            Some(arg_text)
                                        } else {
                                            None
                                        }
                                    })
                            }
                        };
                        let model_name = match model_name {
                            Some(n) => n,
                            None => continue,
                        };
                        let resolves = project
                            .map(|p| {
                                columns_of_for_table_expr(db, workspace, p, model_name.clone())
                                    .is_ok()
                            })
                            .unwrap_or(false);
                        if !resolves {
                            let call_range = node.text_range();
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                message: meta_reflection_diagnostic_message_with_table_expr(
                                    DiagnosticCode::ColumnsOfUnresolvableSchema,
                                    None,
                                    None,
                                    Some(&model_name),
                                ),
                                range: call_range,
                                code: Some(DiagnosticCode::ColumnsOfUnresolvableSchema),
                                data: None,
                            })
                            .accumulate(db);
                        }
                    }
                }
            }

            // Phase C (meta-language) — ColumnRefFieldUnknown HOF dispatcher.
            //
            // For each `map`/`filter` HOF call whose first argument is
            // `smelt.columns_of(…)`, walk the lambda body and emit
            // `ColumnRefFieldUnknown` for any `<param>.<field>` access where
            // `<field>` is not in the closed ColumnRef field set
            // `{name, type, is_numeric}`.
            //
            // This runs on MODEL select statements (the outer `select_stmt`).
            // Function-file SELECT bodies are handled separately in
            // `function_body_diagnostics_for_file`.
            {
                for diag in
                    function_body_check::check_hof_column_ref_field_diagnostics(&select_stmt)
                {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase D (meta-language) — wide-reflection accessor diagnostics.
            //
            // Walks every SMELT_PATH_CALL for `smelt.models.*` / `smelt.sources.*`
            // in the model SELECT statement.  Emits:
            //   - WideReflectionUnknownAccessor: unknown accessor name
            //   - WideReflectionUnexpectedArgument: argument to `all`
            //   - WithTagRequiresText: non-compile-time-Text argument to `with_tag`
            //   - WithTagNamedArgument: named argument to `with_tag`
            //
            // Uses an empty TypeContext (no ModelRef/SourceRef bindings exist at
            // the top-level model SELECT scope).
            {
                let phase_d_ctx = type_inference::TypeContext::new();
                for diag in type_inference::check_wide_reflection_diagnostics(
                    &select_stmt,
                    &phase_d_ctx,
                    text,
                ) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase D (meta-language) — ModelRef / SourceRef HOF field dispatcher.
            //
            // For each `map`/`filter` HOF call whose first argument is a
            // `smelt.models.*` / `smelt.sources.*` wide-reflection call, walk
            // the lambda body and emit `ModelRefFieldUnknown` /
            // `SourceRefFieldUnknown` for any `<param>.<field>` access where
            // `<field>` is not in the closed field set `{path, name, tags, columns}`.
            //
            // This runs on MODEL select statements (the outer `select_stmt`).
            // Function-file SELECT bodies are handled separately in
            // `function_body_diagnostics_for_file` via `check_function_select_body`.
            {
                for diag in function_body_check::check_hof_model_ref_source_ref_field_diagnostics(
                    &select_stmt,
                ) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            let from_sources = count_from_sources(&select_stmt);
            if from_sources > 1 {
                if let Some(select_list) = select_stmt.select_list() {
                    for item in select_list.items() {
                        if let Some(expr) = item.expression() {
                            if let Some(col_ref) = expr.as_column_ref() {
                                if col_ref.qualifier().is_none() {
                                    let col_name = col_ref.name();
                                    if col_name != "*" {
                                        DiagnosticAcc(Diagnostic {
                                            severity: DiagnosticSeverity::Warning,
                                            message: format!(
                                                "Column '{}' is ambiguous - multiple sources in FROM clause. Consider using a qualified name (e.g., table.{}).",
                                                col_name, col_name
                                            ),
                                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                                            code: Some(DiagnosticCode::AmbiguousColumn),
                                            data: None,
                                        })
                                        .accumulate(db);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase F (meta-language) — File-level dangling THEN_KW detection.
        //
        // The parser's error recovery may eject a bare `then` keyword to the
        // top-level FILE node when it appears in an unexpected expression
        // position (e.g. `SELECT then x FROM t`).  `check_ternary_expr_diagnostics`
        // walks only the SelectStmt subtree and cannot reach FILE-level tokens.
        // This block walks the FULL file syntax so dangling THEN_KW tokens are
        // always caught, regardless of where error recovery placed them.
        //
        // Emits: TernaryDanglingThen.
        {
            let file_syntax = ast.syntax().clone();
            for diag in type_inference::check_dangling_ternary_keywords(&file_syntax) {
                DiagnosticAcc(diag).accumulate(db);
            }
        }
    }
}

/// Resolve a project root path to a `ProjectInput` via the workspace.
///
/// Public so the LSP can derive the caller's project (from the cursor
/// file's `project_root(db)`) when threading the project isolation rule
/// through goto-def, hover, and other features that consult function
/// signatures. See `docs/specs/architecture.md` → "Project isolation rule".
pub fn find_project(
    db: &dyn salsa::Database,
    workspace: Workspace,
    root: &Path,
) -> Option<ProjectInput> {
    workspace
        .projects(db)
        .iter()
        .copied()
        .find(|p| p.root(db) == root)
}

/// Look up the registered [`DeployedSchemaInput`] for `(project_root, table)`
/// via the `Workspace` singleton's `deployed_schemas` list — the enumeration
/// seam a Salsa-tracked query (`&dyn salsa::Database`, no downcast to the
/// concrete `Database`) must use, mirroring `workspace.loader_files(db)`'s
/// lookup pattern in `queries/loader.rs`/`queries/project.rs`.
fn find_deployed_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project_root: &Path,
    table: &str,
) -> Option<DeployedSchemaInput> {
    workspace
        .deployed_schemas(db)
        .iter()
        .copied()
        .find(|s| s.project_root(db) == project_root && &**s.model(db) == table)
}

fn count_from_sources(select_stmt: &smelt_parser::ast::SelectStmt) -> usize {
    let mut count = 0;
    if let Some(from_clause) = select_stmt.from_clause() {
        count += from_clause.table_refs().count();
        count += from_clause.joins().count();
    }
    count
}

// ============================================================================
// Phase 30 — Logical plan construction
// ============================================================================
//
// The Salsa thin wrapper lives here; it gathers inputs from mixed Salsa queries
// (resolve_function, file_signature_inputs, parse_file, project_unstable_schema,
// workspace_function_bodies, function_call_cycle_fn_ids) and delegates to the
// pure builder `smelt_logical::build_logical_plan_pure` (Salsa-purity rule).

use smelt_parser::ast::SmeltPathCall;

/// Build a [`smelt_logical::Plan`] from a single source file.
///
/// This tracked query gathers all Salsa inputs — the parsed AST, resolved
/// signatures, and per-declaration frontmatter — then delegates to the pure
/// helper [`smelt_logical::build_logical_plan_pure`] which takes no `db` reference.
///
/// Returns `None` when the file does not parse as a valid SQL model.
#[salsa::tracked]
pub fn logical_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_logical::Plan> {
    use smelt_logical::Provenance;

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;

    // Determine whether the workspace has opted in to unstable schema features.
    // Uses the Salsa-tracked ProjectInput so changes to smelt.yml invalidate
    // this query via Salsa's dependency graph (no raw filesystem I/O here).
    let project_root = file.project_root(db).clone();
    let unstable_schema = find_project(db, workspace, &project_root)
        .map(|p| project_unstable_schema(db, p))
        .unwrap_or(false);

    // Phase 41: workspace-wide body capture + cycle pre-pass.  The body map
    // lets the call-site loop attach `LogicalNode::Raw` subtrees without
    // re-walking the workspace per call; the cycle set tells us which
    // transparent calls must skip body attachment so the planner does not
    // attempt to inline a non-terminating expansion.
    let bodies = workspace_function_bodies(db, workspace);
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    // Walk the CST to collect all smelt.functions.* (path-form) call sites.
    let call_inputs: Vec<smelt_logical::FnCallInput> = ast
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
        .map(|call| {
            let segments = call.segments();
            let fn_id = segments.last().cloned().unwrap_or_default();

            // Per docs/specs/architecture.md → "Project isolation rule":
            // resolve only against functions declared in the same project as
            // the calling file. Multi-project workspaces (e.g. a monorepo
            // opened in VSCode) must not see cross-project signatures.
            let sig_opt = if fn_id.is_empty() {
                None
            } else {
                find_project(db, workspace, &project_root).and_then(|project| {
                    resolve_function(db, workspace, project, fn_id.clone())
                        .map(|arc| (*arc).clone())
                })
            };

            let transparent = sig_opt
                .as_ref()
                .map(|sig| sig.origin == smelt_types::SigOrigin::Define)
                .unwrap_or(false);

            // Locate the declaring file and read its frontmatter via Salsa.
            let mut properties = sig_opt
                .as_ref()
                .and_then(|_| {
                    workspace
                        .files(db)
                        .iter()
                        .copied()
                        .find(|f| {
                            file_signature_inputs(db, *f)
                                .iter()
                                .any(|s| s.name == fn_id)
                        })
                        .and_then(|decl_file| {
                            let decl_parse = parse_file(db, decl_file);
                            let decl_syntax = decl_parse.syntax();
                            let decl_ast = AstFile::cast(decl_syntax)?;
                            let decl_raw = decl_file.text(db).clone();
                            let fm_with_kind = decl_ast
                                .defines()
                                .find(|d| d.name().as_deref() == Some(fn_id.as_str()))
                                .and_then(|d| {
                                    d.frontmatter(&decl_raw)
                                        .map(|fm| (fm, smelt_core::DeclarationKind::Define))
                                })
                                .or_else(|| {
                                    decl_ast
                                        .externs()
                                        .find(|e| e.name().as_deref() == Some(fn_id.as_str()))
                                        .and_then(|e| {
                                            e.frontmatter(&decl_raw)
                                                .map(|fm| (fm, smelt_core::DeclarationKind::Extern))
                                        })
                                });
                            // Ignore frontmatter diagnostics here — they are surfaced via
                            // `provenance_unstable_diagnostics_for_file` (called from
                            // `check_file_diagnostics`), which has the declaration's name range
                            // for proper anchoring. The logical-plan path only needs the props.
                            fm_with_kind.map(|(text, kind)| {
                                smelt_logical::parse_function_properties(&text, kind).0
                            })
                        })
                })
                .unwrap_or_default();

            // Phase 31: enforce unstable_schema gate on `provenance:`.
            // If the function declared provenance but the workspace flag is
            // absent, silently return Unknown here. The diagnostic is emitted
            // by `provenance_unstable_diagnostics_for_file`, which is called
            // from `check_file_diagnostics` so it surfaces through
            // `file_diagnostics`.
            let resolved_provenance =
                if matches!(properties.provenance, Provenance::Declared(_)) && !unstable_schema {
                    Provenance::Unknown
                } else {
                    // Either the flag is set (use declared provenance) or
                    // provenance is already Unknown (pass through).
                    std::mem::replace(&mut properties.provenance, Provenance::Unknown)
                };

            // Phase 41: attach body text for transparent calls whose declaring
            // function is not in a cycle.  Opaque (`smelt.extern`) calls and
            // cycle participants leave `body_text: None` so the expansion
            // rule falls back to the marker-only behaviour from Phase 32.
            let body_text = if transparent && !cycle_set.contains(&fn_id) {
                bodies.get(&fn_id).cloned()
            } else {
                None
            };

            smelt_logical::FnCallInput {
                fn_id,
                transparent,
                properties,
                provenance: resolved_provenance,
                body_text,
            }
        })
        .collect();

    Some(smelt_logical::build_logical_plan_pure(call_inputs))
}

/// Phase 41: workspace-wide map from `fn_id` → body text for every
/// `smelt.define`. Opaque externs are not included (they have no body).
///
/// Salsa-tracked so the cycle pre-pass and body-attachment paths share one
/// cache entry per workspace.  The return is wrapped in `Arc` to satisfy
/// Salsa's interning / equality requirements (the same shape used by
/// `all_models`).
#[salsa::tracked]
pub(crate) fn workspace_function_bodies(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for f in workspace.files(db).iter().copied() {
        let parse = parse_file(db, f);
        let syntax = parse.syntax();
        let Some(ast) = AstFile::cast(syntax) else {
            continue;
        };
        for define in ast.defines() {
            let Some(name) = define.name() else { continue };
            let Some(body) = define.body() else { continue };
            let body_text = body.syntax().text().to_string();
            // First wins on duplicates — duplicate-define diagnostics catch
            // the second occurrence elsewhere.
            out.entry(name).or_insert(body_text);
        }
    }
    Arc::new(out)
}

/// Workspace-wide call graph for `smelt.define` declarations.
///
/// Returns a map from caller `fn_id` → callees (set of `fn_id`s reached from
/// the body's `smelt.functions.*` call sites). Externs and unresolved references
/// are dropped — they are sinks in the graph.  Salsa-tracked so each
/// workspace pays the walk once per parse-graph epoch.
#[salsa::tracked]
pub(crate) fn workspace_function_call_graph(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashMap<String, Vec<String>>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for f in workspace.files(db).iter().copied() {
        let parse = parse_file(db, f);
        let syntax = parse.syntax();
        let Some(ast) = AstFile::cast(syntax) else {
            continue;
        };
        for define in ast.defines() {
            let Some(caller) = define.name() else {
                continue;
            };
            let Some(body) = define.body() else { continue };
            let mut callees: Vec<String> = body
                .syntax()
                .descendants()
                .filter_map(SmeltPathCall::cast)
                .filter_map(|c| c.segments().last().cloned())
                .filter(|s| !s.is_empty())
                .collect();
            callees.sort();
            callees.dedup();
            out.entry(caller).or_insert(callees);
        }
    }
    Arc::new(out)
}

/// Phase 41 — pure DFS cycle detector over the workspace call graph.
/// Returns the set of `fn_id`s that participate in any cycle.
pub fn find_function_call_cycles(
    graph: &std::collections::HashMap<String, Vec<String>>,
) -> std::collections::HashSet<String> {
    use std::collections::HashSet;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Grey,
        Black,
    }
    let mut color: HashMap<&str, Color> = HashMap::new();
    let mut in_cycle: HashSet<String> = HashSet::new();

    for node in graph.keys() {
        color.insert(node.as_str(), Color::White);
    }

    fn dfs<'a>(
        node: &'a str,
        graph: &'a std::collections::HashMap<String, Vec<String>>,
        color: &mut HashMap<&'a str, Color>,
        stack: &mut Vec<&'a str>,
        in_cycle: &mut HashSet<String>,
    ) {
        color.insert(node, Color::Grey);
        stack.push(node);
        if let Some(callees) = graph.get(node) {
            for callee in callees {
                let key = callee.as_str();
                match color.get(key).copied().unwrap_or(Color::White) {
                    Color::White => {
                        if graph.contains_key(callee) {
                            dfs(key, graph, color, stack, in_cycle);
                        } else {
                            // sink: not in graph
                            color.insert(key, Color::Black);
                        }
                    }
                    Color::Grey => {
                        // Found a back-edge — every Grey node from `key` to
                        // the top of `stack` is on the cycle.
                        let mut on_cycle = false;
                        for &s in stack.iter() {
                            if s == key {
                                on_cycle = true;
                            }
                            if on_cycle {
                                in_cycle.insert(s.to_string());
                            }
                        }
                    }
                    Color::Black => {}
                }
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
    }

    let nodes: Vec<&str> = graph.keys().map(|s| s.as_str()).collect();
    for node in nodes {
        if matches!(
            color.get(node).copied().unwrap_or(Color::White),
            Color::White
        ) {
            let mut stack: Vec<&str> = Vec::new();
            dfs(node, graph, &mut color, &mut stack, &mut in_cycle);
        }
    }

    in_cycle
}

/// Cached union of cycle-participant `fn_id`s for the current workspace.
#[salsa::tracked]
pub(crate) fn function_call_cycle_fn_ids(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashSet<String>> {
    let graph = workspace_function_call_graph(db, workspace);
    Arc::new(find_function_call_cycles(graph.as_ref()))
}

/// Phase 41 — emit [`DiagnosticCode::FunctionCallCycle`] for every
/// `smelt.define` in `file` whose `fn_id` is reachable inside a cycle in the
/// workspace call graph. Anchored at the declaration's name range.
pub fn function_call_cycle_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let cycle_set = function_call_cycle_fn_ids(db, workspace);
    if cycle_set.is_empty() {
        return Vec::new();
    }

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else { continue };
        if !cycle_set.contains(&name) {
            continue;
        }
        let range = sigs
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.name_range)
            .unwrap_or(rowan::TextRange::empty(rowan::TextSize::from(0)));
        out.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "function `{name}` participates in a cyclic call graph; \
                 transparent expansion is suppressed for this function and \
                 every other function on the cycle"
            ),
            range,
            code: Some(DiagnosticCode::FunctionCallCycle),
            data: None,
        });
    }
    out
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod test_harness;

#[cfg(test)]
mod tests;
