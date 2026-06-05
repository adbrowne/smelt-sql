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
use std::sync::{Arc, RwLock};

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
    cannot_infer_type_for_schema, check_expression_types_for_select, check_type_diagnostics,
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
    smelt_fn_call_diagnostics_for_file, unknown_context_diagnostics_for_file,
    workspace_function_diagnostics,
};
pub use queries::functions::{
    file_signature_inputs, function_body, function_signature, functions_in_file, resolve_function,
    resolve_function_path, BodyRange, NameRange,
};
pub use queries::loader::{
    loader_call_diagnostics_for_file, loader_call_diagnostics_for_file_with_content,
    loader_call_diagnostics_for_syntax, loader_file_parsed, loader_resolved_value,
    loader_resolved_value_with_overlay, parse_smelt_type_from_field_annotation,
    smelt_record_declarations, LoaderCallSiteId, LoaderResolvedValue,
};
pub use queries::parse::{
    model_path_refs, model_sources, parse_file, parse_model, PathRefLocation,
};
pub use queries::project::{
    all_models, emitted_model_body_analysis, emitted_model_smelt_path, emitted_model_typed_schema,
    emitted_models, evaluate_generator, generator_files, models_all, models_all_with_generators,
    models_with_tag, project_active_backends, project_address_collisions, project_paths,
    project_seeds, project_source_diagnostics, project_sources, project_unstable_schema,
    resolve_seed_or_source_path, smelt_yml_vars_query, sources_all, sources_config,
    sources_type_errors, sources_with_tag, sources_yaml_error, AddressCollisionDiagnostic,
    EmissionBodyAnalysis, EmittedModelDef, EmittedModelsResult, EvaluatedGenerator,
    SourceDiagnostic, SourceTypeError, YamlParseError,
};
pub use queries::schema::{
    add_source_info_to_type_context, available_columns, build_type_context,
    columns_of_for_table_expr, columns_to_column_ref_values, model_function_type,
    model_input_constraints, model_schema, resolved_model_schema, type_context, typed_model_schema,
    RefSchemaProvider, SalsaRefSchemaProvider, StaticRefSchemaProvider,
};

// ============================================================================
// Salsa inputs
// ============================================================================

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
}

// ============================================================================
// Loader file inputs (Phase E1 Phase 5)
// ============================================================================

/// A per-loader-call file path registered as a Salsa input.
///
/// Phase 5: one `LoaderFileInput` is created per unique loader-target path
/// encountered in the workspace. The LSP (and CLI build orchestration) creates
/// and updates these inputs when the corresponding files change on disk.
///
/// Phase 6 will add per-target overlay inputs; the shape below is designed to
/// accommodate the Phase 6 overlay query without restructuring.
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
}

