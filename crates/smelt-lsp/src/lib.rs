use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    check_type_diagnostics, file_diagnostics, functions_in_file,
    yaml_edits::{find_source_column_yaml_rename, find_source_table_yaml_rename},
    Database, Diagnostic as DbDiagnostic, DiagnosticAcc, DiagnosticCode as DbCode,
    DiagnosticData as DbData, DiagnosticSeverity as DbSeverity, ProjectInput, SourceFile,
    Workspace,
};
// salsa is not a direct dependency of smelt-lsp; we access
// the Salsa DB exclusively through smelt_db's public API.

mod python_scan;
use python_scan::PythonModelCache;

// ---------------------------------------------------------------------------
// Thin helpers mapping path-based LSP operations onto the new salsa 0.26 API.
//
// The new `smelt_db::Database` exposes inputs as structs (`SourceFile`,
// `ProjectInput`) rather than keyed queries. The LSP still thinks in terms of
// file paths, so these helpers look up the right input struct and call the
// matching free-function tracked query.
//
// Writes flow through `Backend.db` (`Arc<tokio::Mutex<Database>>`) for
// coordination. Reads snapshot the DB (`Database: Clone` with internal
// Arc-storage) and drop the mutex before running queries, so concurrent
// readers never serialize on the write lock.
// ---------------------------------------------------------------------------

/// Look up the `SourceFile` input for `path`, returning `None` if not
/// registered yet.
fn lookup_file(db: &Database, path: &Path) -> Option<SourceFile> {
    db.source_file(path)
}

/// All source files currently registered in the workspace.
fn workspace_files(db: &Database) -> Vec<SourceFile> {
    match Workspace::try_get(db) {
        Some(ws) => ws.files(db).clone(),
        None => Vec::new(),
    }
}

/// All known file paths.
fn all_file_paths(db: &Database) -> Vec<PathBuf> {
    workspace_files(db)
        .into_iter()
        .map(|f| f.path(db).clone())
        .collect()
}

/// Look up a `ProjectInput` by its root path.
fn lookup_project(db: &Database, root: &Path) -> Option<ProjectInput> {
    db.project_input(root)
}

/// Resolve a model name to the file that defines it (via the workspace).
fn resolve_ref_path(db: &Database, model_name: &str) -> Option<PathBuf> {
    let ws = Workspace::try_get(db)?;
    smelt_db::resolve_ref(db, ws, model_name.to_string()).map(|f| f.path(db).clone())
}

/// Shorthand for calling the `file_diagnostics` query given a file path.
fn diagnostics_for(db: &Database, path: &Path) -> Vec<DbDiagnostic> {
    let Some(file) = lookup_file(db, path) else {
        return Vec::new();
    };
    let ws = match Workspace::try_get(db) {
        Some(w) => w,
        None => return Vec::new(),
    };
    let mut diags = file_diagnostics(db, ws, file);
    diags.extend(
        check_type_diagnostics::accumulated::<DiagnosticAcc>(db, ws, file)
            .into_iter()
            .map(|d| d.0.clone()),
    );
    diags
}

/// Project root recorded on the `SourceFile` input for `path`.
fn file_project_root(db: &Database, path: &Path) -> PathBuf {
    lookup_file(db, path)
        .map(|f| f.project_root(db).clone())
        .unwrap_or_default()
}

/// File text for `path`; returns empty string if the file isn't registered.
fn file_text(db: &Database, path: &Path) -> String {
    lookup_file(db, path)
        .map(|f| f.text(db).clone())
        .unwrap_or_default()
}

