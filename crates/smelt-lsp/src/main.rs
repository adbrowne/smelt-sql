use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use smelt_core::{
    find_config_file, find_project_root_by_walking_up, find_project_root_for_file,
    find_smelt_projects, is_sources_file,
    metadata::{extract_file_metadata, FileMetadata},
};
use smelt_db::{
    Database, Diagnostic as DbDiagnostic, DiagnosticCode as DbCode, DiagnosticData as DbData,
    DiagnosticSeverity as DbSeverity, Inputs, Schema, Semantic, Syntax, TypeChecking,
};

mod python_scan;
use python_scan::PythonModelCache;
use smelt_parser::ast::File as AstFile;
use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};
use smelt_types::TypedColumn;

/// Tracks errors that occurred during workspace initialization
#[derive(Default)]
struct InitErrors {
    workspace_errors: Vec<String>,
    source_errors: Vec<String>,
    model_errors: Vec<String>,
}

impl InitErrors {
    fn has_errors(&self) -> bool {
        !self.workspace_errors.is_empty()
            || !self.source_errors.is_empty()
            || !self.model_errors.is_empty()
    }

    fn total_count(&self) -> usize {
        self.workspace_errors.len() + self.source_errors.len() + self.model_errors.len()
    }
}

/// Validate that a string is a valid SQL identifier.
/// Must be non-empty, start with a letter or underscore, and contain only
/// alphanumeric characters and underscores.
fn is_valid_sql_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Format a TypedColumn for display in hover/completion
fn format_type(typed_col: &TypedColumn) -> String {
    let nullable_suffix = if typed_col.nullable { "?" } else { "" };
    format!("{}{}", typed_col.data_type, nullable_suffix)
}

/// Find the 0-indexed line number of a table definition inside a sources.yml file.
/// Searches for the table name under the correct source name section.
fn find_source_table_line(
    sources_path: &std::path::Path,
    source_name: &str,
    table_name: &str,
) -> u32 {
    let content = match std::fs::read_to_string(sources_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut in_source = false;
    let mut in_tables = false;
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Detect source name (e.g., "  raw:" at source level)
        if !trimmed.starts_with('-') && trimmed.starts_with(&format!("{}:", source_name)) {
            in_source = true;
            in_tables = false;
            continue;
        }

        if in_source {
            // Detect tables section
            if trimmed == "tables:" {
                in_tables = true;
                continue;
            }

            // A new top-level key resets context (left-aligned or less indented non-tables key)
            if !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.starts_with('#')
                && !line.starts_with(' ')
            {
                in_source = false;
                in_tables = false;
                continue;
            }
        }

        if in_source && in_tables {
            // Look for the table name as a YAML key (e.g., "      users:" or "      - name: users")
            if trimmed.starts_with(&format!("{}:", table_name)) {
                return i as u32;
            }
        }
    }

    0
}

/// Find the 0-indexed line number of a column definition inside a sources.yml file.
fn find_source_column_line(
    sources_path: &std::path::Path,
    source_name: &str,
    table_name: &str,
    column_name: &str,
) -> u32 {
    let content = match std::fs::read_to_string(sources_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut in_source = false;
    let mut in_table = false;
    let mut in_columns = false;
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        if !trimmed.starts_with('-') && trimmed.starts_with(&format!("{}:", source_name)) {
            in_source = true;
            in_table = false;
            in_columns = false;
            continue;
        }
        if in_source && trimmed.starts_with(&format!("{}:", table_name)) {
            in_table = true;
            in_columns = false;
            continue;
        }
        if in_source && in_table && trimmed == "columns:" {
            in_columns = true;
            continue;
        }
        // Reset on new table or source
        if in_source
            && in_table
            && in_columns
            && !trimmed.is_empty()
            && !trimmed.starts_with('-')
            && !trimmed.starts_with('#')
            && !trimmed.contains("name:")
            && !trimmed.contains("type:")
            && !trimmed.contains("description:")
            && !trimmed.contains("data_latency:")
        {
            // Likely a new section key; stop looking
            break;
        }

        if in_source && in_table && in_columns {
            // Match "- name: column_name"
            if trimmed.starts_with("- name:") {
                let name = trimmed.trim_start_matches("- name:").trim();
                if name == column_name {
                    return i as u32;
                }
            }
        }
    }

    // Fall back to table line
    find_source_table_line(sources_path, source_name, table_name)
}

/// A resolved column definition location for goto-definition
#[derive(Debug, Clone)]
struct ColumnDefLocation {
    path: PathBuf,
    line: u32,
    col: u32,
    end_line: u32,
    end_col: u32,
}

/// Resolve a column reference to its definition location(s).
///
/// Traces through wildcard (`SELECT *`) chains until finding an explicit column definition.
/// Returns multiple locations for ambiguous (unqualified) columns.
fn resolve_column_definitions(
    db: &Database,
    current_path: &std::path::Path,
    qualifier: Option<&str>,
    column_name: &str,
) -> Vec<ColumnDefLocation> {
    let ctx = db.type_context(current_path.to_path_buf());

    // Determine which sources this column could come from
    let resolved_qualifier =
        qualifier.and_then(|q| ctx.resolve_alias(q).or_else(|| Some(q.to_string())));
    let effective_qualifier = resolved_qualifier.as_deref().or(qualifier);

    // Try to find the column in each source type
    let mut locations = Vec::new();

    // Check CTE columns
    find_column_in_ctes(
        db,
        current_path,
        effective_qualifier,
        column_name,
        &ctx,
        &mut locations,
    );

    // Check model columns (from smelt.ref() sources)
    find_column_in_models(
        db,
        current_path,
        effective_qualifier,
        column_name,
        &ctx,
        &mut locations,
    );

    // Check source columns (from smelt.source())
    find_column_in_sources(
        db,
        current_path,
        effective_qualifier,
        column_name,
        &ctx,
        &mut locations,
    );

    locations
}