#[salsa::db]
impl salsa::Database for Database {}

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
        let existing = self.files.read().unwrap().get(&path).copied();
        match existing {
            Some(file) => {
                file.set_text(self).to(text);
                file.set_project_root(self).to(project_root);
                file
            }
            None => {
                let file = SourceFile::new(self, path.clone(), text, project_root);
                self.files.write().unwrap().insert(path, file);
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
        let existing = self.projects.read().unwrap().get(&root).copied();
        match existing {
            Some(project) => {
                project.set_sources_yaml(self).to(sources_yaml);
                project.set_smelt_yml_text(self).to(smelt_yml_text);
                project
            }
            None => {
                let project = ProjectInput::new(self, root.clone(), sources_yaml, smelt_yml_text);
                self.projects.write().unwrap().insert(root, project);
                project
            }
        }
    }

    /// Update the `smelt.yml` text for an already-registered project. Called by
    /// the LSP whenever the file changes on disk; Salsa propagates the
    /// invalidation through `project_unstable_schema` and any query that reads it.
    pub fn set_project_smelt_yml(&mut self, root: &Path, smelt_yml_text: String) {
        let project = self.projects.read().unwrap().get(root).copied();
        if let Some(project) = project {
            project.set_smelt_yml_text(self).to(smelt_yml_text);
        }
    }

    /// Look up an already-registered `SourceFile` by path.
    pub fn source_file(&self, path: &Path) -> Option<SourceFile> {
        self.files.read().unwrap().get(path).copied()
    }

    /// Look up an already-registered `ProjectInput` by root path.
    pub fn project_input(&self, root: &Path) -> Option<ProjectInput> {
        self.projects.read().unwrap().get(root).copied()
    }

    /// Set (or create) the workspace singleton with the given file and project lists.
    ///
    /// Preserves the existing `loader_files` list if the workspace already exists;
    /// `set_loader_file` is responsible for keeping that list up to date.
    pub fn set_workspace(&mut self, files: Vec<SourceFile>, projects: Vec<ProjectInput>) {
        match Workspace::try_get(self) {
            Some(ws) => {
                ws.set_files(self).to(files);
                ws.set_projects(self).to(projects);
                // loader_files is preserved as-is; set_loader_file manages it.
            }
            None => {
                Workspace::new(self, files, projects, Vec::new());
            }
        }
    }

    /// Convenience accessor: the workspace singleton, creating it empty if missing.
    pub fn workspace(&mut self) -> Workspace {
        match Workspace::try_get(self) {
            Some(ws) => ws,
            None => Workspace::new(self, Vec::new(), Vec::new(), Vec::new()),
        }
    }

    /// Create or update the `LoaderFileInput` for a workspace-relative path.
    ///
    /// Called by the LSP / CLI build orchestration whenever a loader-target file
    /// is discovered (during workspace load) or edited (during an LSP edit
    /// session). Salsa propagates invalidations to `loader_file_parsed` and
    /// `loader_resolved_value` automatically.
    ///
    /// Phase 6 will extend this to register per-target overlay files alongside
    /// the base file.
    pub fn set_loader_file(
        &mut self,
        path: Arc<str>,
        text: Arc<str>,
        exists: bool,
    ) -> LoaderFileInput {
        let path_str = path.to_string();
        let existing = self.loader_files.read().unwrap().get(&path_str).copied();
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
                self.loader_files.write().unwrap().insert(path_str, input);
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
        self.loader_files.read().unwrap().get(path).copied()
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
/// - `.sql` file with `materialization: test` (Phase 2a stand-in for
///   the future `smelt.test` declaration kind) → `Test`
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

        // SQL files: walk every workspace file, compute its
        // workspace-relative path tuple, and compare.
        for file in workspace.files(db).iter().copied() {
            let file_path = file.path(db);
            // Accept .sql files, .py files (Python models whose content is
            // generated SQL), and virtual `*.sql::model_name` paths created
            // by multi-model file splitting.
            let path_str = file_path.to_str().unwrap_or("");
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "sql" && ext != "py" && !path_str.contains(".sql::") {
                continue;
            }
            // Match if file_path lives under project_root.
            let file_tuple = match file_path_tuple(&project_root, file_path, file, db, &scan_roots)
            {
                Some(t) => t,
                None => continue,
            };
            if file_tuple == path {
                let kind = sql_file_kind(db, file);
                return Some(ResolvedRef {
                    kind,
                    source_file: Some(file),
                    path,
                });
            }
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
    let raw_text = file.text(db);
    // 1. `smelt.define` → Function. Inspect the parsed AST; this is
    //    cheap because the parse is already cached via Salsa.
    let parse = parse_file(db, file);
    if let Some(ast) = AstFile::cast(parse.syntax()) {
        if ast.defines().next().is_some() {
            return RefKind::Function;
        }
    }
    // 2. `materialization: test` frontmatter (Phase 2a stand-in for the
    //    forthcoming `smelt.test` declaration). Use `extract_file_metadata`
    //    so multi-model files with mixed materializations are handled.
    if let Ok(meta) = smelt_core::extract_file_metadata(raw_text) {
        match meta {
            smelt_core::FileMetadata::Single { metadata, .. } => {
                if metadata.materialization == Some(smelt_core::Materialization::Test) {
                    return RefKind::Test;
                }
            }
            smelt_core::FileMetadata::Multi { models } => {
                if models
                    .iter()
                    .all(|s| s.metadata.materialization == Some(smelt_core::Materialization::Test))
                    && !models.is_empty()
                {
                    return RefKind::Test;
                }
            }
            smelt_core::FileMetadata::Empty => {}
            // Generator files produce models via meta-language evaluation;
            // they are not test files.
            smelt_core::FileMetadata::Generator { .. } => {}
        }
    }
    // 3. Default: Model.
    RefKind::Model
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

        // Only SQL files can be models.
        let file_path = file.path(db);
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let path_str = file_path.to_str().unwrap_or("");
        if ext != "sql" && !path_str.contains(".sql::") {
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

/// Map a planner-rule diagnostic code onto smelt-db's diagnostic-code
/// catalogue. The 1:1 mapping is the seam the Diagnostic-parity rule relies on
/// (`architecture.md` §"Planner scope").
fn rule_diagnostic_code(code: smelt_planner::RuleDiagnosticCode) -> DiagnosticCode {
    use smelt_planner::RuleDiagnosticCode as R;
    match code {
        R::CumulativeRequiresGroupBy => DiagnosticCode::CumulativeRequiresGroupBy,
        R::CumulativeUnknownAggregator => DiagnosticCode::CumulativeUnknownAggregator,
        R::CumulativeGroupByContainsPartitionColumn => {
            DiagnosticCode::CumulativeGroupByContainsPartitionColumn
        }
        R::CumulativeForbidsWindowFunctions => DiagnosticCode::CumulativeForbidsWindowFunctions,
        R::CumulativeForbidsNondeterministic => DiagnosticCode::CumulativeForbidsNondeterministic,
        R::CumulativeNoDrivingSource => DiagnosticCode::CumulativeNoDrivingSource,
        R::CumulativeMultipleDrivingSources => DiagnosticCode::CumulativeMultipleDrivingSources,
        R::CumulativeSqlNotParseable => DiagnosticCode::CumulativeSqlNotParseable,
        R::IncrementalNotBatchSafe => DiagnosticCode::IncrementalNotBatchSafe,
    }
}

/// Resolve a `smelt.<path>` ref string to its definition's frontmatter
/// `timeseries:` block, when it resolves to a model that declares one. This
/// reconstructs (project-scoped) the `smelt.<path> → timeseries` lookup the
/// runtime builds from the model graph, so the cumulative classifier sees the
/// same driving sources in the editor as it does at build time.
fn ref_timeseries_config(
    db: &dyn salsa::Database,
    workspace: Workspace,
    ref_str: &str,
) -> Option<smelt_core::config::TimeseriesConfig> {
    let segments: Vec<String> = ref_str
        .strip_prefix("smelt.")?
        .split('.')
        .map(|s| s.to_string())
        .collect();
    let leaf = segments.last()?.clone();
    let resolved = resolve_ref_path(db, workspace, segments)?;
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
    // this block. Calls parse_frontmatter(text, Model) to surface unknown-key
    // errors and inapplicable-key warnings. Also tries to deserialize
    // ModelMetadata from the validated map to catch nested sub-field failures
    // (e.g. a bad timeseries.granularity value) that would previously be swallowed.
    let is_function_file = {
        let p = parse_file(db, file);
        AstFile::cast(p.syntax())
            .map(|ast| ast.defines().next().is_some() || ast.externs().next().is_some())
            .unwrap_or(false)
    };
    if !is_function_file {
        if let Some(yaml_text) = smelt_core::frontmatter_yaml_text(text) {
            use smelt_core::{FrontmatterSeverity, ModelMetadata};
            let (validated_map, fm_diags) =
                smelt_core::parse_frontmatter(&yaml_text, smelt_core::DeclarationKind::Model);

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
                smelt_core::metadata::MetadataError::TimeseriesRequiredForIncremental => Some((
                    ts_err.to_string(),
                    DiagnosticCode::TimeseriesRequiredForIncremental,
                )),
                smelt_core::metadata::MetadataError::MalformedTimeseries { .. } => {
                    Some((ts_err.to_string(), DiagnosticCode::MalformedTimeseries))
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

        // Built-in planner-rule diagnostics (cumulative classifier, incremental
        // batch-safety) surfaced through the uniform rule → diagnostics
        // interface. The checks live in `smelt-planner` (analysis-pure); this
        // query only gathers inputs and aggregates, so the editor and the build
        // reach an identical verdict (architecture.md §"Diagnostic parity rule"
        // + §"Planner scope"). Anchored at the model SQL body start.
        let materialization = if metadata.materialization
            == Some(smelt_core::config::Materialization::CumulativeAggregate)
        {
            "cumulative_aggregate"
        } else if metadata.incremental.is_some() {
            "incremental"
        } else {
            ""
        };
        if !materialization.is_empty() {
            let stripped = smelt_parser::strip_frontmatter(text);
            let refs = smelt_planner::collect_path_refs(&stripped);
            // The cumulative classifier resolves its driving source by looking
            // each ref up in this map; the incremental rule does not use it.
            let mut source_timeseries: smelt_planner::SourceTimeseriesMap = HashMap::new();
            if materialization == "cumulative_aggregate" {
                for r in &refs {
                    if let Some(ts) = ref_timeseries_config(db, workspace, r) {
                        source_timeseries.insert(r.clone(), ts);
                    }
                }
            }
            let model_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let ctx = smelt_planner::RuleContext {
                model_name: &model_name,
                materialization,
                sql: &stripped,
                refs: &refs,
                source_timeseries: &source_timeseries,
                timeseries_config: metadata.timeseries.as_ref(),
                incremental_config: metadata.incremental.as_ref(),
            };
            let body_start = rowan::TextSize::from(sql_offset as u32);
            for rd in smelt_planner::detect_builtin_rules(&ctx) {
                DiagnosticAcc(Diagnostic {
                    severity: match rd.severity {
                        smelt_planner::RuleSeverity::Error => DiagnosticSeverity::Error,
                        smelt_planner::RuleSeverity::Warning => DiagnosticSeverity::Warning,
                    },
                    message: rd.message,
                    range: rowan::TextRange::empty(body_start),
                    code: Some(rule_diagnostic_code(rd.code)),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }

    // Parse errors
    let parse = parse_file(db, file);
    for error in parse.errors.iter() {
        let range = error.range;
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            range,
            code: Some(DiagnosticCode::ParseError),
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

    // Check if model is valid
    if parse_model(db, file).is_none() {
        let path_str = path.to_str().unwrap_or("");
        let is_virtual_submodel = path_str.contains("::");
        if !is_virtual_submodel && path_str.contains("models/") {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "File does not contain a valid SQL query".to_string(),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::InvalidModel),
                data: None,
            })
            .accumulate(db);
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

    if !sources.is_empty() {
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
// Kept in lib.rs because the call sites are pre-gathered with mixed Salsa
// queries (resolve_function, file_signature_inputs, parse_file,
// project_unstable_schema, workspace_function_bodies, function_call_cycle_fn_ids)
// and the only consumer is the planner. The pure plan builder
// (`build_logical_plan_pure`) follows the pure-function rule.

use smelt_parser::ast::SmeltPathCall;

/// Pre-gathered inputs for one `smelt.fn.*` call site, collected by the Salsa
/// query before passing to the pure plan builder.
struct FnCallInput {
    fn_id: String,
    transparent: bool,
    properties: smelt_planner::logical::FunctionProperties,
    /// Resolved provenance: either the declared provenance (when the workspace
    /// opted in to `unstable_schema`) or `Unknown`.
    provenance: smelt_planner::logical::Provenance,
    /// Phase 41: the callee's body text, captured eagerly by the Salsa query.
    /// `None` for opaque calls, unresolved references, and calls suppressed by
    /// the cycle pre-pass.  When `Some`, the body is attached to the
    /// `FunctionCall` plan node as a `LogicalNode::Raw { sql_text }` subtree;
    /// the Phase 41 expansion rule clones it into the resulting `ExpandedCall`.
    body_text: Option<String>,
}

/// Build a [`smelt_planner::logical::Plan`] from a single source file.
///
/// This tracked query gathers all Salsa inputs — the parsed AST, resolved
/// signatures, and per-declaration frontmatter — then delegates to the pure
/// helper [`build_logical_plan_pure`] which takes no `db` reference.
///
/// Returns `None` when the file does not parse as a valid SQL model.
#[salsa::tracked]
pub fn logical_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_planner::logical::Plan> {
    use smelt_planner::logical::Provenance;

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
    let call_inputs: Vec<FnCallInput> = ast
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
                                smelt_planner::logical::parse_function_properties(&text, kind).0
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

            FnCallInput {
                fn_id,
                transparent,
                properties,
                provenance: resolved_provenance,
                body_text,
            }
        })
        .collect();

    Some(build_logical_plan_pure(call_inputs))
}

/// Pure plan builder — takes no `db` reference and calls no Salsa queries.
///
/// Constructs a minimal `Select` root with the first collected `FunctionCall`
/// as its `from` child. Phase 32+ replaces this with a full projection tree.
fn build_logical_plan_pure(call_inputs: Vec<FnCallInput>) -> smelt_planner::logical::Plan {
    use smelt_planner::logical::LogicalNode;

    let fn_call_nodes: Vec<Arc<LogicalNode>> = call_inputs
        .into_iter()
        .map(|input| {
            let body = input
                .body_text
                .map(|t| Arc::new(LogicalNode::Raw { sql_text: t }));
            Arc::new(LogicalNode::FunctionCall {
                fn_id: input.fn_id,
                args: Vec::new(), // Phase 30 stub — arg sub-plans deferred to Phase 32+
                transparent: input.transparent,
                provenance: input.provenance,
                properties: input.properties,
                pushed_filter: None,
                body,
            })
        })
        .collect();

    if fn_call_nodes.is_empty() {
        Arc::new(LogicalNode::Select {
            projections: Vec::new(),
            from: None,
            filter: None,
        })
    } else {
        let first = fn_call_nodes.into_iter().next().unwrap();
        Arc::new(LogicalNode::Select {
            projections: Vec::new(),
            from: Some(first),
            filter: None,
        })
    }
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