/// Raw sources.yml text for the project rooted at `root`.
fn project_sources_yaml(db: &Database, root: &Path) -> String {
    lookup_project(db, root)
        .map(|p| p.sources_yaml(db).clone())
        .unwrap_or_default()
}
use smelt_parser::ast::File as AstFile;
use smelt_parser::is_valid_sql_identifier;
use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};
use smelt_types::{format_smelt_type_hover, TypedColumn};

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
    let Some(current_file) = lookup_file(db, current_path) else {
        return Vec::new();
    };
    let Some(ws) = Workspace::try_get(db) else {
        return Vec::new();
    };
    let ctx = smelt_db::type_context(db, ws, current_file);

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
    let Some(current_file) = lookup_file(db, current_path) else {
        return;
    };
    let parse = smelt_db::parse_file(db, current_file);
    let text = current_file.text(db).clone();
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
                                    if let Some(upstream_path) = resolve_ref_path(db, &model_name) {
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
        if let Some(upstream_path) = resolve_ref_path(db, model_name) {
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

    let Some(file) = lookup_file(db, model_path) else {
        return false;
    };
    let schema = smelt_db::model_schema(db, file);
    let text = file.text(db).clone();

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
        if let Some(upstream_path) = resolve_ref_path(db, &ext.ref_name) {
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
    let project_root = file_project_root(db, current_path);
    let sources_path = project_root.join("sources.yml");
    if !sources_path.exists() {
        return;
    }

    // Get source references from FROM clause
    let Some(current_file) = lookup_file(db, current_path) else {
        return;
    };
    let parse = smelt_db::parse_file(db, current_file);
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
        let project = match lookup_project(db, &project_root) {
            Some(p) => p,
            None => continue,
        };
        if let Some(table_def) =
            smelt_db::resolve_source(db, project, source_name.clone(), table_name.clone())
        {
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
    let file = match lookup_file(db, path) {
        Some(f) => f,
        None => return vec![],
    };
    let parse = smelt_db::parse_file(db, file);
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

/// Trace upstream to find and collect column definition edits in an upstream model.
/// Follows wildcard (SELECT *) chains up to 10 levels deep.
fn trace_upstream_column(
    db: &Database,
    all_files: &[PathBuf],
    model_name: &str,
    column_name: &str,
    edits: &mut Vec<(PathBuf, u32, u32, u32, u32)>,
) {
    for upstream_path in all_files.iter() {
        let up_name = upstream_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if up_name == model_name {
            trace_upstream_column_chain(db, upstream_path, column_name, 10, edits);
            break;
        }
    }
}

/// Recursively trace a column definition through upstream models,
/// following wildcard (SELECT *) chains.
fn trace_upstream_column_chain(
    db: &Database,
    model_path: &std::path::Path,
    column_name: &str,
    depth_limit: usize,
    edits: &mut Vec<(PathBuf, u32, u32, u32, u32)>,
) -> bool {
    if depth_limit == 0 {
        return false;
    }

    let up_file_input = match lookup_file(db, model_path) {
        Some(f) => f,
        None => return false,
    };
    let up_text = up_file_input.text(db).clone();
    let up_parse = smelt_db::parse_file(db, up_file_input);
    let up_syntax = up_parse.syntax();
    if let Some(up_file) = AstFile::cast(up_syntax) {
        if let Some(def_range) =
            smelt_db::references::find_column_definition_in_select(&up_file, column_name)
        {
            let r = smelt_parser::ast::text_range_to_range(&up_text, def_range);
            edits.push((
                model_path.to_path_buf(),
                r.start.line,
                r.start.column,
                r.end.line,
                r.end.column,
            ));
            return true;
        }

        // Check wildcard extensions (SELECT *)
        let schema = smelt_db::model_schema(db, up_file_input);
        for ext in &schema.row_extensions {
            if let Some(upstream_path) = resolve_ref_path(db, &ext.ref_name) {
                if trace_upstream_column_chain(
                    db,
                    &upstream_path,
                    column_name,
                    depth_limit - 1,
                    edits,
                ) {
                    return true;
                }
            }
        }
    }
    false
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

/// Render a database diagnostic into the LSP message body + per-frame
/// `related_information` list.
///
/// Pulled out of [`Backend::to_lsp_diagnostic`] as a pure function so it
/// can be unit-tested without needing to construct a live `Backend`/LSP
/// `Client`. The returned tuple is `(message, related_information)`:
///
/// * `message` — the primary diagnostic text. If the diagnostic carries
///   an `ExpansionFrames` payload, one trailer line per frame is appended
///   in outer-to-inner order (Phase 12 §16 #16 Step 2 renderer).
/// * `related_information` — `Some(..)` with one entry per frame that
///   carries both `decl_path` and `decl_range`; `None` otherwise (no
///   frames, or no frame had resolvable location metadata).
///
/// The underlying `FrameInfo` vector is stored innermost-first →
/// outermost-last (the canonical merge order used by
/// `smelt_db::check_smelt_fn_call`), so we iterate in reverse to render
/// the outermost (user-visible) call first, then walk down to the
/// innermost cause.
pub fn render_expansion_frames(
    diag: &DbDiagnostic,
) -> (String, Option<Vec<DiagnosticRelatedInformation>>) {
    let mut message = diag.message.clone();
    let Some(DbData::ExpansionFrames(frames)) = diag.data.as_ref() else {
        return (message, None);
    };
    let mut related: Vec<DiagnosticRelatedInformation> = Vec::new();
    for frame in frames.iter().rev() {
        message.push_str(&format!(
            "\nin expansion of `{}`, `{}` was bound to {}",
            frame.function, frame.param, frame.bound_type,
        ));
        if let (Some(path), Some(range)) = (&frame.decl_path, frame.decl_range.as_ref()) {
            if let Ok(uri) = Url::from_file_path(path) {
                related.push(DiagnosticRelatedInformation {
                    location: Location {
                        uri,
                        range: Range {
                            start: Position {
                                line: range.start.line,
                                character: range.start.column,
                            },
                            end: Position {
                                line: range.end.line,
                                character: range.end.column,
                            },
                        },
                    },
                    message: format!(
                        "in expansion of `{}`, `{}` was bound to {}",
                        frame.function, frame.param, frame.bound_type,
                    ),
                });
            }
        }
    }
    let related_information = if related.is_empty() {
        None
    } else {
        Some(related)
    };
    (message, related_information)
}

/// Find the innermost `SMELT_FN_CALL` whose text range contains `offset`.
///
/// Used by hover and completion to dispatch on a `smelt.fn.<name>(...)`
/// call site (Phase 48: hover wiring + PASSING-body completion).
pub fn find_smelt_fn_call_at_cursor(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    offset: usize,
) -> Option<smelt_parser::ast::SmeltFnCall> {
    let mut best: Option<smelt_parser::ast::SmeltFnCall> = None;
    let mut best_size: usize = usize::MAX;
    for node in syntax.descendants() {
        if let Some(call) = smelt_parser::ast::SmeltFnCall::cast(node) {
            let r = call.text_range();
            let start: usize = r.start().into();
            let end: usize = r.end().into();
            if offset >= start && offset <= end {
                let size = end.saturating_sub(start);
                if size < best_size {
                    best = Some(call);
                    best_size = size;
                }
            }
        }
    }
    best
}

/// Phase 48 — completion helper: resolve the columns of a function-body
/// CTE named `ctx_name` for use in PASSING-body completion. The function
/// body lives in the workspace, so we walk all files to find the
/// signature's declaration and look up the matching CTE in its body.
///
/// Returns an empty vector when the context can't be resolved (function
/// not found, body shape unexpected, no matching CTE).
pub fn passing_body_completion_columns(
    db: &smelt_db::Database,
    workspace: smelt_db::Workspace,
    sig: &smelt_types::FunctionSig,
    ctx_name: &str,
) -> Vec<(String, smelt_types::TypedColumn)> {
    let files: Vec<smelt_db::SourceFile> = workspace.files(db).to_vec();
    for f in files {
        let parse = smelt_db::parse_file(db, f);
        let ast = match smelt_parser::ast::File::cast(parse.syntax()) {
            Some(a) => a,
            None => continue,
        };
        for define in ast.defines() {
            if define.name().as_deref() != Some(sig.name.as_str()) {
                continue;
            }
            let Some(body) = define.body() else { continue };
            // Body may be a SELECT (TableExpr) or a wrapped expression. We
            // only mine CTEs from the SELECT shape.
            let Some(select) = body.select_stmt() else {
                continue;
            };
            let Some(with_clause) = select.with_clause() else {
                continue;
            };
            for cte in with_clause.ctes() {
                if cte.name().as_deref() != Some(ctx_name) {
                    continue;
                }
                // Use a minimal type context — the goal is just to
                // surface column names; type info is best-effort.
                let ctx = smelt_db::TypeContext::new();
                return smelt_db::infer_cte_columns(&cte, &ctx)
                    .into_iter()
                    .filter(|(n, _)| n != "*")
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Phase 48 — completion helper: canonical aggregate function labels for
/// PASSING-body completion when the parameter kind is `Agg` or higher.
/// Scoped down per the plan to a small high-signal set (`COUNT`, `SUM`,
/// `AVG`, `MIN`, `MAX`); the full kind-filtered set is future work.
pub fn passing_body_aggregate_labels() -> Vec<&'static str> {
    vec!["COUNT", "SUM", "AVG", "MIN", "MAX"]
}

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

                    // Load sources config for this project
                    match find_config_file(&project_root, "sources") {
                        Ok(Some(sources_path)) => match std::fs::read_to_string(&sources_path) {
                            Ok(content) => {
                                db.set_project_input(project_root.clone(), content);
                            }
                            Err(e) => {
                                init_errors.source_errors.push(format!(
                                    "Failed to read {}: {}",
                                    sources_path.display(),
                                    e
                                ));
                                db.set_project_input(project_root.clone(), String::new());
                            }
                        },
                        Ok(None) => {
                            // No sources file - that's fine
                            db.set_project_input(project_root.clone(), String::new());
                        }
                        Err(msg) => {
                            init_errors.source_errors.push(msg);
                            db.set_project_input(project_root.clone(), String::new());
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
                        Some(SymbolAtCursor::RefCall { name }) => {
                            resolve_ref_path(&db, &name).map(GotoTarget::RefModel)
                        }
                        Some(SymbolAtCursor::SourceCall {
                            source_name,
                            table_name,
                        }) => {
                            let project_root = file_project_root(&db, &effective_path);
                            let project = lookup_project(&db, &project_root);
                            if project
                                .and_then(|p| {
                                    smelt_db::resolve_source(
                                        &db,
                                        p,
                                        source_name.clone(),
                                        table_name.clone(),
                                    )
                                })
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
                        None => None,
                    }
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
                    match symbol_at_cursor(&file, &text, cursor_offset) {
                        Some(SymbolAtCursor::RefCall { name }) => {
                            let all_files = workspace_files(&db);
                            let all_file_refs: Vec<_> = all_files
                                .iter()
                                .map(|f| {
                                    (
                                        f.path(&db).clone(),
                                        (*smelt_db::model_refs(&db, *f)).clone(),
                                    )
                                })
                                .collect();
                            let refs =
                                smelt_db::references::find_model_references(&name, &all_file_refs);
                            RefResult::PathRanges(refs)
                        }
                        Some(SymbolAtCursor::SourceCall {
                            source_name,
                            table_name,
                        }) => {
                            let qualified_name = format!("{}.{}", source_name, table_name);
                            let all_files = workspace_files(&db);
                            let all_file_sources: Vec<_> = all_files
                                .iter()
                                .map(|f| {
                                    (
                                        f.path(&db).clone(),
                                        (*smelt_db::model_sources(&db, *f)).clone(),
                                    )
                                })
                                .collect();
                            let refs = smelt_db::references::find_source_references(
                                &qualified_name,
                                &all_file_sources,
                            );
                            RefResult::PathRanges(refs)
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
                    Some(SymbolAtCursor::RefCall { name }) => {
                        // For ref calls, return the content range (inside quotes)
                        let mut found_range = None;
                        for ref_call in file.refs() {
                            if ref_call.model_name().as_deref() == Some(name.as_str()) {
                                if let Some(content_range) = ref_call.content_range() {
                                    let r = smelt_parser::ast::text_range_to_range(
                                        &text,
                                        content_range,
                                    );
                                    found_range = Some((
                                        r.start.line,
                                        r.start.column,
                                        r.end.line,
                                        r.end.column,
                                    ));
                                    break;
                                }
                            }
                        }
                        found_range.map(|(sl, sc, el, ec)| (sl, sc, el, ec, name))
                    }
                    Some(SymbolAtCursor::SourceCall {
                        source_name,
                        table_name,
                    }) => {
                        // For source calls, return the table_name_range (just the table part)
                        let mut found_range = None;
                        for source_call in file.sources() {
                            if source_call.source_name().as_deref() == Some(source_name.as_str())
                                && source_call.table_name().as_deref() == Some(table_name.as_str())
                            {
                                if let Some(tn_range) = source_call.table_name_range() {
                                    let r = smelt_parser::ast::text_range_to_range(&text, tn_range);
                                    found_range = Some((
                                        r.start.line,
                                        r.start.column,
                                        r.end.line,
                                        r.end.column,
                                    ));
                                    break;
                                }
                            }
                        }
                        found_range.map(|(sl, sc, el, ec)| (sl, sc, el, ec, table_name))
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
                    _ => None,
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
            Source {
                /// (file_path, start_line, start_col, end_line, end_col) for SQL edits
                sql_edits: Vec<(PathBuf, u32, u32, u32, u32)>,
                /// YAML line edit: (line_number, old_line, new_line)
                yaml_edit: Option<(u32, String, String)>,
                /// Path to sources.yml
                sources_yml_path: PathBuf,
            },
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
                        Some(SymbolAtCursor::RefCall { name }) => {
                            // Check for naming conflict
                            let all_files_paths = all_file_paths(&db);
                            let all_files_inputs = workspace_files(&db);
                            let new_model_path = effective_path
                                .parent()
                                .unwrap_or(effective_path.as_ref())
                                .join(format!("{}.sql", new_name));
                            if all_files_paths.contains(&new_model_path) {
                                return Err(tower_lsp::jsonrpc::Error::invalid_params(format!(
                                    "A model named '{}' already exists",
                                    new_name
                                )));
                            }

                            // Collect all ref locations across the project
                            let all_file_refs: Vec<_> = all_files_inputs
                                .iter()
                                .map(|f| {
                                    (
                                        f.path(&db).clone(),
                                        (*smelt_db::model_refs(&db, *f)).clone(),
                                    )
                                })
                                .collect();
                            let ref_locations =
                                smelt_db::references::find_model_references(&name, &all_file_refs);

                            // For each ref location, get the content range (inside quotes)
                            let mut edits = Vec::new();
                            for (ref_path, _) in &ref_locations {
                                let ref_file = match lookup_file(&db, ref_path) {
                                    Some(f) => f,
                                    None => continue,
                                };
                                let ref_text = ref_file.text(&db).clone();
                                let ref_parse = smelt_db::parse_file(&db, ref_file);
                                let ref_syntax = ref_parse.syntax();
                                if let Some(ref_file) = AstFile::cast(ref_syntax) {
                                    for ref_call in ref_file.refs() {
                                        if ref_call.model_name().as_deref() == Some(&name) {
                                            if let Some(content_range) = ref_call.content_range() {
                                                let r = smelt_parser::ast::text_range_to_range(
                                                    &ref_text,
                                                    content_range,
                                                );
                                                edits.push((
                                                    ref_path.clone(),
                                                    r.start.line,
                                                    r.start.column,
                                                    r.end.line,
                                                    r.end.column,
                                                ));
                                            }
                                        }
                                    }
                                }
                            }

                            // Check if the model .sql file exists
                            let model_dir =
                                effective_path.parent().unwrap_or(effective_path.as_ref());
                            let old_model_path = model_dir.join(format!("{}.sql", name));
                            let old_path = if all_files_paths.contains(&old_model_path) {
                                Some(old_model_path)
                            } else {
                                None
                            };

                            Some(RenameKind::Model {
                                model_name: name,
                                edits,
                                old_model_path: old_path,
                            })
                        }
                        Some(SymbolAtCursor::SourceCall {
                            source_name,
                            table_name,
                        }) => {
                            let qualified = format!("{}.{}", source_name, table_name);

                            // Collect all source() call sites across the project
                            let all_files_inputs = workspace_files(&db);
                            let all_file_sources: Vec<_> = all_files_inputs
                                .iter()
                                .map(|f| {
                                    (
                                        f.path(&db).clone(),
                                        (*smelt_db::model_sources(&db, *f)).clone(),
                                    )
                                })
                                .collect();
                            let source_locations = smelt_db::references::find_source_references(
                                &qualified,
                                &all_file_sources,
                            );

                            // For each source location, get the table_name_range
                            let mut sql_edits = Vec::new();
                            for (ref_path, _) in &source_locations {
                                let ref_file = match lookup_file(&db, ref_path) {
                                    Some(f) => f,
                                    None => continue,
                                };
                                let ref_text = ref_file.text(&db).clone();
                                let ref_parse = smelt_db::parse_file(&db, ref_file);
                                let ref_syntax = ref_parse.syntax();
                                if let Some(ref_file) = AstFile::cast(ref_syntax) {
                                    for source_call in ref_file.sources() {
                                        if source_call.source_name().as_deref()
                                            == Some(source_name.as_str())
                                            && source_call.table_name().as_deref()
                                                == Some(table_name.as_str())
                                        {
                                            if let Some(tn_range) = source_call.table_name_range() {
                                                let r = smelt_parser::ast::text_range_to_range(
                                                    &ref_text, tn_range,
                                                );
                                                sql_edits.push((
                                                    ref_path.clone(),
                                                    r.start.line,
                                                    r.start.column,
                                                    r.end.line,
                                                    r.end.column,
                                                ));
                                            }
                                        }
                                    }
                                }
                            }

                            // Find the YAML table key line for rename
                            let project_root = file_project_root(&db, &effective_path);
                            let sources_yml_content = project_sources_yaml(&db, &project_root);
                            let sources_yml_path = project_root.join("sources.yml");

                            let yaml_edit = find_source_table_yaml_rename(
                                &sources_yml_content,
                                &source_name,
                                &table_name,
                                &new_name,
                            );

                            Some(RenameKind::Source {
                                sql_edits,
                                yaml_edit,
                                sources_yml_path,
                            })
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
                                        let down_model_refs =
                                            smelt_db::model_refs(&db, down_file_input);
                                        if !down_model_refs.iter().any(|r| r.name == *exposing) {
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
                        _ => None,
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
            Some(RenameKind::Source {
                sql_edits,
                yaml_edit,
                sources_yml_path,
            }) => {
                if sql_edits.is_empty() && yaml_edit.is_none() {
                    return Ok(None);
                }

                let mut document_changes: Vec<DocumentChangeOperation> = Vec::new();

                // Group SQL text edits by file path
                let mut edits_by_file: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
                for (file_path, sl, sc, el, ec) in sql_edits {
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

                // Add YAML edit for the table key rename
                if let Some((line_num, _old_line, new_line)) = yaml_edit {
                    let yaml_uri =
                        Url::from_file_path(&sources_yml_path).unwrap_or_else(|_| uri.clone());
                    // Replace the entire line containing the old table key
                    // We need to figure out the line length. Since we have the old line,
                    // we use it to compute the end column.
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

        // Check if hovering over a ref() or source() call
        if let Some(syntax) = syntax {
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

                // Phase 48: hover on a `smelt.fn.<name>(...)` call site —
                // surface the declared return type or the parameter binding
                // for a `PASSING <name> AS (...)` clause.
                if let Some(call) = find_smelt_fn_call_at_cursor(file.syntax(), cursor_offset) {
                    let segments = call.call_path().map(|p| p.segments()).unwrap_or_default();
                    let fn_name = segments.last().cloned().unwrap_or_default();
                    let ws = Workspace::try_get(&db);
                    let sig = ws.and_then(|w| smelt_db::resolve_function(&db, w, fn_name.clone()));

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
                if let Some(w) = ws {
                    if let Some(sig) =
                        smelt_db::resolve_function(&db, w, callee.clone()).map(|arc| (*arc).clone())
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
    /// Phase 48: cursor inside the body of a `PASSING <name> AS (|)` clause
    /// attached to a `smelt.fn.<callee>(...)` call. Carries the parameter
    /// name and the trailing call-path segment so the completion list can
    /// be filtered by the callee's signature.
    InPassingBody {
        callee: String,
        passing_name: String,
    },
    None,
}

/// Determine what kind of completion to provide based on cursor position
fn determine_completion_context(text: &str, offset: usize) -> CompletionContext {
    // Look backward from cursor to determine context
    let before_cursor = &text[..offset.min(text.len())];

    // Phase 48: detect cursor sitting inside the body of a
    // `PASSING <name> AS (|)` clause. Heuristic: walk backwards from the
    // cursor for an unmatched `(` whose preceding tokens form
    // `PASSING <ident> AS`. The callee name is the last segment of the
    // most recent `smelt.fn.<...>` call before the PASSING.
    if let Some(ctx) = detect_passing_body(before_cursor) {
        return ctx;
    }

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

/// Phase 48: heuristically detect whether the cursor sits inside the body
/// of a `PASSING <name> AS (|)` clause attached to a `smelt.fn.<callee>(...)`
/// call.
///
/// The heuristic walks backwards from the cursor:
/// 1. Find the nearest unmatched `(`. The cursor lies inside whatever
///    parenthesised expression that opener belongs to.
/// 2. Just before that `(` (allowing whitespace), look for the literal
///    `AS`. Before that, an identifier (the parameter name). Before that,
///    the keyword `PASSING`.
/// 3. Before the `PASSING`, the most recent `smelt.fn.<...>(...)` call
///    determines the callee name (last dot-segment of the call path).
///
/// Returns `None` for non-PASSING-body cursors so the rest of the
/// dispatcher takes over.
fn detect_passing_body(before_cursor: &str) -> Option<CompletionContext> {
    // Step 1: find the nearest unmatched open-paren walking right-to-left.
    let mut depth = 0i32;
    let mut open_paren: Option<usize> = None;
    for (i, ch) in before_cursor.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    open_paren = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let open_paren = open_paren?;

    // Step 2: directly before the `(` we should see `AS` (case-insensitive,
    // possibly with surrounding whitespace).
    let pre = before_cursor[..open_paren].trim_end();
    let as_end = pre.len();
    if !pre.to_ascii_uppercase().ends_with("AS") {
        return None;
    }
    let after_as = &pre[..as_end - 2];
    let after_as_trimmed = after_as.trim_end();

    // Step 3: extract the identifier before AS — the PASSING name.
    let mut name_end = after_as_trimmed.len();
    let mut name_start = name_end;
    for (i, ch) in after_as_trimmed.char_indices().rev() {
        if ch.is_alphanumeric() || ch == '_' {
            name_start = i;
        } else {
            name_end = name_start;
            break;
        }
        // If we walk to the very start, name_end stays at full length.
    }
    if name_start == after_as_trimmed.len() {
        return None;
    }
    let passing_name = &after_as_trimmed[name_start..name_end.max(name_start + 1)];
    if passing_name.is_empty() {
        return None;
    }

    // Step 4: before the parameter name, the keyword `PASSING`.
    let pre_name = after_as_trimmed[..name_start].trim_end();
    if !pre_name.to_ascii_uppercase().ends_with("PASSING") {
        return None;
    }

    // Step 5: extract the callee name — last `smelt.fn.<...>` call before
    // the `PASSING`. We look for the most recent `smelt.fn.` literal in
    // `before_cursor` and take the dotted-identifier that follows.
    let smelt_fn = before_cursor.rfind("smelt.fn.")?;
    let after = &before_cursor[smelt_fn + "smelt.fn.".len()..];
    let mut last_segment_end = 0usize;
    for (i, ch) in after.char_indices() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            last_segment_end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    let dotted = &after[..last_segment_end];
    let callee = dotted.split('.').next_back()?.to_string();
    if callee.is_empty() {
        return None;
    }

    Some(CompletionContext::InPassingBody {
        callee,
        passing_name: passing_name.to_string(),
    })
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

    // =====================================================================
    // Phase 12 — multi-level frame rendering (smelt-functions Step 2).
    //
    // These tests exercise `render_expansion_frames` directly because it
    // is a pure function over a `DbDiagnostic` — we don't need a running
    // `Backend` / tower-lsp `Client` to validate the renderer contract.
    // =====================================================================

    use smelt_db::{
        Diagnostic as DbDiagnosticT, DiagnosticCode, DiagnosticData,
        DiagnosticSeverity as DbSeverityT, Range as DbRange,
    };
    use smelt_parser::ast::Position as DbPosition;
    use smelt_types::FrameInfo;

    fn make_db_range(line: u32, col: u32) -> DbRange {
        DbRange {
            start: DbPosition { line, column: col },
            end: DbPosition {
                line,
                column: col + 1,
            },
        }
    }

    fn make_frame(function: &str, param: &str, bound_type: &str, decl_line: u32) -> FrameInfo {
        // Use a temp-dir file path so `Url::from_file_path` succeeds on
        // Linux/macOS (the path must be absolute). We can't reach for a
        // real tempfile in a unit test without pulling tempfile into
        // dev-deps; using the conventional `/tmp/...` absolute path keeps
        // the URL builder happy on the CI runner.
        let path = PathBuf::from(format!("/tmp/smelt-lsp-test-{function}.sql"));
        FrameInfo {
            function: function.to_string(),
            param: param.to_string(),
            bound_type: bound_type.to_string(),
            decl_path: Some(path),
            decl_range: Some(make_db_range(decl_line, 0)),
            call_site_range: Some(make_db_range(decl_line + 10, 0)),
        }
    }

    fn make_db_diag(message: &str, frames: Vec<FrameInfo>) -> DbDiagnosticT {
        DbDiagnosticT {
            severity: DbSeverityT::Error,
            message: message.to_string(),
            range: make_db_range(0, 0),
            code: Some(DiagnosticCode::UnknownIdentifier),
            data: Some(DiagnosticData::ExpansionFrames(frames)),
        }
    }

    /// Phase 12 TDD test #4 — LSP e2e: nested-call error includes
    /// `relatedInformation` per frame. A three-level expansion chain
    /// (`outer_call → middle → inner_unary`) must yield exactly three
    /// related-info entries and a message with three trailer lines, all
    /// in outer-to-inner order.
    #[test]
    fn lsp_diagnostic_formats_frames_as_related_information() {
        // Innermost-first → outermost-last data layout, matching the
        // `check_smelt_fn_call` merge contract.
        let frames = vec![
            make_frame("inner_unary", "x", "INTEGER", 1),
            make_frame("middle", "z", "INTEGER", 2),
            make_frame("outer_call", "y", "INTEGER", 3),
        ];
        let diag = make_db_diag("unknown identifier `undefined_var`", frames);

        let (message, related) = render_expansion_frames(&diag);

        // 1. The related-information list must have one entry per frame.
        let related = related.expect("expected three-level chain to produce related_information");
        assert_eq!(
            related.len(),
            3,
            "expected one DiagnosticRelatedInformation per frame, got {related:#?}"
        );

        // 2. Frame order in related-information is outer-to-inner
        //    (`frames.iter().rev()` in the renderer).
        assert!(
            related[0].message.contains("outer_call"),
            "first related-info entry must be the outermost frame, got: {}",
            related[0].message
        );
        assert!(
            related[1].message.contains("middle"),
            "second related-info entry must be the middle frame, got: {}",
            related[1].message
        );
        assert!(
            related[2].message.contains("inner_unary"),
            "third related-info entry must be the innermost frame, got: {}",
            related[2].message
        );

        // 3. URIs must resolve to a real file-scheme URL.
        for info in &related {
            assert_eq!(info.location.uri.scheme(), "file");
            assert!(
                info.location.uri.to_file_path().is_ok(),
                "related-info URI must round-trip to a file path: {}",
                info.location.uri
            );
        }

        // 4. The message body is expanded with one trailer line per frame,
        //    outer-to-inner.
        let pos_outer = message
            .find("outer_call")
            .expect("rendered message must mention outer_call");
        let pos_middle = message
            .find("middle")
            .expect("rendered message must mention middle");
        let pos_inner = message
            .find("inner_unary")
            .expect("rendered message must mention inner_unary");
        assert!(
            pos_outer < pos_middle && pos_middle < pos_inner,
            "message trailers must render outer-to-inner; got {pos_outer}/{pos_middle}/{pos_inner} — rendered:\n{message}"
        );
    }

    /// Phase 12 — single-frame diagnostics still render one trailer line
    /// and exactly one related-information entry (Phase 6 behaviour
    /// preserved).
    #[test]
    fn lsp_single_level_frame_preserves_phase6_rendering() {
        let frames = vec![make_frame("safe_divide", "numerator", "TEXT", 0)];
        let diag = make_db_diag("type mismatch in body", frames);

        let (message, related) = render_expansion_frames(&diag);

        let related = related.expect("single frame must still produce related_information");
        assert_eq!(
            related.len(),
            1,
            "single-frame diagnostics must emit exactly one related-info entry"
        );
        assert!(related[0].message.contains("safe_divide"));

        // Exactly one trailer line was appended.
        let trailer_count = message.matches("\nin expansion of `").count();
        assert_eq!(
            trailer_count, 1,
            "single-frame diagnostic must have exactly one trailer line, got: {message}"
        );
    }

    /// Phase 12 — diagnostics without any `ExpansionFrames` payload must
    /// pass through untouched. This is the common case for non-function
    /// diagnostics (unknown model refs, type mismatches in model SQL,
    /// etc.) and must stay zero-cost.
    #[test]
    fn lsp_non_frame_diagnostics_unaffected() {
        let diag = DbDiagnosticT {
            severity: DbSeverityT::Error,
            message: "undefined model `foo`".to_string(),
            range: make_db_range(0, 0),
            code: Some(DiagnosticCode::UndefinedModelRef),
            data: None,
        };
        let (message, related) = render_expansion_frames(&diag);
        assert_eq!(message, "undefined model `foo`");
        assert!(related.is_none());
    }
}
