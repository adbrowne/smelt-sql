//! The LSP backend: `Backend` struct and `LanguageServer` trait impl.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use smelt_core::{
    find_config_file, find_project_root_by_walking_up, find_project_root_for_file,
    find_smelt_projects, is_sources_file,
    metadata::{extract_file_metadata, FileMetadata},
};
use smelt_db::{
    functions_in_file, yaml_edits::find_source_column_yaml_rename, Database,
    Diagnostic as DbDiagnostic, DiagnosticCode as DbCode, DiagnosticData as DbData,
    DiagnosticSeverity as DbSeverity, ProjectInput, SourceFile, Workspace,
};
use smelt_parser::ast::File as AstFile;
use smelt_parser::is_valid_sql_identifier;
use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};
use smelt_types::{format_smelt_type_hover, TypedColumn};

use crate::column_resolution::{
    build_python_context, collect_from_model_names, format_type, resolve_column_definitions,
    trace_upstream_column, ColumnDefLocation,
};
use crate::completion::{
    determine_completion_context, extract_from_aliases, AliasTarget, CompletionContext,
};
use crate::db_helpers::{
    all_file_paths, diagnostics_for, file_project_root, file_text, lookup_file, lookup_project,
    project_sources_yaml,
};
use crate::hover::{
    column_ref_field_completions,
    columns_of_arg_completions_for_sql,
    // Phase E2 goto-def + completion helpers
    completion_for_generates_value,
    completion_for_model_def_field_key,
    completion_item_for_if_snippet,
    completion_items_for_reduce_second_arg_with_snippets,
    find_smelt_fn_call_at_cursor,
    find_var_line_in_smelt_yml,
    goto_def_for_emitted_model_reference,
    hover_text_for_column_ref_field,
    hover_text_for_columns_of_call,
    hover_text_for_generates_frontmatter,
    hover_text_for_hof_meta_language,
    hover_text_for_list_literal_dual,
    hover_text_for_list_spread,
    hover_text_for_model_def_body_field_value,
    hover_text_for_model_def_literal_open_brace,
    hover_text_for_model_def_name_field_value,
    hover_text_for_model_def_optional_field_value,
    hover_text_for_model_ref_field,
    hover_text_for_models_all,
    hover_text_for_models_with_tag_call,
    hover_text_for_pipe_expr,
    hover_text_for_source_ref_field,
    hover_text_for_sources_all,
    hover_text_for_sources_with_tag_call,
    is_column_ref_param_before_dot,
    is_model_ref_param_before_dot,
    is_source_ref_param_before_dot,
    lambda_param_binder_range,
    lambda_params_for_completion,
    model_ref_field_completions,
    passing_body_aggregate_labels,
    passing_body_completion_columns,
    render_expansion_frames,
    source_ref_field_completions,
    wide_reflection_accessor_completions,
};
use crate::python_scan::PythonModelCache;

/// Tracks errors that occurred during workspace initialization
#[derive(Default)]
pub(crate) struct InitErrors {
    pub(crate) workspace_errors: Vec<String>,
    pub(crate) source_errors: Vec<String>,
    pub(crate) model_errors: Vec<String>,
}

impl InitErrors {
    pub(crate) fn has_errors(&self) -> bool {
        !self.workspace_errors.is_empty()
            || !self.source_errors.is_empty()
            || !self.model_errors.is_empty()
    }

    pub(crate) fn total_count(&self) -> usize {
        self.workspace_errors.len() + self.source_errors.len() + self.model_errors.len()
    }
}

/// (virtual_path, start_line_offset) for each section in a multi-model file.
pub(crate) type MultiModelEntry = Vec<(PathBuf, u32)>;