/// Find column definitions in CTEs
fn find_column_in_ctes(
    db: &Database,
    current_path: &std::path::Path,
    qualifier: Option<&str>,
    column_name: &str,
    ctx: &smelt_db::TypeContext,
    locations: &mut Vec<ColumnDefLocation>,
) {
    // If qualifier is specified, only check that CTE
    let cte_names: Vec<String> = if let Some(q) = qualifier {
        if ctx.is_cte(q) {
            vec![q.to_string()]
        } else {
            vec![]
        }
    } else {
        ctx.cte_names().map(|s| s.to_string()).collect()
    };

    if cte_names.is_empty() {
        return;
    }

    // Parse the current file to find CTE definitions
    let parse = db.parse_file(current_path.to_path_buf());
    let text = db.file_text(current_path.to_path_buf());
    let syntax = parse.syntax();
    let file = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return,
    };

    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return,
    };
    let with_clause = match select_stmt.with_clause() {
        Some(w) => w,
        None => return,
    };

    for cte in with_clause.ctes() {
        let cte_name = match cte.name() {
            Some(n) => n,
            None => continue,
        };
        if !cte_names.contains(&cte_name) {
            continue;
        }

        // Check if this CTE has the column — look at its SELECT list
        let cte_select = match cte.query().and_then(|q| q.select_stmt()) {
            Some(s) => s,
            None => continue,
        };
        let cte_select_list = match cte_select.select_list() {
            Some(l) => l,
            None => continue,
        };

        // Check explicit column list first
        let explicit_names = cte.column_names();
        if !explicit_names.is_empty() {
            // CTE has explicit column names — match by position
            for (i, explicit_name) in explicit_names.iter().enumerate() {
                if explicit_name == column_name {
                    // Point to the i-th select item
                    if let Some(item) = cte_select_list.items().nth(i) {
                        let pr = smelt_parser::ast::text_range_to_range(&text, item.range());
                        locations.push(ColumnDefLocation {
                            path: current_path.to_path_buf(),
                            line: pr.start.line,
                            col: pr.start.column,
                            end_line: pr.end.line,
                            end_col: pr.end.column,
                        });
                    }
                    break;
                }
            }
        } else {
            // No explicit column names — first check named columns, then wildcards
            let mut found_explicit = false;
            let mut has_wildcard = false;
            for item in cte_select_list.items() {
                if item.is_wildcard() {
                    has_wildcard = true;
                    continue;
                }
                if let Some(name) = item.column_name() {
                    if name == column_name {
                        let pr = smelt_parser::ast::text_range_to_range(&text, item.range());
                        locations.push(ColumnDefLocation {
                            path: current_path.to_path_buf(),
                            line: pr.start.line,
                            col: pr.start.column,
                            end_line: pr.end.line,
                            end_col: pr.end.column,
                        });
                        found_explicit = true;
                        break;
                    }
                }
            }
            // If no explicit match, trace through wildcards
            if !found_explicit && has_wildcard {
                if let Some(from_clause) = cte_select.from_clause() {
                    let all_refs = from_clause
                        .table_refs()
                        .chain(from_clause.joins().filter_map(|j| j.table_ref()));
                    for table_ref in all_refs {
                        if let Some(func) = table_ref.function_call() {
                            if let Some(ref_call) =
                                smelt_parser::ast::RefCall::from_function_call(func)
                            {
                                if let Some(model_name) = ref_call.model_name() {
                                    if let Some(upstream_path) = db.resolve_ref(model_name) {
                                        find_column_in_model_chain(
                                            db,
                                            &upstream_path,
                                            column_name,
                                            10,
                                            locations,
                                        );
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

/// Find column definitions in upstream models (through smelt.ref() calls).
/// Traces through wildcard chains until finding an explicit definition.
fn find_column_in_models(
    db: &Database,
    current_path: &std::path::Path,
    qualifier: Option<&str>,
    column_name: &str,
    ctx: &smelt_db::TypeContext,
    locations: &mut Vec<ColumnDefLocation>,
) {
    // Determine which models to check
    let model_names: Vec<String> = if let Some(q) = qualifier {
        // Qualified: check only this model
        if ctx.is_cte(q) {
            return; // Already handled by CTE check
        }
        vec![q.to_string()]
    } else {
        // Unqualified: check all models in FROM clause
        collect_from_model_names(db, current_path)
    };

    for model_name in &model_names {
        if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
            if find_column_in_model_chain(db, &upstream_path, column_name, 10, locations)
                && qualifier.is_some()
            {
                return; // Qualified lookup: stop after first match
            }
        }
    }
}

/// Recursively find a column definition through wildcard chains.
/// Returns true if the column was found.
fn find_column_in_model_chain(
    db: &Database,
    model_path: &std::path::Path,
    column_name: &str,
    depth_limit: usize,
    locations: &mut Vec<ColumnDefLocation>,
) -> bool {
    if depth_limit == 0 {
        return false;
    }

    let schema = db.model_schema(model_path.to_path_buf());
    let text = db.file_text(model_path.to_path_buf());

    // First check explicit columns
    for col in &schema.columns {
        if col.name == column_name {
            let pr = smelt_parser::ast::text_range_to_range(&text, col.range);
            locations.push(ColumnDefLocation {
                path: model_path.to_path_buf(),
                line: pr.start.line,
                col: pr.start.column,
                end_line: pr.end.line,
                end_col: pr.end.column,
            });
            return true;
        }
    }

    // If not found in explicit columns, check wildcard extensions
    for ext in &schema.row_extensions {
        if let Some(upstream_path) = db.resolve_ref(ext.ref_name.clone()) {
            if find_column_in_model_chain(
                db,
                &upstream_path,
                column_name,
                depth_limit - 1,
                locations,
            ) {
                return true;
            }
        }
    }

    false
}

/// Find column definitions in source tables (from smelt.source() calls)
fn find_column_in_sources(
    db: &Database,
    current_path: &std::path::Path,
    qualifier: Option<&str>,
    column_name: &str,
    ctx: &smelt_db::TypeContext,
    locations: &mut Vec<ColumnDefLocation>,
) {
    let project_root = db.file_project_root(current_path.to_path_buf());
    let sources_path = project_root.join("sources.yml");
    if !sources_path.exists() {
        return;
    }

    // Get source references from FROM clause
    let parse = db.parse_file(current_path.to_path_buf());
    let syntax = parse.syntax();
    let file = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return,
    };

    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return,
    };
    let from_clause = match select_stmt.from_clause() {
        Some(f) => f,
        None => return,
    };

    // Collect source references with their aliases
    let mut source_refs: Vec<(String, String, Option<String>)> = Vec::new(); // (source_name, table_name, alias)
    let all_table_refs = from_clause
        .table_refs()
        .chain(from_clause.joins().filter_map(|j| j.table_ref()));

    for table_ref in all_table_refs {
        if let Some(func) = table_ref.function_call() {
            if let Some(source_call) = smelt_parser::ast::SourceCall::from_function_call(func) {
                if let (Some(sn), Some(tn)) = (source_call.source_name(), source_call.table_name())
                {
                    let alias = table_ref.alias();
                    source_refs.push((sn, tn, alias));
                }
            }
        }
    }

    for (source_name, table_name, alias) in &source_refs {
        // Check if this source matches the qualifier
        if let Some(q) = qualifier {
            let resolved_q = ctx.resolve_alias(q).unwrap_or_else(|| q.to_string());
            let qualified_name = format!("{}.{}", source_name, table_name);
            if resolved_q != qualified_name && q != table_name && Some(q.to_string()) != *alias {
                continue;
            }
        }

        // Check if the source has this column
        if let Some(table_def) = db.resolve_source(
            project_root.clone(),
            source_name.clone(),
            table_name.clone(),
        ) {
            if table_def.columns.iter().any(|c| c.name == column_name) {
                let line =
                    find_source_column_line(&sources_path, source_name, table_name, column_name);
                locations.push(ColumnDefLocation {
                    path: sources_path.clone(),
                    line,
                    col: 0,
                    end_line: line,
                    end_col: 0,
                });
                if qualifier.is_some() {
                    return;
                }
            }
        }
    }
}

/// Collect model names from FROM clause ref() calls
fn collect_from_model_names(db: &Database, path: &std::path::Path) -> Vec<String> {
    let parse = db.parse_file(path.to_path_buf());
    let syntax = parse.syntax();
    let file = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return vec![],
    };
    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return vec![],
    };
    let from_clause = match select_stmt.from_clause() {
        Some(f) => f,
        None => return vec![],
    };

    let mut names = Vec::new();
    let all_refs = from_clause
        .table_refs()
        .chain(from_clause.joins().filter_map(|j| j.table_ref()));

    for table_ref in all_refs {
        if let Some(func) = table_ref.function_call() {
            if let Some(ref_call) = smelt_parser::ast::RefCall::from_function_call(func) {
                if let Some(model_name) = ref_call.model_name() {
                    names.push(model_name);
                }
            }
        }
    }
    names
}

/// Build project context JSON from discovered files for Python model execution.
/// Extracts model names, tags, and directories from the file paths registered in Salsa.
fn build_python_context(all_files: &[PathBuf], config: &smelt_core::Config) -> String {
    use smelt_core::python_utils::{ProjectContextData, ProjectModelInfo};

    let mut models = Vec::new();
    for path in all_files {
        let path_str = path.to_string_lossy();

        // Extract model name: for virtual paths like "file.sql::model_name", use the segment
        // after "::". For regular paths, use the file stem.
        let name = if let Some(pos) = path_str.find("::") {
            path_str[pos + 2..].to_string()
        } else {
            match path.file_stem().and_then(|s| s.to_str()) {
                Some(stem) => stem.to_string(),
                None => continue,
            }
        };

        // Extract directory from parent path's file name
        let directory = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let tags = config.get_tags(&name, None);

        models.push(ProjectModelInfo {
            name,
            tags,
            directory,
        });
    }

    let context = ProjectContextData { models };
    serde_json::to_string(&context).expect("Failed to serialize project context")
}

/// (virtual_path, start_line_offset) for each section in a multi-model file.
type MultiModelEntry = Vec<(PathBuf, u32)>;

struct Backend {
    client: Client,
    db: Arc<Mutex<Database>>,
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
    fn new(client: Client) -> Self {
        Self {
            client,
            db: Arc::new(Mutex::new(Database::default())),
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
        });

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
            }),
            message: diag.message.clone(),
            source: Some("smelt".to_string()),
            code,
            data,
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
        let known_files = db.all_files();

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

                // Only mutate Salsa inputs when values actually changed.
                // Unnecessary mutations increment Salsa's global revision, which
                // triggers memo validation across ALL queries. During validation
                // of circular model dependencies, Salsa 0.16.1 panics because its
                // cycle detection expects queries in the stack but the validation
                // path (maybe_changed_since -> read_upgrade) sets InProgress
                // without pushing to the stack.
                let is_known = known_files.contains(&virtual_path);
                if !is_known || *db.file_text(virtual_path.clone()) != sql_content {
                    db.set_file_text(virtual_path.clone(), Arc::new(sql_content.to_string()));
                }
                if !is_known {
                    db.set_file_project_root(virtual_path.clone(), project_root.to_path_buf());
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
            let is_known = known_files.contains(&path_buf);
            if !is_known || *db.file_text(path_buf.clone()) != content {
                db.set_file_text(path_buf.clone(), Arc::new(content.to_string()));
            }
            if !is_known {
                db.set_file_project_root(path_buf.clone(), project_root.to_path_buf());
            }
            registered.push(path_buf);

            // Clean up any old multi-model mapping
            let mut mm = self.multi_model_files.lock().await;
            mm.remove(real_path);
        }

        registered
    }

    /// Query Salsa diagnostics for a file, catching any panics from Salsa's
    /// cycle detection bug (salsa 0.16.1 panics during memo validation when
    /// circular model dependencies exist).
    fn query_diagnostics(db: &Database, path: PathBuf) -> Vec<DbDiagnostic> {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let db = AssertUnwindSafe(db);
        let path_for_log = path.clone();
        let path2 = path.clone();
        match catch_unwind(move || {
            let file_diags = db.file_diagnostics(path);
            let type_diags = db.type_diagnostics(path2);
            file_diags
                .iter()
                .chain(type_diags.iter())
                .cloned()
                .collect::<Vec<_>>()
        }) {
            Ok(diags) => diags,
            Err(_) => {
                // Salsa panicked (likely cycle detection during memo validation
                // with circular model dependencies). The PanicGuard cleanup resets
                // InProgress states to NotComputed, so the database is still usable.
                eprintln!(
                    "[WARN] Diagnostics unavailable for {} (Salsa cycle detection panic caught — likely circular model dependency)",
                    path_for_log.display()
                );
                Vec::new()
            }
        }
    }

    /// Publish diagnostics for a file
    async fn publish_diagnostics(&self, uri: Url) {
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Check if this is a multi-model file
        let mm = self.multi_model_files.lock().await;
        if let Some(virtual_entries) = mm.get(&path) {
            // Aggregate diagnostics from all virtual paths with line offset adjustment
            let db = self.db.lock().await;
            let mut lsp_diagnostics = Vec::new();

            for (virtual_path, start_line) in virtual_entries {
                let diagnostics = Self::query_diagnostics(&db, virtual_path.clone());

                for d in &diagnostics {
                    let mut lsp_diag = self.to_lsp_diagnostic(d);
                    // Adjust line numbers to be relative to the original file
                    lsp_diag.range.start.line += start_line;
                    lsp_diag.range.end.line += start_line;
                    lsp_diagnostics.push(lsp_diag);
                }
            }

            drop(db);
            drop(mm);
            self.client
                .publish_diagnostics(uri, lsp_diagnostics, None)
                .await;
        } else {
            drop(mm);
            let db = self.db.lock().await;
            let diagnostics = Self::query_diagnostics(&db, path);

            let lsp_diagnostics: Vec<lsp_types::Diagnostic> = diagnostics
                .iter()
                .map(|d| self.to_lsp_diagnostic(d))
                .collect();

            drop(db);
            self.client
                .publish_diagnostics(uri, lsp_diagnostics, None)
                .await;
        }
    }

    /// Publish diagnostics for all known model files
    async fn publish_all_diagnostics(&self) {
        let db = self.db.lock().await;
        let files = db.all_files();
        let files = files.clone();
        drop(db);

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
        let py_sources = self.python_model_sources.clone();
        let py_diags = self.python_diagnostics.clone();
        let cache = self.python_cache.clone();
        let client = self.client.clone();

        // Build context from current model list
        let context_json = {
            let db_guard = db.lock().await;
            let all_files = db_guard.all_files();
            let config =
                smelt_core::Config::load(&project_root).unwrap_or_else(|_| smelt_core::Config {
                    name: String::new(),
                    version: 1,
                    model_paths: vec!["models".to_string()],
                    seed_paths: vec!["seeds".to_string()],
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
                python_scan::execute_single_python_file(
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

                // Remove old virtual paths from this .py file
                let old_virtual_paths: Vec<PathBuf> = sources
                    .iter()
                    .filter(|(_, (src, _))| *src == py_path)
                    .map(|(vp, _)| vp.clone())
                    .collect();

                let mut files = (*db_guard.all_files()).clone();
                for old_vp in &old_virtual_paths {
                    sources.remove(old_vp);
                    files.retain(|f| f != old_vp);
                }

                // Register new models (skip mutations when values unchanged)
                let mut files_changed = false;
                for py_model in &scan_result.models {
                    let virtual_sql_path = py_model
                        .source_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(format!("{}.sql", py_model.name));

                    let is_known = files.contains(&virtual_sql_path);
                    if !is_known || *db_guard.file_text(virtual_sql_path.clone()) != py_model.sql {
                        db_guard.set_file_text(
                            virtual_sql_path.clone(),
                            Arc::new(py_model.sql.clone()),
                        );
                    }
                    if !is_known {
                        db_guard
                            .set_file_project_root(virtual_sql_path.clone(), project_root.clone());
                    }
                    sources.insert(
                        virtual_sql_path.clone(),
                        (py_model.source_path.clone(), py_model.decorator_line),
                    );
                    if !is_known {
                        files.push(virtual_sql_path);
                        files_changed = true;
                    }
                }

                if files_changed {
                    db_guard.set_all_files(Arc::new(files));
                }
            }

            // Republish all diagnostics since ref resolution may have changed
            let db_guard = db.lock().await;
            let files = db_guard.all_files().clone();
            drop(db_guard);

            for path in files.iter() {
                if let Ok(uri) = Url::from_file_path(path) {
                    let db_guard = db.lock().await;
                    let diagnostics = Backend::query_diagnostics(&db_guard, path.clone());
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
                            }),
                            message: d.message.clone(),
                            source: Some("smelt".to_string()),
                            ..Default::default()
                        })
                        .collect();
                    drop(db_guard);
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
            db.set_all_files(Arc::new(Vec::new()));
            db.set_all_project_roots(Arc::new(Vec::new()));
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

                    // Load sources config for this project
                    match find_config_file(&project_root, "sources") {
                        Ok(Some(sources_path)) => match std::fs::read_to_string(&sources_path) {
                            Ok(content) => {
                                db.set_project_sources_yaml(
                                    project_root.clone(),
                                    Arc::new(content),
                                );
                            }
                            Err(e) => {
                                init_errors.source_errors.push(format!(
                                    "Failed to read {}: {}",
                                    sources_path.display(),
                                    e
                                ));
                                db.set_project_sources_yaml(
                                    project_root.clone(),
                                    Arc::new(String::new()),
                                );
                            }
                        },
                        Ok(None) => {
                            // No sources file - that's fine
                            db.set_project_sources_yaml(
                                project_root.clone(),
                                Arc::new(String::new()),
                            );
                        }
                        Err(msg) => {
                            init_errors.source_errors.push(msg);
                            db.set_project_sources_yaml(
                                project_root.clone(),
                                Arc::new(String::new()),
                            );
                        }
                    }

                    // Load config (defaults to a minimal config with model_paths = ["models"])
                    let config = smelt_core::Config::load(&project_root).unwrap_or_else(|_| {
                        smelt_core::Config {
                            name: String::new(),
                            version: 1,
                            model_paths: vec!["models".to_string()],
                            seed_paths: vec!["seeds".to_string()],
                            targets: std::collections::HashMap::new(),
                            default_materialization: smelt_core::Materialization::View,
                            models: std::collections::HashMap::new(),
                            python: None,
                        }
                    });
                    let model_paths = config.model_paths.clone();

                    // Scan model directories for this project
                    for model_path in &model_paths {
                        let models_path = project_root.join(model_path);
                        match std::fs::read_dir(&models_path) {
                            Ok(entries) => {
                                for entry_result in entries {
                                    match entry_result {
                                        Ok(entry) => {
                                            let entry_path = entry.path();
                                            if entry_path.extension().and_then(|s| s.to_str())
                                                == Some("sql")
                                            {
                                                match std::fs::read_to_string(&entry_path) {
                                                    Ok(content) => {
                                                        let paths = self
                                                            .register_sql_content(
                                                                &mut db,
                                                                &entry_path,
                                                                &content,
                                                                &project_root,
                                                            )
                                                            .await;
                                                        all_files.extend(paths);
                                                    }
                                                    Err(e) => {
                                                        init_errors.model_errors.push(format!(
                                                            "Failed to read {}: {}",
                                                            entry_path.display(),
                                                            e
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            init_errors.model_errors.push(format!(
                                                "Failed to read directory entry in {}: {}",
                                                models_path.display(),
                                                e
                                            ));
                                        }
                                    }
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                // Not an error - model directory is optional
                            }
                            Err(e) => {
                                init_errors.workspace_errors.push(format!(
                                    "Failed to read {}: {}",
                                    models_path.display(),
                                    e
                                ));
                            }
                        }

                        // Discover Python models and register their generated SQL
                        let context_json = build_python_context(&all_files, &config);
                        let mut cache = self.python_cache.lock().await;
                        *cache = PythonModelCache::load(&project_root);
                        let scan_result = python_scan::discover_python_models(
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

                                db.set_file_text(
                                    virtual_sql_path.clone(),
                                    Arc::new(py_model.sql.clone()),
                                );
                                db.set_file_project_root(
                                    virtual_sql_path.clone(),
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

            db.set_all_files(Arc::new(all_files));
            db.set_all_project_roots(Arc::new(all_project_roots.clone()));

            // Store project roots for file-change handling
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

        // Register file watchers for .py files (dynamic registration)
        let registration = Registration {
            id: "python-file-watcher".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: Some(
                serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                    watchers: vec![FileSystemWatcher {
                        glob_pattern: GlobPattern::String("**/models/**/*.py".to_string()),
                        kind: Some(WatchKind::all()),
                    }],
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
                db.set_project_sources_yaml(project_root, Arc::new(params.text_document.text));
                drop(db);
                self.publish_all_diagnostics().await;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let mut db = self.db.lock().await;
            // If this file wasn't seen during init, find its project root
            let project_roots = db.all_project_roots();
            let has_project_root = project_roots.iter().any(|root| path.starts_with(root));
            if !has_project_root {
                // Try to discover project root by walking up
                if let Some(project_root) = find_project_root_by_walking_up(&path) {
                    // Register this new project
                    let mut roots = (*project_roots).clone();
                    if !roots.contains(&project_root) {
                        roots.push(project_root.clone());
                        db.set_all_project_roots(Arc::new(roots));
                        // Load sources for this project
                        let sources_content = find_config_file(&project_root, "sources")
                            .ok()
                            .flatten()
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .unwrap_or_default();
                        db.set_project_sources_yaml(
                            project_root.clone(),
                            Arc::new(sources_content),
                        );
                    }
                    db.set_file_project_root(path.clone(), project_root);
                }
            }
            // Register file content (handles multi-model splitting)
            // register_sql_content skips mutations when content hasn't changed
            let project_root_for_reg = db
                .all_project_roots()
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
            // Only update all_files if new paths were registered
            let current_files = db.all_files();
            let new_paths: Vec<_> = registered_paths
                .iter()
                .filter(|rp| !current_files.contains(rp))
                .cloned()
                .collect();
            if !new_paths.is_empty() {
                let mut files = (*current_files).clone();
                files.extend(new_paths);
                db.set_all_files(Arc::new(files));
            }
            drop(db);
            self.publish_diagnostics(uri).await;
        } else if path.extension().and_then(|s| s.to_str()) != Some("py") {
            // Non-SQL, non-sources, non-Python file
            let mut db = self.db.lock().await;
            db.set_file_text(path, Arc::new(params.text_document.text));
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
                    db.set_project_sources_yaml(project_root, Arc::new(change.text));
                    drop(db);
                    self.publish_all_diagnostics().await;
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                let mut db = self.db.lock().await;
                let project_root = db
                    .all_project_roots()
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned()
                    .unwrap_or_default();
                let registered_paths = self
                    .register_sql_content(&mut db, &path, &change.text, &project_root)
                    .await;
                // Ensure all registered paths are in all_files
                let mut files = (*db.all_files()).clone();
                let mut changed = false;
                for rp in &registered_paths {
                    if !files.contains(rp) {
                        files.push(rp.clone());
                        changed = true;
                    }
                }
                if changed {
                    db.set_all_files(Arc::new(files));
                }
                drop(db);
                self.publish_diagnostics(uri).await;
            } else if path.extension().and_then(|s| s.to_str()) != Some("py") {
                let mut db = self.db.lock().await;
                db.set_file_text(path, Arc::new(change.text));
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
                        db.set_project_sources_yaml(project_root, Arc::new(content));
                        drop(db);
                        self.publish_all_diagnostics().await;
                    }
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

        let db = self.db.lock().await;

        // Get file content and parse tree
        let text = db.file_text(effective_path.clone());
        let parse = db.parse_file(effective_path.clone());
        let syntax = parse.syntax();

        // Convert cursor position to offset
        let cursor_offset =
            position_to_offset(&text, effective_position.line, effective_position.character);

        // Resolve goto-definition target while holding the db lock and AST.
        // We collect the result as plain data (no Rowan nodes) so we can drop
        // the non-Send AST before any await points.
        enum GotoTarget {
            RefModel(PathBuf),
            SourceFile {
                sources_path: PathBuf,
                source_name: String,
                table_name: String,
            },
            /// CTE definition in the same file — target is an LSP Range
            SameFile(Range),
            /// Column definitions (potentially multiple for ambiguous refs)
            ColumnDefs(Vec<ColumnDefLocation>),
        }

        let target = if let Some(file) = AstFile::cast(syntax) {
            match symbol_at_cursor(&file, &text, cursor_offset) {
                Some(SymbolAtCursor::RefCall { name }) => {
                    db.resolve_ref(name).map(GotoTarget::RefModel)
                }
                Some(SymbolAtCursor::SourceCall {
                    source_name,
                    table_name,
                }) => {
                    let project_root = db.file_project_root(effective_path.clone());
                    if db
                        .resolve_source(
                            project_root.clone(),
                            source_name.clone(),
                            table_name.clone(),
                        )
                        .is_some()
                    {
                        let sources_path = project_root.join("sources.yml");
                        if sources_path.exists() {
                            Some(GotoTarget::SourceFile {
                                sources_path,
                                source_name,
                                table_name,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
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
                                        start: Position::new(pr.start.line, pr.start.column),
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
                                if cursor_offset >= start && cursor_offset <= end && len <= best_len
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
                                        let start: usize = first_ident.text_range().start().into();
                                        let end: usize = first_ident.text_range().end().into();
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
                                            start: Position::new(pr.start.line, pr.start.column),
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
                                        .chain(from_clause.joins().filter_map(|j| j.table_ref()))
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
                                                end: Position::new(pr.end.line, pr.end.column),
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
                None => None,
            }
        } else {
            None
        };
        drop(db);

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
            Some(GotoTarget::SourceFile {
                sources_path,
                source_name,
                table_name,
            }) => {
                let target_line = find_source_table_line(&sources_path, &source_name, &table_name);

                if let Ok(target_uri) = Url::from_file_path(&sources_path) {
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

        let db = self.db.lock().await;

        let text = db.file_text(effective_path.clone());
        let parse = db.parse_file(effective_path.clone());
        let syntax = parse.syntax();

        let cursor_offset =
            position_to_offset(&text, effective_position.line, effective_position.character);

        // Collect reference data as plain types while holding the lock.
        // We use an enum to avoid holding AST nodes across await points.
        enum RefResult {
            PathRanges(Vec<(PathBuf, smelt_parser::ast::Range)>),
            CteRanges(PathBuf, Vec<(u32, u32, u32, u32)>),
            Empty,
        }

        let ref_result = if let Some(file) = AstFile::cast(syntax) {
            match symbol_at_cursor(&file, &text, cursor_offset) {
                Some(SymbolAtCursor::RefCall { name }) => {
                    let all_file_refs: Vec<_> = db
                        .all_files()
                        .iter()
                        .map(|p| (p.clone(), (*db.model_refs(p.clone())).clone()))
                        .collect();
                    let refs = smelt_db::references::find_model_references(&name, &all_file_refs);
                    RefResult::PathRanges(refs)
                }
                Some(SymbolAtCursor::SourceCall {
                    source_name,
                    table_name,
                }) => {
                    let qualified_name = format!("{}.{}", source_name, table_name);
                    let all_file_sources: Vec<_> = db
                        .all_files()
                        .iter()
                        .map(|p| (p.clone(), (*db.model_sources(p.clone())).clone()))
                        .collect();
                    let refs = smelt_db::references::find_source_references(
                        &qualified_name,
                        &all_file_sources,
                    );
                    RefResult::PathRanges(refs)
                }
                Some(SymbolAtCursor::CteDefinition { name })
                | Some(SymbolAtCursor::CteReference { name }) => {
                    let cte_refs = smelt_db::references::find_cte_references(&file, &text, &name);
                    let ranges: Vec<_> = cte_refs
                        .iter()
                        .map(|text_range| {
                            let r = smelt_parser::ast::text_range_to_range(&text, *text_range);
                            (r.start.line, r.start.column, r.end.line, r.end.column)
                        })
                        .collect();
                    RefResult::CteRanges(effective_path.clone(), ranges)
                }
                _ => RefResult::Empty,
            }
        } else {
            RefResult::Empty
        };
        drop(db);

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

        let db = self.db.lock().await;
        let text = db.file_text(effective_path.clone());

        // Collect diagnostics overlapping the request range
        let mut all_diags = (*db.file_diagnostics(effective_path.clone())).clone();
        all_diags.extend((*db.type_diagnostics(effective_path.clone())).clone());

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

        let mut actions = Vec::new();
        for diag in &matching {
            let suggestions = smelt_db::code_actions::generate_code_actions(diag, &text);
            for suggestion in suggestions {
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

        let db = self.db.lock().await;
        let text = db.file_text(effective_path.clone());
        let parse = db.parse_file(effective_path);
        let syntax = parse.syntax();

        let result = if let Some(file) = AstFile::cast(syntax) {
            let offset =
                position_to_offset(&text, effective_position.line, effective_position.character);
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
                _ => None,
            }
        } else {
            None
        };
        drop(db);

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

        let db = self.db.lock().await;
        let text = db.file_text(effective_path.clone());
        let parse = db.parse_file(effective_path);
        let syntax = parse.syntax();

        // Collect rename edits as plain data (no AST nodes across await)
        let edits: Vec<(u32, u32, u32, u32)> = if let Some(file) = AstFile::cast(syntax) {
            let offset =
                position_to_offset(&text, effective_position.line, effective_position.character);
            match symbol_at_cursor(&file, &text, offset) {
                Some(SymbolAtCursor::CteDefinition { name })
                | Some(SymbolAtCursor::CteReference { name }) => {
                    let cte_refs = smelt_db::references::find_cte_references(&file, &text, &name);
                    cte_refs
                        .iter()
                        .map(|text_range| {
                            let r = smelt_parser::ast::text_range_to_range(&text, *text_range);
                            (r.start.line, r.start.column, r.end.line, r.end.column)
                        })
                        .collect()
                }
                _ => vec![],
            }
        } else {
            vec![]
        };
        drop(db);

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

        let db = self.db.lock().await;

        // Get file content and parse tree
        let text = db.file_text(effective_path.clone());
        let parse = db.parse_file(effective_path.clone());
        let syntax = parse.syntax();

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

        // Check if hovering over a ref() or source() call
        if let Some(file) = AstFile::cast(syntax) {
            // Check ref() calls
            for ref_call in file.refs() {
                let range = ref_call.range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();

                // Check if cursor is within this ref call
                if cursor_offset >= start && cursor_offset <= end {
                    if let Some(model_name) = ref_call.model_name() {
                        // Resolve upstream model and show its resolved schema
                        if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
                            // Use resolved_model_schema to get type information through wildcards
                            let resolved = db.resolved_model_schema(upstream_path.clone());

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
                                        if !col.expression.is_empty() && col.expression != col.name
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
                            let constraints = db.model_input_constraints(upstream_path);
                            if !constraints.is_empty() {
                                content.push_str("\n**Requires:**\n");
                                for constraint in constraints.iter() {
                                    for (col_name, col_constraint) in &constraint.required_columns {
                                        if let Some(ref typed_col) = col_constraint.expected_type {
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

            // Check source() calls
            for source_call in file.sources() {
                let range = source_call.range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();

                // Check if cursor is within this source call
                if cursor_offset >= start && cursor_offset <= end {
                    if let (Some(source_name), Some(table_name)) =
                        (source_call.source_name(), source_call.table_name())
                    {
                        let qualified_name = source_call.qualified_name().unwrap_or_default();

                        // Try to resolve the source
                        let project_root = db.file_project_root(effective_path.clone());
                        if let Some(table_def) =
                            db.resolve_source(project_root, source_name.clone(), table_name.clone())
                        {
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
                            let content =
                                format!("**Source: {}**\n\n⚠️ *Undefined source*", qualified_name);

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

        let db = self.db.lock().await;

        // Get file content
        let text = db.file_text(effective_path.clone());

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

        // Determine completion context
        let context = determine_completion_context(&text, cursor_offset);

        let items = match context {
            CompletionContext::InsideRef => {
                // Complete model names
                let models = db.all_models();
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
                let project_root = db.file_project_root(effective_path.clone());
                let config = db.sources_config(project_root);
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
                let typed_schema = db.typed_model_schema(effective_path.clone());
                let available = db.available_columns(effective_path.clone());

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
                let parse = db.parse_file(effective_path.clone());
                let syntax = parse.syntax();

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
                                    let project_root = db.file_project_root(effective_path.clone());
                                    let config = db.sources_config(project_root);
                                    for source in &config.sources {
                                        if source.name == *source_name {
                                            for table in &source.tables {
                                                if table.name == *table_name {
                                                    return Ok(Some(CompletionResponse::Array(
                                                        table
                                                            .columns
                                                            .iter()
                                                            .map(|col| {
                                                                let type_str = col
                                                                    .data_type
                                                                    .as_ref()
                                                                    .map(|t| t.to_string())
                                                                    .unwrap_or_else(|| {
                                                                        "unknown".to_string()
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
                                                    )));
                                                }
                                            }
                                        }
                                    }
                                }
                                AliasTarget::Model { model_name } => {
                                    // Get columns from the model schema
                                    let models = db.all_models();
                                    if let Some(model) =
                                        models.values().find(|m| m.name == *model_name)
                                    {
                                        let schema = db.model_schema(model.path.clone());
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
                Vec::new()
            }
            CompletionContext::FromClause => {
                // Offer CTE names defined in the current query's WITH clause
                let parse = db.parse_file(effective_path.clone());
                let syntax = parse.syntax();

                let mut items = Vec::new();

                if let Some(file) = smelt_parser::ast::File::cast(syntax) {
                    if let Some(select_stmt) = file.select_stmt() {
                        if let Some(with_clause) = select_stmt.with_clause() {
                            let type_ctx = db.type_context(effective_path.clone());

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

/// Completion context types
#[derive(Debug)]
enum CompletionContext {
    InsideRef,               // Cursor inside ref('|')
    InsideSource,            // Cursor inside source('|')
    ColumnName,              // Cursor in a position where column name is expected
    QualifiedColumn(String), // Cursor after alias. (e.g., "t." for table alias t)
    FromClause,              // Cursor in FROM/JOIN position (offer CTE names)
    None,
}

/// Determine what kind of completion to provide based on cursor position
fn determine_completion_context(text: &str, offset: usize) -> CompletionContext {
    // Look backward from cursor to determine context
    let before_cursor = &text[..offset.min(text.len())];

    // Check if we're inside source('')
    // Simple heuristic: look for source(' before cursor and no closing )
    if let Some(source_start) = before_cursor.rfind("source(") {
        let after_source = &before_cursor[source_start..];
        // Check if we're inside the quotes
        let quote_count = after_source
            .chars()
            .filter(|&c| c == '\'' || c == '"')
            .count();
        if quote_count == 1 && !after_source.contains(')') {
            // Odd number of quotes means we're inside a string, and no closing paren yet
            return CompletionContext::InsideSource;
        }
    }

    // Check if we're inside ref('')
    // Simple heuristic: look for ref(' before cursor and no closing )
    if let Some(ref_start) = before_cursor.rfind("ref(") {
        let after_ref = &before_cursor[ref_start..];
        // Check if we're inside the quotes
        let quote_count = after_ref.chars().filter(|&c| c == '\'' || c == '"').count();
        if quote_count == 1 && !after_ref.contains(')') {
            // Odd number of quotes means we're inside a string, and no closing paren yet
            return CompletionContext::InsideRef;
        }
    }

    // Check if we're after alias. (e.g., "t." for qualified column completion)
    // Look for pattern: identifier followed by dot at or just before cursor
    if let Some(alias) = extract_alias_before_dot(before_cursor) {
        return CompletionContext::QualifiedColumn(alias);
    }

    // Check if we're in a column context (after SELECT, comma in SELECT list)
    let before_trimmed = before_cursor.trim_end();

    // Look for SELECT keyword
    if let Some(select_pos) = before_trimmed.rfind("SELECT") {
        let after_select = &before_trimmed[select_pos..];
        // Make sure we haven't hit FROM yet
        if !after_select.contains("FROM") {
            // We're in the SELECT list
            return CompletionContext::ColumnName;
        }
    }

    // Check if we're in a FROM/JOIN position (after FROM or JOIN keyword)
    // Look for the last FROM or JOIN keyword and check we're in table-ref position
    let upper = before_trimmed.to_uppercase();
    if is_in_from_position(&upper) {
        return CompletionContext::FromClause;
    }

    CompletionContext::None
}

/// Check if cursor is in a FROM/JOIN table reference position
fn is_in_from_position(upper_text: &str) -> bool {
    // Find the last occurrence of FROM or JOIN keywords
    let from_pos = upper_text.rfind("FROM");
    let join_pos = upper_text.rfind("JOIN");

    let keyword_end = match (from_pos, join_pos) {
        (Some(f), Some(j)) => {
            if f > j {
                Some(f + 4) // "FROM" is 4 chars
            } else {
                Some(j + 4) // "JOIN" is 4 chars
            }
        }
        (Some(f), None) => Some(f + 4),
        (None, Some(j)) => Some(j + 4),
        (None, None) => None,
    };

    let keyword_end = match keyword_end {
        Some(e) => e,
        None => return false,
    };

    // Text after the keyword
    let after_keyword = &upper_text[keyword_end..];

    // We're in FROM position if:
    // 1. Nothing after keyword (just whitespace) - typing the first table ref
    // 2. Or after a comma (additional table ref in comma-separated list)
    // But NOT if we've already entered a complete expression (have ON, WHERE, etc.)
    let trimmed = after_keyword.trim();
    if trimmed.is_empty() {
        return true;
    }

    // If we see clause keywords after the FROM/JOIN, we've moved past table position
    let terminating_keywords = [
        "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "UNION", "ON", "USING", "INNER", "LEFT",
        "RIGHT", "FULL", "CROSS", "SELECT",
    ];
    for kw in &terminating_keywords {
        if trimmed.contains(kw) {
            return false;
        }
    }

    // If the text after keyword is just whitespace or a partial identifier, we're in position
    // Check: no complete table expression yet (no whitespace-separated tokens beyond one)
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    // If 0 tokens (just spaces) or 1 partial token being typed - we're in FROM position
    tokens.len() <= 1
}

/// Extract the alias/identifier before a dot at the end of the text
/// Returns Some(alias) if text ends with "identifier." or "identifier.partial"
fn extract_alias_before_dot(text: &str) -> Option<String> {
    // Find the last dot
    let dot_pos = text.rfind('.')?;

    // Check what's after the dot - should be empty or partial identifier
    let after_dot = &text[dot_pos + 1..];
    if !after_dot.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    // Find the identifier before the dot
    let before_dot = &text[..dot_pos];
    let before_dot_trimmed = before_dot.trim_end();

    // Walk backward to find the start of the identifier
    let mut ident_start = before_dot_trimmed.len();
    for (i, c) in before_dot_trimmed.char_indices().rev() {
        if c.is_alphanumeric() || c == '_' {
            ident_start = i;
        } else {
            break;
        }
    }

    let alias = &before_dot_trimmed[ident_start..];

    // Must be a valid identifier (not empty, starts with letter or underscore)
    if alias.is_empty() {
        return None;
    }
    let first_char = alias.chars().next()?;
    if !first_char.is_alphabetic() && first_char != '_' {
        return None;
    }

    // Avoid triggering on smelt.source() or smelt.ref() - these have dot but aren't aliases
    // Check if the identifier is "smelt" and followed by source or ref
    if alias.eq_ignore_ascii_case("smelt") {
        let after_dot_lower = after_dot.to_lowercase();
        if after_dot_lower.starts_with("source") || after_dot_lower.starts_with("ref") {
            return None;
        }
    }

    Some(alias.to_string())
}

/// Target of a table alias in FROM clause
#[derive(Debug, Clone)]
enum AliasTarget {
    Source {
        source_name: String,
        table_name: String,
    },
    Model {
        model_name: String,
    },
}

/// Extract alias mappings from a SELECT statement's FROM clause
fn extract_from_aliases(
    select_stmt: &smelt_parser::ast::SelectStmt,
    db: &smelt_db::Database,
) -> std::collections::HashMap<String, AliasTarget> {
    use smelt_parser::ast::{RefCall, SourceCall};

    let mut aliases = std::collections::HashMap::new();

    if let Some(from_clause) = select_stmt.from_clause() {
        // Process main table refs in FROM clause
        for table_ref in from_clause.table_refs() {
            if let Some(func) = table_ref.function_call() {
                // Check for smelt.source()
                if let Some(source_call) = SourceCall::from_function_call(func.clone()) {
                    if let (Some(source_name), Some(table_name)) =
                        (source_call.source_name(), source_call.table_name())
                    {
                        // Use explicit alias if present, otherwise use table name
                        let alias_name = table_ref.alias().unwrap_or_else(|| table_name.clone());
                        aliases.insert(
                            alias_name,
                            AliasTarget::Source {
                                source_name,
                                table_name,
                            },
                        );
                    }
                }
                // Check for smelt.ref()
                else if let Some(ref_call) = RefCall::from_function_call(func) {
                    if let Some(model_name) = ref_call.model_name() {
                        // Use explicit alias if present, otherwise use model name
                        let alias_name = table_ref.alias().unwrap_or_else(|| model_name.clone());
                        aliases.insert(alias_name, AliasTarget::Model { model_name });
                    }
                }
            }
        }

        // Process JOINed table refs
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if let Some(func) = table_ref.function_call() {
                    // Check for smelt.source()
                    if let Some(source_call) = SourceCall::from_function_call(func.clone()) {
                        if let (Some(source_name), Some(table_name)) =
                            (source_call.source_name(), source_call.table_name())
                        {
                            let alias_name =
                                table_ref.alias().unwrap_or_else(|| table_name.clone());
                            aliases.insert(
                                alias_name,
                                AliasTarget::Source {
                                    source_name,
                                    table_name,
                                },
                            );
                        }
                    }
                    // Check for smelt.ref()
                    else if let Some(ref_call) = RefCall::from_function_call(func) {
                        if let Some(model_name) = ref_call.model_name() {
                            let alias_name =
                                table_ref.alias().unwrap_or_else(|| model_name.clone());
                            aliases.insert(alias_name, AliasTarget::Model { model_name });
                        }
                    }
                }
            }
        }
    }

    // Note: db parameter reserved for future use (e.g., resolving model schemas)
    let _ = db;

    aliases
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_clause_context_after_from_keyword() {
        let text = "WITH cte AS (SELECT 1) SELECT * FROM ";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_from_clause_context_partial_identifier() {
        let text = "WITH cte AS (SELECT 1) SELECT * FROM ct";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_from_clause_context_after_join() {
        let text = "SELECT * FROM a JOIN ";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_not_from_context_inside_ref() {
        let text = "SELECT * FROM smelt.ref('";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::InsideRef));
    }

    #[test]
    fn test_not_from_context_inside_source() {
        let text = "SELECT * FROM smelt.source('";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::InsideSource));
    }

    #[test]
    fn test_not_from_context_after_where() {
        // After WHERE, we're past the FROM clause table position
        let text = "SELECT * FROM t WHERE ";
        let ctx = determine_completion_context(text, text.len());
        assert!(!matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_not_from_context_after_on() {
        let text = "SELECT * FROM a JOIN b ON ";
        let ctx = determine_completion_context(text, text.len());
        assert!(!matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_from_position_empty_after_from() {
        assert!(is_in_from_position("SELECT * FROM "));
    }

    #[test]
    fn test_from_position_partial_identifier() {
        assert!(is_in_from_position("SELECT * FROM CT"));
    }

    #[test]
    fn test_from_position_after_join() {
        assert!(is_in_from_position("SELECT * FROM A JOIN "));
    }

    #[test]
    fn test_not_from_position_complete_table_ref() {
        // After a complete table ref with alias, we're past the position
        assert!(!is_in_from_position("SELECT * FROM TABLE_A T"));
    }
}