pub struct Backend {
    client: Client,
    /// The salsa database. `Database: Clone` with internally-Arc'd storage, so
    /// reads snapshot via `self.snapshot()` (lock briefly, clone, drop lock).
    /// Writes lock the mutex for the duration of the input mutation.
    db: Arc<Mutex<Database>>,
    /// Tracked set of file paths registered in the DB (mirror of the
    /// Workspace singleton's `files`). Kept in sync whenever inputs change.
    tracked_files: Arc<Mutex<Vec<PathBuf>>>,
    /// Errors collected during initialization, reported after `initialized` notification
    init_errors: Arc<Mutex<Option<InitErrors>>>,
    /// Maps virtual .sql paths (used in Salsa) back to actual .py source paths + decorator line
    /// for goto-definition. The u32 is the 0-indexed line of the `@model` decorator.
    python_model_sources: Arc<Mutex<HashMap<PathBuf, (PathBuf, u32)>>>,
    /// Cache of Python model results (keyed by content hash)
    python_cache: Arc<Mutex<PythonModelCache>>,
    /// Diagnostics for Python files (separate from Salsa-managed SQL diagnostics)
    python_diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<lsp_types::Diagnostic>>>>,
    /// Project roots discovered during init (needed for file-change handling)
    project_roots: Arc<Mutex<Vec<PathBuf>>>,
    /// Maps real file paths to their virtual sub-paths for multi-model files.
    /// Each entry is (virtual_path, start_line_offset) where virtual_path uses
    /// the `file.sql::model_name` convention and start_line_offset is the
    /// 0-based line in the original file where the section's SQL begins.
    multi_model_files: Arc<Mutex<HashMap<PathBuf, MultiModelEntry>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            db: Arc::new(Mutex::new(Database::default())),
            tracked_files: Arc::new(Mutex::new(Vec::new())),
            init_errors: Arc::new(Mutex::new(None)),
            python_model_sources: Arc::new(Mutex::new(HashMap::new())),
            python_cache: Arc::new(Mutex::new(PythonModelCache::default())),
            python_diagnostics: Arc::new(Mutex::new(HashMap::new())),
            project_roots: Arc::new(Mutex::new(Vec::new())),
            multi_model_files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Convert URI to file path, logging a warning if conversion fails.
    /// Returns None for non-file URIs (e.g., untitled:, git:).
    async fn uri_to_path(&self, uri: &Url) -> Option<PathBuf> {
        match uri.to_file_path() {
            Ok(p) => Some(p),
            Err(_) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Cannot process non-file URI: {}", uri),
                    )
                    .await;
                None
            }
        }
    }

    /// Convert (PathBuf, Range) reference locations to LSP Location objects.
    async fn ref_locations_to_lsp(
        &self,
        refs: &[(PathBuf, smelt_parser::ast::Range)],
    ) -> Vec<Location> {
        let py_sources = self.python_model_sources.lock().await;
        refs.iter()
            .filter_map(|(path, range)| {
                let (actual_path, line_offset) = py_sources
                    .get(path)
                    .map(|(p, line)| (p.clone(), *line))
                    .unwrap_or((path.clone(), 0));
                let uri = Url::from_file_path(&actual_path).ok()?;
                Some(Location {
                    uri,
                    range: Range {
                        start: Position::new(range.start.line + line_offset, range.start.column),
                        end: Position::new(range.end.line + line_offset, range.end.column),
                    },
                })
            })
            .collect()
    }

    /// Convert our database diagnostic to LSP diagnostic
    fn to_lsp_diagnostic(&self, diag: &DbDiagnostic) -> lsp_types::Diagnostic {
        let code = diag.code.map(|c| {
            let code_str = match c {
                DbCode::ParseError => "parse-error",
                DbCode::InvalidModel => "invalid-model",
                DbCode::UndefinedModelRef => "undefined-model-ref",
                DbCode::UndefinedSource => "undefined-source",
                DbCode::CannotInferType => "cannot-infer-type",
                DbCode::UndeclaredColumn => "undeclared-column",
                DbCode::TypeMismatch => "type-mismatch",
                DbCode::CircularDependency => "circular-dependency",
                DbCode::UnsupportedConstruct => "unsupported-construct",
                DbCode::YamlParseError => "yaml-parse-error",
                DbCode::SourceTypeError => "source-type-error",
                DbCode::MalformedSource => "malformed-source",
                DbCode::AmbiguousColumn => "ambiguous-column",
                DbCode::UnknownCastType => "unknown-cast-type",
                DbCode::UnrecognizedFunction => "unrecognized-function",
                DbCode::DuplicateFunctionDefinition => "duplicate-function-definition",
                DbCode::InvalidFunctionTypeRef => "invalid-function-type-ref",
                DbCode::FunctionBodyTypeMismatch => "function-body-type-mismatch",
                DbCode::UnknownIdentifier => "unknown-identifier",
                DbCode::DuplicateParameterName => "duplicate-parameter-name",
                DbCode::UnknownSmeltFn => "unknown-smelt-fn",
                DbCode::MissingArgument => "missing-argument",
                DbCode::ArgTypeMismatch => "arg-type-mismatch",
                DbCode::ExternCollidesWithBuiltin => "extern-collides-with-builtin",
                DbCode::BackendsWideningNotAllowed => "backends-widening-not-allowed",
                DbCode::WindowInScalarContext => "window-in-scalar-context",
                DbCode::ParameterShadowsColumn => "parameter-shadows-column",
                DbCode::RowRequirementUnsatisfied => "row-requirement-unsatisfied",
                DbCode::UnknownContext => "unknown-context",
                DbCode::CteCycle => "cte-cycle",
                DbCode::ContextMismatch => "context-mismatch",
                DbCode::FragmentColumnMissing => "fragment-column-missing",
                DbCode::AnnotationTooWide => "annotation-too-wide",
                DbCode::FragmentKindMismatch => "fragment-kind-mismatch",
                DbCode::ReturnTypeMismatch => "return-type-mismatch",
                DbCode::UnknownPassingParameter => "unknown-passing-parameter",
                DbCode::UnstableSchemaRequired => "unstable-schema-required",
                DbCode::AsStructUnsupportedBackend => "as-struct-unsupported-backend",
                DbCode::FunctionCallCycle => "function-call-cycle",
                DbCode::FrontmatterParseError => "frontmatter-parse-error",
                DbCode::ProvenanceMismatch => "provenance-mismatch",
                DbCode::JoinsMismatch => "joins-mismatch",
                DbCode::DeclaredCardinalityUnverifiable => "declared-cardinality-unverifiable",
                DbCode::MissingProvenancePushdownAdvisory => "missing-provenance-pushdown-advisory",
                DbCode::ExternFragmentParamUnsupported => "extern-fragment-param-unsupported",
                DbCode::KindMismatch => "kind-mismatch",
                DbCode::MissingSeedSidecar => "missing-seed-sidecar",
                // Phase A (meta-language) diagnostic codes.
                DbCode::MetaListEmptyTypeUnknown => "meta-list-empty-type-unknown",
                DbCode::MetaListHeterogeneous => "meta-list-heterogeneous",
                DbCode::MetaSpreadInForbiddenPosition => "meta-spread-in-forbidden-position",
                DbCode::MetaSpreadOnNonList => "meta-spread-on-non-list",
                // Phase B (meta-language) diagnostic codes.
                DbCode::LambdaInForbiddenPosition => "lambda-in-forbidden-position",
                DbCode::LambdaArityMismatch => "lambda-arity-mismatch",
                DbCode::LambdaZeroParameters => "lambda-zero-parameters",
                DbCode::LambdaDuplicateParameter => "lambda-duplicate-parameter",
                DbCode::LambdaResultTypeMismatch => "lambda-result-type-mismatch",
                DbCode::HofExpectsLambda => "hof-expects-lambda",
                DbCode::HofExpectsReducer => "hof-expects-reducer",
                DbCode::HofNameShadowed => "hof-name-shadowed",
                DbCode::ReducerNameShadowed => "reducer-name-shadowed",
                DbCode::PipeRhsNotCall => "pipe-rhs-not-call",
                DbCode::PipeInDataPosition => "pipe-in-data-position",
                DbCode::ReducerInputTypeMismatch => "reducer-input-type-mismatch",
                DbCode::ReducerEmptyNoIdentity => "reducer-empty-no-identity",
                // Phase F (meta-language) parameterised reducer + ternary codes.
                DbCode::ReducerArityMismatch => "reducer-arity-mismatch",
                DbCode::ReducerArgTypeMismatch => "reducer-arg-type-mismatch",
                DbCode::ReducerArgNotCompileTime => "reducer-arg-not-compile-time",
                DbCode::ReducerNamedArgument => "reducer-named-argument",
                DbCode::TernaryConditionNotBoolean => "ternary-condition-not-boolean",
                DbCode::TernaryBranchTypeMismatch => "ternary-branch-type-mismatch",
                DbCode::TernaryKeywordShadowed => "ternary-keyword-shadowed",
                DbCode::TernaryInDataPosition => "ternary-in-data-position",
                DbCode::TernaryDanglingThen => "ternary-dangling-then",
                DbCode::TernaryDanglingElse => "ternary-dangling-else",
                DbCode::ConfigVarNotFound => "config-var-not-found",
                DbCode::ConfigVarNameNotLiteral => "config-var-name-not-literal",
                DbCode::ConfigVarNullCoercion => "config-var-null-coercion",
                // Phase C (meta-language) diagnostic codes.
                DbCode::ColumnsOfRequiresTableExpr => "columns-of-requires-table-expr",
                DbCode::ColumnsOfNamedArgument => "columns-of-named-argument",
                DbCode::ColumnRefFieldUnknown => "column-ref-field-unknown",
                DbCode::ColumnsOfUnresolvableSchema => "columns-of-unresolvable-schema",
                // Phase D (meta-language) diagnostic codes.
                DbCode::WithTagRequiresText => "with-tag-requires-text",
                DbCode::WithTagNamedArgument => "with-tag-named-argument",
                DbCode::WideReflectionUnknownAccessor => "wide-reflection-unknown-accessor",
                DbCode::WideReflectionUnexpectedArgument => "wide-reflection-unexpected-argument",
                DbCode::ModelRefFieldUnknown => "model-ref-field-unknown",
                DbCode::SourceRefFieldUnknown => "source-ref-field-unknown",
                // Phase E1 (meta-language) record diagnostic codes.
                DbCode::SmeltRecordRedefinition => "smelt-record-redefinition",
                DbCode::RecordFieldUnknown => "record-field-unknown",
                DbCode::RecordFieldMissing => "record-field-missing",
                DbCode::RecordFieldDuplicate => "record-field-duplicate",
                DbCode::RecordFieldTypeMismatch => "record-field-type-mismatch",
                DbCode::RecordLiteralUnknownTarget => "record-literal-unknown-target",
                DbCode::RecordFieldNotProjectable => "record-field-not-projectable",
                DbCode::RecordFieldTypeForbidden => "record-field-type-forbidden",
                DbCode::RecordCyclicDeclaration => "record-cyclic-declaration",
                DbCode::RecordInDataWorld => "record-in-data-world",
                // Phase E1 (meta-language) map diagnostic codes.
                DbCode::MapKeyTypeNotText => "map-key-type-not-text",
                DbCode::MapApiUnknown => "map-api-unknown",
                DbCode::MapApiArityMismatch => "map-api-arity-mismatch",
                DbCode::MapApiNamedArgument => "map-api-named-argument",
                DbCode::MapApiUnexpectedArgument => "map-api-unexpected-argument",
                DbCode::MapGetMissingKey => "map-get-missing-key",
                DbCode::MapApiArgTypeMismatch => "map-api-arg-type-mismatch",
                // Phase E1 (meta-language) loader diagnostic codes.
                DbCode::ConfigLoaderPathNotLiteral => "config-loader-path-not-literal",
                DbCode::ConfigLoaderPathEscapesWorkspace => "config-loader-path-escapes-workspace",
                DbCode::ConfigLoaderPathBackslash => "config-loader-path-backslash",
                DbCode::ConfigLoaderFileNotFound => "config-loader-file-not-found",
                DbCode::ConfigLoaderSchemaForbidden => "config-loader-schema-forbidden",
                DbCode::ConfigLoaderTomlNotYetSupported => "config-loader-toml-not-yet-supported",
                DbCode::ConfigLoaderParseError => "config-loader-parse-error",
                DbCode::ConfigLoaderRequiredFieldMissing => "config-loader-required-field-missing",
                DbCode::ConfigLoaderUnknownField => "config-loader-unknown-field",
                DbCode::ConfigLoaderTypeMismatch => "config-loader-type-mismatch",
                DbCode::ConfigLoaderRootShapeMismatch => "config-loader-root-shape-mismatch",
                DbCode::ConfigLoaderDuplicateMapKey => "config-loader-duplicate-map-key",
                DbCode::ConfigLoaderNullCoercion => "config-loader-null-coercion",
                // Multi-model production diagnostic codes.
                DbCode::GeneratesUnknownValue => "generates-unknown-value",
                DbCode::GeneratesMixedWithBareModel => "generates-mixed-with-bare-model",
                DbCode::GenerateFileBareSelectForbidden => "generate-file-bare-select-forbidden",
                DbCode::GenerateFileBodyTypeError => "generate-file-body-type-error",
                DbCode::ModelDefOutsideGeneratorFile => "model-def-outside-generator-file",
                DbCode::ModelDefInvalidName => "model-def-invalid-name",
                DbCode::ModelDefInvalidMaterialization => "model-def-invalid-materialization",
                DbCode::ModelDefDuplicateName => "model-def-duplicate-name",
                DbCode::ModelDefHandAuthoredCollision => "model-def-hand-authored-collision",
                DbCode::GeneratorBodyForbidsModelReflection => {
                    "generator-body-forbids-model-reflection"
                }
            };
            NumberOrString::String(code_str.to_string())
        });

        let data = diag.data.as_ref().map(|d| match d {
            DbData::UndefinedRef { model_name } => {
                serde_json::json!({ "kind": "undefined-ref", "modelName": model_name })
            }
            DbData::UndefinedSource {
                source_name,
                table_name,
            } => {
                serde_json::json!({ "kind": "undefined-source", "sourceName": source_name, "tableName": table_name })
            }
            DbData::CannotInferType { column_name } => {
                serde_json::json!({ "kind": "cannot-infer-type", "columnName": column_name })
            }
            DbData::UndeclaredColumn {
                qualifier,
                column_name,
            } => {
                serde_json::json!({ "kind": "undeclared-column", "qualifier": qualifier, "columnName": column_name })
            }
            DbData::TypeMismatch {
                column_name,
                ref_name,
                actual_type,
                expected_type,
            } => {
                serde_json::json!({
                    "kind": "type-mismatch",
                    "columnName": column_name,
                    "refName": ref_name,
                    "actualType": actual_type,
                    "expectedType": expected_type
                })
            }
            DbData::ExpansionFrames(frames) => {
                let frames_json: Vec<_> = frames
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "function": f.function,
                            "param": f.param,
                            "boundType": f.bound_type,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "kind": "expansion-frames",
                    "frames": frames_json,
                })
            }
            DbData::MissingSeedSidecar {
                csv_path,
                sidecar_path,
            } => {
                serde_json::json!({
                    "kind": "missing-seed-sidecar",
                    "csvPath": csv_path,
                    "sidecarPath": sidecar_path,
                })
            }
        });

        // Phase 12 (smelt-functions Step 1): expand the message body and
        // `DiagnosticRelatedInformation` list from the diagnostic's
        // `ExpansionFrames` payload. The pure helper below is unit-testable
        // directly (see `render_expansion_frames` tests).
        let (message, related_information) = render_expansion_frames(diag);

        lsp_types::Diagnostic {
            range: Range {
                start: Position {
                    line: diag.range.start.line,
                    character: diag.range.start.column,
                },
                end: Position {
                    line: diag.range.end.line,
                    character: diag.range.end.column,
                },
            },
            severity: Some(match diag.severity {
                DbSeverity::Error => DiagnosticSeverity::ERROR,
                DbSeverity::Warning => DiagnosticSeverity::WARNING,
                DbSeverity::Info => DiagnosticSeverity::INFORMATION,
                DbSeverity::Hint => DiagnosticSeverity::HINT,
            }),
            message,
            source: Some("smelt".to_string()),
            code,
            data,
            related_information,
            ..Default::default()
        }
    }

    /// For a multi-model file, resolve a cursor position to the virtual path
    /// and adjusted line number within that section. Returns None for single-model files.
    async fn resolve_virtual_path(
        &self,
        real_path: &std::path::Path,
        line: u32,
    ) -> Option<(PathBuf, u32)> {
        let mm = self.multi_model_files.lock().await;
        let entries = mm.get(real_path)?;

        // Find the section that contains this line (last section whose start_line <= line)
        let mut best: Option<&(PathBuf, u32)> = None;
        for entry in entries {
            if entry.1 <= line {
                best = Some(entry);
            }
        }

        best.map(|(vp, start_line)| (vp.clone(), line - start_line))
    }

    /// Register a SQL file's content in the Salsa database, handling multi-model
    /// files by splitting them into virtual paths.
    ///
    /// Returns the list of paths that were registered (either `[real_path]` for
    /// single-model files, or `[real_path::name1, real_path::name2, ...]` for
    /// multi-model files).
    async fn register_sql_content(
        &self,
        db: &mut Database,
        real_path: &std::path::Path,
        content: &str,
        project_root: &std::path::Path,
    ) -> Vec<PathBuf> {
        let mut registered = Vec::new();

        // Try to detect multi-model file
        if let Ok(FileMetadata::Multi { models }) = extract_file_metadata(content) {
            let mut virtual_entries = Vec::new();

            for section in &models {
                let model_name = match &section.metadata.name {
                    Some(n) => n.clone(),
                    None => continue,
                };

                let virtual_path =
                    PathBuf::from(format!("{}::{}", real_path.display(), model_name));
                let sql_content = &content[section.sql_range.clone()];

                // Calculate the starting line of this section's SQL in the original file
                let start_line = content[..section.sql_range.start]
                    .chars()
                    .filter(|&c| c == '\n')
                    .count() as u32;

                // Upsert the SourceFile input. `set_source_file` only mutates the
                // underlying text/project_root when they differ, so spurious
                // revision bumps are avoided.
                let should_update = match db.source_file(&virtual_path) {
                    Some(f) => f.text(db) != sql_content,
                    None => true,
                };
                if should_update {
                    db.set_source_file(
                        virtual_path.clone(),
                        sql_content.to_string(),
                        project_root.to_path_buf(),
                    );
                }

                virtual_entries.push((virtual_path.clone(), start_line));
                registered.push(virtual_path);
            }

            // Store the mapping for diagnostics aggregation
            let mut mm = self.multi_model_files.lock().await;
            mm.insert(real_path.to_path_buf(), virtual_entries);
        } else {
            // Single-model or no frontmatter: register as-is
            let path_buf = real_path.to_path_buf();
            let should_update = match db.source_file(&path_buf) {
                Some(f) => f.text(db) != content,
                None => true,
            };
            if should_update {
                db.set_source_file(
                    path_buf.clone(),
                    content.to_string(),
                    project_root.to_path_buf(),
                );
            }
            registered.push(path_buf);

            // Clean up any old multi-model mapping
            let mut mm = self.multi_model_files.lock().await;
            mm.remove(real_path);
        }

        registered
    }

    /// Snapshot the DB under the write lock and return a cheap clone for
    /// lock-free reads. `Database: Clone` shares salsa storage internally via
    /// `Arc`, so this is a constant-time, memory-cheap operation.
    async fn snapshot(&self) -> Database {
        self.db.lock().await.clone()
    }

    /// Rebuild the `Workspace` singleton from every currently-registered
    /// `SourceFile` + `ProjectInput`. Call after any input-set change so
    /// `all_models` / `resolve_ref` / diagnostics see the new set.
    fn sync_workspace(db: &mut Database, paths: &[PathBuf], project_roots: &[PathBuf]) {
        let files: Vec<SourceFile> = paths.iter().filter_map(|p| db.source_file(p)).collect();
        let projects: Vec<ProjectInput> = project_roots
            .iter()
            .filter_map(|r| db.project_input(r))
            .collect();
        db.set_workspace(files, projects);
    }

    /// Publish diagnostics for a file
    async fn publish_diagnostics(&self, uri: Url) {
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Check if this is a multi-model file
        let mm = self.multi_model_files.lock().await;
        let multi_entries = mm.get(&path).cloned();
        drop(mm);

        let db = self.snapshot().await;

        let lsp_diagnostics: Vec<lsp_types::Diagnostic> =
            if let Some(virtual_entries) = multi_entries {
                let mut lsp_diagnostics = Vec::new();
                for (virtual_path, start_line) in virtual_entries {
                    let diagnostics = diagnostics_for(&db, &virtual_path);
                    for d in &diagnostics {
                        let mut lsp_diag = self.to_lsp_diagnostic(d);
                        lsp_diag.range.start.line += start_line;
                        lsp_diag.range.end.line += start_line;
                        lsp_diagnostics.push(lsp_diag);
                    }
                }
                lsp_diagnostics
            } else {
                diagnostics_for(&db, &path)
                    .iter()
                    .map(|d| self.to_lsp_diagnostic(d))
                    .collect()
            };

        self.client
            .publish_diagnostics(uri, lsp_diagnostics, None)
            .await;
    }

    /// Publish diagnostics for all known model files
    async fn publish_all_diagnostics(&self) {
        let files = self.tracked_files.lock().await.clone();

        // Collect real file paths for multi-model files
        let mm = self.multi_model_files.lock().await;
        let multi_model_real_paths: Vec<PathBuf> = mm.keys().cloned().collect();
        // Collect all virtual paths so we can skip them in the main loop
        let virtual_paths: std::collections::HashSet<PathBuf> = mm
            .values()
            .flat_map(|entries| entries.iter().map(|(vp, _)| vp.clone()))
            .collect();
        drop(mm);

        for path in files.iter() {
            // Skip virtual paths — they'll be handled via their real file
            if virtual_paths.contains(path) {
                continue;
            }
            if let Ok(uri) = Url::from_file_path(path) {
                self.publish_diagnostics(uri).await;
            }
        }

        // Publish diagnostics for multi-model real files
        for path in &multi_model_real_paths {
            if let Ok(uri) = Url::from_file_path(path) {
                self.publish_diagnostics(uri).await;
            }
        }
    }

    /// Handle a Python model file change: re-execute and update Salsa.
    /// Uses background execution with last-known-good fallback on failure.
    async fn handle_python_file_change(&self, py_path: &std::path::Path) {
        // Find the project root for this file
        let project_roots = self.project_roots.lock().await.clone();
        let project_root = match find_project_root_for_file(py_path, &project_roots) {
            Some(root) => root,
            None => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!(
                            "Cannot find project root for Python model: {}",
                            py_path.display()
                        ),
                    )
                    .await;
                return;
            }
        };

        let py_path = py_path.to_path_buf();
        let db = self.db.clone();
        let tracked_files = self.tracked_files.clone();
        let project_roots_handle = self.project_roots.clone();
        let py_sources = self.python_model_sources.clone();
        let py_diags = self.python_diagnostics.clone();
        let cache = self.python_cache.clone();
        let client = self.client.clone();

        // Build context from current model list
        let context_json = {
            let all_files = self.tracked_files.lock().await.clone();
            let config =
                smelt_core::Config::load(&project_root).unwrap_or_else(|_| smelt_core::Config {
                    name: String::new(),
                    version: 1,
                    paths: vec!["models".to_string()],
                    targets: std::collections::HashMap::new(),
                    default_materialization: smelt_core::Materialization::View,
                    models: std::collections::HashMap::new(),
                    python: None,
                });
            build_python_context(&all_files, &config)
        };

        // Spawn background task for subprocess execution
        tokio::task::spawn(async move {
            let py_path_for_blocking = py_path.clone();
            let project_root_for_blocking = project_root.clone();
            let cache_for_blocking = cache.clone();

            let scan_result = tokio::task::spawn_blocking(move || {
                let mut cache_guard = cache_for_blocking.blocking_lock();
                crate::python_scan::execute_single_python_file(
                    &py_path_for_blocking,
                    &project_root_for_blocking,
                    &mut cache_guard,
                    &context_json,
                )
            })
            .await;

            let scan_result = match scan_result {
                Ok(r) => r,
                Err(e) => {
                    client
                        .log_message(
                            MessageType::ERROR,
                            format!("Python model re-execution panicked: {}", e),
                        )
                        .await;
                    return;
                }
            };

            // Update Python diagnostics for this file
            {
                let mut diags = py_diags.lock().await;
                if scan_result.errors.is_empty() {
                    // Clear previous errors
                    diags.remove(&py_path);
                    if let Ok(uri) = Url::from_file_path(&py_path) {
                        client.publish_diagnostics(uri, Vec::new(), None).await;
                    }
                } else {
                    let file_diags: Vec<lsp_types::Diagnostic> = scan_result
                        .errors
                        .iter()
                        .map(|error| {
                            let line = error.line.unwrap_or(1).saturating_sub(1);
                            lsp_types::Diagnostic {
                                range: Range {
                                    start: Position::new(line, 0),
                                    end: Position::new(line, 0),
                                },
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: error.message.clone(),
                                source: Some("smelt-python".to_string()),
                                ..Default::default()
                            }
                        })
                        .collect();
                    diags.insert(py_path.clone(), file_diags.clone());
                    if let Ok(uri) = Url::from_file_path(&py_path) {
                        client.publish_diagnostics(uri, file_diags, None).await;
                    }
                }
            }

            // On failure, keep last-known-good SQL in Salsa (don't update)
            if scan_result.models.is_empty() && !scan_result.errors.is_empty() {
                client
                    .log_message(
                        MessageType::WARNING,
                        format!(
                            "Python model {} failed, keeping last-known-good SQL",
                            py_path.display()
                        ),
                    )
                    .await;
                return;
            }

            // Update Salsa with new SQL
            {
                let mut db_guard = db.lock().await;
                let mut sources = py_sources.lock().await;
                let mut files = tracked_files.lock().await;

                // Remove old virtual paths from this .py file
                let old_virtual_paths: Vec<PathBuf> = sources
                    .iter()
                    .filter(|(_, (src, _))| *src == py_path)
                    .map(|(vp, _)| vp.clone())
                    .collect();

                for old_vp in &old_virtual_paths {
                    sources.remove(old_vp);
                    files.retain(|f| f != old_vp);
                }

                // Register new models (skip mutations when values unchanged)
                for py_model in &scan_result.models {
                    let virtual_sql_path = py_model
                        .source_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(format!("{}.sql", py_model.name));

                    let should_update = match db_guard.source_file(&virtual_sql_path) {
                        Some(f) => f.text(&*db_guard) != &py_model.sql,
                        None => true,
                    };
                    if should_update {
                        db_guard.set_source_file(
                            virtual_sql_path.clone(),
                            py_model.sql.clone(),
                            project_root.clone(),
                        );
                    }
                    sources.insert(
                        virtual_sql_path.clone(),
                        (py_model.source_path.clone(), py_model.decorator_line),
                    );
                    if !files.contains(&virtual_sql_path) {
                        files.push(virtual_sql_path);
                    }
                }

                let project_roots = project_roots_handle.lock().await.clone();
                Backend::sync_workspace(&mut db_guard, &files, &project_roots);
            }

            // Republish all diagnostics since ref resolution may have changed
            let files = tracked_files.lock().await.clone();
            let db_snapshot = db.lock().await.clone();

            for path in files.iter() {
                if let Ok(uri) = Url::from_file_path(path) {
                    let diagnostics = diagnostics_for(&db_snapshot, path);
                    let lsp_diagnostics: Vec<lsp_types::Diagnostic> = diagnostics
                        .iter()
                        .map(|d| lsp_types::Diagnostic {
                            range: Range {
                                start: Position {
                                    line: d.range.start.line,
                                    character: d.range.start.column,
                                },
                                end: Position {
                                    line: d.range.end.line,
                                    character: d.range.end.column,
                                },
                            },
                            severity: Some(match d.severity {
                                DbSeverity::Error => DiagnosticSeverity::ERROR,
                                DbSeverity::Warning => DiagnosticSeverity::WARNING,
                                DbSeverity::Info => DiagnosticSeverity::INFORMATION,
                                DbSeverity::Hint => DiagnosticSeverity::HINT,
                            }),
                            message: d.message.clone(),
                            source: Some("smelt".to_string()),
                            ..Default::default()
                        })
                        .collect();
                    client.publish_diagnostics(uri, lsp_diagnostics, None).await;
                }
            }

            client
                .log_message(
                    MessageType::INFO,
                    format!(
                        "Python model {} re-executed successfully ({} model(s))",
                        py_path.display(),
                        scan_result.models.len()
                    ),
                )
                .await;
        });
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut init_errors = InitErrors::default();

        // Initialize inputs to empty first - ensures Salsa queries are always set
        // even if workspace folders aren't provided or models/ doesn't exist
        {
            let mut db = self.db.lock().await;
            Backend::sync_workspace(&mut db, &[], &[]);
        }

        // Get workspace folders if provided
        if let Some(workspace_folders) = params.workspace_folders {
            let mut db = self.db.lock().await;
            let mut all_files = Vec::new();
            let mut all_project_roots = Vec::new();

            for folder in &workspace_folders {
                let workspace_path = match folder.uri.to_file_path() {
                    Ok(p) => p,
                    Err(_) => {
                        init_errors.workspace_errors.push(format!(
                            "Cannot process workspace folder URI: {}",
                            folder.uri
                        ));
                        continue;
                    }
                };

                // Recursively discover smelt projects
                let project_roots = find_smelt_projects(&workspace_path);

                for project_root in project_roots {
                    // Check for ambiguous smelt config
                    if project_root.join("smelt.yml").exists()
                        && project_root.join("smelt.yaml").exists()
                    {
                        init_errors.workspace_errors.push(format!(
                            "Both smelt.yml and smelt.yaml exist in {}",
                            project_root.display()
                        ));
                        continue;
                    }

                    all_project_roots.push(project_root.clone());

                    // Canonical workspace loading — single source of truth for
                    // CLI and LSP. See docs/specs/architecture.md →
                    // "Workspace loading parity rule (CLI ↔ LSP)".
                    let loaded = smelt_core::load_workspace(&project_root);
                    init_errors
                        .workspace_errors
                        .extend(loaded.errors.workspace_errors.iter().cloned());
                    init_errors
                        .source_errors
                        .extend(loaded.errors.source_errors.iter().cloned());
                    // The "no models found" soft warning is noise for empty /
                    // functions-only workspaces; drop it from the LSP surface.
                    let model_errors: Vec<String> = loaded
                        .errors
                        .model_errors
                        .iter()
                        .filter(|e| !e.starts_with("No models found"))
                        .cloned()
                        .collect();
                    init_errors.model_errors.extend(model_errors);

                    // Sources input + loader files (the latter was previously
                    // missing in the LSP — smelt.config.load_yaml(...) in
                    // generator files didn't resolve).
                    db.set_project_input(project_root.clone(), loaded.sources_text.clone());
                    smelt_db::workspace_ingest::register_loader_files_from_disk(
                        &mut db,
                        &project_root,
                    );

                    // Register SQL files via register_sql_content so the LSP's
                    // multi-model line-offset tracking populates correctly.
                    // Dedup by real path — multi-model files appear once per
                    // section in loaded.sql_files but share one real file.
                    let mut seen_real_paths: std::collections::HashSet<PathBuf> =
                        std::collections::HashSet::new();
                    for model in &loaded.sql_files {
                        let real_path = model.model_id.source_path().to_path_buf();
                        if !seen_real_paths.insert(real_path.clone()) {
                            continue;
                        }
                        match std::fs::read_to_string(&real_path) {
                            Ok(content) => {
                                let paths = self
                                    .register_sql_content(
                                        &mut db,
                                        &real_path,
                                        &content,
                                        &project_root,
                                    )
                                    .await;
                                all_files.extend(paths);
                            }
                            Err(e) => {
                                init_errors.model_errors.push(format!(
                                    "Failed to read {}: {}",
                                    real_path.display(),
                                    e
                                ));
                            }
                        }
                    }

                    // Python discovery — kept inline; runs python_scan with
                    // LSP-specific state (python_cache, python_model_sources)
                    // and emits LSP diagnostics for execution errors. Not yet
                    // shared with the CLI's run-Python pipeline.
                    let config = &loaded.config;
                    for model_path in &config.paths {
                        let models_path = project_root.join(model_path);
                        let context_json = build_python_context(&all_files, config);
                        let mut cache = self.python_cache.lock().await;
                        *cache = PythonModelCache::load(&project_root);
                        let scan_result = crate::python_scan::discover_python_models(
                            &models_path,
                            &project_root,
                            &mut cache,
                            &context_json,
                        );
                        drop(cache);

                        if !scan_result.models.is_empty() {
                            let mut py_sources = self.python_model_sources.lock().await;
                            for py_model in &scan_result.models {
                                // Use <name>.sql so file_stem() yields the model name directly.
                                // Multi-model .py files each get their own virtual file.
                                let virtual_sql_path = py_model
                                    .source_path
                                    .parent()
                                    .unwrap_or_else(|| std::path::Path::new("."))
                                    .join(format!("{}.sql", py_model.name));

                                db.set_source_file(
                                    virtual_sql_path.clone(),
                                    py_model.sql.clone(),
                                    project_root.clone(),
                                );
                                // Map virtual path back to actual .py source for goto-definition
                                py_sources.insert(
                                    virtual_sql_path.clone(),
                                    (py_model.source_path.clone(), py_model.decorator_line),
                                );
                                all_files.push(virtual_sql_path);
                            }
                        }

                        // Collect Python model errors as diagnostics
                        if !scan_result.errors.is_empty() {
                            let mut py_diags = self.python_diagnostics.lock().await;
                            for error in &scan_result.errors {
                                let line = error.line.unwrap_or(1).saturating_sub(1);
                                let diag = lsp_types::Diagnostic {
                                    range: Range {
                                        start: Position::new(line, 0),
                                        end: Position::new(line, 0),
                                    },
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    message: error.message.clone(),
                                    source: Some("smelt-python".to_string()),
                                    ..Default::default()
                                };
                                py_diags
                                    .entry(error.source_path.clone())
                                    .or_default()
                                    .push(diag);
                                init_errors.model_errors.push(format!(
                                    "Python model error in {}: {}",
                                    error.source_path.display(),
                                    error.message,
                                ));
                            }
                        }
                    }
                }
            }

            Backend::sync_workspace(&mut db, &all_files, &all_project_roots);

            // Store project roots and file list for file-change handling
            *self.tracked_files.lock().await = all_files;
            *self.project_roots.lock().await = all_project_roots;
        }

        // Store errors for reporting after initialized notification
        *self.init_errors.lock().await = Some(init_errors);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "'".to_string(),
                        "(".to_string(),
                        ".".to_string(),
                    ]),
                    ..Default::default()
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "smelt language server initialized")
            .await;

        // Register file watchers (dynamic registration). We watch:
        //   - `**/models/**/*.py` for Python model changes
        //   - `**/functions/**/*.sql` so that external edits to function
        //     definitions (git checkout, sed, etc.) re-trigger diagnostics
        //     on dependent models. In-editor edits go through `did_change`.
        let registration = Registration {
            id: "smelt-file-watcher".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: Some(
                serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                    watchers: vec![
                        FileSystemWatcher {
                            glob_pattern: GlobPattern::String("**/models/**/*.py".to_string()),
                            kind: Some(WatchKind::all()),
                        },
                        FileSystemWatcher {
                            glob_pattern: GlobPattern::String("**/functions/**/*.sql".to_string()),
                            kind: Some(WatchKind::all()),
                        },
                    ],
                })
                .unwrap(),
            ),
        };
        let _ = self.client.register_capability(vec![registration]).await;

        // Report any initialization errors
        if let Some(errors) = self.init_errors.lock().await.take() {
            if errors.has_errors() {
                // Log each error
                for err in &errors.workspace_errors {
                    self.client.log_message(MessageType::ERROR, err).await;
                }
                for err in &errors.source_errors {
                    self.client.log_message(MessageType::WARNING, err).await;
                }
                for err in &errors.model_errors {
                    self.client.log_message(MessageType::WARNING, err).await;
                }

                // Show summary notification to user
                self.client
                    .show_message(
                        MessageType::WARNING,
                        format!(
                            "smelt: {} file(s) failed to load. Check Output for details.",
                            errors.total_count()
                        ),
                    )
                    .await;
            }
        }

        // Publish Python diagnostics collected during init
        let py_diags = self.python_diagnostics.lock().await;
        for (path, diagnostics) in py_diags.iter() {
            if let Ok(uri) = Url::from_file_path(path) {
                self.client
                    .publish_diagnostics(uri, diagnostics.clone(), None)
                    .await;
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Check if this is sources.yml/yaml - update sources config and refresh all diagnostics
        if is_sources_file(&path) {
            if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                let mut db = self.db.lock().await;
                db.set_project_input(project_root, params.text_document.text);
                drop(db);
                self.publish_all_diagnostics().await;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let mut db = self.db.lock().await;
            // If this file wasn't seen during init, find its project root
            let project_roots = self.project_roots.lock().await.clone();
            let has_project_root = project_roots.iter().any(|root| path.starts_with(root));
            if !has_project_root {
                // Try to discover project root by walking up
                if let Some(project_root) = find_project_root_by_walking_up(&path) {
                    // Register this new project
                    let mut roots = self.project_roots.lock().await;
                    if !roots.contains(&project_root) {
                        roots.push(project_root.clone());
                        // Load sources for this project
                        let sources_content = find_config_file(&project_root, "sources")
                            .ok()
                            .flatten()
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .unwrap_or_default();
                        db.set_project_input(project_root.clone(), sources_content);
                    }
                    drop(roots);
                }
            }
            // Register file content (handles multi-model splitting)
            // register_sql_content skips mutations when content hasn't changed
            let project_root_for_reg = project_roots
                .iter()
                .find(|root| path.starts_with(root))
                .cloned()
                .unwrap_or_default();
            let registered_paths = self
                .register_sql_content(
                    &mut db,
                    &path,
                    &params.text_document.text,
                    &project_root_for_reg,
                )
                .await;
            // Only update tracked_files + workspace if new paths were registered
            let mut tracked = self.tracked_files.lock().await;
            let mut changed = false;
            for rp in &registered_paths {
                if !tracked.contains(rp) {
                    tracked.push(rp.clone());
                    changed = true;
                }
            }
            if changed {
                Backend::sync_workspace(&mut db, &tracked, &project_roots);
            }
            drop(tracked);
            drop(db);
            self.publish_diagnostics(uri).await;
        } else if path.extension().and_then(|s| s.to_str()) != Some("py") {
            // Non-SQL, non-sources, non-Python file — register as a source file
            let mut db = self.db.lock().await;
            let project_roots = self.project_roots.lock().await.clone();
            let project_root = project_roots
                .iter()
                .find(|root| path.starts_with(root))
                .cloned()
                .unwrap_or_default();
            db.set_source_file(path, params.text_document.text, project_root);
            drop(db);
            self.publish_diagnostics(uri).await;
        }
        // Skip .py files - they are handled during init via subprocess execution,
        // and parsing them as SQL would produce spurious diagnostics
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Get new text (we use FULL sync, so there's only one change)
        if let Some(change) = params.content_changes.into_iter().next() {
            if is_sources_file(&path) {
                if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                    let mut db = self.db.lock().await;
                    db.set_project_input(project_root, change.text);
                    drop(db);
                    self.publish_all_diagnostics().await;
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                let mut db = self.db.lock().await;
                let project_roots = self.project_roots.lock().await.clone();
                let project_root = project_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned()
                    .unwrap_or_default();
                let registered_paths = self
                    .register_sql_content(&mut db, &path, &change.text, &project_root)
                    .await;
                // Ensure all registered paths are in tracked_files + workspace
                let mut tracked = self.tracked_files.lock().await;
                let mut changed = false;
                for rp in &registered_paths {
                    if !tracked.contains(rp) {
                        tracked.push(rp.clone());
                        changed = true;
                    }
                }
                if changed {
                    Backend::sync_workspace(&mut db, &tracked, &project_roots);
                }
                drop(tracked);
                drop(db);
                self.publish_diagnostics(uri).await;
            } else if path.extension().and_then(|s| s.to_str()) != Some("py") {
                let mut db = self.db.lock().await;
                let project_roots = self.project_roots.lock().await.clone();
                let project_root = project_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned()
                    .unwrap_or_default();
                db.set_source_file(path, change.text, project_root);
                drop(db);
                self.publish_diagnostics(uri).await;
            }
            // Skip .py files - parsing as SQL would produce spurious diagnostics
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for change in params.changes {
            let path = match change.uri.to_file_path() {
                Ok(p) => p,
                Err(_) => continue,
            };

            if path.extension().and_then(|s| s.to_str()) == Some("py") {
                self.handle_python_file_change(&path).await;
            } else if is_sources_file(&path) {
                // Re-read sources.yml from disk when changed outside the editor
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                        let mut db = self.db.lock().await;
                        db.set_project_input(project_root, content);
                        drop(db);
                        self.publish_all_diagnostics().await;
                    }
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                // External `.sql` change (currently only `functions/**/*.sql`
                // is watched). Re-read content into the DB and refresh all
                // diagnostics so dependents pick up the new signature.
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut db = self.db.lock().await;
                    let project_roots = self.project_roots.lock().await.clone();
                    let project_root = project_roots
                        .iter()
                        .find(|root| path.starts_with(root))
                        .cloned()
                        .unwrap_or_default();
                    let registered_paths = self
                        .register_sql_content(&mut db, &path, &content, &project_root)
                        .await;
                    let mut tracked = self.tracked_files.lock().await;
                    let mut changed = false;
                    for rp in &registered_paths {
                        if !tracked.contains(rp) {
                            tracked.push(rp.clone());
                            changed = true;
                        }
                    }
                    if changed {
                        Backend::sync_workspace(&mut db, &tracked, &project_roots);
                    }
                    drop(tracked);
                    drop(db);
                    self.publish_all_diagnostics().await;
                }
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path and adjust position
        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        // Resolve goto-definition target while holding the db snapshot and AST.
        // We collect the result as plain data (no Rowan nodes) so we can drop
        // the non-Send AST before any await points.
        enum GotoTarget {
            RefModel(PathBuf),
            /// CTE definition in the same file — target is an LSP Range
            SameFile(Range),
            /// Column definitions (potentially multiple for ambiguous refs)
            ColumnDefs(Vec<ColumnDefLocation>),
            /// Lambda parameter binder in the same file (Phase B).
            LambdaParam {
                binder_start: u32,
                binder_col: u32,
                /// End column of the binder token (exclusive), so the
                /// full param name is highlighted, not just the first char.
                binder_end_col: u32,
            },
            /// smelt.config.var('x') — resolves to a line in smelt.yml (Phase B).
            ConfigVarYml {
                yml_path: PathBuf,
                line: u32,
            },
            /// Phase E2: goto-def from a generator-emitted model reference to the
            /// emitting `ModelDef.name` field's value-token in the generator file.
            EmittedModelRef {
                gen_file: PathBuf,
                name_range: Range,
            },
            /// Goto-def from a `smelt.functions.<name>(...)` call to the
            /// `smelt.define <name>(...)` declaration. Lands the cursor on
            /// the name token (precise position derived from the file's
            /// current text via `name_range`).
            FunctionDef {
                target_file: PathBuf,
                name_start: u32,
                name_end: u32,
            },
        }

        let target = {
            let db = self.snapshot().await;
            let text = file_text(&db, &effective_path);
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            let syntax = parse.as_ref().map(|p| p.syntax());
            let cursor_offset =
                position_to_offset(&text, effective_position.line, effective_position.character);

            if let Some(syntax) = syntax {
                if let Some(file) = AstFile::cast(syntax) {
                    match symbol_at_cursor(&file, &text, cursor_offset) {
                        Some(SymbolAtCursor::CteReference { name }) => {
                            // Jump to CTE definition
                            let mut result = None;
                            if let Some(select_stmt) = file.select_stmt() {
                                if let Some(with_clause) = select_stmt.with_clause() {
                                    for cte in with_clause.ctes() {
                                        if cte.name().as_deref() == Some(name.as_str()) {
                                            let pr = smelt_parser::ast::text_range_to_range(
                                                &text,
                                                cte.syntax().text_range(),
                                            );
                                            result = Some(GotoTarget::SameFile(Range {
                                                start: Position::new(
                                                    pr.start.line,
                                                    pr.start.column,
                                                ),
                                                end: Position::new(pr.end.line, pr.end.column),
                                            }));
                                            break;
                                        }
                                    }
                                }
                            }
                            result
                        }
                        Some(SymbolAtCursor::CteDefinition { .. }) => {
                            // Already at definition site — no-op
                            None
                        }
                        Some(SymbolAtCursor::ColumnRef { qualifier, name }) => {
                            // Check if cursor is on the qualifier token — if so, jump to
                            // the CTE or table alias definition rather than doing column resolution
                            let cursor_on_qualifier = qualifier.is_some() && {
                                // Find the tightest Expr at cursor and check if cursor is on first IDENT
                                let mut best_expr: Option<smelt_parser::ast::Expr> = None;
                                let mut best_len = usize::MAX;
                                for node in file.syntax().descendants() {
                                    if let Some(expr) = smelt_parser::ast::Expr::cast(node) {
                                        let range = expr.text_range();
                                        let start: usize = range.start().into();
                                        let end: usize = range.end().into();
                                        let len = end - start;
                                        if cursor_offset >= start
                                            && cursor_offset <= end
                                            && len <= best_len
                                        {
                                            best_len = len;
                                            best_expr = Some(expr);
                                        }
                                    }
                                }
                                best_expr
                                    .map(|expr| {
                                        use smelt_parser::SyntaxKind::{DOT, IDENT};
                                        expr.syntax()
                                            .children_with_tokens()
                                            .filter_map(|e| e.into_token())
                                            .find(|t| t.kind() == IDENT || t.kind() == DOT)
                                            .map(|first_ident| {
                                                let start: usize =
                                                    first_ident.text_range().start().into();
                                                let end: usize =
                                                    first_ident.text_range().end().into();
                                                first_ident.kind() == IDENT
                                                    && cursor_offset >= start
                                                    && cursor_offset <= end
                                            })
                                            .unwrap_or(false)
                                    })
                                    .unwrap_or(false)
                            };

                            if cursor_on_qualifier {
                                let qualifier_str = qualifier.as_deref().unwrap();
                                let mut result = None;

                                // Check if qualifier is a CTE name
                                if let Some(select_stmt) = file.select_stmt() {
                                    if let Some(with_clause) = select_stmt.with_clause() {
                                        for cte in with_clause.ctes() {
                                            if cte.name().as_deref() == Some(qualifier_str) {
                                                let pr = smelt_parser::ast::text_range_to_range(
                                                    &text,
                                                    cte.syntax().text_range(),
                                                );
                                                result = Some(GotoTarget::SameFile(Range {
                                                    start: Position::new(
                                                        pr.start.line,
                                                        pr.start.column,
                                                    ),
                                                    end: Position::new(pr.end.line, pr.end.column),
                                                }));
                                                break;
                                            }
                                        }
                                    }
                                }

                                // Check if qualifier is a table alias in FROM/JOIN
                                if result.is_none() {
                                    if let Some(select_stmt) = file.select_stmt() {
                                        if let Some(from_clause) = select_stmt.from_clause() {
                                            let table_refs: Vec<_> = from_clause
                                                .table_refs()
                                                .chain(
                                                    from_clause
                                                        .joins()
                                                        .filter_map(|j| j.table_ref()),
                                                )
                                                .collect();

                                            for table_ref in table_refs {
                                                let matches = table_ref.alias().as_deref()
                                                    == Some(qualifier_str)
                                                    || table_ref.identifier().as_deref()
                                                        == Some(qualifier_str);
                                                if matches {
                                                    let pr = smelt_parser::ast::text_range_to_range(
                                                        &text,
                                                        table_ref.syntax().text_range(),
                                                    );
                                                    result = Some(GotoTarget::SameFile(Range {
                                                        start: Position::new(
                                                            pr.start.line,
                                                            pr.start.column,
                                                        ),
                                                        end: Position::new(
                                                            pr.end.line,
                                                            pr.end.column,
                                                        ),
                                                    }));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                result
                            } else {
                                let defs = resolve_column_definitions(
                                    &db,
                                    &effective_path,
                                    qualifier.as_deref(),
                                    &name,
                                );
                                if !defs.is_empty() {
                                    Some(GotoTarget::ColumnDefs(defs))
                                } else {
                                    None
                                }
                            }
                        }
                        Some(SymbolAtCursor::PathRef { segments }) => {
                            // Resolve via the unified path data plane (Phase 2a).
                            // SQL files come back via `source_file`; seeds and
                            // sources (which aren't Salsa SourceFiles) fall
                            // through to `resolve_seed_or_source_path` which
                            // returns the on-disk `.csv` / `.yml` path.
                            let ws = Workspace::try_get(&db);
                            ws.and_then(|w| {
                                if let Some(sf) =
                                    smelt_db::resolve_ref_path(&db, w, segments.clone())
                                        .and_then(|r| r.source_file)
                                {
                                    Some(GotoTarget::RefModel(sf.path(&db).clone()))
                                } else {
                                    smelt_db::resolve_seed_or_source_path(&db, w, segments)
                                        .map(GotoTarget::RefModel)
                                }
                            })
                        }
                        Some(SymbolAtCursor::FunctionCall { segments }) => {
                            // Route `smelt.functions.<name>(...)` calls to the
                            // `smelt.define <name>(...)` declaration. Other call
                            // shapes (e.g. `smelt.metrics.foo`, when that namespace
                            // ships) fall through to None.
                            //
                            // Project isolation: resolve against functions
                            // declared in the same project as the call site.
                            // See docs/specs/architecture.md → "Project
                            // isolation rule".
                            if segments.len() == 2 && segments[0] == "functions" {
                                let name = segments[1].clone();
                                let ws = Workspace::try_get(&db);
                                ws.and_then(|w| {
                                    let project = file_input.and_then(|sf| {
                                        smelt_db::find_project(
                                            &db,
                                            w,
                                            &sf.project_root(&db).clone(),
                                        )
                                    })?;
                                    smelt_db::resolve_function_path(&db, w, project, name).map(
                                        |(f, name_range)| GotoTarget::FunctionDef {
                                            target_file: f.path(&db).clone(),
                                            name_start: name_range.start,
                                            name_end: name_range.end,
                                        },
                                    )
                                })
                            } else {
                                None
                            }
                        }
                        None => None,
                    }
                    // Fall through to Phase B checks when symbol_at_cursor returned None
                    // (lambda params and config.var args are not yet handled by the symbol scanner).
                    .or_else(|| {
                        use smelt_parser::syntax_kind::SyntaxKind;

                        // Phase B: goto-def on a lambda parameter IDENT in the body —
                        // jump to the binding occurrence in LAMBDA_PARAM_LIST.
                        let lambda_node = file
                            .syntax()
                            .descendants()
                            .filter(|n| n.kind() == SyntaxKind::LAMBDA)
                            .filter(|n| {
                                let s: usize = n.text_range().start().into();
                                let e: usize = n.text_range().end().into();
                                cursor_offset >= s && cursor_offset <= e
                            })
                            .min_by_key(|n| {
                                let s: usize = n.text_range().start().into();
                                let e: usize = n.text_range().end().into();
                                e - s
                            });
                        if let Some(ln) = lambda_node {
                            if let Some(lambda) = smelt_parser::ast::Lambda::cast(ln) {
                                for param_name in lambda.params() {
                                    if let Some(binder_range) =
                                        lambda_param_binder_range(&lambda, &param_name)
                                    {
                                        // Only navigate when the cursor is on the binder
                                        // itself or on a body-use IDENT with the same
                                        // name.  Without this guard, any cursor position
                                        // inside the lambda (e.g. on `=>`, whitespace,
                                        // or an unrelated sub-expression) would jump.
                                        let binder_s: usize = binder_range.start().into();
                                        let binder_e: usize = binder_range.end().into();
                                        let on_binder =
                                            cursor_offset >= binder_s && cursor_offset <= binder_e;
                                        let on_body_use = lambda.body().is_some_and(|body| {
                                            body.syntax()
                                                .descendants_with_tokens()
                                                .filter_map(|e| e.into_token())
                                                .filter(|t| {
                                                    t.kind() == SyntaxKind::IDENT
                                                        && t.text() == param_name.as_str()
                                                })
                                                .any(|t| {
                                                    let s: usize = t.text_range().start().into();
                                                    let e: usize = t.text_range().end().into();
                                                    cursor_offset >= s && cursor_offset <= e
                                                })
                                        });
                                        if !on_binder && !on_body_use {
                                            continue;
                                        }
                                        // Convert the binder range to an LSP Range.
                                        let pr = smelt_parser::ast::text_range_to_range(
                                            &text,
                                            binder_range,
                                        );
                                        return Some(GotoTarget::LambdaParam {
                                            binder_start: pr.start.line,
                                            binder_col: pr.start.column,
                                            binder_end_col: pr.end.column,
                                        });
                                    }
                                }
                            }
                        }

                        // Phase B: goto-def on `smelt.config.var('x')` argument —
                        // jump to `vars.x:` line in smelt.yml.
                        let var_call = file
                            .syntax()
                            .descendants()
                            .filter_map(smelt_parser::ast::FunctionCall::cast)
                            .find(|c| {
                                c.name().as_deref() == Some("var") && {
                                    let s: usize = c.syntax().text_range().start().into();
                                    let e: usize = c.syntax().text_range().end().into();
                                    cursor_offset >= s && cursor_offset <= e
                                }
                            });
                        if let Some(vc) = var_call {
                            let args = vc.arguments();
                            if let Some(arg) = args.first() {
                                if smelt_db::config_vars::is_string_literal_expr(arg) {
                                    if let Some(var_name) =
                                        smelt_db::config_vars::extract_string_literal_value(arg)
                                    {
                                        let project_root = file_project_root(&db, &effective_path);
                                        let project = lookup_project(&db, &project_root);
                                        let smelt_yml_text = project
                                            .map(|p| p.smelt_yml_text(&db).clone())
                                            .unwrap_or_default();
                                        // Only navigate when the variable is actually declared
                                        // in smelt.yml; return None for undeclared vars so we
                                        // don't silently land at the top of the file.
                                        if let Some(line) =
                                            find_var_line_in_smelt_yml(&smelt_yml_text, &var_name)
                                        {
                                            let yml_path = project_root.join("smelt.yml");
                                            return Some(GotoTarget::ConfigVarYml {
                                                yml_path,
                                                line,
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        // Phase E2: goto-def on a `smelt.<path>` ref that resolves to a
                        // generator-emitted model — jump to the `ModelDef.name` field's
                        // value-token in the generator file.
                        //
                        // We look at `smelt_db::emitted_models` to find a survivor whose
                        // computed smelt path matches the dotted path under the cursor.
                        // The path must NOT be a `smelt.models.*` or `smelt.sources.*`
                        // accessor call — those are already handled above.
                        {
                            // Find the SmeltPathRef under the cursor (excluding models/sources).
                            let path_ref_under_cursor = file
                                .syntax()
                                .descendants()
                                .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                                .filter(|pr| {
                                    let segs = pr.segments();
                                    let first = segs.first().map(|s| s.as_str());
                                    first != Some("models") && first != Some("sources")
                                })
                                .find(|pr| {
                                    let r = pr.text_range();
                                    let s: usize = r.start().into();
                                    let e: usize = r.end().into();
                                    cursor_offset >= s && cursor_offset <= e
                                });

                            if let Some(pr) = path_ref_under_cursor {
                                let segments = pr.segments();
                                let cursor_path = segments.join(".");
                                let ws = Workspace::try_get(&db);
                                if let Some(w) = ws {
                                    let survivors = smelt_db::emitted_models(&db, w);
                                    let project_root = file_project_root(&db, &effective_path);
                                    let project = lookup_project(&db, &project_root);
                                    let scan_roots = project
                                        .map(|p| smelt_db::project_paths(&db, p).as_ref().clone())
                                        .unwrap_or_else(|| vec!["models".to_string()]);
                                    if let Some(em) = survivors.survivors.iter().find(|em| {
                                        let sp = smelt_db::emitted_model_smelt_path(
                                            &em.generator_file,
                                            &project_root,
                                            &scan_roots,
                                            &em.name,
                                        );
                                        sp == cursor_path
                                    }) {
                                        // Convert the name_span (TextRange) to an LSP Range.
                                        let gen_text = std::fs::read_to_string(&em.generator_file)
                                            .unwrap_or_default();
                                        let pr_range = smelt_parser::ast::text_range_to_range(
                                            &gen_text,
                                            em.name_span,
                                        );
                                        let name_range = Range {
                                            start: Position::new(
                                                pr_range.start.line,
                                                pr_range.start.column,
                                            ),
                                            end: Position::new(
                                                pr_range.end.line,
                                                pr_range.end.column,
                                            ),
                                        };
                                        return Some(GotoTarget::EmittedModelRef {
                                            gen_file: em.generator_file.clone(),
                                            name_range,
                                        });
                                    }
                                }
                            }
                        }

                        None
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }; // end of block — parse/syntax dropped here, before any awaits

        // Convert target to LSP response
        match target {
            Some(GotoTarget::RefModel(target_path)) => {
                // Map virtual .sql paths back to .py sources
                let py_sources = self.python_model_sources.lock().await;
                let (actual_path, target_line) = py_sources
                    .get(&target_path)
                    .map(|(p, line)| (p.clone(), *line))
                    .unwrap_or((target_path, 0));
                drop(py_sources);

                if let Ok(target_uri) = Url::from_file_path(&actual_path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: Range {
                            start: Position::new(target_line, 0),
                            end: Position::new(target_line, 0),
                        },
                    })))
                } else {
                    Ok(None)
                }
            }
            Some(GotoTarget::SameFile(target_range)) => {
                if let Ok(target_uri) = Url::from_file_path(&path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: target_range,
                    })))
                } else {
                    Ok(None)
                }
            }
            Some(GotoTarget::ColumnDefs(defs)) => {
                let locations: Vec<Location> = defs
                    .iter()
                    .filter_map(|def| {
                        Url::from_file_path(&def.path).ok().map(|uri| Location {
                            uri,
                            range: Range {
                                start: Position::new(def.line, def.col),
                                end: Position::new(def.end_line, def.end_col),
                            },
                        })
                    })
                    .collect();

                match locations.len() {
                    0 => Ok(None),
                    1 => Ok(Some(GotoDefinitionResponse::Scalar(
                        locations.into_iter().next().unwrap(),
                    ))),
                    _ => Ok(Some(GotoDefinitionResponse::Array(locations))),
                }
            }
            // Phase B: lambda param binder — jump to binder in same file.
            Some(GotoTarget::LambdaParam {
                binder_start,
                binder_col,
                binder_end_col,
            }) => {
                if let Ok(target_uri) = Url::from_file_path(&path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: Range {
                            start: Position::new(binder_start, binder_col),
                            end: Position::new(binder_start, binder_end_col),
                        },
                    })))
                } else {
                    Ok(None)
                }
            }
            // Phase B: config.var goto — jump to vars.<name>: in smelt.yml.
            Some(GotoTarget::ConfigVarYml { yml_path, line }) => {
                if let Ok(target_uri) = Url::from_file_path(&yml_path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: Range {
                            start: Position::new(line, 0),
                            end: Position::new(line, 0),
                        },
                    })))
                } else {
                    Ok(None)
                }
            }
            // Phase E2: emitted-model ref — jump to the ModelDef.name value-token.
            Some(GotoTarget::EmittedModelRef {
                gen_file,
                name_range,
            }) => {
                if let Ok(target_uri) = Url::from_file_path(&gen_file) {
                    let loc = goto_def_for_emitted_model_reference(&gen_file, name_range);
                    if let Some(location) = loc {
                        Ok(Some(GotoDefinitionResponse::Scalar(Location {
                            uri: target_uri,
                            range: location.range,
                        })))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }
            // smelt.functions.<name>(...) → smelt.define <name>(...).
            // Convert the stored byte range to LSP line/col using the target
            // file's current text. Done outside the AST-holding block so
            // there's no Salsa snapshot lifetime issue.
            Some(GotoTarget::FunctionDef {
                target_file,
                name_start,
                name_end,
            }) => {
                let target_uri = match Url::from_file_path(&target_file) {
                    Ok(u) => u,
                    Err(_) => return Ok(None),
                };
                let target_text = std::fs::read_to_string(&target_file).unwrap_or_default();
                // `define.name_range()` returns offsets into the
                // frontmatter-stripped source (parse_file strips before
                // parsing). Strip here too so byte→line/col mapping aligns.
                let stripped = smelt_parser::strip_frontmatter(&target_text);
                let start = smelt_parser::ast::offset_to_position(&stripped, name_start as usize);
                let end = smelt_parser::ast::offset_to_position(&stripped, name_end as usize);
                Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: target_uri,
                    range: Range {
                        start: Position::new(start.line, start.column),
                        end: Position::new(end.line, end.column),
                    },
                })))
            }
            None => Ok(None),
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path and adjust position
        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        // Collect reference data as plain types.
        // We use an enum to avoid holding AST nodes across await points.
        enum RefResult {
            PathRanges(Vec<(PathBuf, smelt_parser::ast::Range)>),
            CteRanges(PathBuf, Vec<(u32, u32, u32, u32)>),
            Empty,
        }

        let ref_result = {
            let db = self.snapshot().await;
            let text = file_text(&db, &effective_path);
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            let syntax = parse.as_ref().map(|p| p.syntax());
            let cursor_offset =
                position_to_offset(&text, effective_position.line, effective_position.character);

            if let Some(syntax) = syntax {
                if let Some(file) = AstFile::cast(syntax) {
                    // Project-scope the search per architecture.md → "Project
                    // isolation rule": a workspace folder may contain multiple
                    // smelt projects, and references do not cross project
                    // boundaries. Derive the project from the cursor file.
                    let project_files: Vec<smelt_db::SourceFile> = {
                        let ws = Workspace::try_get(&db);
                        match (ws, file_input) {
                            (Some(w), Some(sf)) => {
                                let project_root = sf.project_root(&db).clone();
                                w.files(&db)
                                    .iter()
                                    .copied()
                                    .filter(|f| f.project_root(&db) == &project_root)
                                    .collect()
                            }
                            _ => Vec::new(),
                        }
                    };

                    match symbol_at_cursor(&file, &text, cursor_offset) {
                        Some(SymbolAtCursor::PathRef { segments }) => {
                            let mut all_refs: Vec<(PathBuf, smelt_parser::ast::Range)> = Vec::new();
                            for f in &project_files {
                                let path_refs = smelt_db::model_path_refs(&db, *f);
                                for loc in path_refs.iter() {
                                    if loc.path == segments {
                                        all_refs.push((f.path(&db).clone(), loc.range));
                                    }
                                }
                            }
                            RefResult::PathRanges(all_refs)
                        }
                        Some(SymbolAtCursor::FunctionCall { segments }) => {
                            // Only `smelt.functions.<name>` calls are findable
                            // today. Other call shapes have no def to anchor on.
                            if segments.len() == 2 && segments[0] == "functions" {
                                let name = &segments[1];
                                let mut all_refs: Vec<(PathBuf, smelt_parser::ast::Range)> =
                                    Vec::new();
                                for f in &project_files {
                                    let parse = smelt_db::parse_file(&db, *f);
                                    let Some(ast) = AstFile::cast(parse.syntax()) else {
                                        continue;
                                    };
                                    let f_text = f.text(&db);
                                    for trange in
                                        smelt_db::references::find_function_call_sites_in_file(
                                            &ast, name,
                                        )
                                    {
                                        let r =
                                            smelt_parser::ast::text_range_to_range(f_text, trange);
                                        all_refs.push((f.path(&db).clone(), r));
                                    }
                                }
                                RefResult::PathRanges(all_refs)
                            } else {
                                RefResult::Empty
                            }
                        }
                        Some(SymbolAtCursor::CteDefinition { name })
                        | Some(SymbolAtCursor::CteReference { name }) => {
                            let cte_refs =
                                smelt_db::references::find_cte_references(&file, &text, &name);
                            let ranges: Vec<_> = cte_refs
                                .iter()
                                .map(|text_range| {
                                    let r =
                                        smelt_parser::ast::text_range_to_range(&text, *text_range);
                                    (r.start.line, r.start.column, r.end.line, r.end.column)
                                })
                                .collect();
                            RefResult::CteRanges(effective_path.clone(), ranges)
                        }
                        _ => RefResult::Empty,
                    }
                } else {
                    RefResult::Empty
                }
            } else {
                RefResult::Empty
            }
        }; // end of block — parse/syntax dropped before awaits

        let locations = match ref_result {
            RefResult::PathRanges(refs) => self.ref_locations_to_lsp(&refs).await,
            RefResult::CteRanges(path, ranges) => ranges
                .into_iter()
                .filter_map(|(sl, sc, el, ec)| {
                    let uri = Url::from_file_path(&path).ok()?;
                    Some(Location {
                        uri,
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                    })
                })
                .collect(),
            RefResult::Empty => vec![],
        };

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let request_range = params.range;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path
        let (effective_path, line_offset) = if let Some((vp, adjusted_line)) = self
            .resolve_virtual_path(&path, request_range.start.line)
            .await
        {
            let offset = request_range.start.line - adjusted_line;
            (vp, offset)
        } else {
            (path.clone(), 0)
        };

        let db = self.snapshot().await;
        let text = file_text(&db, &effective_path);

        // Collect diagnostics overlapping the request range
        let all_diags = diagnostics_for(&db, &effective_path);

        // Adjust request range for virtual path offset
        let adj_start_line = request_range.start.line.saturating_sub(line_offset);
        let adj_end_line = request_range.end.line.saturating_sub(line_offset);

        let matching: Vec<_> = all_diags
            .into_iter()
            .filter(|d| {
                let r = &d.range;
                // Diagnostic overlaps the request range
                !(r.end.line < adj_start_line
                    || (r.end.line == adj_start_line
                        && r.end.column < request_range.start.character)
                    || r.start.line > adj_end_line
                    || (r.start.line == adj_end_line
                        && r.start.column > request_range.end.character))
            })
            .collect();

        // Read sources.yml for YAML-editing code actions
        let project_root = file_project_root(&db, &effective_path);
        let sources_yml_content = project_sources_yaml(&db, &project_root);
        let sources_yml_path = project_root.join("sources.yml");

        let mut actions = Vec::new();

        // Diagnostic-based code actions
        for diag in &matching {
            use smelt_db::code_actions::CodeActionKind as CAK;

            let action_kinds = smelt_db::code_actions::generate_all_code_actions(
                diag,
                &text,
                &sources_yml_content,
            );
            for kind in action_kinds {
                match kind {
                    CAK::TextEdit(suggestion) => {
                        let range = Range {
                            start: Position::new(
                                suggestion.range.start.line + line_offset,
                                suggestion.range.start.column,
                            ),
                            end: Position::new(
                                suggestion.range.end.line + line_offset,
                                suggestion.range.end.column,
                            ),
                        };
                        let edit = TextEdit {
                            range,
                            new_text: suggestion.new_text,
                        };
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(uri.clone(), vec![edit]);
                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                changes: Some(changes),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                    CAK::CreateModel(suggestion) => {
                        // Build the new model file path in the same directory as the current file
                        let model_dir = effective_path.parent().unwrap_or(project_root.as_ref());
                        let new_file_path =
                            model_dir.join(format!("{}.sql", suggestion.model_name));
                        let new_file_uri =
                            Url::from_file_path(&new_file_path).unwrap_or_else(|_| uri.clone());

                        let document_changes = vec![
                            DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                                uri: new_file_uri.clone(),
                                options: None,
                                annotation_id: None,
                            })),
                            DocumentChangeOperation::Edit(TextDocumentEdit {
                                text_document: OptionalVersionedTextDocumentIdentifier {
                                    uri: new_file_uri,
                                    version: None,
                                },
                                edits: vec![OneOf::Left(TextEdit {
                                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                                    new_text: suggestion.skeleton_sql,
                                })],
                            }),
                        ];
                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                document_changes: Some(DocumentChanges::Operations(
                                    document_changes,
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                    CAK::YamlEdit(suggestion) => {
                        let yaml_uri =
                            Url::from_file_path(&sources_yml_path).unwrap_or_else(|_| uri.clone());
                        // Insert new lines after the specified line
                        let insert_line = (suggestion.insert_after_line + 1) as u32;
                        let new_text = suggestion.new_lines.join("\n") + "\n";
                        let edit = TextEdit {
                            range: Range::new(
                                Position::new(insert_line, 0),
                                Position::new(insert_line, 0),
                            ),
                            new_text,
                        };
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(yaml_uri, vec![edit]);
                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                changes: Some(changes),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                    CAK::PinSeedSchema(suggestion) => {
                        // Build the sidecar YAML content from inferred columns.
                        let mut yaml_content = String::from("columns:\n");
                        for (name, dtype) in &suggestion.inferred_columns {
                            yaml_content
                                .push_str(&format!("  - name: {}\n    type: {}\n", name, dtype));
                        }

                        let sidecar_uri = Url::from_file_path(&suggestion.sidecar_path)
                            .unwrap_or_else(|_| uri.clone());

                        let document_changes = vec![
                            DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                                uri: sidecar_uri.clone(),
                                options: None,
                                annotation_id: None,
                            })),
                            DocumentChangeOperation::Edit(TextDocumentEdit {
                                text_document: OptionalVersionedTextDocumentIdentifier {
                                    uri: sidecar_uri,
                                    version: None,
                                },
                                edits: vec![OneOf::Left(TextEdit {
                                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                                    new_text: yaml_content,
                                })],
                            }),
                        ];

                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                document_changes: Some(DocumentChanges::Operations(
                                    document_changes,
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                }
            }
        }

        // Cursor-based CTE refactorings
        if let Some(result) = smelt_db::code_actions::find_extract_cte_suggestion(
            &text,
            adj_start_line,
            request_range.start.character,
        ) {
            let edits: Vec<TextEdit> = result
                .edits
                .iter()
                .map(|e| TextEdit {
                    range: Range {
                        start: Position::new(
                            e.range.start.line + line_offset,
                            e.range.start.column,
                        ),
                        end: Position::new(e.range.end.line + line_offset, e.range.end.column),
                    },
                    new_text: e.new_text.clone(),
                })
                .collect();
            let mut changes = std::collections::HashMap::new();
            changes.insert(uri.clone(), edits);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: result.title,
                kind: Some(CodeActionKind::REFACTOR_EXTRACT),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        if let Some(result) = smelt_db::code_actions::find_inline_cte_suggestion(
            &text,
            adj_start_line,
            request_range.start.character,
        ) {
            let edits: Vec<TextEdit> = result
                .edits
                .iter()
                .map(|e| TextEdit {
                    range: Range {
                        start: Position::new(
                            e.range.start.line + line_offset,
                            e.range.start.column,
                        ),
                        end: Position::new(e.range.end.line + line_offset, e.range.end.column),
                    },
                    new_text: e.new_text.clone(),
                })
                .collect();
            let mut changes = std::collections::HashMap::new();
            changes.insert(uri.clone(), edits);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: result.title,
                kind: Some(CodeActionKind::REFACTOR_INLINE),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        let db = self.snapshot().await;
        let text = file_text(&db, &effective_path);
        let file_input = lookup_file(&db, &effective_path);
        let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
        let syntax = parse.as_ref().map(|p| p.syntax());

        let result = if let Some(syntax) = syntax {
            if let Some(file) = AstFile::cast(syntax) {
                let offset = position_to_offset(
                    &text,
                    effective_position.line,
                    effective_position.character,
                );
                match symbol_at_cursor(&file, &text, offset) {
                    Some(SymbolAtCursor::CteDefinition { name })
                    | Some(SymbolAtCursor::CteReference { name }) => {
                        // Find the CTE definition's name range for prepareRename
                        let mut found_range = None;
                        if let Some(select_stmt) = file.select_stmt() {
                            if let Some(with_clause) = select_stmt.with_clause() {
                                for cte in with_clause.ctes() {
                                    if cte.name().as_deref() == Some(&name) {
                                        if let Some(name_range) = cte.name_range() {
                                            let r = smelt_parser::ast::text_range_to_range(
                                                &text, name_range,
                                            );
                                            found_range = Some((
                                                r.start.line,
                                                r.start.column,
                                                r.end.line,
                                                r.end.column,
                                            ));
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        found_range.map(|(sl, sc, el, ec)| (sl, sc, el, ec, name))
                    }
                    Some(SymbolAtCursor::ColumnRef { qualifier: _, name }) => {
                        // For column references, find the IDENT token at the cursor
                        // and return its range
                        let mut best_range = None;
                        let mut best_len = usize::MAX;
                        for node in file.syntax().descendants() {
                            if let Some(expr) = smelt_parser::ast::Expr::cast(node) {
                                let range = expr.text_range();
                                let start: usize = range.start().into();
                                let end: usize = range.end().into();
                                let len = end - start;
                                if offset >= start && offset <= end && len <= best_len {
                                    if let Some(col_ref) = expr.as_column_ref() {
                                        if col_ref.name() == name {
                                            // Get the name IDENT token range
                                            let tokens: Vec<_> = expr
                                                .syntax()
                                                .children_with_tokens()
                                                .filter_map(|e| e.into_token())
                                                .filter(|t| {
                                                    t.kind() == smelt_parser::SyntaxKind::IDENT
                                                        || t.kind() == smelt_parser::SyntaxKind::DOT
                                                })
                                                .collect();
                                            let name_token = if tokens.len() >= 3 {
                                                Some(&tokens[2]) // qualified: table.column
                                            } else if tokens.len() == 1 {
                                                Some(&tokens[0]) // unqualified
                                            } else {
                                                None
                                            };
                                            if let Some(tok) = name_token {
                                                let r = smelt_parser::ast::text_range_to_range(
                                                    &text,
                                                    tok.text_range(),
                                                );
                                                best_range = Some((
                                                    r.start.line,
                                                    r.start.column,
                                                    r.end.line,
                                                    r.end.column,
                                                ));
                                                best_len = len;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        best_range.map(|(sl, sc, el, ec)| (sl, sc, el, ec, name))
                    }
                    Some(SymbolAtCursor::PathRef { segments }) => {
                        // For path refs, return the range of the entire smelt.<path> node
                        for path_ref in file
                            .syntax()
                            .descendants()
                            .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                        {
                            if path_ref.segments() == segments {
                                let r = smelt_parser::ast::text_range_to_range(
                                    &text,
                                    path_ref.text_range(),
                                );
                                let placeholder = segments.last().cloned().unwrap_or_default();
                                return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                                    range: Range {
                                        start: Position::new(r.start.line, r.start.column),
                                        end: Position::new(r.end.line, r.end.column),
                                    },
                                    placeholder,
                                }));
                            }
                        }
                        return Ok(None);
                    }
                    _ => {
                        // Try lambda-parameter prepare-rename as a fallback.
                        if let Some((start_byte, end_byte, placeholder)) =
                            crate::rename_lambda::prepare_rename_lambda_param(&file, &text, offset)
                        {
                            use smelt_parser::TextRange;
                            let range = TextRange::new(
                                (start_byte as u32).into(),
                                (end_byte as u32).into(),
                            );
                            let r = smelt_parser::ast::text_range_to_range(&text, range);
                            return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                                range: Range {
                                    start: Position::new(r.start.line, r.start.column),
                                    end: Position::new(r.end.line, r.end.column),
                                },
                                placeholder,
                            }));
                        }
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        match result {
            Some((sl, sc, el, ec, placeholder)) => {
                Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: Range {
                        start: Position::new(sl, sc),
                        end: Position::new(el, ec),
                    },
                    placeholder,
                }))
            }
            None => Ok(None),
        }
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        // Validate that new_name is a valid SQL identifier
        if !is_valid_sql_identifier(&new_name) {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(format!(
                "'{}' is not a valid SQL identifier",
                new_name
            )));
        }

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        enum RenameKind {
            Cte {
                edits: Vec<(u32, u32, u32, u32)>,
            },
            Model {
                #[allow(dead_code)]
                model_name: String,
                /// (file_path, start_line, start_col, end_line, end_col)
                edits: Vec<(PathBuf, u32, u32, u32, u32)>,
                /// old .sql file path (if it exists in the project)
                old_model_path: Option<PathBuf>,
            },
            // RenameKind::Source removed in Phase 4: smelt.source() is a parse error;
            // source renames are handled through path-form refs (smelt.sources.*).
            Column {
                /// Local edits in the current file: (start_line, start_col, end_line, end_col)
                local_edits: Vec<(u32, u32, u32, u32)>,
                /// Cross-file edits: (file_path, start_line, start_col, end_line, end_col)
                cross_file_edits: Vec<(PathBuf, u32, u32, u32, u32)>,
                /// YAML column rename edit
                yaml_edit: Option<(u32, String, String)>,
                /// Path to sources.yml
                sources_yml_path: PathBuf,
            },
            /// Lambda parameter — binder + every use in the lambda body.
            LambdaParam {
                /// (start_line, start_col, end_line, end_col) for each renamed span.
                edits: Vec<(u32, u32, u32, u32)>,
            },
        }

        let rename_kind = {
            let db = self.snapshot().await;
            let text = file_text(&db, &effective_path);
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            let syntax = parse.as_ref().map(|p| p.syntax());

            if let Some(syntax) = syntax {
                if let Some(file) = AstFile::cast(syntax) {
                    let offset = position_to_offset(
                        &text,
                        effective_position.line,
                        effective_position.character,
                    );
                    match symbol_at_cursor(&file, &text, offset) {
                        Some(SymbolAtCursor::CteDefinition { name })
                        | Some(SymbolAtCursor::CteReference { name }) => {
                            let cte_refs =
                                smelt_db::references::find_cte_references(&file, &text, &name);
                            let edits = cte_refs
                                .iter()
                                .map(|text_range| {
                                    let r =
                                        smelt_parser::ast::text_range_to_range(&text, *text_range);
                                    (r.start.line, r.start.column, r.end.line, r.end.column)
                                })
                                .collect();
                            Some(RenameKind::Cte { edits })
                        }
                        Some(SymbolAtCursor::ColumnRef {
                            qualifier,
                            name: column_name,
                        }) => {
                            // Find all column references in the current file
                            let local_refs = smelt_db::references::find_column_references_in_file(
                                &file,
                                &column_name,
                                qualifier.as_deref(),
                            );
                            let mut local_edits: Vec<(u32, u32, u32, u32)> = local_refs
                                .iter()
                                .map(|r| {
                                    let range =
                                        smelt_parser::ast::text_range_to_range(&text, r.name_range);
                                    (
                                        range.start.line,
                                        range.start.column,
                                        range.end.line,
                                        range.end.column,
                                    )
                                })
                                .collect();

                            // Include column definition in SELECT list
                            if let Some(def_range) =
                                smelt_db::references::find_column_definition_in_select(
                                    &file,
                                    &column_name,
                                )
                            {
                                let range =
                                    smelt_parser::ast::text_range_to_range(&text, def_range);
                                let edit = (
                                    range.start.line,
                                    range.start.column,
                                    range.end.line,
                                    range.end.column,
                                );
                                if !local_edits.contains(&edit) {
                                    local_edits.push(edit);
                                }
                            }
                            local_edits.sort();
                            local_edits.dedup();

                            // Cross-file tracing
                            let mut cross_file_edits = Vec::new();
                            let all_files = all_file_paths(&db);
                            let schema = file_input
                                .map(|f| smelt_db::model_schema(&db, f))
                                .unwrap_or_else(|| Arc::new(smelt_db::ModelSchema::empty()));
                            let ws = Workspace::try_get(&db);
                            let ctx = file_input
                                .and_then(|f| ws.map(|w| smelt_db::type_context(&db, w, f)))
                                .unwrap_or_else(|| Arc::new(smelt_db::TypeContext::new()));

                            // Upstream tracing: resolve which models to check
                            // First try ColumnSource::FromModel (column is in SELECT list)
                            let mut upstream_traced = false;
                            if let Some(col) = schema.columns.iter().find(|c| c.name == column_name)
                            {
                                if let smelt_db::schema::ColumnSource::FromModel {
                                    model_name,
                                    column_name: ref upstream_col,
                                } = &col.source
                                {
                                    if upstream_col == &column_name {
                                        trace_upstream_column(
                                            &db,
                                            &all_files,
                                            model_name,
                                            &column_name,
                                            &mut cross_file_edits,
                                        );
                                        upstream_traced = true;
                                    }
                                }
                            }

                            // If column not in schema (used in expressions like e.col ->> 'key'),
                            // resolve the qualifier alias to find the upstream model
                            if !upstream_traced {
                                let model_names: Vec<String> = if let Some(ref q) = qualifier {
                                    // Resolve alias (e.g., "e" -> "events")
                                    let resolved =
                                        ctx.resolve_alias(q).unwrap_or_else(|| q.to_string());
                                    if !ctx.is_cte(&resolved) {
                                        vec![resolved]
                                    } else {
                                        vec![]
                                    }
                                } else {
                                    collect_from_model_names(&db, &effective_path)
                                };

                                for model_name in &model_names {
                                    trace_upstream_column(
                                        &db,
                                        &all_files,
                                        model_name,
                                        &column_name,
                                        &mut cross_file_edits,
                                    );
                                }
                            }

                            // Downstream tracing via BFS through model graph
                            let current_model_name = effective_path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_string();
                            let mut models_exposing: Vec<String> = vec![current_model_name.clone()];
                            let mut visited = std::collections::HashSet::new();
                            visited.insert(current_model_name);
                            let mut depth = 0;

                            while depth < 10 {
                                let mut next_batch = Vec::new();
                                for exposing in &models_exposing {
                                    for down_path in all_files.iter() {
                                        if *down_path == effective_path {
                                            continue;
                                        }
                                        let down_name = down_path
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if visited.contains(&down_name) {
                                            continue;
                                        }
                                        let down_file_input = match lookup_file(&db, down_path) {
                                            Some(f) => f,
                                            None => continue,
                                        };
                                        let down_model_path_refs =
                                            smelt_db::model_path_refs(&db, down_file_input);
                                        if !down_model_path_refs.iter().any(|r| {
                                            r.path.first().map(|s| s.as_str()) == Some("models")
                                                && r.path.get(1).map(|s| s.as_str())
                                                    == Some(exposing.as_str())
                                        }) {
                                            continue;
                                        }
                                        let down_text = down_file_input.text(&db).clone();
                                        let down_parse = smelt_db::parse_file(&db, down_file_input);
                                        let down_syntax = down_parse.syntax();
                                        if let Some(down_file) = AstFile::cast(down_syntax) {
                                            let col_refs =
                                        smelt_db::references::find_column_references_in_file(
                                            &down_file,
                                            &column_name,
                                            None,
                                        );
                                            for col_ref in &col_refs {
                                                let r = smelt_parser::ast::text_range_to_range(
                                                    &down_text,
                                                    col_ref.name_range,
                                                );
                                                cross_file_edits.push((
                                                    down_path.clone(),
                                                    r.start.line,
                                                    r.start.column,
                                                    r.end.line,
                                                    r.end.column,
                                                ));
                                            }
                                            // Check for SELECT * passthrough
                                            let down_schema =
                                                smelt_db::model_schema(&db, down_file_input);
                                            if down_schema
                                                .row_extensions
                                                .iter()
                                                .any(|ext| ext.ref_name == *exposing)
                                            {
                                                next_batch.push(down_name.clone());
                                            }
                                            visited.insert(down_name);
                                        }
                                    }
                                }
                                if next_batch.is_empty() {
                                    break;
                                }
                                models_exposing = next_batch;
                                depth += 1;
                            }

                            // Source column YAML rename
                            let project_root = file_project_root(&db, &effective_path);
                            let sources_yml_content = project_sources_yaml(&db, &project_root);
                            let sources_yml_path = project_root.join("sources.yml");
                            let yaml_edit = find_source_column_yaml_rename(
                                &sources_yml_content,
                                &column_name,
                                &new_name,
                            );

                            Some(RenameKind::Column {
                                local_edits,
                                cross_file_edits,
                                yaml_edit,
                                sources_yml_path,
                            })
                        }
                        Some(SymbolAtCursor::PathRef { segments })
                            if segments.first().map(|s| s.as_str()) == Some("models") =>
                        {
                            if let Some(model_name) = segments.get(1).cloned() {
                                // Collect all smelt.models.<model_name> path-ref ranges across workspace
                                let ws = Workspace::try_get(&db);
                                let ws_files = ws.map(|w| w.files(&db).clone()).unwrap_or_default();
                                let mut edits: Vec<(PathBuf, u32, u32, u32, u32)> = Vec::new();
                                for f in &ws_files {
                                    let path = f.path(&db).clone();
                                    let f_parse = smelt_db::parse_file(&db, *f);
                                    let f_text = f.text(&db).clone();
                                    let f_syntax = f_parse.syntax();
                                    if let Some(f_file) = AstFile::cast(f_syntax) {
                                        for path_ref in f_file
                                            .syntax()
                                            .descendants()
                                            .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                                        {
                                            let segs = path_ref.segments();
                                            if segs.first().map(|s| s.as_str()) == Some("models")
                                                && segs.get(1).map(|s| s.as_str())
                                                    == Some(model_name.as_str())
                                            {
                                                let r = smelt_parser::ast::text_range_to_range(
                                                    &f_text,
                                                    path_ref.text_range(),
                                                );
                                                edits.push((
                                                    path.clone(),
                                                    r.start.line,
                                                    r.start.column,
                                                    r.end.line,
                                                    r.end.column,
                                                ));
                                            }
                                        }
                                    }
                                }

                                // Compute old model file path
                                let old_model_path = ws
                                    .and_then(|w| smelt_db::resolve_ref(&db, w, model_name.clone()))
                                    .map(|sf| sf.path(&db).clone());

                                Some(RenameKind::Model {
                                    model_name,
                                    edits,
                                    old_model_path,
                                })
                            } else {
                                None
                            }
                        }
                        _ => {
                            // Try lambda-parameter rename as a fallback.
                            match crate::rename_lambda::rename_lambda_param(
                                &file, &text, offset, &new_name,
                            ) {
                                Ok(crate::rename_lambda::RenameLambdaResult::Edits(byte_edits)) => {
                                    let lsp_edits = crate::rename_lambda::byte_edits_to_lsp_ranges(
                                        &text, byte_edits,
                                    );
                                    Some(RenameKind::LambdaParam { edits: lsp_edits })
                                }
                                Ok(crate::rename_lambda::RenameLambdaResult::NotALambdaParam) => {
                                    None
                                }
                                Err(e) => {
                                    return Err(tower_lsp::jsonrpc::Error::invalid_params(
                                        e.to_string(),
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }; // end of block — parse/syntax dropped before awaits

        match rename_kind {
            Some(RenameKind::Cte { edits }) => {
                if edits.is_empty() {
                    return Ok(None);
                }
                let text_edits: Vec<TextEdit> = edits
                    .into_iter()
                    .map(|(sl, sc, el, ec)| TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    })
                    .collect();
                let mut changes = HashMap::new();
                changes.insert(uri, text_edits);
                Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }))
            }
            Some(RenameKind::Model {
                model_name: _,
                edits,
                old_model_path,
            }) => {
                if edits.is_empty() && old_model_path.is_none() {
                    return Ok(None);
                }

                // Build DocumentChanges with text edits per file + optional RenameFile
                let mut document_changes: Vec<DocumentChangeOperation> = Vec::new();

                // Group text edits by file path
                let mut edits_by_file: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
                for (file_path, sl, sc, el, ec) in edits {
                    edits_by_file.entry(file_path).or_default().push(TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    });
                }

                // Add text edit operations
                for (file_path, file_edits) in edits_by_file {
                    let file_uri = Url::from_file_path(&file_path).unwrap_or_else(|_| uri.clone());
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: file_uri,
                            version: None,
                        },
                        edits: file_edits.into_iter().map(OneOf::Left).collect(),
                    }));
                }

                // Add file rename operation and update Salsa DB
                if let Some(old_path) = old_model_path {
                    let new_path = old_path
                        .parent()
                        .unwrap_or(old_path.as_ref())
                        .join(format!("{}.sql", new_name));
                    let old_uri = Url::from_file_path(&old_path).unwrap_or_else(|_| uri.clone());
                    let new_uri = Url::from_file_path(&new_path).unwrap_or_else(|_| uri.clone());
                    document_changes.push(DocumentChangeOperation::Op(ResourceOp::Rename(
                        RenameFile {
                            old_uri,
                            new_uri,
                            options: None,
                            annotation_id: None,
                        },
                    )));

                    // Pre-update the Salsa DB so diagnostics see the new filename
                    // before VSCode sends didOpen/didChange notifications.
                    let mut db = self.db.lock().await;
                    let old_text = file_text(&db, &old_path);
                    let old_project_root = file_project_root(&db, &old_path);
                    db.set_source_file(new_path.clone(), old_text, old_project_root);
                    let mut tracked = self.tracked_files.lock().await;
                    tracked.retain(|p| *p != old_path);
                    if !tracked.contains(&new_path) {
                        tracked.push(new_path);
                    }
                    let project_roots = self.project_roots.lock().await.clone();
                    Backend::sync_workspace(&mut db, &tracked, &project_roots);
                    drop(tracked);
                    drop(db);
                }

                Ok(Some(WorkspaceEdit {
                    document_changes: Some(DocumentChanges::Operations(document_changes)),
                    ..Default::default()
                }))
            }
            Some(RenameKind::Column {
                local_edits,
                cross_file_edits,
                yaml_edit,
                sources_yml_path,
            }) => {
                if local_edits.is_empty() && cross_file_edits.is_empty() {
                    return Ok(None);
                }

                let mut document_changes: Vec<DocumentChangeOperation> = Vec::new();

                // Local edits in the current file
                if !local_edits.is_empty() {
                    let local_text_edits: Vec<OneOf<TextEdit, AnnotatedTextEdit>> = local_edits
                        .into_iter()
                        .map(|(sl, sc, el, ec)| {
                            OneOf::Left(TextEdit {
                                range: Range {
                                    start: Position::new(sl, sc),
                                    end: Position::new(el, ec),
                                },
                                new_text: new_name.clone(),
                            })
                        })
                        .collect();
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: uri.clone(),
                            version: None,
                        },
                        edits: local_text_edits,
                    }));
                }

                // Cross-file edits
                let mut edits_by_file: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
                for (file_path, sl, sc, el, ec) in cross_file_edits {
                    edits_by_file.entry(file_path).or_default().push(TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    });
                }
                for (file_path, file_edits) in edits_by_file {
                    let file_uri = Url::from_file_path(&file_path).unwrap_or_else(|_| uri.clone());
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: file_uri,
                            version: None,
                        },
                        edits: file_edits.into_iter().map(OneOf::Left).collect(),
                    }));
                }

                // YAML column rename
                if let Some((line_num, _old_line, new_line)) = yaml_edit {
                    let yaml_uri =
                        Url::from_file_path(&sources_yml_path).unwrap_or_else(|_| uri.clone());
                    let old_line_len = _old_line.len() as u32;
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: yaml_uri,
                            version: None,
                        },
                        edits: vec![OneOf::Left(TextEdit {
                            range: Range {
                                start: Position::new(line_num, 0),
                                end: Position::new(line_num, old_line_len),
                            },
                            new_text: new_line,
                        })],
                    }));
                }

                Ok(Some(WorkspaceEdit {
                    document_changes: Some(DocumentChanges::Operations(document_changes)),
                    ..Default::default()
                }))
            }
            Some(RenameKind::LambdaParam { edits }) => {
                if edits.is_empty() {
                    return Ok(None);
                }
                let text_edits: Vec<TextEdit> = edits
                    .into_iter()
                    .map(|(sl, sc, el, ec)| TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    })
                    .collect();
                let mut changes = HashMap::new();
                changes.insert(uri, text_edits);
                Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }))
            }
            None => Ok(None),
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path and adjust position
        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        let db = self.snapshot().await;

        // Get file content and parse tree
        let text = file_text(&db, &effective_path);
        let file_input = lookup_file(&db, &effective_path);
        let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
        let syntax = parse.as_ref().map(|p| p.syntax());

        // Convert cursor position to offset
        let cursor_offset = {
            let mut offset = 0usize;
            let mut line = 0u32;
            let mut col = 0u32;

            for ch in text.chars() {
                if line == effective_position.line && col == effective_position.character {
                    break;
                }
                if ch == '\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
                offset += ch.len_utf8();
            }
            offset
        };

        // Check if hovering over a smelt.<path> ref
        if let Some(syntax) = syntax {
            if let Some(file) = AstFile::cast(syntax) {
                // Check smelt.models.<name> path refs
                for path_ref in file
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                {
                    let segments = path_ref.segments();
                    if segments.first().map(|s| s.as_str()) != Some("models") {
                        continue;
                    }
                    let range = path_ref.text_range();
                    let start: usize = range.start().into();
                    let end: usize = range.end().into();

                    // Check if cursor is within this path ref
                    if cursor_offset >= start && cursor_offset <= end {
                        if let Some(model_name) = segments.get(1).cloned() {
                            // Resolve upstream model and show its resolved schema
                            let ws = Workspace::try_get(&db);
                            let upstream_file =
                                ws.and_then(|w| smelt_db::resolve_ref(&db, w, model_name.clone()));
                            if let (Some(upstream), Some(w)) = (upstream_file, ws) {
                                // Use resolved_model_schema to get type information through wildcards
                                let resolved = smelt_db::resolved_model_schema(&db, w, upstream);

                                // Format schema as markdown
                                let mut content = format!("**Model: {}**\n\n", model_name);
                                content.push_str("| Column | Type | Source |\n");
                                content.push_str("|--------|------|--------|\n");

                                for col in resolved.columns.iter() {
                                    // Column name
                                    content.push_str(&format!("| `{}` | ", col.name));

                                    // Type (if known)
                                    if let Some(ref typed_col) = col.data_type {
                                        content.push_str(&format!("`{}`", format_type(typed_col)));
                                    } else {
                                        content.push_str("*unknown*");
                                    }
                                    content.push_str(" | ");

                                    // Source info
                                    match &col.source {
                                        smelt_db::ColumnSource::FromModel {
                                            model_name,
                                            column_name,
                                        } => {
                                            content.push_str(&format!(
                                                "from `{}.{}`",
                                                model_name, column_name
                                            ));
                                        }
                                        smelt_db::ColumnSource::Computed => {
                                            if !col.expression.is_empty()
                                                && col.expression != col.name
                                            {
                                                content.push_str(&format!("`{}`", col.expression));
                                            } else {
                                                content.push_str("computed");
                                            }
                                        }
                                        smelt_db::ColumnSource::Wildcard { model_name } => {
                                            content.push_str(&format!("* from `{}`", model_name));
                                        }
                                        smelt_db::ColumnSource::ExternalTable { table_name } => {
                                            content.push_str(&format!("from `{}`", table_name));
                                        }
                                        smelt_db::ColumnSource::Unknown => {
                                            content.push('-');
                                        }
                                    }

                                    content.push_str(" |\n");
                                }

                                // Show unresolved row extensions
                                if !resolved.unresolved_extensions.is_empty() {
                                    content.push_str("\n*...plus columns from:*\n");
                                    for ext in &resolved.unresolved_extensions {
                                        content.push_str(&format!("- `{}`\n", ext.ref_name));
                                    }
                                }

                                // Show input constraints
                                let constraints =
                                    smelt_db::model_input_constraints(&db, w, upstream);
                                if !constraints.is_empty() {
                                    content.push_str("\n**Requires:**\n");
                                    for constraint in constraints.iter() {
                                        for (col_name, col_constraint) in
                                            &constraint.required_columns
                                        {
                                            if let Some(ref typed_col) =
                                                col_constraint.expected_type
                                            {
                                                content.push_str(&format!(
                                                    "- `{}` (`{}`) from `{}`\n",
                                                    col_name,
                                                    format_type(typed_col),
                                                    constraint.ref_name,
                                                ));
                                            } else {
                                                content.push_str(&format!(
                                                    "- `{}` from `{}`\n",
                                                    col_name, constraint.ref_name,
                                                ));
                                            }
                                        }
                                    }
                                }

                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: content,
                                    }),
                                    range: None,
                                }));
                            }
                        }
                    }
                }

                // Check smelt.sources.<source>.<table> path refs
                for path_ref in file
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                {
                    let segments = path_ref.segments();
                    if segments.first().map(|s| s.as_str()) != Some("sources") {
                        continue;
                    }
                    let range = path_ref.text_range();
                    let start: usize = range.start().into();
                    let end: usize = range.end().into();

                    // Check if cursor is within this path ref
                    if cursor_offset >= start && cursor_offset <= end {
                        if let (Some(source_name), Some(table_name)) =
                            (segments.get(1).cloned(), segments.get(2).cloned())
                        {
                            let qualified_name = format!("{}.{}", source_name, table_name);

                            // Try to resolve the source
                            let project_root = file_project_root(&db, &effective_path);
                            let project = lookup_project(&db, &project_root);
                            if let Some(table_def) = project.and_then(|p| {
                                smelt_db::resolve_source(
                                    &db,
                                    p,
                                    source_name.clone(),
                                    table_name.clone(),
                                )
                            }) {
                                // Format source info as markdown
                                let mut content = format!("**Source: {}**\n\n", qualified_name);

                                // Show table description if available
                                if let Some(ref desc) = table_def.description {
                                    content.push_str(&format!("{}\n\n", desc));
                                }

                                if !table_def.columns.is_empty() {
                                    content.push_str("Columns:\n");
                                    for col in &table_def.columns {
                                        content.push_str(&format!("- `{}`", col.name));
                                        if let Some(ref dtype) = col.data_type {
                                            content.push_str(&format!(" ({})", dtype));
                                        }
                                        if let Some(ref desc) = col.description {
                                            content.push_str(&format!(" - {}", desc));
                                        }
                                        content.push('\n');
                                    }
                                } else {
                                    content.push_str("*(No column definitions)*\n");
                                }

                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: content,
                                    }),
                                    range: None,
                                }));
                            } else {
                                // Source not found - show error hover
                                let content = format!(
                                    "**Source: {}**\n\n⚠️ *Undefined source*",
                                    qualified_name
                                );

                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: content,
                                    }),
                                    range: None,
                                }));
                            }
                        }
                    }
                }

                // Check smelt.<path> seed refs — path segments match address_segments
                for path_ref in file
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                {
                    let segments = path_ref.segments();
                    // Skip refs already handled as models or sources
                    let first = segments.first().map(|s| s.as_str());
                    if first == Some("models") || first == Some("sources") {
                        continue;
                    }
                    let range = path_ref.text_range();
                    let start: usize = range.start().into();
                    let end: usize = range.end().into();

                    if cursor_offset >= start && cursor_offset <= end {
                        let project_root = file_project_root(&db, &effective_path);
                        let project = lookup_project(&db, &project_root);
                        if let Some(proj) = project {
                            let seeds = smelt_db::project_seeds(&db, proj);
                            if let Some(seed) = seeds
                                .iter()
                                .find(|s| s.address_segments == segments.as_slice())
                            {
                                let qualified_name = segments.join(".");
                                let mut content = format!("**Seed: {}**\n\n", qualified_name);

                                if seed.columns.is_empty() {
                                    content.push_str("*(No column definitions)*\n");
                                } else {
                                    content.push_str("Columns:\n");
                                    for (col_name, dtype) in &seed.columns {
                                        content.push_str(&format!("- `{}` ({})", col_name, dtype));
                                        // Include description from sidecar if present
                                        if let Some(ref sidecar) = seed.sidecar {
                                            if let Some(ref cols) = sidecar.columns {
                                                if let Some(sc) =
                                                    cols.iter().find(|c| &c.name == col_name)
                                                {
                                                    if let Some(ref desc) = sc.description {
                                                        content.push_str(&format!(" - {}", desc));
                                                    }
                                                }
                                            }
                                        }
                                        content.push('\n');
                                    }
                                }

                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: content,
                                    }),
                                    range: None,
                                }));
                            }
                        }
                    }
                }

                // Check smelt.define parameters — Phase 18 hover
                if let Some(file_input) = lookup_file(&db, &effective_path) {
                    let fn_sigs = functions_in_file(&db, file_input);
                    for define in file.defines() {
                        let fn_name = define.name().unwrap_or_default();
                        if let Some(param_list) = define.param_list() {
                            for param in param_list.params() {
                                let param_range = param.syntax().text_range();
                                let start: usize = param_range.start().into();
                                let end: usize = param_range.end().into();
                                if cursor_offset >= start && cursor_offset <= end {
                                    let param_name = param.name().unwrap_or_default();
                                    let type_display = fn_sigs
                                        .iter()
                                        .find(|s| s.name == fn_name)
                                        .and_then(|s| {
                                            s.params.iter().find(|p| p.name == param_name)
                                        })
                                        .and_then(|p| {
                                            p.type_ref
                                                .as_ref()?
                                                .as_ref()
                                                .ok()
                                                .map(format_smelt_type_hover)
                                        })
                                        .unwrap_or_else(|| "unknown".to_string());
                                    let content = format!(
                                        "**`{param_name}`** (parameter of `{fn_name}`)\n\n\
                                         `{param_name}: {type_display}`"
                                    );
                                    return Ok(Some(Hover {
                                        contents: HoverContents::Markup(MarkupContent {
                                            kind: MarkupKind::Markdown,
                                            value: content,
                                        }),
                                        range: None,
                                    }));
                                }
                            }
                        }
                    }
                }

                // Phase 4 (meta-language): hover on ARRAY_LITERAL — show the
                // inferred List<T> type (or dual meta + data-world reading).
                //
                // Guard: if the matched ARRAY_LITERAL is the operand child of a
                // LIST_SPREAD node, skip here and let the LIST_SPREAD dispatch
                // below handle it. This ensures "hover on `[…]` inside `...[…]`
                // shows the source list type" is honoured by design rather than
                // by accident (spec rule: hover on spread shows source list type).
                {
                    use smelt_parser::syntax_kind::SyntaxKind;

                    // Walk descendants to find an ARRAY_LITERAL node that
                    // contains the cursor offset and is NOT the operand of a
                    // LIST_SPREAD parent.
                    let array_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::ARRAY_LITERAL)
                        .find(|n| {
                            let start: usize = n.text_range().start().into();
                            let end: usize = n.text_range().end().into();
                            if !(cursor_offset >= start && cursor_offset <= end) {
                                return false;
                            }
                            // Skip if this ARRAY_LITERAL is the direct child of a
                            // LIST_SPREAD — the spread dispatch handles that case.
                            let parent_is_spread = n
                                .parent()
                                .map(|p| p.kind() == SyntaxKind::LIST_SPREAD)
                                .unwrap_or(false);
                            !parent_is_spread
                        });

                    if let Some(arr_node) = array_node {
                        if let Some(arr) = smelt_parser::ast::ArrayLiteral::cast(arr_node) {
                            let elems: Vec<smelt_parser::ast::Expr> = arr.elements();
                            let ctx = smelt_db::TypeContext::new();

                            let value = hover_text_for_list_literal_dual(&elems, &ctx);

                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }

                    // Phase 4 (meta-language): hover on LIST_SPREAD — show
                    // the source list's type.
                    let spread_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::LIST_SPREAD)
                        .find(|n| {
                            let start: usize = n.text_range().start().into();
                            let end: usize = n.text_range().end().into();
                            cursor_offset >= start && cursor_offset <= end
                        });

                    if let Some(sp_node) = spread_node {
                        if let Some(spread) = smelt_parser::ast::ListSpread::cast(sp_node) {
                            let ctx = smelt_db::TypeContext::new();
                            let value = hover_text_for_list_spread(&spread, &ctx);
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }
                }

                // Phase 48: hover on a `smelt.fn.<name>(...)` call site —
                // surface the declared return type or the parameter binding
                // for a `PASSING <name> AS (...)` clause.
                if let Some(call) = find_smelt_fn_call_at_cursor(file.syntax(), cursor_offset) {
                    let segments = call.segments();
                    let fn_name = segments.last().cloned().unwrap_or_default();
                    let ws = Workspace::try_get(&db);
                    // Project isolation: hover resolves the same way the
                    // diagnostic and goto-def code paths do — only against
                    // functions declared in the cursor file's project.
                    let project_root = file_project_root(&db, &effective_path);
                    let project = lookup_project(&db, &project_root);
                    let sig = ws
                        .zip(project)
                        .and_then(|(w, p)| smelt_db::resolve_function(&db, w, p, fn_name.clone()));

                    if let Some(sig) = sig {
                        // Phase 48 test 2: cursor on a PASSING clause name.
                        for passing in call.passing_clauses() {
                            if let Some(name_range) = passing.name_range() {
                                let start: usize = name_range.start().into();
                                let end: usize = name_range.end().into();
                                if cursor_offset >= start && cursor_offset <= end {
                                    if let Some(name) = passing.name() {
                                        let type_text = sig
                                            .params
                                            .iter()
                                            .find(|p| p.name == name)
                                            .and_then(|p| p.type_ref_text.clone())
                                            .unwrap_or_else(|| "unknown".to_string());
                                        let content = format!(
                                            "**`{name}`** (parameter of `{}`)\n\n`{name}: {type_text}`",
                                            sig.name
                                        );
                                        return Ok(Some(Hover {
                                            contents: HoverContents::Markup(MarkupContent {
                                                kind: MarkupKind::Markdown,
                                                value: content,
                                            }),
                                            range: None,
                                        }));
                                    }
                                }
                            }
                        }

                        // Phase 48 test 1: cursor on the call path —
                        // surface the declared return type.
                        if let Some(call_path_range) = call.call_path_range() {
                            let start: usize = call_path_range.start().into();
                            let end: usize = call_path_range.end().into();
                            if cursor_offset >= start && cursor_offset <= end {
                                if let Some(text) = smelt_db::declared_return_hover_text(&sig) {
                                    let content = format!("`{}` `{text}`", sig.name);
                                    return Ok(Some(Hover {
                                        contents: HoverContents::Markup(MarkupContent {
                                            kind: MarkupKind::Markdown,
                                            value: content,
                                        }),
                                        range: None,
                                    }));
                                }
                            }
                        }
                    }
                }

                // Phase D: wide-reflection accessor hover with Salsa-backed resolution.
                //
                // Must run BEFORE `hover_text_for_hof_meta_language` so the richer
                // Salsa-resolved version (with counts + names) wins over the pure
                // fallback (which shows None for workspace state).
                {
                    let wide_call = file
                        .syntax()
                        .descendants()
                        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
                        .find(|c| {
                            let segs = c.segments();
                            let first = segs.first().map(|s| s.as_str());
                            let second = segs.get(1).map(|s| s.as_str());
                            let is_wide = (first == Some("models") || first == Some("sources"))
                                && (second == Some("with_tag") || second == Some("all"));
                            if !is_wide {
                                return false;
                            }
                            let r = c.text_range();
                            let s: usize = r.start().into();
                            let e: usize = r.end().into();
                            cursor_offset >= s && cursor_offset <= e
                        });

                    if let Some(call) = wide_call {
                        let segs = call.segments();
                        let namespace = segs.first().map(|s| s.as_str()).unwrap_or("models");
                        let accessor = segs.get(1).map(|s| s.as_str()).unwrap_or("all");
                        let ws = Workspace::try_get(&db);

                        let value = if namespace == "models" {
                            if accessor == "with_tag" {
                                let tag = call
                                    .arg_list()
                                    .and_then(|al| al.positional_args().into_iter().next())
                                    .map(|a| {
                                        let t = a.text();
                                        t.trim_matches('\'').trim_matches('"').to_string()
                                    })
                                    .unwrap_or_default();
                                let resolved =
                                    ws.map(|w| smelt_db::models_with_tag(&db, w, tag.clone()));
                                hover_text_for_models_with_tag_call(
                                    &tag,
                                    resolved.as_ref().map(|v| v.as_slice()),
                                )
                            } else {
                                let resolved = ws.map(|w| smelt_db::models_all(&db, w));
                                hover_text_for_models_all(resolved.as_ref().map(|v| v.len()))
                            }
                        } else {
                            // sources
                            let project_root = file_project_root(&db, &effective_path);
                            let project = lookup_project(&db, &project_root);
                            if accessor == "with_tag" {
                                let tag = call
                                    .arg_list()
                                    .and_then(|al| al.positional_args().into_iter().next())
                                    .map(|a| {
                                        let t = a.text();
                                        t.trim_matches('\'').trim_matches('"').to_string()
                                    })
                                    .unwrap_or_default();
                                let resolved = project
                                    .map(|p| smelt_db::sources_with_tag(&db, p, tag.clone()));
                                hover_text_for_sources_with_tag_call(
                                    &tag,
                                    resolved.as_ref().map(|v| v.as_slice()),
                                )
                            } else {
                                let resolved = project.map(|p| smelt_db::sources_all(&db, p));
                                hover_text_for_sources_all(resolved.as_ref().map(|v| v.len()))
                            }
                        };
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Phase C: smelt.columns_of hover with Salsa-backed column resolution.
                //
                // Must run BEFORE `hover_text_for_hof_meta_language` so the richer
                // Salsa-resolved version (with column count + names) wins over the
                // pure fallback (which returns only `List<ColumnRef>` with no columns).
                {
                    let columns_of_call = file
                        .syntax()
                        .descendants()
                        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
                        .filter(|c| c.segments() == vec!["columns_of".to_string()])
                        .find(|c| {
                            let r = c.text_range();
                            let s: usize = r.start().into();
                            let e: usize = r.end().into();
                            cursor_offset >= s && cursor_offset <= e
                        });

                    if let Some(call) = columns_of_call {
                        let table_name = call
                            .arg_list()
                            .and_then(|al| al.positional_args().into_iter().next())
                            .map(|a| a.text())
                            .unwrap_or_else(|| "?".to_string());

                        // Try Salsa resolution.
                        let ws = Workspace::try_get(&db);
                        let resolved_cols = ws.and_then(|w| {
                            smelt_db::columns_of_for_table_expr(&db, w, table_name.clone()).ok()
                        });

                        let value = hover_text_for_columns_of_call(
                            &table_name,
                            resolved_cols.as_ref().map(|v| v.as_slice()),
                        );
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Phase B: meta-language hover (reducer name, lambda param binder/body
                // use, HOF result type, smelt.config.var, smelt.columns_of fallback,
                // ColumnRef field projection).  All sub-cases are handled by the
                // `hover_text_for_hof_meta_language` pure helper so they can be tested
                // without a live Backend.
                //
                // NOTE: this block MUST run before the PIPE_EXPR check below.
                // A pipe expression like `[1,2,3] |> filter(fn c => c > 0)` has a
                // PIPE_EXPR ancestor that spans `c`.  If pipe hover ran first it
                // would intercept the lambda-param hover for `c`.
                {
                    let project_root = file_project_root(&db, &effective_path);
                    let project = lookup_project(&db, &project_root);
                    let smelt_yml = project
                        .map(|p| p.smelt_yml_text(&db).clone())
                        .unwrap_or_default();
                    if let Some(value) =
                        hover_text_for_hof_meta_language(&file, cursor_offset, &smelt_yml)
                    {
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Phase B: hover on a PIPE_EXPR node — show result type of the
                // desugared call.
                {
                    use smelt_parser::syntax_kind::SyntaxKind;
                    let pipe_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::PIPE_EXPR)
                        .find(|n| {
                            let start: usize = n.text_range().start().into();
                            let end: usize = n.text_range().end().into();
                            cursor_offset >= start && cursor_offset <= end
                        });
                    if let Some(pn) = pipe_node {
                        if let Some(pipe) = smelt_parser::ast::PipeExpr::cast(pn) {
                            let ctx = smelt_db::TypeContext::new();
                            let value = hover_text_for_pipe_expr(&pipe, &ctx);
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }
                }

                // Phase E2: generator-file hover —
                // (a) cursor in YAML frontmatter on the `generates: models` value,
                // (b) cursor on a `ModelDef { … }` opening brace,
                // (c) cursor on the `name:` field value in a `ModelDef` literal,
                // (d) cursor on the `body:` field value in a `ModelDef` literal.
                {
                    let raw = text.as_str();
                    // Detect generator files by checking frontmatter variant.
                    if let Ok(smelt_core::metadata::FileMetadata::Generator {
                        body_offset, ..
                    }) = smelt_core::metadata::extract_file_metadata(raw)
                    {
                        // (a) Is the cursor in the frontmatter (before body_offset)?
                        if cursor_offset < body_offset {
                            // Check if the line under the cursor contains `generates:`
                            let line_start =
                                raw[..cursor_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
                            let line_end = raw[cursor_offset..]
                                .find('\n')
                                .map(|p| cursor_offset + p)
                                .unwrap_or(raw.len());
                            let line_text = &raw[line_start..line_end];
                            if line_text.trim_start().starts_with("generates:") {
                                // Resolve emission count from Salsa.
                                let ws = Workspace::try_get(&db);
                                let emission_count = ws.and_then(|w| {
                                    let gen_files = smelt_db::generator_files(&db, w);
                                    let file_input = lookup_file(&db, &effective_path);
                                    file_input.and_then(|fi| {
                                        gen_files.iter().find(|&&gf| gf == fi).map(|&gf| {
                                            smelt_db::evaluate_generator(&db, w, gf).emissions.len()
                                        })
                                    })
                                });
                                let value = hover_text_for_generates_frontmatter(emission_count);
                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value,
                                    }),
                                    range: None,
                                }));
                            }
                        } else {
                            // Cursor is in the generator body — check for ModelDef positions.
                            // We look at the CST for RECORD_LITERAL nodes whose first
                            // keyword is `ModelDef`.

                            // Walk record literals to find a ModelDef that contains cursor.
                            use smelt_parser::SyntaxKind;
                            for node in file.syntax().descendants() {
                                if node.kind() != SyntaxKind::RECORD_LITERAL {
                                    continue;
                                }
                                let rec_start: usize = node.text_range().start().into();
                                let rec_end: usize = node.text_range().end().into();
                                if !(cursor_offset >= rec_start && cursor_offset <= rec_end) {
                                    continue;
                                }
                                // Check that this record starts with `ModelDef`.
                                let first_tok = node
                                    .children_with_tokens()
                                    .filter_map(|e| e.into_token())
                                    .find(|t| !t.kind().is_trivia());
                                let is_model_def = first_tok
                                    .as_ref()
                                    .map(|t| t.text() == "ModelDef")
                                    .unwrap_or(false);
                                if !is_model_def {
                                    continue;
                                }

                                // (b) Is the cursor on the `ModelDef` IDENT keyword
                                // or the opening brace?  Both positions serve the
                                // same hover content per the spec.
                                let open_brace_tok = node
                                    .children_with_tokens()
                                    .filter_map(|e| e.into_token())
                                    .find(|t| t.kind() == SyntaxKind::LBRACE);
                                let on_keyword = first_tok
                                    .as_ref()
                                    .map(|t| {
                                        let s: usize = t.text_range().start().into();
                                        let e: usize = t.text_range().end().into();
                                        cursor_offset >= s && cursor_offset <= e
                                    })
                                    .unwrap_or(false);
                                let on_brace = open_brace_tok
                                    .as_ref()
                                    .map(|t| {
                                        let s: usize = t.text_range().start().into();
                                        let e: usize = t.text_range().end().into();
                                        cursor_offset >= s && cursor_offset <= e
                                    })
                                    .unwrap_or(false);
                                if on_keyword || on_brace {
                                    // Resolve emitted smelt path from Salsa survivors.
                                    let ws = Workspace::try_get(&db);
                                    let smelt_path: Option<String> = ws.and_then(|w| {
                                        let survivors = smelt_db::emitted_models(&db, w);
                                        let project_root = file_project_root(&db, &effective_path);
                                        let project = lookup_project(&db, &project_root);
                                        let scan_roots = project
                                            .map(|p| {
                                                smelt_db::project_paths(&db, p).as_ref().clone()
                                            })
                                            .unwrap_or_else(|| vec!["models".to_string()]);
                                        // Find the survivor whose generator_file
                                        // matches this file AND whose name_span
                                        // falls within the RECORD_LITERAL node
                                        // that contains the cursor's open brace.
                                        // This disambiguates multiple ModelDef
                                        // literals in the same generator file.
                                        let rec_start_u: u32 = node.text_range().start().into();
                                        let rec_end_u: u32 = node.text_range().end().into();
                                        survivors
                                            .survivors
                                            .iter()
                                            .find(|em| {
                                                if em.generator_file != effective_path {
                                                    return false;
                                                }
                                                // name_span must be contained within
                                                // this record literal's range.
                                                let ns: u32 = em.name_span.start().into();
                                                let ne: u32 = em.name_span.end().into();
                                                ns >= rec_start_u && ne <= rec_end_u
                                            })
                                            .map(|em| {
                                                smelt_db::emitted_model_smelt_path(
                                                    &em.generator_file,
                                                    &project_root,
                                                    &scan_roots,
                                                    &em.name,
                                                )
                                            })
                                    });
                                    let value = hover_text_for_model_def_literal_open_brace(
                                        smelt_path.as_deref(),
                                    );
                                    return Ok(Some(Hover {
                                        contents: HoverContents::Markup(MarkupContent {
                                            kind: MarkupKind::Markdown,
                                            value,
                                        }),
                                        range: None,
                                    }));
                                }

                                // Walk field entries of the RecordLiteral.
                                for field in node.children() {
                                    if field.kind() != SyntaxKind::RECORD_FIELD {
                                        continue;
                                    }
                                    let field_start: usize = field.text_range().start().into();
                                    let field_end: usize = field.text_range().end().into();
                                    if !(cursor_offset >= field_start && cursor_offset <= field_end)
                                    {
                                        continue;
                                    }
                                    // Extract field key and value tokens.
                                    let mut tokens = field
                                        .children_with_tokens()
                                        .filter_map(|e| e.into_token())
                                        .filter(|t| !t.kind().is_trivia());
                                    let key_tok = tokens.next();
                                    let key_text =
                                        key_tok.as_ref().map(|t| t.text()).unwrap_or_default();
                                    // Skip the colon token.
                                    let _colon = tokens.next();
                                    let val_tok = tokens.next();

                                    if let Some(val) = val_tok {
                                        let vs: usize = val.text_range().start().into();
                                        let ve: usize = val.text_range().end().into();
                                        if cursor_offset >= vs && cursor_offset <= ve {
                                            // (c) cursor on `name:` value.
                                            if key_text == "name" {
                                                let raw_name = val.text();
                                                let model_name =
                                                    raw_name.trim_matches('\'').trim_matches('"');
                                                let ws = Workspace::try_get(&db);
                                                let project_root =
                                                    file_project_root(&db, &effective_path);
                                                let project = lookup_project(&db, &project_root);
                                                let scan_roots = project
                                                    .map(|p| {
                                                        smelt_db::project_paths(&db, p)
                                                            .as_ref()
                                                            .clone()
                                                    })
                                                    .unwrap_or_else(|| vec!["models".to_string()]);
                                                let smelt_path = ws.map(|_w| {
                                                    smelt_db::emitted_model_smelt_path(
                                                        &effective_path,
                                                        &project_root,
                                                        &scan_roots,
                                                        model_name,
                                                    )
                                                });
                                                let value = match &smelt_path {
                                                    Some(p) => {
                                                        hover_text_for_model_def_name_field_value(p)
                                                    }
                                                    None => {
                                                        format!("Emitted as `smelt.{model_name}`")
                                                    }
                                                };
                                                return Ok(Some(Hover {
                                                    contents: HoverContents::Markup(
                                                        MarkupContent {
                                                            kind: MarkupKind::Markdown,
                                                            value,
                                                        },
                                                    ),
                                                    range: None,
                                                }));
                                            }
                                            // (d) cursor on `body:` value.
                                            if key_text == "body" {
                                                let value =
                                                    hover_text_for_model_def_body_field_value(None);
                                                return Ok(Some(Hover {
                                                    contents: HoverContents::Markup(
                                                        MarkupContent {
                                                            kind: MarkupKind::Markdown,
                                                            value,
                                                        },
                                                    ),
                                                    range: None,
                                                }));
                                            }
                                            // (e) cursor on optional field value:
                                            // `materialization`, `tags`, or `description`.
                                            if let Some(value) =
                                                hover_text_for_model_def_optional_field_value(
                                                    key_text,
                                                )
                                            {
                                                return Ok(Some(Hover {
                                                    contents: HoverContents::Markup(
                                                        MarkupContent {
                                                            kind: MarkupKind::Markdown,
                                                            value,
                                                        },
                                                    ),
                                                    range: None,
                                                }));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path and adjust position
        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        let db = self.snapshot().await;

        // Get file content
        let text = file_text(&db, &effective_path);

        // Convert cursor position to offset
        let cursor_offset = {
            let mut offset = 0usize;
            let mut line = 0u32;
            let mut col = 0u32;

            for ch in text.chars() {
                if line == effective_position.line && col == effective_position.character {
                    break;
                }
                if ch == '\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
                offset += ch.len_utf8();
            }
            offset
        };

        // Phase E2: generator-file frontmatter completion — `generates: <cursor>`.
        // Detection: cursor is in the frontmatter of a Generator file, on a line
        // that starts with `generates:` (with optional partial value typed).
        {
            let raw = text.as_str();
            if let Ok(smelt_core::metadata::FileMetadata::Generator { body_offset, .. }) =
                smelt_core::metadata::extract_file_metadata(raw)
            {
                if cursor_offset < body_offset {
                    let line_start = raw[..cursor_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
                    let line_end = raw[cursor_offset..]
                        .find('\n')
                        .map(|p| cursor_offset + p)
                        .unwrap_or(raw.len());
                    let line_text = &raw[line_start..line_end];
                    if line_text.trim_start().starts_with("generates:") {
                        let items = completion_for_generates_value();
                        if !items.is_empty() {
                            return Ok(Some(CompletionResponse::Array(items)));
                        }
                    }
                }
            }
        }

        // Phase E2: ModelDef field-key completion — cursor inside a `ModelDef { <cursor> … }`
        // record literal in a generator file body.  Detection mirrors the hover dispatch.
        {
            let raw = text.as_str();
            if let Ok(smelt_core::metadata::FileMetadata::Generator { body_offset, .. }) =
                smelt_core::metadata::extract_file_metadata(raw)
            {
                if cursor_offset >= body_offset {
                    use smelt_parser::SyntaxKind;
                    let file_input = lookup_file(&db, &effective_path);
                    let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
                    if let Some(syntax) = parse.as_ref().map(|p| p.syntax()) {
                        if let Some(file_ast) = AstFile::cast(syntax) {
                            // Find the tightest ModelDef RECORD_LITERAL containing cursor.
                            let model_def_node = file_ast
                                .syntax()
                                .descendants()
                                .filter(|n| n.kind() == SyntaxKind::RECORD_LITERAL)
                                .filter(|n| {
                                    let s: usize = n.text_range().start().into();
                                    let e: usize = n.text_range().end().into();
                                    cursor_offset >= s && cursor_offset <= e
                                })
                                .filter(|n| {
                                    n.children_with_tokens()
                                        .filter_map(|e| e.into_token())
                                        .find(|t| !t.kind().is_trivia())
                                        .map(|t| t.text() == "ModelDef")
                                        .unwrap_or(false)
                                })
                                .min_by_key(|n| {
                                    let s: usize = n.text_range().start().into();
                                    let e: usize = n.text_range().end().into();
                                    e - s
                                });

                            if let Some(rec_node) = model_def_node {
                                // Collect already-filled field names.
                                let already_filled: Vec<String> = rec_node
                                    .children()
                                    .filter(|n| n.kind() == SyntaxKind::RECORD_FIELD)
                                    .filter_map(|field| {
                                        field
                                            .children_with_tokens()
                                            .filter_map(|e| e.into_token())
                                            .find(|t| !t.kind().is_trivia())
                                            .map(|t| t.text().to_string())
                                    })
                                    .collect();
                                let items = completion_for_model_def_field_key(&already_filled);
                                if !items.is_empty() {
                                    return Ok(Some(CompletionResponse::Array(items)));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase B: check for reduce second-arg position BEFORE the standard
        // context dispatch — this is a meta-language-specific completion that
        // should be offered regardless of the SQL-level context.
        {
            use smelt_parser::syntax_kind::SyntaxKind;
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            if let Some(syntax) = parse.as_ref().map(|p| p.syntax()) {
                if let Some(file) = AstFile::cast(syntax) {
                    // Find a `reduce(...)` call where the cursor is in the second-arg position.
                    let reduce_call = file
                        .syntax()
                        .descendants()
                        .filter_map(smelt_parser::ast::FunctionCall::cast)
                        .find(|c| {
                            c.name().as_deref() == Some("reduce") || {
                                // keyword-name fallback (same as infer_hof)
                                c.syntax()
                                    .children_with_tokens()
                                    .filter_map(|e| e.into_token())
                                    .find(|t| matches!(t.kind(), SyntaxKind::IDENT))
                                    .map(|t| t.text().to_lowercase() == "reduce")
                                    .unwrap_or(false)
                            }
                        });
                    if let Some(reduce) = reduce_call {
                        let args = reduce.arguments();
                        if !args.is_empty() {
                            // Check if cursor is after the first comma inside the call.
                            // We approximate: cursor > end of first argument.
                            let first_end: usize = args
                                .first()
                                .map(|a| a.text_range().end().into())
                                .unwrap_or(0);
                            let call_end: usize = reduce.syntax().text_range().end().into();
                            let call_start: usize = reduce.syntax().text_range().start().into();
                            if cursor_offset > first_end
                                && cursor_offset <= call_end
                                && cursor_offset >= call_start
                            {
                                // We're in the second-arg position. Infer first-arg list type.
                                let ctx = smelt_db::TypeContext::new();
                                use smelt_types::signatures::SmeltType;
                                let list_ty: Option<SmeltType> = args.first().and_then(|a| {
                                    if let Some(arr) = a.as_array_literal() {
                                        let elems = arr.elements();
                                        let r = smelt_db::type_inference::infer_list_literal(
                                            &elems, &ctx, None,
                                        );
                                        Some(r.inferred)
                                    } else {
                                        None
                                    }
                                });
                                let items = completion_items_for_reduce_second_arg_with_snippets(
                                    list_ty.as_ref(),
                                );
                                if !items.is_empty() {
                                    return Ok(Some(CompletionResponse::Array(items)));
                                }
                            }
                        }
                    }

                    // Phase D: smelt.models.<cursor> / smelt.sources.<cursor> accessor
                    // namespace completion — offer the closed accessor set {with_tag, all}.
                    //
                    // Detection: text before cursor ends with `smelt.models.` or
                    // `smelt.sources.` (possibly with a partial accessor name typed).
                    {
                        let before = &text[..cursor_offset.min(text.len())];
                        let is_models_ns = before.ends_with("smelt.models.")
                            || before
                                .rfind("smelt.models.")
                                .map(|p| {
                                    let after = &before[p + "smelt.models.".len()..];
                                    after.chars().all(|c| c.is_alphanumeric() || c == '_')
                                })
                                .unwrap_or(false);
                        let is_sources_ns = !is_models_ns
                            && (before.ends_with("smelt.sources.")
                                || before
                                    .rfind("smelt.sources.")
                                    .map(|p| {
                                        let after = &before[p + "smelt.sources.".len()..];
                                        after.chars().all(|c| c.is_alphanumeric() || c == '_')
                                    })
                                    .unwrap_or(false));
                        if is_models_ns || is_sources_ns {
                            let accessor_names = wide_reflection_accessor_completions();
                            let items: Vec<CompletionItem> = accessor_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FUNCTION),
                                    detail: Some(format!(
                                        "smelt.{}.{}",
                                        if is_models_ns { "models" } else { "sources" },
                                        name
                                    )),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase D: ModelRef field completion — at `m.<cursor>` inside a
                    // lambda body where `m` is a ModelRef-typed parameter,
                    // offer the closed field set {path, name, tags, columns}.
                    {
                        if is_model_ref_param_before_dot(&file, &text, cursor_offset).is_some() {
                            let field_names = model_ref_field_completions();
                            let items: Vec<CompletionItem> = field_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: hover_text_for_model_ref_field(&name),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase D: SourceRef field completion — at `s.<cursor>` inside a
                    // lambda body where `s` is a SourceRef-typed parameter,
                    // offer the closed field set {path, name, tags, columns}.
                    {
                        if is_source_ref_param_before_dot(&file, &text, cursor_offset).is_some() {
                            let field_names = source_ref_field_completions();
                            let items: Vec<CompletionItem> = field_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: hover_text_for_source_ref_field(&name),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase C: ColumnRef field completion — at `c.<cursor>` inside
                    // a lambda body where `c` is a ColumnRef-typed parameter,
                    // offer the closed field set.
                    //
                    // Detection: check if the text immediately before the cursor
                    // (within the lambda body) ends with `<ident>.` where `<ident>`
                    // is a ColumnRef-typed lambda parameter name.  We use
                    // `is_column_ref_param_before_dot` which checks that the
                    // receiver param is bound by a HOF whose first arg is
                    // `smelt.columns_of(...)`.  This prevents false-positive
                    // completions when an unrelated `smelt.columns_of` call
                    // appears elsewhere in the file.
                    {
                        if is_column_ref_param_before_dot(&file, &text, cursor_offset).is_some() {
                            let field_names = column_ref_field_completions();
                            let items: Vec<CompletionItem> = field_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: hover_text_for_column_ref_field(&name),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase C: smelt.columns_of(<cursor>) argument completion —
                    // offer in-scope TableExpr-valued names.
                    {
                        let before = &text[..cursor_offset.min(text.len())];
                        // Detect cursor inside `smelt.columns_of(` argument position.
                        // Simple heuristic: text before cursor contains `columns_of(`
                        // without a matching `)`.
                        if let Some(call_start) = before.rfind("columns_of(") {
                            let after_paren = &before[call_start + "columns_of(".len()..];
                            let paren_depth: i32 = after_paren.chars().fold(0i32, |d, c| match c {
                                '(' => d + 1,
                                ')' => d - 1,
                                _ => d,
                            });
                            // paren_depth >= 0 means we are inside the argument list
                            // (not yet closed by a matching `)`).
                            if paren_depth >= 0 {
                                let names = columns_of_arg_completions_for_sql(&text);
                                // Also add Salsa-backed model names from the workspace.
                                let ws = Workspace::try_get(&db);
                                let mut all_names = names;
                                if let Some(w) = ws {
                                    let models = smelt_db::all_models(&db, w);
                                    for model in models.values() {
                                        if !all_names.contains(&model.name) {
                                            all_names.push(model.name.clone());
                                        }
                                    }
                                }
                                if !all_names.is_empty() {
                                    let items: Vec<CompletionItem> = all_names
                                        .into_iter()
                                        .map(|name| CompletionItem {
                                            label: name.clone(),
                                            kind: Some(CompletionItemKind::MODULE),
                                            detail: Some(format!("model: {name}")),
                                            ..Default::default()
                                        })
                                        .collect();
                                    return Ok(Some(CompletionResponse::Array(items)));
                                }
                            }
                        }
                    }

                    // Phase B: check if cursor is inside a lambda body — prepend
                    // the bound lambda parameter to the completion list.
                    let lambda_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
                        .filter(|n| {
                            let s: usize = n.text_range().start().into();
                            let e: usize = n.text_range().end().into();
                            cursor_offset >= s && cursor_offset <= e
                        })
                        .min_by_key(|n| {
                            let s: usize = n.text_range().start().into();
                            let e: usize = n.text_range().end().into();
                            e - s
                        });
                    if let Some(ln) = lambda_node {
                        if let Some(lambda) = smelt_parser::ast::Lambda::cast(ln) {
                            // Only inject param completions when cursor is in the BODY,
                            // i.e. past the lambda arrow token.
                            let arrow_pos: Option<usize> =
                                lambda.syntax().children_with_tokens().find_map(|c| {
                                    c.as_token()
                                        .filter(|t| t.kind() == SyntaxKind::ARROW)
                                        .map(|t| t.text_range().end().into())
                                });
                            if arrow_pos.map(|p| cursor_offset >= p).unwrap_or(false) {
                                let params = lambda_params_for_completion(&lambda);
                                // Build param completions — they will be prepended
                                // to the standard column completions below.
                                // Phase F: also prepend the `if` snippet since a lambda
                                // body is a meta-expression context where ternary is valid.
                                let mut param_items: Vec<CompletionItem> =
                                    vec![completion_item_for_if_snippet()];
                                param_items.extend(params.iter().map(|p| CompletionItem {
                                    label: p.clone(),
                                    kind: Some(CompletionItemKind::VARIABLE),
                                    detail: Some("lambda parameter".to_string()),
                                    sort_text: Some(format!("0_{p}")), // sort first
                                    ..Default::default()
                                }));
                                if !param_items.is_empty() {
                                    // Return the param completions immediately so they
                                    // appear first in the list.
                                    return Ok(Some(CompletionResponse::Array(param_items)));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase F: `if` snippet fallback for generator-file body context.
        // If none of the earlier meta-language blocks claimed the cursor (reduce
        // second-arg, ModelDef field-key, lambda body, etc.), and the cursor is
        // in the body of a Generator file, offer `if … then … else …` as the
        // sole completion item.  Generator bodies are meta-expression contexts
        // where ternary is valid.
        {
            let raw = text.as_str();
            if let Ok(smelt_core::metadata::FileMetadata::Generator { body_offset, .. }) =
                smelt_core::metadata::extract_file_metadata(raw)
            {
                if cursor_offset >= body_offset {
                    return Ok(Some(CompletionResponse::Array(vec![
                        completion_item_for_if_snippet(),
                    ])));
                }
            }
        }

        // Determine completion context
        let context = determine_completion_context(&text, cursor_offset);

        let items = match context {
            CompletionContext::InsideRef => {
                // Complete model names
                let ws = Workspace::try_get(&db);
                let models = ws.map(|w| smelt_db::all_models(&db, w)).unwrap_or_default();
                models
                    .values()
                    .map(|model| CompletionItem {
                        label: model.name.clone(),
                        kind: Some(CompletionItemKind::MODULE),
                        detail: Some(format!("Model: {}", model.name)),
                        ..Default::default()
                    })
                    .collect()
            }
            CompletionContext::InsideSource => {
                // Complete source.table names
                let project_root = file_project_root(&db, &effective_path);
                let project = lookup_project(&db, &project_root);
                let config = project
                    .map(|p| smelt_db::sources_config(&db, p))
                    .unwrap_or_default();
                let mut items = Vec::new();

                for source in &config.sources {
                    for table in &source.tables {
                        let qualified_name = format!("{}.{}", source.name, table.name);
                        let detail = table
                            .description
                            .clone()
                            .unwrap_or_else(|| format!("Source table: {}", qualified_name));
                        items.push(CompletionItem {
                            label: qualified_name.clone(),
                            kind: Some(CompletionItemKind::FILE),
                            detail: Some(detail),
                            documentation: if !table.columns.is_empty() {
                                let cols: Vec<_> =
                                    table.columns.iter().map(|c| c.name.as_str()).collect();
                                Some(Documentation::String(format!(
                                    "Columns: {}",
                                    cols.join(", ")
                                )))
                            } else {
                                None
                            },
                            ..Default::default()
                        });
                    }
                }

                items
            }
            CompletionContext::ColumnName => {
                // Complete column names from available columns
                // Use typed schema for type information
                let ws = Workspace::try_get(&db);
                let fi = lookup_file(&db, &effective_path);
                let typed_schema = match (ws, fi) {
                    (Some(w), Some(f)) => smelt_db::typed_model_schema(&db, w, f),
                    _ => Arc::new(smelt_db::ModelSchema::empty()),
                };
                let available = match (ws, fi) {
                    (Some(w), Some(f)) => smelt_db::available_columns(&db, w, f),
                    _ => Arc::new(Vec::new()),
                };

                // Build a map of column names to types from the typed schema
                let type_map: std::collections::HashMap<&str, &TypedColumn> = typed_schema
                    .columns
                    .iter()
                    .filter_map(|col| col.data_type.as_ref().map(|t| (col.name.as_str(), t)))
                    .collect();

                available
                    .iter()
                    .filter(|col| col.name != "*")
                    .map(|col| {
                        // Build detail with type info
                        let type_str = col
                            .data_type
                            .as_ref()
                            .or_else(|| type_map.get(col.name.as_str()).copied())
                            .map(format_type)
                            .unwrap_or_else(|| "unknown".to_string());

                        let detail = format!("{}: {}", col.name, type_str);

                        // Build documentation with expression and source info
                        let mut doc_parts = Vec::new();
                        if !col.expression.is_empty() && col.expression != col.name {
                            doc_parts.push(format!("Expression: `{}`", col.expression));
                        }
                        match &col.source {
                            smelt_db::ColumnSource::FromModel {
                                model_name,
                                column_name,
                            } => {
                                doc_parts.push(format!(
                                    "From model '{}', column '{}'",
                                    model_name, column_name
                                ));
                            }
                            smelt_db::ColumnSource::Computed => {
                                doc_parts.push("Computed column".to_string());
                            }
                            _ => {}
                        }

                        CompletionItem {
                            label: col.name.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(detail),
                            documentation: if doc_parts.is_empty() {
                                None
                            } else {
                                Some(Documentation::String(doc_parts.join("\n")))
                            },
                            ..Default::default()
                        }
                    })
                    .collect()
            }
            CompletionContext::QualifiedColumn(alias) => {
                // Complete columns for the specified table alias
                // Parse the file to find what the alias refers to
                let fi = lookup_file(&db, &effective_path);
                let parse = fi.map(|f| smelt_db::parse_file(&db, f));
                let syntax = parse.as_ref().map(|p| p.syntax());

                if let Some(syntax) = syntax {
                    if let Some(file) = smelt_parser::ast::File::cast(syntax) {
                        if let Some(select_stmt) = file.select_stmt() {
                            // Extract alias mappings from FROM clause
                            let alias_map = extract_from_aliases(&select_stmt, &db);

                            // Look up what this alias refers to
                            if let Some(target) = alias_map.get(&alias) {
                                match target {
                                    AliasTarget::Source {
                                        source_name,
                                        table_name,
                                    } => {
                                        // Get columns from sources.yml
                                        let project_root = file_project_root(&db, &effective_path);
                                        let project = lookup_project(&db, &project_root);
                                        let config = project
                                            .map(|p| smelt_db::sources_config(&db, p))
                                            .unwrap_or_default();
                                        for source in &config.sources {
                                            if source.name == *source_name {
                                                for table in &source.tables {
                                                    if table.name == *table_name {
                                                        return Ok(Some(
                                                            CompletionResponse::Array(
                                                                table
                                                                    .columns
                                                                    .iter()
                                                                    .map(|col| {
                                                                        let type_str = col
                                                                            .data_type
                                                                            .as_ref()
                                                                            .map(|t| t.to_string())
                                                                            .unwrap_or_else(|| {
                                                                                "unknown"
                                                                                    .to_string()
                                                                            });
                                                                        CompletionItem {
                                                                    label: col.name.clone(),
                                                                    kind: Some(
                                                                        CompletionItemKind::FIELD,
                                                                    ),
                                                                    detail: Some(format!(
                                                                        "{}: {}",
                                                                        col.name, type_str
                                                                    )),
                                                                    documentation: col
                                                                        .description
                                                                        .as_ref()
                                                                        .map(|d| {
                                                                            Documentation::String(
                                                                                d.clone(),
                                                                            )
                                                                        }),
                                                                    ..Default::default()
                                                                }
                                                                    })
                                                                    .collect(),
                                                            ),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    AliasTarget::Model { model_name } => {
                                        // Get columns from the model schema
                                        let ws = Workspace::try_get(&db);
                                        let models = ws
                                            .map(|w| smelt_db::all_models(&db, w))
                                            .unwrap_or_default();
                                        if let Some(model) =
                                            models.values().find(|m| m.name == *model_name)
                                        {
                                            let model_file = lookup_file(&db, &model.path);
                                            let schema = model_file
                                                .map(|f| smelt_db::model_schema(&db, f))
                                                .unwrap_or_else(|| {
                                                    Arc::new(smelt_db::ModelSchema::empty())
                                                });
                                            return Ok(Some(CompletionResponse::Array(
                                                schema
                                                    .columns
                                                    .iter()
                                                    .filter(|col| col.name != "*")
                                                    .map(|col| CompletionItem {
                                                        label: col.name.clone(),
                                                        kind: Some(CompletionItemKind::FIELD),
                                                        detail: Some(format!(
                                                            "Column from {}",
                                                            model_name
                                                        )),
                                                        ..Default::default()
                                                    })
                                                    .collect(),
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Vec::new()
            }
            CompletionContext::FromClause => {
                // Offer CTE names defined in the current query's WITH clause
                let fi = lookup_file(&db, &effective_path);
                let parse = fi.map(|f| smelt_db::parse_file(&db, f));
                let syntax = parse.as_ref().map(|p| p.syntax());

                let mut items = Vec::new();

                if let Some(syntax) = syntax {
                    if let Some(file) = smelt_parser::ast::File::cast(syntax) {
                        if let Some(select_stmt) = file.select_stmt() {
                            if let Some(with_clause) = select_stmt.with_clause() {
                                let ws = Workspace::try_get(&db);
                                let type_ctx = match (ws, fi) {
                                    (Some(w), Some(f)) => smelt_db::type_context(&db, w, f),
                                    _ => Arc::new(smelt_db::TypeContext::new()),
                                };

                                for cte in with_clause.ctes() {
                                    if let Some(cte_name) = cte.name() {
                                        // Get column info for documentation
                                        let columns = type_ctx.cte_columns(&cte_name);
                                        let doc = if columns.is_empty() {
                                            None
                                        } else {
                                            let col_strs: Vec<String> = columns
                                                .iter()
                                                .map(|(name, typed_col)| {
                                                    format!("{}: {}", name, format_type(typed_col))
                                                })
                                                .collect();
                                            Some(Documentation::String(col_strs.join("\n")))
                                        };

                                        items.push(CompletionItem {
                                            label: cte_name.clone(),
                                            kind: Some(CompletionItemKind::STRUCT),
                                            detail: Some("CTE".to_string()),
                                            documentation: doc,
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                items
            }
            // Phase 48: PASSING-body completions — offer aggregate functions
            // and any columns from the parameter's declared context schema.
            CompletionContext::InPassingBody {
                callee,
                passing_name,
            } => {
                let ws = Workspace::try_get(&db);
                let mut items: Vec<CompletionItem> = Vec::new();

                // Resolve the callee's signature to find the parameter's
                // declared context (e.g. `SelectItems<Agg, sessionized>`).
                // Project isolation: resolve in the cursor file's project.
                let project_root = file_project_root(&db, &effective_path);
                let project = lookup_project(&db, &project_root);
                if let (Some(w), Some(p)) = (ws, project) {
                    if let Some(sig) = smelt_db::resolve_function(&db, w, p, callee.clone())
                        .map(|arc| (*arc).clone())
                    {
                        // Look up the parameter by name.
                        if let Some(param) = sig.params.iter().find(|p| p.name == passing_name) {
                            use smelt_types::signatures::SmeltType;
                            if let Some(Ok(SmeltType::SelectItems {
                                context: Some(smelt_types::signatures::ContextRef(ctx_name)),
                                ..
                            })) = &param.type_ref
                            {
                                // Surface columns from the context schema (e.g. the
                                // `sessionized` CTE) so the user can pick column refs.
                                let cols = passing_body_completion_columns(&db, w, &sig, ctx_name);
                                for (col_name, typed_col) in &cols {
                                    items.push(CompletionItem {
                                        label: col_name.clone(),
                                        kind: Some(CompletionItemKind::FIELD),
                                        detail: Some(format_type(typed_col)),
                                        ..Default::default()
                                    });
                                }
                            }

                            // Always offer aggregate function keywords for
                            // `SelectItems<Agg>`-kinded parameters.
                            use smelt_types::signatures::ExprKind;
                            let needs_agg = matches!(
                                &param.type_ref,
                                Some(Ok(SmeltType::SelectItems {
                                    kind: ExprKind::Agg | ExprKind::Window,
                                    ..
                                }))
                            );
                            if needs_agg {
                                for label in passing_body_aggregate_labels() {
                                    items.push(CompletionItem {
                                        label: label.to_string(),
                                        kind: Some(CompletionItemKind::FUNCTION),
                                        detail: Some("aggregate function".to_string()),
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                }

                items
            }
            CompletionContext::SmeltPath => {
                // Phase 2c: return all workspace entities as `smelt.<segments>` labels.
                let ws = Workspace::try_get(&db);
                let Some(w) = ws else { return Ok(None) };
                let all_files = w.files(&db).clone();
                // Determine the project root from the current file.
                let project_root = file_project_root(&db, &effective_path);
                let mut items: Vec<CompletionItem> = all_files
                    .iter()
                    .filter_map(|f| {
                        let file_path = f.path(&db);
                        // Only SQL files.
                        if file_path.extension().and_then(|e| e.to_str()) != Some("sql") {
                            return None;
                        }
                        let rel = file_path.strip_prefix(&project_root).ok()?;
                        let parent = rel.parent()?;
                        let mut segments: Vec<String> = parent
                            .components()
                            .filter_map(|c| match c {
                                std::path::Component::Normal(s) => {
                                    Some(s.to_string_lossy().into_owned())
                                }
                                _ => None,
                            })
                            .collect();
                        let stem = file_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())?;
                        segments.push(stem.clone());
                        let label = format!("smelt.{}", segments.join("."));
                        let insert = segments.join(".");
                        Some(CompletionItem {
                            label,
                            insert_text: Some(insert),
                            kind: Some(CompletionItemKind::MODULE),
                            ..Default::default()
                        })
                    })
                    .collect();
                items.sort_by(|a, b| a.label.cmp(&b.label));
                items
            }
            CompletionContext::None => Vec::new(),
        };

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }
}
