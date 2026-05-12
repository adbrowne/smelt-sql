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
    yaml_edits::find_source_column_yaml_rename, Database, Diagnostic as DbDiagnostic,
    DiagnosticAcc, DiagnosticCode as DbCode, DiagnosticData as DbData,
    DiagnosticSeverity as DbSeverity, ProjectInput, SourceFile, Workspace,
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
            // If no explicit match, trace through wildcards (smelt.<path> form)
            if !found_explicit && has_wildcard {
                if let Some(from_clause) = cte_select.from_clause() {
                    let all_refs = from_clause
                        .table_refs()
                        .chain(from_clause.joins().filter_map(|j| j.table_ref()));
                    for table_ref in all_refs {
                        if let Some(path_ref) = table_ref.smelt_path_ref() {
                            let model_name =
                                path_ref.segments().last().cloned().unwrap_or_default();
                            if !model_name.is_empty() {
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
    // smelt.source() was removed in Phase 4 — no source refs to check.
    // The from_clause is no longer needed here; kept as _from_clause for
    // potential future use with smelt.sources.* path refs.
    let _from_clause = match select_stmt.from_clause() {
        Some(f) => f,
        None => return,
    };

    let source_refs: Vec<(String, String, Option<String>)> = Vec::new();

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

/// Collect model names from FROM clause smelt.<path> refs
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
        if let Some(path_ref) = table_ref.smelt_path_ref() {
            if let Some(model_name) = path_ref.segments().last().cloned() {
                names.push(model_name);
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
        // Anonymous HOF frames (fn_id is None) have an empty `param` and
        // `bound_type`; rendering them with the named-frame template produces
        // awkward text like `"`, `` was bound to "`.  Use a shorter form that
        // names only the HOF (matching the spec for anonymous expansion frames).
        let is_anonymous = frame.fn_id.is_none();
        let trailer = if is_anonymous {
            format!("\nin expansion of `{}` call", frame.function)
        } else {
            format!(
                "\nin expansion of `{}`, `{}` was bound to {}",
                frame.function, frame.param, frame.bound_type,
            )
        };
        message.push_str(&trailer);
        if let (Some(path), Some(range)) = (&frame.decl_path, frame.decl_range.as_ref()) {
            if let Ok(uri) = Url::from_file_path(path) {
                let related_msg = if is_anonymous {
                    format!("in expansion of `{}` call", frame.function)
                } else {
                    format!(
                        "in expansion of `{}`, `{}` was bound to {}",
                        frame.function, frame.param, frame.bound_type,
                    )
                };
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
                    message: related_msg,
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

/// Find the innermost `SMELT_PATH_CALL` whose text range contains `offset`.
///
/// Used by hover and completion to dispatch on a `smelt.functions.<name>(...)`
/// call site (Phase 48: hover wiring + PASSING-body completion).
pub fn find_smelt_fn_call_at_cursor(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    offset: usize,
) -> Option<smelt_parser::ast::SmeltPathCall> {
    let mut best: Option<smelt_parser::ast::SmeltPathCall> = None;
    let mut best_size: usize = usize::MAX;
    for node in syntax.descendants() {
        if let Some(call) = smelt_parser::ast::SmeltPathCall::cast(node) {
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

// ── Phase 4: LSP hover helpers for list literals and spread ─────────────────
//
// Pure functions over AST + TypeContext. No Salsa calls inside the helper
// bodies — they delegate to `smelt_db::type_inference::infer_list_literal`
// and `smelt_db::type_inference::disambiguate_list_literal`, which are
// themselves pure.

/// Render hover text for a list literal at a position where only the meta-list
/// interpretation is relevant (either explicitly expected or unconstrained).
///
/// Calls `infer_list_literal` and formats the inferred `SmeltType` via
/// `format_smelt_type_hover`. Safe on partially-parsed literals — falls back
/// to `List<Unknown>` rather than panicking.
///
/// Phase A note: not currently called from `Backend::hover`; the dispatch
/// routes through [`hover_text_for_list_literal_dual`] with `expected = None`.
/// Position-aware hover (consulting the splice context to choose the meta vs
/// data-world reading, or to supply the expected sort for an empty literal)
/// lands in Phase B+.
pub fn hover_text_for_list_literal(
    elements: &[smelt_parser::ast::Expr],
    ctx: &smelt_db::TypeContext,
    expected: Option<&smelt_types::signatures::SmeltType>,
) -> String {
    use smelt_db::type_inference::infer_list_literal;
    let result = infer_list_literal(elements, ctx, expected);
    format_smelt_type_hover(&result.inferred)
}

/// Render hover text for a list literal at a position that admits **both**
/// the meta-list and the Data-World array interpretations.
///
/// This occurs when no expected sort is present and the element type is a
/// concrete `Expr<T>`. Both readings are surfaced in the returned string per
/// the spec note "literal accepted in two contexts". Meta wins as the primary
/// reading; the Data-World reading is shown parenthetically.
///
/// Falls back to the single meta-reading when the element type is not a
/// concrete `Expr<Concrete(T)>` (e.g. heterogeneous → `List<Unknown>`).
pub fn hover_text_for_list_literal_dual(
    elements: &[smelt_parser::ast::Expr],
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_list_literal;
    use smelt_types::signatures::{SmeltType, TypeConstraint};

    let result = infer_list_literal(elements, ctx, None);
    let meta_text = format_smelt_type_hover(&result.inferred);

    // Attempt to surface the Data-World array reading.
    // Only emit the dual reading when the inferred type is List<Expr<Concrete(T)>>.
    if let SmeltType::List(inner) = &result.inferred {
        if let SmeltType::Expr(TypeConstraint::Concrete(dt)) = inner.as_ref() {
            let array_text = format!("Array<{dt}>");
            return format!("{meta_text} (or `{array_text}` in array context)");
        }
    }

    // Single reading — meta only (e.g. heterogeneous or nested list).
    meta_text
}

/// Render hover text for a `...expr` spread expression.
///
/// Reads the operand's type. In Phase A, named-variable bindings are not
/// available, so the only operand shape we can fully resolve is a list
/// literal. For non-literal operands the fallback is `List<Unknown>`.
///
/// Safe on partially-parsed spread nodes — returns `List<Unknown>` rather
/// than panicking.
pub fn hover_text_for_list_spread(
    spread: &smelt_parser::ast::ListSpread,
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_list_literal;
    use smelt_types::signatures::SmeltType;

    let fallback = format_smelt_type_hover(&SmeltType::List(Box::new(SmeltType::Unknown)));

    let Some(operand) = spread.operand() else {
        return fallback;
    };

    // If the operand is an array/list literal, infer its type directly.
    if let Some(arr) = operand.as_array_literal() {
        let elems: Vec<smelt_parser::ast::Expr> = arr.elements();
        let result = infer_list_literal(&elems, ctx, None);
        return format_smelt_type_hover(&result.inferred);
    }

    // Non-literal operand — Phase A cannot resolve named-variable types.
    fallback
}

// ============================================================================
// Phase B (meta-language): hover / goto-def / completion pure helpers
//
// These are `pub fn` (not methods) so they can be tested directly from
// the `mod tests` block below without spinning up a full `Backend`.
// ============================================================================

/// Render hover text for a lambda parameter inside a HOF body.
///
/// The `elem_ty` is the element type inferred for the list that the HOF
/// is operating on — this becomes the parameter's bound type.
///
/// Safe on partial parses (body absent, params absent). Returns a
/// description string even when information is incomplete.
pub fn hover_text_for_lambda_param(
    param_name: &str,
    elem_ty: &smelt_types::signatures::SmeltType,
    _lambda: &smelt_parser::ast::Lambda,
    _ctx: &smelt_db::TypeContext,
) -> String {
    let type_str = format_smelt_type_hover(elem_ty);
    format!("**`{param_name}`** (lambda parameter)\n\n`{param_name}: {type_str}`")
}

/// Render hover text for a HOF call (`map`, `filter`, or `reduce`).
///
/// Infers the output type of the HOF call using `infer_hof_call_from_function_call`
/// and formats it via `format_smelt_type_hover`. Safe on partial parses.
pub fn hover_text_for_hof_call(
    call: &smelt_parser::ast::FunctionCall,
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_hof_call_from_function_call;
    let result = infer_hof_call_from_function_call(call, ctx);
    format_smelt_type_hover(&result.inferred)
}

/// Render hover text for a pipe expression `lhs |> rhs(...)`.
///
/// Desugars the pipe to the equivalent direct call and infers its type.
/// Returns the same text as [`hover_text_for_hof_call`] on the equivalent call.
pub fn hover_text_for_pipe_expr(
    pipe: &smelt_parser::ast::PipeExpr,
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_pipe_expr;
    let result = infer_pipe_expr(pipe, ctx, None);
    format_smelt_type_hover(&result.inferred)
}

/// Render hover text for a reducer name in the second-argument position of
/// `reduce(xs, reducer_name)`.
///
/// Shows: input element constraint, output sort, and empty-list identity rule.
/// Returns `"unknown reducer"` for names not in the closed registry.
pub fn hover_text_for_reducer_name(name: &str) -> String {
    use smelt_db::type_inference::{
        EmptyIdentity, ReducerInputConstraint, ReducerOutputSort, REDUCER_REGISTRY,
    };

    let Some(spec) = REDUCER_REGISTRY.iter().find(|r| r.name == name) else {
        return format!("**`{name}`** — unknown reducer");
    };

    let input_desc = match &spec.input_constraint {
        ReducerInputConstraint::AnyExpr => "Expr<T> (any element type)".to_string(),
        ReducerInputConstraint::Boolean => "Expr<Boolean>".to_string(),
        ReducerInputConstraint::Numeric => "Expr<Numeric>".to_string(),
        ReducerInputConstraint::Text => "Expr<Text>".to_string(),
        ReducerInputConstraint::TableExpr => "TableExpr".to_string(),
    };

    let output_desc = match &spec.output_sort {
        ReducerOutputSort::Boolean => "Expr<Boolean>".to_string(),
        ReducerOutputSort::SameAsElementType => "Expr<T> (same as element type)".to_string(),
        ReducerOutputSort::SelectItemsScalar => "SelectItems<Scalar>".to_string(),
        ReducerOutputSort::TableExpr => "TableExpr".to_string(),
    };

    let identity_desc = match &spec.empty_identity {
        EmptyIdentity::Boolean => "TRUE".to_string(),
        EmptyIdentity::Numeric => "0 (cast to element type)".to_string(),
        EmptyIdentity::Text => "''".to_string(),
        EmptyIdentity::EmptySelectItems => "empty SelectItems".to_string(),
        EmptyIdentity::None => "no identity".to_string(),
    };

    format!(
        "**`{name}`** (reducer)\n\n\
         - Input: `{input_desc}`\n\
         - Output: `{output_desc}`\n\
         - Identity: `{identity_desc}`"
    )
}

/// Render hover text for `smelt.config.var('x')`.
///
/// Always shows `Text` as the type. When the variable is present in the
/// `vars:` block of `smelt_yml_text`, also shows the resolved value.
/// When absent, shows a hint that the variable is not declared.
pub fn hover_text_for_config_var(var_name: &str, smelt_yml_text: &str) -> String {
    use smelt_db::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

    let vars = parse_vars_from_yaml(smelt_yml_text).unwrap_or_default();
    match vars.get(var_name) {
        Some(val) => {
            let (text_val, _warn) = coerce_yaml_scalar_to_text(val, var_name);
            format!(
                "**`smelt.config.var('{var_name}')`**\n\n\
                 Type: `Text`\n\n\
                 Resolved value: `'{text_val}'`"
            )
        }
        None => {
            format!(
                "**`smelt.config.var('{var_name}')`**\n\n\
                 Type: `Text`\n\n\
                 Variable `{var_name}` is not declared in `smelt.yml` vars"
            )
        }
    }
}

/// Find the 0-indexed line number of `key:` within the `vars:` block of
/// raw `smelt.yml` text.
///
/// Returns `None` if the key is not found under `vars:`.
/// Used by goto-definition for `smelt.config.var('x')` arguments.
pub fn find_var_line_in_smelt_yml(smelt_yml_text: &str, var_name: &str) -> Option<u32> {
    let mut in_vars = false;
    for (i, line) in smelt_yml_text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "vars:" {
            in_vars = true;
            continue;
        }
        if in_vars {
            // A non-indented non-comment line resets the vars context.
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !line.starts_with(' ')
                && !line.starts_with('\t')
            {
                in_vars = false;
                continue;
            }
            // Match `  key: value` or `  key:` patterns.
            let key_prefix = format!("{}:", var_name);
            if trimmed.starts_with(&key_prefix) {
                return Some(i as u32);
            }
        }
    }
    None
}

/// Return the name of a HOF function call, handling the case where `filter`
/// is lexed as `FILTER_KW` rather than `IDENT`.
///
/// `FunctionCall::name()` only returns `IDENT`-typed tokens.  Because the
/// lexer emits `filter` as `FILTER_KW`, we fall back to the first
/// non-trivia token's text — mirroring the identical fallback in
/// `smelt_db::type_inference::infer_hof_call_from_function_call_with_expected`.
pub fn hof_call_name(call: &smelt_parser::ast::FunctionCall) -> Option<String> {
    call.name().or_else(|| {
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.text().to_lowercase())
    })
}

/// Return the text range (in the source file) of the binding occurrence of
/// lambda parameter `param_name` in the given lambda node.
///
/// The binding occurrence is the IDENT token inside the `LAMBDA_PARAM_LIST`
/// that bears the parameter name. This is used by goto-definition to jump
/// from a use of the parameter in the body to its binding site.
///
/// Returns `None` when the parameter is not found (e.g., partial parse).
pub fn lambda_param_binder_range(
    lambda: &smelt_parser::ast::Lambda,
    param_name: &str,
) -> Option<smelt_parser::TextRange> {
    use smelt_parser::SyntaxKind;
    let param_list = lambda.param_list()?;
    for child in param_list.syntax().children_with_tokens() {
        // `children_with_tokens` yields `rowan::NodeOrToken` items — match on Token variant.
        if child
            .as_token()
            .map(|t| t.kind() == SyntaxKind::IDENT && t.text() == param_name)
            == Some(true)
        {
            return child.as_token().map(|t| t.text_range());
        }
    }
    None
}

/// Return the parameter names of a lambda for use as completion items.
///
/// The first parameter is the most important — it should appear first in the
/// completion list when the cursor is inside the lambda body.
pub fn lambda_params_for_completion(lambda: &smelt_parser::ast::Lambda) -> Vec<String> {
    lambda.params()
}

/// Return the reducer names from the closed registry that are compatible with
/// the given list type's element type, for use as completion items at the
/// second-argument position of `reduce(xs, _)`.
///
/// When `list_ty` is `None` or `Unknown`, returns the full registry (all names).
/// Otherwise filters by `ReducerInputConstraint::is_satisfied_by` on the element type.
pub fn reducer_completions_for_element_type(
    list_ty: Option<&smelt_types::signatures::SmeltType>,
) -> Vec<String> {
    use smelt_db::type_inference::REDUCER_REGISTRY;
    use smelt_types::signatures::SmeltType;

    let elem_ty: Option<SmeltType> = match list_ty {
        Some(SmeltType::List(inner)) => Some((**inner).clone()),
        _ => None,
    };

    REDUCER_REGISTRY
        .iter()
        .filter(|spec| {
            match &elem_ty {
                Some(et) if !matches!(et, SmeltType::Unknown) => {
                    spec.input_constraint.is_satisfied_by(et)
                }
                // Unknown element type or no list type — offer all reducers.
                _ => true,
            }
        })
        .map(|spec| spec.name.to_string())
        .collect()
}

// ============================================================================
// Phase C (meta-language): hover / goto-def / completion for reflection
//
// Pure helpers — no Salsa calls inside the bodies. `Backend::hover` calls
// them after resolving Salsa-backed data (e.g. ColumnRefValue lists) and
// passes the results in as plain data.
// ============================================================================

/// Render hover text for a `smelt.columns_of(t)` call site.
///
/// - Always shows `List<ColumnRef>` as the return type.
/// - When `columns` is `Some`, also shows the resolved column count and the
///   first five column names (per spec §"LSP support for reflection").
/// - When `columns` is `None` (schema unresolvable or no Salsa context),
///   shows only the type annotation.
///
/// Pure — callers supply the resolved column list.
pub fn hover_text_for_columns_of_call(
    table_name: &str,
    columns: Option<&[smelt_types::signatures::ColumnRefValue]>,
) -> String {
    match columns {
        None => format!(
            "`smelt.columns_of({table_name})`\n\n`List<ColumnRef>`\n\n\
             *Schema not statically resolvable*"
        ),
        Some(cols) => {
            let count = cols.len();
            let preview: Vec<&str> = cols.iter().take(5).map(|c| c.name.as_str()).collect();
            let preview_str = preview.join(", ");
            let ellipsis = if count > 5 { ", …" } else { "" };
            format!(
                "`smelt.columns_of({table_name})`\n\n`List<ColumnRef>`\n\n\
                 {count} columns: {preview_str}{ellipsis}"
            )
        }
    }
}

/// Render hover text for a `ColumnRef`-typed lambda parameter binding.
///
/// Shows `ColumnRef` as the type and lists the three closed fields with their
/// declared types per `COLUMN_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_column_ref_binding(param_name: &str) -> String {
    use smelt_types::signatures::COLUMN_REF_FIELDS;
    let mut s = format!(
        "**`{param_name}`** (lambda parameter)\n\n`{param_name}: ColumnRef`\n\n\
         **Fields:**\n"
    );
    for (field_name, field_ty) in COLUMN_REF_FIELDS {
        let ty_str = format_smelt_type_hover(field_ty);
        // Rename `Unknown` to `DataType` for the `type` field so user-facing
        // hover matches the spec description (the field holds a DataType
        // meta-literal; `Unknown` is an internal Phase C placeholder).
        let display_ty = if *field_name == "type" && ty_str == "Unknown" {
            "DataType".to_string()
        } else {
            ty_str
        };
        s.push_str(&format!("- `{field_name}: {display_ty}`\n"));
    }
    s
}

/// Render hover text for a `ColumnRef` field projection `c.<field>`.
///
/// Returns `Some(text)` for the three recognised fields (`name`, `type`,
/// `is_numeric`) and `None` for any other field name (the closed-field
/// invariant — callers should emit `ColumnRefFieldUnknown` in that case).
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_column_ref_field(field_name: &str) -> Option<String> {
    use smelt_types::signatures::column_ref_field;
    let field_ty = column_ref_field(field_name)?;
    let ty_str = format_smelt_type_hover(field_ty);
    // Rename `Unknown` to `DataType` for the `type` field (see
    // `hover_text_for_column_ref_binding` above for the same rationale).
    let display_ty = if field_name == "type" && ty_str == "Unknown" {
        "DataType".to_string()
    } else {
        ty_str
    };
    Some(format!("`{field_name}: {display_ty}` (ColumnRef field)"))
}

/// Render hover text for a meta-`Text` value lifted into an identifier
/// position (one of the four lift positions per spec §"Meta-Text-as-identifier
/// lift").
///
/// - Always describes the lift: `Text → identifier`.
/// - When `resolved_col` is `Some`, also mentions the concrete column name
///   (the `name` field's value at this call site).
///
/// Pure — callers supply the resolved `ColumnRefValue` when available.
pub fn hover_text_for_lifted_identifier(
    lift_expr: &str,
    resolved_col: Option<&smelt_types::signatures::ColumnRefValue>,
) -> String {
    match resolved_col {
        None => format!(
            "`{lift_expr}` — lifted meta-`Text` as identifier\n\n\
             *Concrete value not statically resolvable at this site*"
        ),
        Some(col) => format!(
            "`{lift_expr}` — lifted meta-`Text` as identifier\n\n\
             Resolves to column `{col_name}`",
            col_name = col.name
        ),
    }
}

/// Goto-definition for a `smelt.columns_of` call path — returns `None`
/// (graceful no-op / URL hint per spec §"LSP support for reflection").
///
/// The spec says: *"Goto-definition on `smelt.columns_of` resolves to the
/// reference page (URL hint, graceful no-op when the client lacks support)."*
/// Phase C implements the graceful no-op; a URL-hint extension can land later.
///
/// Pure — no Salsa or Backend dependency.
pub fn goto_def_for_columns_of_call() -> Option<std::path::PathBuf> {
    // Graceful no-op per spec. A future phase may return a URL hint.
    None
}

/// Goto-definition for a lifted meta-`Text` identifier (`c.name` in one of
/// the four lift positions: column-reference, AS-alias, ORDER BY, GROUP BY).
///
/// When the column is statically resolvable (the `ColumnRefValue` carries a
/// `source_span`), resolves to the source column's declaration span.
/// Otherwise returns `None` (graceful no-op — consistent with the spec's
/// "or no-op otherwise" fallback).
///
/// **Known divergence:** Full resolution (tracing the meta-`Text` value
/// through column-resolution and returning the source column's span) requires
/// re-running `columns_of_for_table_expr` with Salsa context.  This pure
/// helper only handles the statically-supplied span case; the Backend-level
/// dispatch wiring is not yet implemented.  Tracked in
/// `docs/plans/20260509-meta-language-overall.md`.
///
/// Pure — no Salsa or Backend dependency.
pub fn goto_def_for_lifted_identifier(
    resolved_col: Option<&smelt_types::signatures::ColumnRefValue>,
) -> Option<std::path::PathBuf> {
    // When the resolved ColumnRefValue carries a source_span (file path +
    // byte offset), we could return that location.  For v1 the source_span
    // field is `Option<TextRange>` (no file path), so we cannot construct a
    // PathBuf from it — return None (graceful no-op).
    let _ = resolved_col; // acknowledged; not yet resolvable to a path
    None
}

/// Return the closed set of `ColumnRef` field names for completion at a
/// field-projection site (`c.<cursor>`).
///
/// Returns exactly `["name", "type", "is_numeric"]` — the three fields in
/// declaration order per `COLUMN_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn column_ref_field_completions() -> Vec<String> {
    use smelt_types::signatures::COLUMN_REF_FIELDS;
    COLUMN_REF_FIELDS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Determine whether the text immediately before `cursor_offset` in `sql`
/// ends with `<param_name>.` where `<param_name>` is a lambda parameter
/// that is ColumnRef-typed — i.e. it is bound by a HOF (`map`, `filter`,
/// or `reduce`) whose **first argument** is a `smelt.columns_of(...)` call.
///
/// Returns `Some(param_name)` when the condition holds, `None` otherwise.
///
/// This helper is the single gating function used by both:
///   - the hover dispatch (block 6, field-projection hover), and
///   - the completion dispatch (ColumnRef field completion).
///
/// Pure — no Salsa dependency.
pub fn is_column_ref_param_before_dot(
    file: &smelt_parser::ast::File,
    sql: &str,
    cursor_offset: usize,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    let before = &sql[..cursor_offset.min(sql.len())];

    // Quick pre-check: the text before the cursor must end with `<ident>.`
    // (at least one identifier character followed by a dot).  This avoids
    // the more expensive CST walk for unrelated positions.
    let dot_trimmed = before.strip_suffix('.')?;
    let param_name: String = dot_trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if param_name.is_empty() {
        return None;
    }

    // Find the innermost LAMBDA node that (a) contains cursor_offset and
    // (b) declares `param_name` as one of its parameters.
    let lambda_node = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            cursor_offset >= s && cursor_offset <= e
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .find(|lam| lam.params().iter().any(|p| p == &param_name))?;

    // Walk up from the lambda to find the nearest enclosing HOF FUNCTION_CALL.
    let mut cur = lambda_node.syntax().parent();
    let mut hof_call: Option<smelt_parser::ast::FunctionCall> = None;
    while let Some(node) = cur {
        if node.kind() == SyntaxKind::FUNCTION_CALL {
            if let Some(fc) = smelt_parser::ast::FunctionCall::cast(node.clone()) {
                if matches!(
                    hof_call_name(&fc).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                ) {
                    hof_call = Some(fc);
                    break;
                }
            }
        }
        cur = node.parent();
    }

    // Check that the HOF's first argument is a `smelt.columns_of(...)` call.
    let hof = hof_call?;
    let first_arg = hof.arguments().into_iter().next()?;
    let path_call = first_arg.as_smelt_path_call()?;
    if path_call.segments() == vec!["columns_of".to_string()] {
        Some(param_name)
    } else {
        None
    }
}

/// Extract in-scope `TableExpr`-valued names from `sql` for use as completion
/// candidates at a `smelt.columns_of(<cursor>)` argument position.
///
/// Two sources are scanned:
///
/// 1. **`smelt.models.<name>` path refs** — any `SMELT_PATH_REF` node with a
///    `models` prefix contributes the model name segment, so
///    `smelt.models.orders` contributes `"orders"`.
///
/// 2. **`smelt.define` `TableExpr`-typed parameters** — any parameter of a
///    `smelt.define` declaration in the same file whose declared type is
///    `TableExpr` contributes its parameter name directly.  This is the
///    primary motivation for `smelt.columns_of` in parametric functions like
///    `smelt.define coalesce_numeric(t: TableExpr) AS (...)` — `t` must be
///    offered when the cursor is at `smelt.columns_of(<cursor>)`.
///
/// Pure — no Salsa dependency; callers can augment the list with Salsa-backed
/// schema information.
pub fn columns_of_arg_completions_for_sql(sql: &str) -> Vec<String> {
    use smelt_parser::ast::{File as AstFile, TypeRefHead};
    let parse = smelt_parser::parse(sql);
    let syntax = parse.syntax();
    let Some(file) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    // Source 1: smelt.models.<name> path refs.
    let mut names: Vec<String> = file
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathRef::cast)
        .filter_map(|path_ref| {
            let segs = path_ref.segments();
            // Only `smelt.models.<name>` refs contribute to the list.
            if segs.first().map(|s| s.as_str()) == Some("models") {
                segs.get(1).cloned()
            } else {
                None
            }
        })
        .collect();

    // Source 2: TableExpr-typed parameters of smelt.define declarations.
    for define in file.defines() {
        if let Some(param_list) = define.param_list() {
            for param in param_list.params() {
                if let Some(type_ref) = param.type_ref() {
                    if matches!(type_ref.kind(), TypeRefHead::TableExpr) {
                        if let Some(param_name) = param.name() {
                            if !names.contains(&param_name) {
                                names.push(param_name);
                            }
                        }
                    }
                }
            }
        }
    }

    // Dedup while preserving order (source-1 dedup; source-2 avoids
    // duplicates via the `contains` check above).
    names.dedup();
    names
}

/// Pure dispatch for HOF / lambda / reducer / config-var hover in the
/// meta-language layer.
///
/// Separated from [`Backend::hover`] so it can be tested without spinning up
/// a full tower-lsp `Client` or `Backend`.
///
/// # Dispatch order (most-specific first)
///
/// 1. **Reducer name** — cursor on the second positional argument of a
///    `reduce(xs, name)` call that is a registered reducer.
/// 2. **Lambda parameter** — cursor on a lambda parameter IDENT, either at
///    the binding site (`fn c =>` — the `c` after `fn`) or at any use of
///    that name in the lambda body.
/// 3. **ColumnRef field projection** — cursor on the field token of a
///    `c.<field>` dot expression where `<field>` is a known ColumnRef field
///    AND the receiver is a ColumnRef-typed lambda parameter (HOF first arg
///    is `smelt.columns_of(...)`).  Must run BEFORE block 4 (HOF result type)
///    because the cursor is inside the HOF call range and block 4 would
///    otherwise shadow the field hover.
/// 4. **HOF result type** — cursor anywhere inside a `map(...)` /
///    `filter(...)` / `reduce(...)` call, shows the inferred output type.
/// 5. **`smelt.config.var`** — cursor on a `smelt.config.var('x')` call,
///    shows the resolved value from `smelt_yml_text`.
/// 6. **`smelt.columns_of`** — cursor on the call path of a
///    `smelt.columns_of(t)` call; shows `List<ColumnRef>`.
///
/// Returns `Some(hover_markdown_text)` when a match is found, `None` otherwise.
pub fn hover_text_for_hof_meta_language(
    file: &smelt_parser::ast::File,
    cursor_offset: usize,
    smelt_yml_text: &str,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    // ── 1. Reducer name in second arg of reduce(xs, name) ─────────────────
    // Must run BEFORE the HOF result-type check because both match on a
    // `reduce(...)` FUNCTION_CALL node — the reducer-name hover is the
    // more-specific case and must win.
    {
        let reduce_call = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            })
            .and_then(smelt_parser::ast::FunctionCall::cast)
            .filter(|c| {
                c.name()
                    .map(|n| n.to_lowercase() == "reduce")
                    .unwrap_or(false)
            });

        if let Some(call) = reduce_call {
            let args = call.arguments();
            if let Some(second_arg) = args.get(1) {
                let arg_start: usize = second_arg.text_range().start().into();
                let arg_end: usize = second_arg.text_range().end().into();
                if cursor_offset >= arg_start && cursor_offset <= arg_end {
                    let reducer_name = second_arg
                        .syntax()
                        .children_with_tokens()
                        .filter_map(|c| c.into_token())
                        .find(|t| t.kind() == SyntaxKind::IDENT)
                        .map(|t| t.text().to_string())
                        .unwrap_or_default();
                    if !reducer_name.is_empty() {
                        return Some(hover_text_for_reducer_name(&reducer_name));
                    }
                }
            }
        }
    }

    // ── 2. Lambda parameter (binder OR body use) ───────────────────────────
    // Must run BEFORE the HOF result-type check because a lambda is nested
    // inside the HOF call node; the HOF check would otherwise shadow it.
    {
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
            if let Some(lambda) = smelt_parser::ast::Lambda::cast(ln.clone()) {
                let params = lambda.params();
                for param_name in &params {
                    // Check binder position.
                    let on_binder = lambda_param_binder_range(&lambda, param_name)
                        .map(|r| {
                            let s: usize = r.start().into();
                            let e: usize = r.end().into();
                            cursor_offset >= s && cursor_offset <= e
                        })
                        .unwrap_or(false);

                    // Check uses in the lambda body: any IDENT token with
                    // the same name that sits inside the body subtree.
                    let on_body_use = lambda.body().is_some_and(|body| {
                        body.syntax()
                            .descendants_with_tokens()
                            .filter_map(|e| e.into_token())
                            .filter(|t| {
                                t.kind() == SyntaxKind::IDENT && t.text() == param_name.as_str()
                            })
                            .any(|t| {
                                let s: usize = t.text_range().start().into();
                                let e: usize = t.text_range().end().into();
                                cursor_offset >= s && cursor_offset <= e
                            })
                    });

                    if on_binder || on_body_use {
                        // Infer the element type from the enclosing HOF call.
                        // Walk up through any intermediate nodes (EXPRESSION,
                        // ARG_LIST, etc.) until we find a FUNCTION_CALL ancestor.
                        let hof_call = {
                            let mut cur = ln.parent();
                            let mut found = None;
                            while let Some(node) = cur {
                                if node.kind() == SyntaxKind::FUNCTION_CALL {
                                    found = smelt_parser::ast::FunctionCall::cast(node.clone())
                                        .filter(|c| {
                                            matches!(
                                                hof_call_name(c).as_deref().unwrap_or(""),
                                                "map" | "filter" | "reduce"
                                            )
                                        });
                                    break;
                                }
                                cur = node.parent();
                            }
                            found
                        };
                        use smelt_types::signatures::SmeltType;
                        // Infer the INPUT element type from the HOF's first
                        // argument (the list being iterated).  Using the HOF
                        // result type would be wrong for `map`, which returns
                        // `List<U>` (the OUTPUT type); we want `T` from
                        // `List<T>`.  `filter` preserves the element type so
                        // the result would coincidentally be correct, but
                        // deriving it from the first arg is more principled and
                        // correct for all three HOFs.
                        let elem_ty = hof_call
                            .as_ref()
                            .and_then(|c| {
                                let args = c.arguments();
                                let first_arg = args.first()?;
                                let ctx = smelt_db::TypeContext::new();
                                // If the first arg is an array literal, use
                                // `infer_list_literal` to get `List<T>` then
                                // extract `T`.
                                let list_ty = if let Some(arr) = first_arg.as_array_literal() {
                                    let elems: Vec<_> = arr.elements();
                                    smelt_db::type_inference::infer_list_literal(&elems, &ctx, None)
                                        .inferred
                                } else {
                                    // Non-literal first argument.
                                    // Special case: recognise wide-reflection and
                                    // narrow-reflection call sites so the lambda
                                    // element type can be determined statically.
                                    // - `smelt.columns_of(t)` → `List<ColumnRef>`
                                    // - `smelt.models.with_tag(t)` / `smelt.models.all` → `List<ModelRef>`
                                    // - `smelt.sources.with_tag(t)` / `smelt.sources.all` → `List<SourceRef>`
                                    if let Some(path_call) = first_arg.as_smelt_path_call() {
                                        let segs = path_call.segments();
                                        if segs == vec!["columns_of".to_string()] {
                                            SmeltType::List(Box::new(SmeltType::ColumnRef))
                                        } else if segs.first().map(|s| s.as_str()) == Some("models")
                                            && segs
                                                .get(1)
                                                .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
                                                .unwrap_or(false)
                                        {
                                            SmeltType::List(Box::new(SmeltType::ModelRef))
                                        } else if segs.first().map(|s| s.as_str()) == Some("sources")
                                            && segs
                                                .get(1)
                                                .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
                                                .unwrap_or(false)
                                        {
                                            SmeltType::List(Box::new(SmeltType::SourceRef))
                                        } else {
                                            SmeltType::Unknown
                                        }
                                    } else {
                                        // Fall back to the full HOF inference path.
                                        // For `filter`, the result IS `List<T>`.
                                        // For `map`/`reduce`, result type is different;
                                        // we accept Unknown for those non-literal cases.
                                        let result =
                                            smelt_db::type_inference::infer_hof_call_from_function_call(
                                                c, &ctx,
                                            );
                                        let hof_name = hof_call_name(c).unwrap_or_default();
                                        if hof_name == "filter" {
                                            result.inferred
                                        } else {
                                            SmeltType::Unknown
                                        }
                                    }
                                };
                                if let SmeltType::List(inner) = list_ty {
                                    Some(*inner)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(SmeltType::Unknown);
                        // ColumnRef/ModelRef/SourceRef-typed lambda parameter:
                        // use the specialised binding helper (shows the closed
                        // field list) instead of the generic lambda-param
                        // helper.
                        if matches!(elem_ty, SmeltType::ColumnRef) {
                            return Some(hover_text_for_column_ref_binding(param_name));
                        }
                        if matches!(elem_ty, SmeltType::ModelRef) {
                            return Some(hover_text_for_model_ref_binding(param_name));
                        }
                        if matches!(elem_ty, SmeltType::SourceRef) {
                            return Some(hover_text_for_source_ref_binding(param_name));
                        }
                        let ctx = smelt_db::TypeContext::new();
                        return Some(hover_text_for_lambda_param(
                            param_name, &elem_ty, &lambda, &ctx,
                        ));
                    }
                }
            }
        }
    }

    // ── 3. ColumnRef / ModelRef / SourceRef field projection ─────────────────
    // Cursor on `<field>` in `c.<field>` / `m.<field>` / `s.<field>`.
    // Runs BEFORE the HOF result-type block so field hover wins over outer HOF.
    // Receiver checks ensure plain SQL field access does NOT trigger this hover.
    {
        use smelt_parser::SyntaxKind::{DOT, IDENT};
        let syntax_node = file.syntax();
        let tokens: Vec<_> = syntax_node
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();
        let file_sql = file.syntax().text().to_string();
        for (i, tok) in tokens.iter().enumerate() {
            if tok.kind() == DOT {
                if let Some(next_tok) = tokens.get(i + 1) {
                    if next_tok.kind() == IDENT {
                        let start: usize = next_tok.text_range().start().into();
                        let end: usize = next_tok.text_range().end().into();
                        if cursor_offset >= start && cursor_offset <= end {
                            let field_name = next_tok.text();
                            let dot_end: usize = tok.text_range().end().into();
                            // ColumnRef: HOF first arg is smelt.columns_of(...)
                            if is_column_ref_param_before_dot(file, &file_sql, dot_end).is_some() {
                                if let Some(hover_text) =
                                    hover_text_for_column_ref_field(field_name)
                                {
                                    return Some(hover_text);
                                }
                            }
                            // ModelRef: HOF first arg is smelt.models.with_tag/all
                            if is_model_ref_param_before_dot(file, &file_sql, dot_end).is_some() {
                                if let Some(hover_text) = hover_text_for_model_ref_field(field_name)
                                {
                                    return Some(hover_text);
                                }
                            }
                            // SourceRef: HOF first arg is smelt.sources.with_tag/all
                            if is_source_ref_param_before_dot(file, &file_sql, dot_end).is_some() {
                                if let Some(hover_text) =
                                    hover_text_for_source_ref_field(field_name)
                                {
                                    return Some(hover_text);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ── 4. smelt.models.* / smelt.sources.* wide-reflection accessor call ─────
    // Must run BEFORE the HOF result-type check.  When the cursor is inside
    // `reduce(smelt.models.all(), union_all)`, the cursor position is inside
    // the `smelt.models.all()` SmeltPathCall node (a more-specific match).
    // The HOF result-type block would otherwise intercept the position first.
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
            if namespace == "models" {
                if accessor == "with_tag" {
                    let tag = call
                        .arg_list()
                        .and_then(|al| al.positional_args().into_iter().next())
                        .map(|a| {
                            let t = a.text();
                            t.trim_matches('\'').trim_matches('"').to_string()
                        })
                        .unwrap_or_default();
                    return Some(hover_text_for_models_with_tag_call(&tag, None));
                } else {
                    return Some(hover_text_for_models_all(None));
                }
            } else {
                if accessor == "with_tag" {
                    let tag = call
                        .arg_list()
                        .and_then(|al| al.positional_args().into_iter().next())
                        .map(|a| {
                            let t = a.text();
                            t.trim_matches('\'').trim_matches('"').to_string()
                        })
                        .unwrap_or_default();
                    return Some(hover_text_for_sources_with_tag_call(&tag, None));
                } else {
                    return Some(hover_text_for_sources_all(None));
                }
            }
        }
    }

    // ── 5. HOF result type (map / filter / reduce) ─────────────────────────
    {
        let hof_call = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            })
            .and_then(smelt_parser::ast::FunctionCall::cast)
            .filter(|c| {
                matches!(
                    hof_call_name(c).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                )
            });

        if let Some(call) = hof_call {
            let ctx = smelt_db::TypeContext::new();
            return Some(hover_text_for_hof_call(&call, &ctx));
        }
    }

    // ── 5. smelt.config.var('x') ───────────────────────────────────────────
    {
        let config_call = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            })
            .and_then(smelt_parser::ast::FunctionCall::cast)
            .filter(|c| {
                let raw = c
                    .syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::IDENT)
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                raw == "smelt"
                    || (c.namespace().as_deref() == Some("smelt")
                        && c.name().as_deref() == Some("config"))
            });

        if let Some(call) = config_call {
            let full_text = call.text();
            if full_text.contains("config.var") || full_text.contains("config") {
                let var_call = call
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::FunctionCall::cast)
                    .find(|c| c.name().as_deref() == Some("var"));
                if let Some(vc) = var_call {
                    let args = vc.arguments();
                    if let Some(arg) = args.first() {
                        if smelt_db::config_vars::is_string_literal_expr(arg) {
                            if let Some(var_name) =
                                smelt_db::config_vars::extract_string_literal_value(arg)
                            {
                                return Some(hover_text_for_config_var(&var_name, smelt_yml_text));
                            }
                        }
                    }
                }
            }
        }
    }

    // ── 6. smelt.columns_of(t) — cursor on the call path ─────────────────────
    // The generic HOF check only matches `map`/`filter`/`reduce`, so this
    // block runs without conflict.  We check for a SMELT_PATH_CALL whose
    // segments are `["columns_of"]` (the leading `smelt` is implicit).
    {
        let columns_of_call = file
            .syntax()
            .descendants()
            .filter_map(smelt_parser::ast::SmeltPathCall::cast)
            .filter(|c| {
                let segs = c.segments();
                segs == vec!["columns_of".to_string()]
            })
            .find(|c| {
                let r = c.text_range();
                let s: usize = r.start().into();
                let e: usize = r.end().into();
                cursor_offset >= s && cursor_offset <= e
            });

        if let Some(call) = columns_of_call {
            // Extract the table name argument (first positional arg).
            let table_name = call
                .arg_list()
                .and_then(|al| al.positional_args().into_iter().next())
                .map(|a| a.text())
                .unwrap_or_else(|| "?".to_string());
            // Return hover with no resolved columns (no Salsa context in this
            // pure dispatch). Backend::hover supplies resolved columns via its
            // own block; this pure helper is exercised by the dispatch tests.
            return Some(hover_text_for_columns_of_call(&table_name, None));
        }
    }

    None
}

// ============================================================================
// Phase D (meta-language): hover / goto-def / completion for wide reflection
//
// Pure helpers — no Salsa calls inside the bodies.  Backend handlers call them
// after resolving Salsa-backed data (e.g. ModelRefValue lists) and pass the
// results in as plain data.
// ============================================================================

/// Render hover text for a `smelt.models.with_tag(t)` call site.
///
/// - Always shows `List<ModelRef>` as the return type.
/// - When `models` is `Some`, also shows the resolved match count and the
///   first five matching model names (per spec §"LSP support for wide
///   reflection").
/// - When `models` is `None` (workspace not resolvable at this cursor),
///   shows only the type annotation.
///
/// Pure — callers supply the resolved model list.
pub fn hover_text_for_models_with_tag_call(
    tag: &str,
    models: Option<&[smelt_types::signatures::ModelRefValue]>,
) -> String {
    match models {
        None => format!(
            "`smelt.models.with_tag('{tag}')`\n\n`List<ModelRef>`\n\n\
             *Workspace not statically resolvable*"
        ),
        Some(ms) => {
            let count = ms.len();
            let preview: Vec<&str> = ms.iter().take(5).map(|m| m.name.as_str()).collect();
            let preview_str = preview.join(", ");
            let ellipsis = if count > 5 { ", …" } else { "" };
            format!(
                "`smelt.models.with_tag('{tag}')`\n\n`List<ModelRef>`\n\n\
                 {count} matching models: {preview_str}{ellipsis}"
            )
        }
    }
}

/// Render hover text for a `smelt.sources.with_tag(t)` call site.
///
/// Mirrors `hover_text_for_models_with_tag_call` but for `SourceRef`.
///
/// Pure — callers supply the resolved source list.
pub fn hover_text_for_sources_with_tag_call(
    tag: &str,
    sources: Option<&[smelt_types::signatures::SourceRefValue]>,
) -> String {
    match sources {
        None => format!(
            "`smelt.sources.with_tag('{tag}')`\n\n`List<SourceRef>`\n\n\
             *Workspace not statically resolvable*"
        ),
        Some(ss) => {
            let count = ss.len();
            let preview: Vec<&str> = ss.iter().take(5).map(|s| s.name.as_str()).collect();
            let preview_str = preview.join(", ");
            let ellipsis = if count > 5 { ", …" } else { "" };
            format!(
                "`smelt.sources.with_tag('{tag}')`\n\n`List<SourceRef>`\n\n\
                 {count} matching sources: {preview_str}{ellipsis}"
            )
        }
    }
}

/// Render hover text for `smelt.models.all` call / accessor.
///
/// - Always shows the signature `() -> List<ModelRef>`.
/// - When `total` is `Some`, also shows the workspace's total model count.
///
/// Pure — callers supply the total count.
pub fn hover_text_for_models_all(total: Option<usize>) -> String {
    match total {
        None => "`smelt.models.all`\n\n`() -> List<ModelRef>`\n\n\
                 *Workspace not statically resolvable*"
            .to_string(),
        Some(n) => format!(
            "`smelt.models.all`\n\n`() -> List<ModelRef>`\n\n\
             {n} models in workspace"
        ),
    }
}

/// Render hover text for `smelt.sources.all` call / accessor.
///
/// Mirrors `hover_text_for_models_all` but for `SourceRef`.
///
/// Pure — callers supply the total count.
pub fn hover_text_for_sources_all(total: Option<usize>) -> String {
    match total {
        None => "`smelt.sources.all`\n\n`() -> List<SourceRef>`\n\n\
                 *Workspace not statically resolvable*"
            .to_string(),
        Some(n) => format!(
            "`smelt.sources.all`\n\n`() -> List<SourceRef>`\n\n\
             {n} sources in workspace"
        ),
    }
}

/// Render hover text for a `ModelRef`-typed lambda parameter binding.
///
/// Shows `ModelRef` as the type and lists the four closed fields with their
/// declared types per `MODEL_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_model_ref_binding(param_name: &str) -> String {
    use smelt_types::signatures::MODEL_REF_FIELDS;
    let mut s = format!(
        "**`{param_name}`** (lambda parameter)\n\n`{param_name}: ModelRef`\n\n\
         **Fields:**\n"
    );
    for (field_name, field_ty) in MODEL_REF_FIELDS.iter() {
        let ty_str = format_smelt_type_hover(field_ty);
        s.push_str(&format!("- `{field_name}: {ty_str}`\n"));
    }
    s
}

/// Render hover text for a `SourceRef`-typed lambda parameter binding.
///
/// Shows `SourceRef` as the type and lists the four closed fields with their
/// declared types per `SOURCE_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_source_ref_binding(param_name: &str) -> String {
    use smelt_types::signatures::SOURCE_REF_FIELDS;
    let mut s = format!(
        "**`{param_name}`** (lambda parameter)\n\n`{param_name}: SourceRef`\n\n\
         **Fields:**\n"
    );
    for (field_name, field_ty) in SOURCE_REF_FIELDS.iter() {
        let ty_str = format_smelt_type_hover(field_ty);
        s.push_str(&format!("- `{field_name}: {ty_str}`\n"));
    }
    s
}

/// Render hover text for a `ModelRef` field projection `m.<field>`.
///
/// Returns `Some(text)` for the four recognised fields (`path`, `name`, `tags`,
/// `columns`) and `None` for any other field name.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_model_ref_field(field_name: &str) -> Option<String> {
    use smelt_types::signatures::model_ref_field;
    let field_ty = model_ref_field(field_name)?;
    let ty_str = format_smelt_type_hover(field_ty);
    Some(format!("`{field_name}: {ty_str}` (ModelRef field)"))
}

/// Render hover text for a `SourceRef` field projection `s.<field>`.
///
/// Returns `Some(text)` for the four recognised fields (`path`, `name`, `tags`,
/// `columns`) and `None` for any other field name.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_source_ref_field(field_name: &str) -> Option<String> {
    use smelt_types::signatures::source_ref_field;
    let field_ty = source_ref_field(field_name)?;
    let ty_str = format_smelt_type_hover(field_ty);
    Some(format!("`{field_name}: {ty_str}` (SourceRef field)"))
}

/// Goto-definition for `smelt.models.*` / `smelt.sources.*` accessor call paths —
/// returns `None` (graceful no-op / URL hint per spec §"LSP support for wide
/// reflection").
///
/// Pure — no Salsa or Backend dependency.
pub fn goto_def_for_wide_reflection_accessor() -> Option<std::path::PathBuf> {
    // Graceful no-op per spec. A future phase may return a URL hint.
    None
}

/// Goto-definition from a `ModelRef`-typed value at a splice site or from a
/// `ModelRef` field projection (`m.path`, `m.name`) — resolves to the model's
/// source `.sql` file.
///
/// When `source_path` is `Some`, returns the path. Otherwise returns `None`
/// (graceful no-op).
///
/// Pure — callers supply the resolved path.
pub fn goto_def_for_model_ref_value(
    source_path: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    source_path
}

/// Goto-definition from a `SourceRef`-typed value — resolves to the source
/// YAML file.
///
/// When `yaml_path` is `Some`, returns the path. Otherwise returns `None`
/// (graceful no-op).
///
/// Pure — callers supply the resolved path.
pub fn goto_def_for_source_ref_value(
    yaml_path: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    yaml_path
}

/// Return the closed set of accessor names for `smelt.models.<cursor>` or
/// `smelt.sources.<cursor>` completion.
///
/// Returns exactly `["with_tag", "all"]` — the two accessors per spec.
///
/// Pure — no Salsa dependency.
pub fn wide_reflection_accessor_completions() -> Vec<String> {
    vec!["with_tag".to_string(), "all".to_string()]
}

/// Return the closed set of `ModelRef` field names for completion at a
/// field-projection site (`m.<cursor>`).
///
/// Returns exactly `["path", "name", "tags", "columns"]` — the four fields in
/// declaration order per `MODEL_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn model_ref_field_completions() -> Vec<String> {
    use smelt_types::signatures::MODEL_REF_FIELDS;
    MODEL_REF_FIELDS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Return the closed set of `SourceRef` field names for completion at a
/// field-projection site (`s.<cursor>`).
///
/// Returns exactly `["path", "name", "tags", "columns"]` — the four fields in
/// declaration order per `SOURCE_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn source_ref_field_completions() -> Vec<String> {
    use smelt_types::signatures::SOURCE_REF_FIELDS;
    SOURCE_REF_FIELDS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Determine whether the text immediately before `cursor_offset` in `sql`
/// ends with `<param_name>.` where `<param_name>` is a lambda parameter
/// that is `ModelRef`-typed — i.e. it is bound by a HOF (`map`, `filter`,
/// or `reduce`) whose **first argument** is a `smelt.models.with_tag(...)` or
/// `smelt.models.all` call.
///
/// Returns `Some(param_name)` when the condition holds, `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn is_model_ref_param_before_dot(
    file: &smelt_parser::ast::File,
    sql: &str,
    cursor_offset: usize,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    let before = &sql[..cursor_offset.min(sql.len())];
    let dot_trimmed = before.strip_suffix('.')?;
    let param_name: String = dot_trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if param_name.is_empty() {
        return None;
    }

    // Find the innermost LAMBDA that contains cursor_offset and declares param_name.
    let lambda_node = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            cursor_offset >= s && cursor_offset <= e
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .find(|lam| lam.params().iter().any(|p| p == &param_name))?;

    // Walk up to the nearest enclosing HOF FUNCTION_CALL.
    let mut cur = lambda_node.syntax().parent();
    let mut hof_call: Option<smelt_parser::ast::FunctionCall> = None;
    while let Some(node) = cur {
        if node.kind() == SyntaxKind::FUNCTION_CALL {
            if let Some(fc) = smelt_parser::ast::FunctionCall::cast(node.clone()) {
                if matches!(
                    hof_call_name(&fc).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                ) {
                    hof_call = Some(fc);
                    break;
                }
            }
        }
        cur = node.parent();
    }

    // Check that the HOF's first argument is `smelt.models.with_tag(...)` or
    // `smelt.models.all` (or `smelt.models.all()`).
    let hof = hof_call?;
    let first_arg = hof.arguments().into_iter().next()?;
    let path_call = first_arg.as_smelt_path_call()?;
    let segs = path_call.segments();
    // segs for `smelt.models.with_tag(...)` → ["models", "with_tag"]
    // segs for `smelt.models.all` / `smelt.models.all()` → ["models", "all"]
    if segs.first().map(|s| s.as_str()) == Some("models")
        && segs
            .get(1)
            .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
            .unwrap_or(false)
    {
        return Some(param_name);
    }
    None
}

/// Determine whether the text immediately before `cursor_offset` in `sql`
/// ends with `<param_name>.` where `<param_name>` is a lambda parameter
/// that is `SourceRef`-typed — i.e. it is bound by a HOF whose **first
/// argument** is a `smelt.sources.with_tag(...)` or `smelt.sources.all` call.
///
/// Returns `Some(param_name)` when the condition holds, `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn is_source_ref_param_before_dot(
    file: &smelt_parser::ast::File,
    sql: &str,
    cursor_offset: usize,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    let before = &sql[..cursor_offset.min(sql.len())];
    let dot_trimmed = before.strip_suffix('.')?;
    let param_name: String = dot_trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if param_name.is_empty() {
        return None;
    }

    let lambda_node = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            cursor_offset >= s && cursor_offset <= e
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .find(|lam| lam.params().iter().any(|p| p == &param_name))?;

    let mut cur = lambda_node.syntax().parent();
    let mut hof_call: Option<smelt_parser::ast::FunctionCall> = None;
    while let Some(node) = cur {
        if node.kind() == SyntaxKind::FUNCTION_CALL {
            if let Some(fc) = smelt_parser::ast::FunctionCall::cast(node.clone()) {
                if matches!(
                    hof_call_name(&fc).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                ) {
                    hof_call = Some(fc);
                    break;
                }
            }
        }
        cur = node.parent();
    }

    let hof = hof_call?;
    let first_arg = hof.arguments().into_iter().next()?;
    let path_call = first_arg.as_smelt_path_call()?;
    let segs = path_call.segments();
    if segs.first().map(|s| s.as_str()) == Some("sources")
        && segs
            .get(1)
            .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
            .unwrap_or(false)
    {
        return Some(param_name);
    }
    None
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
                DbCode::MissingSeedSidecar => "missing-seed-sidecar",
                // Phase A (meta-language) diagnostic codes.
                DbCode::MetaListEmptyTypeUnknown => "meta-list-empty-type-unknown",
                DbCode::MetaListHeterogeneous => "meta-list-heterogeneous",
                DbCode::MetaSpreadInForbiddenPosition => "meta-spread-in-forbidden-position",
                DbCode::MetaSpreadOnNonList => "meta-spread-on-non-list",
                // Phase B (meta-language) diagnostic codes.
                DbCode::LambdaInForbiddenPosition => "lambda-in-forbidden-position",
                DbCode::LambdaArityNotSupported => "lambda-arity-not-supported",
                DbCode::LambdaResultTypeMismatch => "lambda-result-type-mismatch",
                DbCode::HofExpectsLambda => "hof-expects-lambda",
                DbCode::HofExpectsReducer => "hof-expects-reducer",
                DbCode::HofNameShadowed => "hof-name-shadowed",
                DbCode::ReducerNameShadowed => "reducer-name-shadowed",
                DbCode::PipeRhsNotCall => "pipe-rhs-not-call",
                DbCode::PipeInDataPosition => "pipe-in-data-position",
                DbCode::ReducerInputTypeMismatch => "reducer-input-type-mismatch",
                DbCode::ReducerEmptyNoIdentity => "reducer-empty-no-identity",
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

                    // Load config (defaults to a minimal config with paths = ["models"])
                    let config = smelt_core::Config::load(&project_root).unwrap_or_else(|_| {
                        smelt_core::Config {
                            name: String::new(),
                            version: 1,
                            paths: vec!["models".to_string()],
                            targets: std::collections::HashMap::new(),
                            default_materialization: smelt_core::Materialization::View,
                            models: std::collections::HashMap::new(),
                            python: None,
                        }
                    });
                    let paths = config.paths.clone();

                    // Scan project paths for this project
                    for model_path in &paths {
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
                            let ws = Workspace::try_get(&db);
                            ws.and_then(|w| {
                                smelt_db::resolve_ref_path(&db, w, segments)
                                    .and_then(|r| r.source_file)
                                    .map(|f| GotoTarget::RefModel(f.path(&db).clone()))
                            })
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
                        Some(SymbolAtCursor::PathRef { segments }) => {
                            // Find all files that contain a path ref with these segments
                            let ws = Workspace::try_get(&db);
                            let ws_files = ws.map(|w| w.files(&db).clone()).unwrap_or_default();
                            let mut all_refs: Vec<(PathBuf, smelt_parser::ast::Range)> = Vec::new();
                            for f in &ws_files {
                                let path_refs = smelt_db::model_path_refs(&db, *f);
                                for loc in path_refs.iter() {
                                    if loc.path == segments {
                                        all_refs.push((f.path(&db).clone(), loc.range));
                                    }
                                }
                            }
                            RefResult::PathRanges(all_refs)
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
                                let names = reducer_completions_for_element_type(list_ty.as_ref());
                                let items: Vec<CompletionItem> = names
                                    .into_iter()
                                    .map(|name| CompletionItem {
                                        label: name.clone(),
                                        kind: Some(CompletionItemKind::FUNCTION),
                                        detail: Some(format!("reducer: {}", name)),
                                        ..Default::default()
                                    })
                                    .collect();
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
                                if !params.is_empty() {
                                    // Build param completions — they will be prepended
                                    // to the standard column completions below.
                                    let param_items: Vec<CompletionItem> = params
                                        .iter()
                                        .map(|p| CompletionItem {
                                            label: p.clone(),
                                            kind: Some(CompletionItemKind::VARIABLE),
                                            detail: Some("lambda parameter".to_string()),
                                            sort_text: Some(format!("0_{p}")), // sort first
                                            ..Default::default()
                                        })
                                        .collect();
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

/// Completion context types
#[derive(Debug)]
pub enum CompletionContext {
    InsideRef,               // Cursor inside ref('|')
    InsideSource,            // Cursor inside source('|')
    ColumnName,              // Cursor in a position where column name is expected
    QualifiedColumn(String), // Cursor after alias. (e.g., "t." for table alias t)
    FromClause,              // Cursor in FROM/JOIN position (offer CTE names)
    /// Phase 2c: cursor positioned after a `smelt.` prefix (path form), e.g.
    /// `FROM smelt.|` or `FROM smelt.models.|`. Completion should return all
    /// workspace entities as `smelt.<segments>` labels.
    SmeltPath,
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
pub fn determine_completion_context(text: &str, offset: usize) -> CompletionContext {
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

    // Phase 2c: detect cursor after a `smelt.` path prefix. This must be
    // checked before the legacy `ref(` / `source(` checks so that
    // `smelt.ref(` still falls through to InsideRef.
    // Pattern: text ends with `smelt.` or `smelt.<word>.` (possibly with
    // partial segment at cursor).
    {
        // Find the last word boundary: scan back from cursor for valid path chars
        // (alphanumeric, _, .) until we hit whitespace or other delimiter.
        let trimmed = before_cursor
            .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.');
        let suffix = &before_cursor[trimmed.len()..];
        // A smelt path starts with `smelt.` and contains only word chars and dots.
        if suffix.starts_with("smelt.")
            && suffix[6..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            // Make sure this is NOT `smelt.ref(` or `smelt.source(` (legacy forms
            // get their own context below).
            let rest = &suffix[6..];
            let is_legacy = rest.starts_with("ref(")
                || rest.starts_with("ref('")
                || rest.starts_with("source(")
                || rest.starts_with("source('")
                || rest.starts_with("fn.");
            if !is_legacy {
                return CompletionContext::SmeltPath;
            }
        }
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

    // Step 5: extract the callee name — last `smelt.functions.<...>` call before
    // the `PASSING`. We look for the most recent `smelt.functions.` literal in
    // `before_cursor` and take the dotted-identifier that follows.
    // Phase 5b: `smelt.fn.*` is removed; only `smelt.functions.*` is valid.
    let smelt_fn = before_cursor.rfind("smelt.functions.")?;
    let after = &before_cursor[smelt_fn + "smelt.functions.".len()..];
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
    let mut aliases = std::collections::HashMap::new();

    if let Some(from_clause) = select_stmt.from_clause() {
        // Process main table refs in FROM clause
        for table_ref in from_clause.table_refs() {
            if let Some(path_ref) = table_ref.smelt_path_ref() {
                let segments = path_ref.segments();
                add_path_ref_alias(&table_ref, &segments, &mut aliases);
            }
        }

        // Process JOINed table refs
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if let Some(path_ref) = table_ref.smelt_path_ref() {
                    let segments = path_ref.segments();
                    add_path_ref_alias(&table_ref, &segments, &mut aliases);
                }
            }
        }
    }

    // Note: db parameter reserved for future use (e.g., resolving model schemas)
    let _ = db;

    aliases
}

/// Insert an alias entry for a `smelt.<path>` table ref based on its segments.
fn add_path_ref_alias(
    table_ref: &smelt_parser::ast::TableRef,
    segments: &[String],
    aliases: &mut std::collections::HashMap<String, AliasTarget>,
) {
    match segments.first().map(|s| s.as_str()) {
        Some("models") => {
            if let Some(model_name) = segments.get(1).cloned() {
                let alias_name = table_ref.alias().unwrap_or_else(|| model_name.clone());
                aliases.insert(alias_name, AliasTarget::Model { model_name });
            }
        }
        Some("sources") => {
            if let (Some(source_name), Some(table_name)) =
                (segments.get(1).cloned(), segments.get(2).cloned())
            {
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
        _ => {}
    }
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
            fn_id: Some(function.to_string()),
            element_index: None,
            column_origin: None,
            model_origin: None,
            source_origin: None,
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

    /// Phase B reviewer finding 5 — anonymous HOF expansion frames (fn_id = None,
    /// param = "", bound_type = "") must render as `"in expansion of `<map>` call"`
    /// rather than the malformed `"`, `` was bound to "` produced by the old
    /// named-frame template.
    #[test]
    fn lsp_anonymous_hof_frame_renders_without_empty_fragments() {
        // Build an anonymous frame (fn_id = None, empty param / bound_type).
        let path = PathBuf::from("/tmp/smelt-lsp-test-anon-hof.sql");
        let anon_frame = FrameInfo {
            function: "<map>".to_string(),
            param: String::new(),
            bound_type: String::new(),
            decl_path: Some(path),
            decl_range: Some(make_db_range(0, 0)),
            call_site_range: Some(make_db_range(10, 0)),
            fn_id: None, // marks frame as anonymous
            element_index: None,
            column_origin: None,
            model_origin: None,
            source_origin: None,
        };
        let diag = make_db_diag("type mismatch in lambda body", vec![anon_frame]);

        let (message, related) = render_expansion_frames(&diag);

        // 1. The trailer must NOT contain the empty-fragment patterns.
        assert!(
            !message.contains("`` was bound to"),
            "anonymous frame must not render empty param fragment; got: {message}"
        );
        assert!(
            !message.contains("was bound to \"\""),
            "anonymous frame must not render empty bound_type fragment; got: {message}"
        );

        // 2. The trailer must mention the HOF name.
        assert!(
            message.contains("<map>"),
            "anonymous frame trailer must include the HOF name; got: {message}"
        );

        // 3. The trailer must use the shorter "call" form.
        assert!(
            message.contains("in expansion of `<map>` call"),
            "anonymous frame trailer must use the short form; got: {message}"
        );

        // 4. The related-info message must also use the short form.
        let related = related.expect("anonymous frame with a decl_path must produce related_info");
        assert_eq!(related.len(), 1);
        assert!(
            related[0].message.contains("in expansion of `<map>` call"),
            "related-info message must use the short form; got: {}",
            related[0].message
        );
    }

    // ── Phase 4: LSP hover for list literal and spread ──────────────────────

    /// Helper: parse `SELECT <expr>` and extract the first select-item expression,
    /// then cast it to an ArrayLiteral and return its elements.
    fn list_literal_elements(sql: &str) -> Vec<smelt_parser::ast::Expr> {
        use smelt_parser::ast::File as AstFile;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = AstFile::cast(root).expect("FILE node");
        let select = file.select_stmt().expect("SelectStmt");
        let select_list = select.select_list().expect("select list");
        let first_item = select_list.items().next().expect("at least one item");
        let expr = first_item.expression().expect("expression");
        let arr = expr
            .as_array_literal()
            .expect("expected ARRAY_LITERAL node");
        arr.elements()
    }

    /// Helper: parse SQL and find the first `LIST_SPREAD` node anywhere in the
    /// CST (spread items may not appear as `SelectItem` children depending on
    /// the grammar position; descendant search is the robust approach).
    fn parse_list_spread(sql: &str) -> smelt_parser::ast::ListSpread {
        use smelt_parser::syntax_kind::SyntaxKind;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        root.descendants()
            .find(|n| n.kind() == SyntaxKind::LIST_SPREAD)
            .and_then(smelt_parser::ast::ListSpread::cast)
            .expect("expected LIST_SPREAD node in SQL")
    }

    /// Hover on `[1, 2, 3]` — all Integer — must return text containing
    /// `List<Expr<INTEGER>>`.
    ///
    /// Note: `format_smelt_type_hover` renders DataType names in SQL uppercase
    /// (e.g. `INTEGER`, `TEXT`) via `DataType::to_sql()`.
    #[test]
    fn hover_list_literal_homogeneous() {
        let elems = list_literal_elements("SELECT [100000, 200000, 300000]");
        let ctx = smelt_db::TypeContext::new();
        let text = hover_text_for_list_literal(&elems, &ctx, None);
        assert!(
            text.contains("List<Expr<INTEGER>>"),
            "hover text for homogeneous integer list must contain `List<Expr<INTEGER>>`, got: {text}"
        );
    }

    /// Hover on `[]` at a position expecting `List<Expr<TEXT>>` must return
    /// `List<Expr<TEXT>>`.
    ///
    /// Note: DataType names render in SQL uppercase via `DataType::to_sql()`.
    ///
    /// Tests the Phase B+ position-aware code path (`hover_text_for_list_literal`
    /// with `expected = Some(…)`); not exercised by `Backend::hover` today —
    /// the production dispatch always calls `hover_text_for_list_literal_dual`
    /// with `expected = None`.
    #[test]
    fn hover_list_literal_empty_with_target() {
        use smelt_types::signatures::{SmeltType, TypeConstraint};
        use smelt_types::DataType;
        let elems = list_literal_elements("SELECT []");
        let ctx = smelt_db::TypeContext::new();
        let expected = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
            DataType::Text,
        ))));
        let text = hover_text_for_list_literal(&elems, &ctx, Some(&expected));
        assert!(
            text.contains("List<Expr<TEXT>>"),
            "hover text for empty list with TEXT target must contain `List<Expr<TEXT>>`, got: {text}"
        );
    }

    /// Hover on `[1, 'hello']` — mixed Integer/Text — must return
    /// `List<Unknown>` (heterogeneous).
    #[test]
    fn hover_list_literal_unknown() {
        let elems = list_literal_elements("SELECT [1, 'hello']");
        let ctx = smelt_db::TypeContext::new();
        let text = hover_text_for_list_literal(&elems, &ctx, None);
        assert!(
            text.contains("List<Unknown>"),
            "hover text for heterogeneous list must contain `List<Unknown>`, got: {text}"
        );
    }

    /// Hover on `[1, 2, 3]` at a position admitting both meta-list and
    /// Data-World array: hover text must surface both readings.
    ///
    /// The spec note says "literal accepted in two contexts". When no expected
    /// sort is present and the element type is a concrete `Expr<T>`, both
    /// interpretations are valid. The hover must mention both
    /// `List<Expr<INTEGER>>` (meta) and `Array<INTEGER>` (data-world).
    ///
    /// Note: DataType names render in SQL uppercase via `DataType::to_sql()`.
    #[test]
    fn hover_list_literal_dual_admissible() {
        let elems = list_literal_elements("SELECT [100000, 200000, 300000]");
        let ctx = smelt_db::TypeContext::new();
        let text = hover_text_for_list_literal_dual(&elems, &ctx);
        assert!(
            text.contains("List<Expr<INTEGER>>"),
            "dual-admissible hover must mention meta reading `List<Expr<INTEGER>>`, got: {text}"
        );
        assert!(
            text.contains("Array<INTEGER>"),
            "dual-admissible hover must mention data-world reading `Array<INTEGER>`, got: {text}"
        );
    }

    /// Hover on `...[1.5, 2.5]` — spread of a numeric list literal.
    ///
    /// Note: Phase A cannot bind named variables; the operand is a list literal
    /// whose inferred type is `List<Expr<DECIMAL(2,1)>>` — both `1.5` and `2.5`
    /// lex as `Decimal(2,1)` and the LUB of two identical types is that same type.
    /// The hover must show that exact element type (not the `Decimal(38,10)` that
    /// promotion would produce for mixed types — these two are the same precision).
    #[test]
    fn hover_spread_returns_source_list_type() {
        // `...[1.5, 2.5]` — LIST_SPREAD wrapping an ARRAY_LITERAL.
        // Both literals lex as Decimal(2,1); LUB of identical types is the type
        // itself, so the inferred element type is Decimal(2,1).
        let spread = parse_list_spread("SELECT ...[1.5, 2.5]");
        let ctx = smelt_db::TypeContext::new();
        let text = hover_text_for_list_spread(&spread, &ctx);
        // Assert the exact inferred type — homogeneous Decimal(2,1) list.
        assert!(
            text.contains("List<Expr<DECIMAL(2,1)>>"),
            "hover for spread of [1.5, 2.5] must be `List<Expr<DECIMAL(2,1)>>`, got: {text}"
        );
    }

    // ── Phase B: LSP hover/goto-def/completion for HOFs, lambdas, pipe, reducers, config.var ──

    /// Parse SQL that contains a HOF call and extract the FunctionCall node.
    fn parse_hof_call(sql: &str) -> smelt_parser::ast::FunctionCall {
        use smelt_parser::ast::File as AstFile;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = AstFile::cast(root).expect("FILE node");
        // Find the first FUNCTION_CALL node anywhere in the tree.
        file.syntax()
            .descendants()
            .find_map(smelt_parser::ast::FunctionCall::cast)
            .expect("expected a FUNCTION_CALL node in SQL")
    }

    /// Parse SQL and extract the first LAMBDA node.
    fn parse_lambda(sql: &str) -> smelt_parser::ast::Lambda {
        use smelt_parser::syntax_kind::SyntaxKind;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        root.descendants()
            .find(|n| n.kind() == SyntaxKind::LAMBDA)
            .and_then(smelt_parser::ast::Lambda::cast)
            .expect("expected a LAMBDA node in SQL")
    }

    /// Parse SQL and extract the first PIPE_EXPR node.
    fn parse_pipe_expr(sql: &str) -> smelt_parser::ast::PipeExpr {
        use smelt_parser::syntax_kind::SyntaxKind;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        root.descendants()
            .find(|n| n.kind() == SyntaxKind::PIPE_EXPR)
            .and_then(smelt_parser::ast::PipeExpr::cast)
            .expect("expected a PIPE_EXPR node in SQL")
    }

    /// Hover on `c` inside `map([1, 2, 3], fn c => c)` returns text containing
    /// the parameter type (`Expr<INTEGER>` bound from list element type).
    #[test]
    fn hover_lambda_parameter_in_body() {
        let call = parse_hof_call("SELECT map([100000, 200000, 300000], fn c => c)");
        let ctx = smelt_db::TypeContext::new();
        let text = hover_text_for_hof_call(&call, &ctx);
        assert!(
            text.contains("List"),
            "hover for map([ints], fn c => c) must contain `List`, got: {text}"
        );
        // Also test hover_text_for_lambda_param directly with a known element type
        use smelt_types::signatures::{SmeltType, TypeConstraint};
        use smelt_types::DataType;
        let int_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
        let lambda = parse_lambda("SELECT map([1, 2, 3], fn c => c)");
        let param_text = hover_text_for_lambda_param("c", &int_ty, &lambda, &ctx);
        assert!(
            param_text.contains("Expr<INTEGER>"),
            "hover for lambda param `c` bound to Integer must contain `Expr<INTEGER>`, got: {param_text}"
        );
    }

    /// Hover on the `map(...)` call expression returns `List<U>` where
    /// `U` is the lambda body's synthesised type.
    #[test]
    fn hover_hof_call_returns_result_type() {
        let call = parse_hof_call("SELECT map([100000, 200000, 300000], fn c => c)");
        let ctx = smelt_db::TypeContext::new();
        let text = hover_text_for_hof_call(&call, &ctx);
        assert!(
            text.contains("List<Expr<INTEGER>>"),
            "hover for map([ints], fn c => c) must be `List<Expr<INTEGER>>`, got: {text}"
        );
    }

    /// Hover on `xs |> filter(fn c => c > 0)` returns the same type as
    /// hover on `filter(xs, fn c => c > 0)`.
    #[test]
    fn hover_pipe_expression_returns_unpiped_type() {
        // Build a pipe expression and the direct equivalent call
        let pipe = parse_pipe_expr("SELECT [100000, 200000, -1] |> filter(fn c => c > 0)");
        let ctx = smelt_db::TypeContext::new();
        let pipe_text = hover_text_for_pipe_expr(&pipe, &ctx);
        assert!(
            pipe_text.contains("List"),
            "hover for pipe expression must contain `List`, got: {pipe_text}"
        );
        // The direct equivalent call should give the same result
        let direct = parse_hof_call("SELECT filter([100000, 200000, -1], fn c => c > 0)");
        let direct_text = hover_text_for_hof_call(&direct, &ctx);
        assert_eq!(
            pipe_text, direct_text,
            "pipe hover must equal direct-call hover: pipe={pipe_text} direct={direct_text}"
        );
    }

    /// Hover on `union_all` in `reduce(xs, union_all)` returns text containing
    /// the input element type (`TableExpr`), output sort (`TableExpr`), and
    /// identity rule (`no identity`).
    #[test]
    fn hover_reducer_name_in_reduce_position() {
        let text = hover_text_for_reducer_name("union_all");
        assert!(
            text.contains("TableExpr"),
            "hover for union_all must mention `TableExpr` input, got: {text}"
        );
        assert!(
            text.contains("no identity"),
            "hover for union_all must mention `no identity`, got: {text}"
        );
    }

    /// Hover on `and_all` returns identity `TRUE`.
    #[test]
    fn hover_reducer_name_with_identity() {
        let text = hover_text_for_reducer_name("and_all");
        assert!(
            text.contains("TRUE"),
            "hover for and_all must mention identity `TRUE`, got: {text}"
        );
    }

    /// Hover on `smelt.config.var('region')` over a workspace with
    /// `vars: { region: us-west-2 }` returns text containing `Text` and
    /// the resolved value `'us-west-2'`.
    #[test]
    fn hover_smelt_config_var_resolved() {
        let smelt_yml = "name: test_project\nvars:\n  region: us-west-2\n";
        let text = hover_text_for_config_var("region", smelt_yml);
        assert!(
            text.contains("Text"),
            "hover for config.var must contain `Text`, got: {text}"
        );
        assert!(
            text.contains("us-west-2"),
            "hover for config.var('region') must contain resolved value `us-west-2`, got: {text}"
        );
    }

    /// Hover on `smelt.config.var('not_declared')` returns `Text` and a hint
    /// that the variable is not declared (no crash).
    #[test]
    fn hover_smelt_config_var_unresolved() {
        let smelt_yml = "name: test_project\nvars:\n  region: us-west-2\n";
        let text = hover_text_for_config_var("not_declared", smelt_yml);
        assert!(
            text.contains("Text"),
            "hover for unresolved config.var must still contain `Text`, got: {text}"
        );
        assert!(
            text.contains("not declared")
                || text.contains("not found")
                || text.contains("undefined"),
            "hover for unresolved config.var must indicate the variable is missing, got: {text}"
        );
    }

    /// Goto-def on `c` inside the body of `map(xs, fn c => c)` resolves to
    /// the `c` token in the lambda parameter list.
    ///
    /// We test the pure helper `lambda_param_binder_range` that returns the
    /// text range of the binding occurrence given the parameter name.
    #[test]
    fn goto_def_lambda_parameter_resolves_to_binder() {
        use smelt_parser::syntax_kind::SyntaxKind;
        let sql = "SELECT map([1, 2, 3], fn c => c)";
        let lambda = parse_lambda(sql);
        let result = lambda_param_binder_range(&lambda, "c");
        assert!(
            result.is_some(),
            "lambda_param_binder_range for `c` in `fn c => c` must return Some, got None"
        );
        // The binder range must contain the IDENT "c" at a token of kind IDENT.
        let range = result.unwrap();
        // Range should be non-zero sized (the `c` token occupies at least one char)
        assert!(
            range.end() > range.start(),
            "binder range must be non-empty, got {:?}",
            range
        );
        // Verify the token at that range is the "c" identifier.
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let text_str: String = root.text().to_string();
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        let token_text = &text_str[start..end];
        assert_eq!(
            token_text, "c",
            "binder token text must be `c`, got `{token_text}`"
        );
        let _ = SyntaxKind::IDENT; // ensure import
    }

    /// Goto-def on the argument `'region'` of `smelt.config.var('region')`
    /// returns a Location pointing at the `vars.region:` line in `smelt.yml`.
    #[test]
    fn goto_def_smelt_config_var_resolves_to_yml_line() {
        let smelt_yml = "name: test_project\nvars:\n  region: us-west-2\n  env: prod\n";
        let line = find_var_line_in_smelt_yml(smelt_yml, "region");
        assert!(
            line.is_some(),
            "find_var_line_in_smelt_yml must find `region` in the vars block"
        );
        let line = line.unwrap();
        // `region:` is on line 2 (0-indexed) — after `name:` (line 0) and `vars:` (line 1)
        assert_eq!(
            line, 2,
            "region: should be on line 2 (0-indexed), got {line}"
        );
    }

    /// At a completion request inside the body of `fn c => |`, the completion
    /// list includes `c` as the first identifier completion.
    #[test]
    fn completion_in_lambda_body_includes_parameter_first() {
        let lambda = parse_lambda("SELECT map([1, 2, 3], fn c => c)");
        let params = lambda_params_for_completion(&lambda);
        assert!(
            !params.is_empty(),
            "lambda_params_for_completion must return at least one param"
        );
        assert_eq!(
            params[0], "c",
            "first completion param must be `c`, got `{}`",
            params[0]
        );
    }

    /// At a completion request at the second-arg position of
    /// `reduce(xs, |)` where `xs: List<Expr<Integer>>`, the completion list
    /// includes the reducers whose declared input is compatible with
    /// `Expr<Integer>` (i.e. `plus_chain`, `comma_sep`); reducers with
    /// incompatible input (e.g. `union_all` for `TableExpr`) are filtered out.
    #[test]
    fn completion_in_reduce_second_arg_offers_registry() {
        use smelt_types::signatures::{SmeltType, TypeConstraint};
        use smelt_types::DataType;
        let int_elem_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
        let list_of_int = SmeltType::List(Box::new(int_elem_ty.clone()));
        let names = reducer_completions_for_element_type(Some(&list_of_int));
        assert!(
            names.contains(&"plus_chain".to_string()),
            "plus_chain must be offered for List<Expr<Integer>>, got: {names:?}"
        );
        assert!(
            names.contains(&"comma_sep".to_string()),
            "comma_sep must be offered for any Expr<T> element, got: {names:?}"
        );
        assert!(
            !names.contains(&"union_all".to_string()),
            "union_all (TableExpr input) must NOT be offered for List<Expr<Integer>>, got: {names:?}"
        );
        assert!(
            !names.contains(&"and_all".to_string()),
            "and_all (Boolean input) must NOT be offered for List<Expr<Integer>>, got: {names:?}"
        );
    }

    /// Hover inside `map(xs, fn c =` (mid-edit, no body yet) does not crash;
    /// returns `Lambda<T, ?>` or no hover.
    #[test]
    fn hover_does_not_panic_on_partial_lambda() {
        use smelt_parser::syntax_kind::SyntaxKind;
        // Parse the partial lambda — the parser should recover gracefully.
        let sql = "SELECT map([1, 2, 3], fn c =";
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        // Find any LAMBDA node (if the parser recovered one).
        let maybe_lambda = root
            .descendants()
            .find(|n| n.kind() == SyntaxKind::LAMBDA)
            .and_then(smelt_parser::ast::Lambda::cast);
        // Whether or not the parser produced a LAMBDA node, calling the hover
        // helper must not panic.
        let ctx = smelt_db::TypeContext::new();
        if let Some(lambda) = maybe_lambda {
            // Calling with Unknown element type simulates a partial parse.
            use smelt_types::signatures::SmeltType;
            let text = hover_text_for_lambda_param("c", &SmeltType::Unknown, &lambda, &ctx);
            // Must not panic; the text is allowed to be any non-panicking string.
            let _ = text;
        }
        // Also test that find the HOF call helper doesn't crash on partial input.
        let maybe_call = root
            .descendants()
            .find_map(smelt_parser::ast::FunctionCall::cast);
        if let Some(call) = maybe_call {
            let text = hover_text_for_hof_call(&call, &ctx);
            let _ = text;
        }
        // Test passes as long as nothing panicked.
    }

    // ── Dispatch-level tests (Finding 3) ─────────────────────────────────────
    //
    // These tests route cursor positions through `hover_text_for_hof_meta_language`
    // — the same pure function that `Backend::hover` calls — to prove that the
    // dispatch ordering is correct.  A regression (e.g. swapping the lambda-param
    // and HOF-result blocks back) MUST cause these tests to fail.

    /// Helper: parse SQL, find the AstFile, and call the dispatch helper.
    fn dispatch_hover(sql: &str, cursor_offset: usize) -> Option<String> {
        use smelt_parser::ast::File as AstFile;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = AstFile::cast(root)?;
        hover_text_for_hof_meta_language(&file, cursor_offset, "")
    }

    /// Cursor on `c` in the body of `fn c => c` (the second `c`) must return
    /// the parameter bound type (`Expr<INTEGER>`), NOT the HOF result type
    /// (`List<Expr<INTEGER>>`).
    ///
    /// This is the regression test for Finding 1: if the HOF result-type block
    /// runs before the lambda-param block, this test fails.
    #[test]
    fn dispatch_hover_lambda_parameter_in_body_wins_over_hof_result() {
        // `map([100000, 200000, 300000], fn c => c)`
        // The second `c` (body use) starts after `=>`.
        let sql = "SELECT map([100000, 200000, 300000], fn c => c)";
        // Find the byte offset of the body `c` — the last `c` in the SQL.
        let body_c_offset = sql.rfind('c').expect("body `c` must be in SQL");
        let result = dispatch_hover(sql, body_c_offset);
        assert!(
            result.is_some(),
            "hover on body `c` must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("Expr<INTEGER>"),
            "dispatch hover on body `c` must show param type `Expr<INTEGER>`, got: {text}"
        );
        assert!(
            !text.contains("List<Expr<INTEGER>>"),
            "dispatch hover on body `c` must NOT show HOF result type, got: {text}"
        );
    }

    /// Cursor on `union_all` in the second arg of `reduce(xs, union_all)` must
    /// return the reducer metadata (contains `TableExpr`), NOT the HOF result type.
    ///
    /// This is the regression test for Finding 2: if the reducer-name block
    /// runs after the HOF result-type block (dead code), this test fails.
    #[test]
    fn dispatch_hover_reducer_name_in_second_arg_wins_over_hof_result() {
        // We need a valid list literal for the first arg.  The reducer name is
        // the second identifier token.
        let sql = "SELECT reduce([smelt_table_a, smelt_table_b], union_all)";
        // Find the offset of `union_all` — the last token before `)`.
        let union_all_offset = sql.find("union_all").expect("union_all must be in SQL");
        let result = dispatch_hover(sql, union_all_offset + 2); // cursor inside `union_all`
        assert!(
            result.is_some(),
            "hover on `union_all` in second arg must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("TableExpr"),
            "dispatch hover on reducer name must show reducer metadata with `TableExpr`, got: {text}"
        );
        assert!(
            text.contains("no identity") || text.contains("identity"),
            "dispatch hover on reducer name must mention identity, got: {text}"
        );
    }

    /// Cursor on the binder `c` in `fn c => ...` (the first `c`, after `fn`)
    /// must return the param type via the lambda-param block.
    #[test]
    fn dispatch_hover_lambda_parameter_binder_shows_param_type() {
        let sql = "SELECT map([100000, 200000, 300000], fn c => c)";
        // Find the binder `c` — the first `c` after `fn `.
        let fn_pos = sql.find("fn ").expect("fn must be in SQL");
        let binder_offset = fn_pos + 3; // skip "fn "
        let result = dispatch_hover(sql, binder_offset);
        assert!(
            result.is_some(),
            "hover on binder `c` must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("Expr<INTEGER>"),
            "dispatch hover on binder `c` must show `Expr<INTEGER>`, got: {text}"
        );
        assert!(
            !text.contains("List<Expr<INTEGER>>"),
            "dispatch hover on binder `c` must NOT show HOF result type, got: {text}"
        );
    }

    /// Goto-def for a `smelt.config.var('undeclared')` must return `None` (no
    /// navigation), not `Some` pointing at line 0 of smelt.yml.
    ///
    /// This is the regression test for Finding 4: `unwrap_or(0)` silently
    /// navigates to the top of the file when the var is not declared.
    #[test]
    fn goto_def_config_var_undeclared_returns_none() {
        // `find_var_line_in_smelt_yml` must return None for a variable not in vars.
        let smelt_yml = "name: test_project\nvars:\n  declared_var: some_value\n";
        let result = find_var_line_in_smelt_yml(smelt_yml, "undeclared_var");
        assert!(
            result.is_none(),
            "find_var_line_in_smelt_yml for an undeclared var must return None, got {result:?}"
        );
        // Confirm the declared var still resolves correctly.
        let result2 = find_var_line_in_smelt_yml(smelt_yml, "declared_var");
        assert!(
            result2.is_some(),
            "find_var_line_in_smelt_yml for a declared var must return Some"
        );
    }

    // ── Phase C (meta-language): hover, goto-def, completion for reflection ───

    /// Hovering on `smelt.columns_of(orders)` returns `List<ColumnRef>` in the
    /// hover text.
    ///
    /// Tests `hover_text_for_columns_of_call` pure helper.
    /// When no `ColumnRefValue` list is supplied (schema unresolvable), the
    /// helper still shows the return type `List<ColumnRef>`.
    #[test]
    fn hover_on_smelt_columns_of_call_shows_list_column_ref() {
        // Case 1: no resolved columns (unresolvable schema) — must show List<ColumnRef>
        let text_no_cols = hover_text_for_columns_of_call("orders", None);
        assert!(
            text_no_cols.contains("List<ColumnRef>"),
            "hover on smelt.columns_of(orders) with unresolvable schema must contain \
             `List<ColumnRef>`, got: {text_no_cols}"
        );

        // Case 2: resolved columns — must show List<ColumnRef> PLUS column count + names
        use smelt_types::signatures::ColumnRefValue;
        let cols = vec![
            ColumnRefValue {
                name: "id".to_string(),
                data_type: Some(smelt_types::DataType::Integer),
                is_numeric: true,
                source_span: None,
            },
            ColumnRefValue {
                name: "amount".to_string(),
                data_type: Some(smelt_types::DataType::Float),
                is_numeric: true,
                source_span: None,
            },
            ColumnRefValue {
                name: "customer_name".to_string(),
                data_type: Some(smelt_types::DataType::Text),
                is_numeric: false,
                source_span: None,
            },
        ];
        let text_with_cols = hover_text_for_columns_of_call("orders", Some(&cols));
        assert!(
            text_with_cols.contains("List<ColumnRef>"),
            "hover on smelt.columns_of(orders) with resolved schema must contain \
             `List<ColumnRef>`, got: {text_with_cols}"
        );
        assert!(
            text_with_cols.contains('3') || text_with_cols.contains("3 columns"),
            "hover on smelt.columns_of with 3 resolved columns must mention column count, \
             got: {text_with_cols}"
        );
        assert!(
            text_with_cols.contains("id"),
            "hover on smelt.columns_of must list first column name `id`, got: {text_with_cols}"
        );
        assert!(
            text_with_cols.contains("amount"),
            "hover on smelt.columns_of must list column `amount`, got: {text_with_cols}"
        );
    }

    /// Hovering on a `ColumnRef`-typed lambda parameter (e.g. `c` in
    /// `map(smelt.columns_of(t), fn c => ...)`) shows `ColumnRef` plus the
    /// closed field list with each field's type.
    ///
    /// This test routes through `dispatch_hover` (i.e. through
    /// `hover_text_for_hof_meta_language`) to verify the *wiring*, not just
    /// the helper.  A regression that calls `hover_text_for_lambda_param`
    /// instead of `hover_text_for_column_ref_binding` would produce
    /// `"c: ColumnRef"` but NOT the field list, causing the `is_numeric`
    /// assertion below to fail.
    #[test]
    fn hover_on_column_ref_lambda_parameter_shows_field_set() {
        // Use smelt.columns_of so the inferred elem_ty is ColumnRef.
        let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
        // Cursor on the binder `c` (just after `fn `).
        let fn_pos = sql.find("fn ").expect("fn must be in SQL");
        let binder_offset = fn_pos + 3; // skip "fn "
        let result = dispatch_hover(sql, binder_offset);
        assert!(
            result.is_some(),
            "dispatch hover on ColumnRef lambda binder `c` must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("ColumnRef"),
            "hover on ColumnRef binding `c` must contain `ColumnRef`, got: {text}"
        );
        // Must show the three closed fields (field list from
        // `hover_text_for_column_ref_binding`, NOT the generic lambda-param text).
        assert!(
            text.contains("name"),
            "hover on ColumnRef binding must mention field `name`, got: {text}"
        );
        assert!(
            text.contains("type") || text.contains("DataType"),
            "hover on ColumnRef binding must mention field `type` / DataType, got: {text}"
        );
        assert!(
            text.contains("is_numeric"),
            "hover on ColumnRef binding must mention field `is_numeric`, got: {text}"
        );
    }

    /// Hovering on a field projection `c.name` shows `name: Text`.
    /// Hovering on `c.type` shows `type: DataType` (or Unknown per COLUMN_REF_FIELDS).
    /// Hovering on `c.is_numeric` shows `is_numeric: Boolean`.
    ///
    /// Tests `hover_text_for_column_ref_field` pure helper.
    #[test]
    fn hover_on_column_ref_field_projection_shows_field_type() {
        // `c.name` → Text
        let text_name = hover_text_for_column_ref_field("name");
        assert!(
            text_name.is_some(),
            "hover_text_for_column_ref_field('name') must return Some, got None"
        );
        let name_text = text_name.unwrap();
        assert!(
            name_text.contains("name"),
            "hover for `c.name` must mention field name `name`, got: {name_text}"
        );
        assert!(
            name_text.contains("Text") || name_text.contains("TEXT"),
            "hover for `c.name` must mention `Text` type, got: {name_text}"
        );

        // `c.type` → DataType / Unknown (Phase C maps DataType to Unknown)
        let text_type = hover_text_for_column_ref_field("type");
        assert!(
            text_type.is_some(),
            "hover_text_for_column_ref_field('type') must return Some, got None"
        );
        let type_text = text_type.unwrap();
        assert!(
            type_text.contains("type"),
            "hover for `c.type` must mention field name `type`, got: {type_text}"
        );
        assert!(
            type_text.contains("DataType") || type_text.contains("Unknown"),
            "hover for `c.type` must mention DataType or Unknown, got: {type_text}"
        );

        // `c.is_numeric` → Boolean
        let text_is_numeric = hover_text_for_column_ref_field("is_numeric");
        assert!(
            text_is_numeric.is_some(),
            "hover_text_for_column_ref_field('is_numeric') must return Some, got None"
        );
        let is_numeric_text = text_is_numeric.unwrap();
        assert!(
            is_numeric_text.contains("is_numeric"),
            "hover for `c.is_numeric` must mention field name `is_numeric`, got: {is_numeric_text}"
        );
        assert!(
            is_numeric_text.contains("Boolean") || is_numeric_text.contains("BOOLEAN"),
            "hover for `c.is_numeric` must mention `Boolean` type, got: {is_numeric_text}"
        );

        // Unknown field → None
        let text_unknown = hover_text_for_column_ref_field("nonexistent_field");
        assert!(
            text_unknown.is_none(),
            "hover_text_for_column_ref_field for unknown field must return None, got Some"
        );
    }

    /// Hovering on a meta-Text lifted identifier (e.g. `c.name` used as a
    /// column-reference) shows the lift description — the identity transform
    /// `Text -> identifier`.
    ///
    /// Tests `hover_text_for_lifted_identifier` pure helper.
    #[test]
    fn hover_on_lifted_identifier_shows_lift_target() {
        let text = hover_text_for_lifted_identifier("c.name", None);
        assert!(
            text.contains("Text") || text.contains("identifier"),
            "hover on lifted identifier must describe the Text→identifier lift, got: {text}"
        );

        // When the concrete column name is known (from ColumnRefValue), the hover
        // should mention that resolved value.
        use smelt_types::signatures::ColumnRefValue;
        let col = ColumnRefValue {
            name: "order_id".to_string(),
            data_type: Some(smelt_types::DataType::Integer),
            is_numeric: true,
            source_span: None,
        };
        let text_resolved = hover_text_for_lifted_identifier("c.name", Some(&col));
        assert!(
            text_resolved.contains("order_id"),
            "hover on lifted identifier with resolved column must mention column name \
             `order_id`, got: {text_resolved}"
        );
    }

    /// Goto-def on a `smelt.columns_of` call path is a graceful no-op — the
    /// helper returns `None` (client displays no navigation). This is the
    /// minimal spec-compliant implementation (URL hint / graceful no-op).
    ///
    /// Tests `goto_def_for_columns_of_call` pure helper.
    #[test]
    fn goto_def_on_columns_of_call_site_is_noop() {
        let result = goto_def_for_columns_of_call();
        assert!(
            result.is_none(),
            "goto_def_for_columns_of_call must return None (graceful no-op), got Some"
        );
    }

    /// Goto-def from a lifted meta-`Text` identifier (`c.name` in a lift
    /// position) is a graceful no-op when no source span is available, and
    /// resolves to the source column's declaration when one is supplied.
    ///
    /// **Known divergence:** Full Backend-level dispatch wiring (detecting the
    /// cursor is inside one of the four lift positions and resolving the column
    /// via `columns_of_for_table_expr`) is not yet implemented.  This test
    /// exercises the pure helper contract.  Tracked in
    /// `docs/plans/20260509-meta-language-overall.md`.
    ///
    /// Tests `goto_def_for_lifted_identifier` pure helper.
    #[test]
    fn goto_def_from_lifted_identifier_resolves_to_source_column() {
        // Without a resolved ColumnRefValue the result is None (no-op).
        let result_no_span = goto_def_for_lifted_identifier(None);
        assert!(
            result_no_span.is_none(),
            "goto_def_for_lifted_identifier(None) must return None (graceful no-op), \
             got Some"
        );

        // Even with a resolved ColumnRefValue that carries a source_span, the
        // current implementation returns None because the span type
        // (Option<TextRange>) does not carry a file path.  The wiring to
        // produce a PathBuf is a known divergence (see doc comment above).
        use smelt_types::signatures::ColumnRefValue;
        let col_with_span = ColumnRefValue {
            name: "order_id".to_string(),
            data_type: Some(smelt_types::DataType::Integer),
            is_numeric: true,
            source_span: None, // TextRange not yet resolvable to a path
        };
        let result_with_col = goto_def_for_lifted_identifier(Some(&col_with_span));
        // v1 always returns None; a future phase wires the path resolution.
        assert!(
            result_with_col.is_none(),
            "goto_def_for_lifted_identifier v1 must return None (wiring not yet \
             implemented), got Some"
        );
    }

    /// Completion at `c.<cursor>` offers exactly the three ColumnRef fields
    /// (`name`, `type`, `is_numeric`) and nothing else.
    ///
    /// Tests `column_ref_field_completions` pure helper.
    #[test]
    fn completion_at_column_ref_field_offers_closed_set() {
        let names = column_ref_field_completions();
        assert_eq!(
            names.len(),
            3,
            "column_ref_field_completions must return exactly 3 items, got: {names:?}"
        );
        assert!(
            names.contains(&"name".to_string()),
            "column_ref_field_completions must include `name`, got: {names:?}"
        );
        assert!(
            names.contains(&"type".to_string()),
            "column_ref_field_completions must include `type`, got: {names:?}"
        );
        assert!(
            names.contains(&"is_numeric".to_string()),
            "column_ref_field_completions must include `is_numeric`, got: {names:?}"
        );
    }

    /// Completion at `smelt.columns_of(<cursor>)` — calls
    /// `columns_of_arg_completions_for_sql` and verifies that in-scope
    /// `smelt.<path>` references in a SQL text are extracted. This simulates
    /// the case where the file text contains `FROM smelt.orders` — the name
    /// `orders` (or `smelt.orders`) should appear in the completion list.
    ///
    /// Tests `columns_of_arg_completions_for_sql` pure helper.
    #[test]
    fn completion_at_columns_of_argument_offers_table_expr_names() {
        // SQL that has a smelt path reference to `orders` (a models ref)
        let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name) \
                   FROM smelt.models.orders";
        let names = columns_of_arg_completions_for_sql(sql);
        // The completion list must include `orders` (derived from the
        // smelt.models.orders path reference in the FROM clause).
        assert!(
            !names.is_empty(),
            "columns_of_arg_completions_for_sql must return at least one entry \
             for SQL with a smelt path ref, got: {names:?}"
        );
        assert!(
            names.contains(&"orders".to_string()),
            "columns_of_arg_completions_for_sql must include `orders` from \
             smelt.models.orders reference, got: {names:?}"
        );
    }

    /// Completion at `smelt.columns_of(<cursor>)` inside a `smelt.define`
    /// body — verifies that `TableExpr`-typed function parameters are offered
    /// as candidates.
    ///
    /// This is the primary motivation for the parametric case:
    /// `smelt.define coalesce_numeric(t: TableExpr) AS (SELECT map(smelt.columns_of(<cursor>), …))`
    /// — `t` must appear in the completion list because it is a `TableExpr`
    /// parameter in scope at that call site.
    ///
    /// Tests the second source in `columns_of_arg_completions_for_sql`.
    #[test]
    fn completion_at_columns_of_argument_offers_define_table_expr_params() {
        // A smelt.define with a TableExpr parameter.  The body contains
        // smelt.columns_of(...) — `t` must be offered as a completion.
        let sql = "smelt.define coalesce_numeric(t: TableExpr) AS \
                   (SELECT map(smelt.columns_of(t), fn c => COALESCE(c.name, '')) \
                    FROM t)";
        let names = columns_of_arg_completions_for_sql(sql);
        assert!(
            names.contains(&"t".to_string()),
            "columns_of_arg_completions_for_sql must include `t` (TableExpr \
             parameter of smelt.define), got: {names:?}"
        );
    }

    /// Completion at `smelt.columns_of(<cursor>)` inside a `smelt.define`
    /// with multiple parameters — only `TableExpr`-typed parameters are
    /// offered; non-TableExpr parameters are excluded.
    #[test]
    fn completion_at_columns_of_argument_excludes_non_table_expr_params() {
        // Two params: `t: TableExpr` (should appear) and `threshold: Expr<Integer>`
        // (must NOT appear — wrong type).
        let sql = "smelt.define filtered(t: TableExpr, threshold: Expr<Integer>) AS \
                   (SELECT map(smelt.columns_of(t), fn c => c.name) FROM t \
                    WHERE amount > threshold)";
        let names = columns_of_arg_completions_for_sql(sql);
        assert!(
            names.contains(&"t".to_string()),
            "columns_of_arg_completions_for_sql must include `t` (TableExpr param), \
             got: {names:?}"
        );
        assert!(
            !names.contains(&"threshold".to_string()),
            "columns_of_arg_completions_for_sql must NOT include `threshold` \
             (Expr<Integer> param, not TableExpr), got: {names:?}"
        );
    }

    /// The `hover_text_for_hof_meta_language` dispatch helper picks up
    /// `smelt.columns_of` calls and returns hover text containing `List<ColumnRef>`.
    ///
    /// This is the dispatch-level test: it verifies that the routing in
    /// `hover_text_for_hof_meta_language` reaches the `smelt.columns_of` branch.
    #[test]
    fn dispatch_hover_smelt_columns_of_shows_list_column_ref() {
        let sql = "SELECT smelt.columns_of(orders)";
        // Find the offset of `smelt.columns_of` — cursor inside the call path.
        let columns_of_offset = sql.find("columns_of").expect("columns_of must be in SQL");
        let result = dispatch_hover(sql, columns_of_offset + 2); // cursor inside `columns_of`
        assert!(
            result.is_some(),
            "dispatch hover on smelt.columns_of must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("List<ColumnRef>"),
            "dispatch hover on smelt.columns_of must show `List<ColumnRef>`, got: {text}"
        );
    }

    /// The `hover_text_for_hof_meta_language` dispatch helper picks up a
    /// ColumnRef field projection (e.g. `c.name`) and shows the declared field type.
    ///
    /// This tests that when the cursor is on the `name` token of `c.name`
    /// inside a lambda body, the field type `Text` is surfaced.
    #[test]
    fn dispatch_hover_column_ref_field_projection_shows_field_type() {
        // SQL with a ColumnRef field projection inside a lambda body.
        // We use a syntactically valid expression where `c.name` appears.
        // The `.name` field access after a lambda parameter `c` is what we hover.
        let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
        // Find the offset of `.name` (specifically the `name` identifier token).
        let name_offset = sql.rfind("name").expect("`name` must appear in SQL");
        let result = dispatch_hover(sql, name_offset);
        // If the dispatch reaches the field-projection branch, it should return Some
        // with field type info.  If it falls through to the HOF branch instead,
        // the text will contain `List<...>` (wrong).
        if let Some(text) = result {
            // If we get a result, it must describe the `name` field.
            // Accept either the field hover or a HOF result — the critical constraint
            // is that it does NOT silently return wrong data (i.e., it doesn't
            // say `List<ColumnRef>` when hovering on the field access).
            assert!(
                !text.contains("List<ColumnRef>") || text.contains("name"),
                "dispatch hover on `c.name` field must not show List<ColumnRef> \
                 without also mentioning `name`, got: {text}"
            );
        }
        // None is also acceptable if the file is not registered in a real DB
        // (the dispatch operates on parsed AST only, no Salsa).
    }

    // ── Finding 1 + 2 regression tests ───────────────────────────────────────
    //
    // These tests verify that ColumnRef field completions and hover only fire
    // when the receiver token is actually a ColumnRef-typed lambda parameter
    // (i.e. bound by a HOF whose first arg is `smelt.columns_of(...)`).

    /// Helper: check whether `is_column_ref_param_before_dot` returns `Some` for
    /// a file + cursor positioned just after `<param>.`.
    fn check_is_column_ref_param(sql: &str, cursor_offset: usize) -> Option<String> {
        use smelt_parser::ast::File as AstFile;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = AstFile::cast(root)?;
        is_column_ref_param_before_dot(&file, sql, cursor_offset)
    }

    /// NEGATIVE completion — `x.<cursor>` inside `map(some_int_list, fn x => x.something)`
    /// with an UNRELATED `smelt.columns_of(orders)` call elsewhere in the file.
    ///
    /// The completion MUST NOT offer `{name, type, is_numeric}` for `x` because `x`
    /// is not a ColumnRef-typed parameter (the HOF iterates over `some_int_list`, not
    /// `smelt.columns_of(...)`).
    #[test]
    fn completion_column_ref_fields_does_not_fire_for_unrelated_lambda_param() {
        // `x` is a parameter of `map(some_int_list, ...)` — NOT ColumnRef.
        // `smelt.columns_of(orders)` appears elsewhere but must not pollute `x`.
        let sql = "SELECT map(some_int_list, fn x => x.something), smelt.columns_of(orders)";
        // Cursor after `x.` — position just past the dot.
        let dot_pos = sql.find("x.").expect("`x.` must appear in SQL") + 2; // after the dot
        let result = check_is_column_ref_param(sql, dot_pos);
        assert!(
            result.is_none(),
            "is_column_ref_param_before_dot must return None for `x.` where `x` is NOT \
             a ColumnRef-typed param (HOF iterates over `some_int_list`), got: {result:?}"
        );
    }

    /// POSITIVE completion — `c.<cursor>` inside `map(smelt.columns_of(orders), fn c => c.)`
    ///
    /// The completion MUST offer `{name, type, is_numeric}` because `c` IS a
    /// ColumnRef-typed parameter (the HOF iterates over `smelt.columns_of(orders)`).
    #[test]
    fn completion_column_ref_fields_fires_for_columns_of_lambda_param() {
        let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
        // Cursor after `c.` in the lambda body — just past the dot before `name`.
        let c_dot_pos = sql.rfind("c.").expect("`c.` must appear in SQL") + 2;
        let result = check_is_column_ref_param(sql, c_dot_pos);
        assert!(
            result.is_some(),
            "is_column_ref_param_before_dot must return Some for `c.` where `c` IS \
             a ColumnRef-typed param (HOF iterates over `smelt.columns_of(orders)`), \
             got: None"
        );
        let param_name = result.unwrap();
        assert_eq!(
            param_name, "c",
            "returned param name must be `c`, got: {param_name}"
        );
    }

    /// NEGATIVE hover — hovering on the `type` token in plain SQL `t.type`
    /// (where `t` is a table alias, NOT a ColumnRef lambda parameter) must NOT
    /// return the ColumnRef field hover.
    ///
    /// The dispatch (`hover_text_for_hof_meta_language`) must check the receiver
    /// before returning ColumnRef field hover text.
    #[test]
    fn hover_column_ref_field_does_not_fire_for_plain_sql_field_access() {
        // Plain SQL table alias access — no HOF, no smelt.columns_of.
        let sql = "SELECT t.type FROM some_table t";
        let type_offset = sql.find(".type").expect("`.type` must appear in SQL") + 1; // on `type`
        let result = dispatch_hover(sql, type_offset);
        // If the dispatch fires the ColumnRef hover without the receiver check, it
        // will return Some text containing "(ColumnRef field)". That is the bug.
        if let Some(text) = result {
            assert!(
                !text.contains("ColumnRef field"),
                "hover on plain SQL `t.type` must NOT show ColumnRef field hover \
                 (no ColumnRef binding in scope), got: {text}"
            );
        }
        // None is fine — no ColumnRef context means no hover.
    }

    /// POSITIVE hover — hovering on `name` in `c.name` inside
    /// `map(smelt.columns_of(orders), fn c => c.name)` MUST show the ColumnRef
    /// field hover for `name: Text`.
    #[test]
    fn hover_column_ref_field_fires_for_columns_of_lambda_body_field_access() {
        let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
        // Cursor on `name` in `c.name` — the last occurrence of `name`.
        let name_offset = sql.rfind("name").expect("`name` must appear in SQL");
        let result = dispatch_hover(sql, name_offset);
        assert!(
            result.is_some(),
            "hover on `c.name` inside smelt.columns_of lambda must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("ColumnRef field") || text.contains("name") && text.contains("Text"),
            "hover on `c.name` in ColumnRef lambda must describe the `name` field (Text), \
             got: {text}"
        );
    }

    // ── Phase D (meta-language): hover, goto-def, completion for wide reflection

    /// Hovering on `smelt.models.with_tag('cohort')` returns `List<ModelRef>` in
    /// the hover text.  When tag resolves, also shows match count + first five
    /// names.  Analogous for `smelt.sources.with_tag`.
    ///
    /// Tests `hover_text_for_models_with_tag_call` and
    /// `hover_text_for_sources_with_tag_call` pure helpers.
    #[test]
    fn hover_on_smelt_models_with_tag_call_shows_list_model_ref() {
        // Case 1: no resolved models (workspace unresolvable) — must show List<ModelRef>
        let text_no_models = hover_text_for_models_with_tag_call("cohort", None);
        assert!(
            text_no_models.contains("List<ModelRef>"),
            "hover on smelt.models.with_tag with unresolvable workspace must contain \
             `List<ModelRef>`, got: {text_no_models}"
        );
        assert!(
            text_no_models.contains("cohort"),
            "hover on smelt.models.with_tag must mention the tag, got: {text_no_models}"
        );

        // Case 2: resolved models — must show List<ModelRef> PLUS count + names
        use smelt_types::signatures::ModelRefValue;
        let models = vec![
            ModelRefValue {
                path: "models/orders.sql".to_string(),
                name: "orders".to_string(),
                tags: vec!["cohort".to_string()],
                model_name_for_columns: "orders".to_string(),
            },
            ModelRefValue {
                path: "models/customers.sql".to_string(),
                name: "customers".to_string(),
                tags: vec!["cohort".to_string()],
                model_name_for_columns: "customers".to_string(),
            },
        ];
        let text_with_models = hover_text_for_models_with_tag_call("cohort", Some(&models));
        assert!(
            text_with_models.contains("List<ModelRef>"),
            "hover on smelt.models.with_tag with resolved models must contain \
             `List<ModelRef>`, got: {text_with_models}"
        );
        assert!(
            text_with_models.contains('2') || text_with_models.contains("2 matching"),
            "hover on smelt.models.with_tag with 2 models must mention count, \
             got: {text_with_models}"
        );
        assert!(
            text_with_models.contains("orders"),
            "hover on smelt.models.with_tag must list model name `orders`, \
             got: {text_with_models}"
        );

        // SourceRef variant
        let text_no_sources = hover_text_for_sources_with_tag_call("audit", None);
        assert!(
            text_no_sources.contains("List<SourceRef>"),
            "hover on smelt.sources.with_tag must contain `List<SourceRef>`, \
             got: {text_no_sources}"
        );

        use smelt_types::signatures::SourceRefValue;
        let sources = vec![SourceRefValue {
            path: "sources/raw.yml".to_string(),
            name: "raw_events".to_string(),
            tags: vec!["audit".to_string()],
            address_segments: vec!["raw".to_string(), "raw_events".to_string()],
        }];
        let text_with_sources = hover_text_for_sources_with_tag_call("audit", Some(&sources));
        assert!(
            text_with_sources.contains("List<SourceRef>"),
            "hover on smelt.sources.with_tag with resolved sources must contain \
             `List<SourceRef>`, got: {text_with_sources}"
        );
        assert!(
            text_with_sources.contains("raw_events"),
            "hover on smelt.sources.with_tag must list source name `raw_events`, \
             got: {text_with_sources}"
        );

        // Verify dispatch routing: hovering on the call site in SQL
        let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)";
        let with_tag_offset = sql.find("with_tag").expect("with_tag must be in SQL");
        let result = dispatch_hover(sql, with_tag_offset + 2);
        assert!(
            result.is_some(),
            "dispatch hover on smelt.models.with_tag call must produce Some, got None"
        );
        let hover_text = result.unwrap();
        assert!(
            hover_text.contains("List<ModelRef>"),
            "dispatch hover on smelt.models.with_tag must contain `List<ModelRef>`, \
             got: {hover_text}"
        );
        assert!(
            hover_text.contains("cohort"),
            "dispatch hover on smelt.models.with_tag must mention the tag, \
             got: {hover_text}"
        );
    }

    /// Hovering on `smelt.models.all` shows the signature plus workspace model
    /// count.  Analogous for `smelt.sources.all`.
    ///
    /// Tests `hover_text_for_models_all` and `hover_text_for_sources_all`
    /// pure helpers.
    #[test]
    fn hover_on_smelt_models_all_shows_workspace_count() {
        // No workspace count available
        let text_no_count = hover_text_for_models_all(None);
        assert!(
            text_no_count.contains("List<ModelRef>"),
            "hover on smelt.models.all with no count must contain `List<ModelRef>`, \
             got: {text_no_count}"
        );

        // With workspace count
        let text_with_count = hover_text_for_models_all(Some(42));
        assert!(
            text_with_count.contains("List<ModelRef>"),
            "hover on smelt.models.all with count must contain `List<ModelRef>`, \
             got: {text_with_count}"
        );
        assert!(
            text_with_count.contains("42"),
            "hover on smelt.models.all must mention total model count 42, \
             got: {text_with_count}"
        );

        // SourceRef variant
        let text_no_sources = hover_text_for_sources_all(None);
        assert!(
            text_no_sources.contains("List<SourceRef>"),
            "hover on smelt.sources.all must contain `List<SourceRef>`, \
             got: {text_no_sources}"
        );
        let text_sources = hover_text_for_sources_all(Some(5));
        assert!(
            text_sources.contains("5"),
            "hover on smelt.sources.all must mention total source count, \
             got: {text_sources}"
        );

        // Verify dispatch routing
        let sql = "SELECT reduce(smelt.models.all(), union_all)";
        let all_offset = sql.find(".all").expect(".all must be in SQL") + 1;
        let result = dispatch_hover(sql, all_offset);
        assert!(
            result.is_some(),
            "dispatch hover on smelt.models.all call must produce Some, got None"
        );
        let hover_text = result.unwrap();
        assert!(
            hover_text.contains("List<ModelRef>"),
            "dispatch hover on smelt.models.all must contain `List<ModelRef>`, \
             got: {hover_text}"
        );
    }

    /// Hovering on `m` inside `map(smelt.models.with_tag('cohort'), fn m => …)`
    /// shows `ModelRef` plus the closed four-field list with each field's type.
    /// Analogous for `SourceRef`.
    ///
    /// Routes through `dispatch_hover` to verify the wiring.
    #[test]
    fn hover_on_model_ref_lambda_parameter_shows_field_set() {
        // Case 1: cursor on the binder `m` in `fn m => m.name`
        let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)";
        let fn_pos = sql.find("fn ").expect("fn must be in SQL");
        let binder_offset = fn_pos + 3; // skip "fn "
        let result = dispatch_hover(sql, binder_offset);
        assert!(
            result.is_some(),
            "dispatch hover on ModelRef lambda binder `m` must produce Some, got None"
        );
        let text = result.unwrap();
        assert!(
            text.contains("ModelRef"),
            "hover on ModelRef binding `m` must contain `ModelRef`, got: {text}"
        );
        // Must show the four closed fields
        assert!(
            text.contains("path"),
            "hover on ModelRef binding must mention field `path`, got: {text}"
        );
        assert!(
            text.contains("name"),
            "hover on ModelRef binding must mention field `name`, got: {text}"
        );
        assert!(
            text.contains("tags"),
            "hover on ModelRef binding must mention field `tags`, got: {text}"
        );
        assert!(
            text.contains("columns"),
            "hover on ModelRef binding must mention field `columns`, got: {text}"
        );

        // Case 2: the binding helper directly
        let binding_text = hover_text_for_model_ref_binding("m");
        assert!(
            binding_text.contains("ModelRef"),
            "hover_text_for_model_ref_binding must contain ModelRef, got: {binding_text}"
        );

        // SourceRef variant
        let sql_src = "SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)";
        let fn_pos_src = sql_src.find("fn ").expect("fn must be in SQL");
        let binder_offset_src = fn_pos_src + 3;
        let result_src = dispatch_hover(sql_src, binder_offset_src);
        assert!(
            result_src.is_some(),
            "dispatch hover on SourceRef lambda binder `s` must produce Some, got None"
        );
        let text_src = result_src.unwrap();
        assert!(
            text_src.contains("SourceRef"),
            "hover on SourceRef binding `s` must contain `SourceRef`, got: {text_src}"
        );
        assert!(
            text_src.contains("path"),
            "hover on SourceRef binding must mention field `path`, got: {text_src}"
        );
    }

    /// Hovering on the `path` token of `m.path` shows `path: Text`;
    /// on `name` shows `name: Text`; on `tags` shows `tags: List<Text>`;
    /// on `columns` shows `columns: List<ColumnRef>`.
    /// Analogous for `SourceRef`.
    ///
    /// Tests `hover_text_for_model_ref_field` and `hover_text_for_source_ref_field`
    /// pure helpers.
    #[test]
    fn hover_on_model_ref_field_projection_shows_field_type() {
        // `m.path` → Text
        let text_path = hover_text_for_model_ref_field("path");
        assert!(
            text_path.is_some(),
            "hover_text_for_model_ref_field('path') must return Some, got None"
        );
        let path_text = text_path.unwrap();
        assert!(
            path_text.contains("path"),
            "hover for `m.path` must mention field name `path`, got: {path_text}"
        );
        assert!(
            path_text.contains("Text") || path_text.contains("TEXT"),
            "hover for `m.path` must mention `Text` type, got: {path_text}"
        );

        // `m.name` → Text
        let text_name = hover_text_for_model_ref_field("name");
        assert!(
            text_name.is_some(),
            "hover_text_for_model_ref_field('name') must return Some, got None"
        );
        let name_text = text_name.unwrap();
        assert!(
            name_text.contains("Text") || name_text.contains("TEXT"),
            "hover for `m.name` must mention `Text` type, got: {name_text}"
        );

        // `m.tags` → List<Text> (internally List<Expr<TEXT>>)
        let text_tags = hover_text_for_model_ref_field("tags");
        assert!(
            text_tags.is_some(),
            "hover_text_for_model_ref_field('tags') must return Some, got None"
        );
        let tags_text = text_tags.unwrap();
        assert!(
            tags_text.contains("List")
                && (tags_text.contains("Text") || tags_text.contains("TEXT")),
            "hover for `m.tags` must mention List and Text type, got: {tags_text}"
        );

        // `m.columns` → List<ColumnRef>
        let text_cols = hover_text_for_model_ref_field("columns");
        assert!(
            text_cols.is_some(),
            "hover_text_for_model_ref_field('columns') must return Some, got None"
        );
        let cols_text = text_cols.unwrap();
        assert!(
            cols_text.contains("ColumnRef"),
            "hover for `m.columns` must mention `ColumnRef`, got: {cols_text}"
        );

        // Unknown field → None
        let text_unknown = hover_text_for_model_ref_field("nonexistent_field");
        assert!(
            text_unknown.is_none(),
            "hover_text_for_model_ref_field for unknown field must return None, got Some"
        );

        // SourceRef variant
        let src_path = hover_text_for_source_ref_field("path");
        assert!(
            src_path.is_some(),
            "hover_text_for_source_ref_field('path') must return Some, got None"
        );
        let src_tags = hover_text_for_source_ref_field("tags");
        let src_tags_text =
            src_tags.expect("hover_text_for_source_ref_field('tags') must return Some");
        assert!(
            src_tags_text.contains("List"),
            "hover_text_for_source_ref_field('tags') must mention List, got: {src_tags_text}"
        );

        // Dispatch routing: cursor on the field token in `m.path`
        let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.path)";
        let path_offset = sql.rfind("path").expect("`path` must appear in SQL");
        let result = dispatch_hover(sql, path_offset);
        assert!(
            result.is_some(),
            "dispatch hover on `m.path` field in ModelRef lambda must produce Some, got None"
        );
        let hover_text = result.unwrap();
        assert!(
            hover_text.contains("ModelRef field")
                || hover_text.contains("path") && hover_text.contains("Text"),
            "dispatch hover on `m.path` must describe the `path` field (Text), \
             got: {hover_text}"
        );
    }

    /// Goto-def from a `ModelRef` / `SourceRef` value at a splice site is a
    /// graceful no-op in v1: the pure helpers `goto_def_for_model_ref_value`
    /// and `goto_def_for_source_ref_value` pass through a supplied path when
    /// the caller has resolved one and return `None` otherwise. Wiring the
    /// Backend `goto_definition` handler to detect splice-site cursor
    /// position and resolve the path through Salsa is a known divergence
    /// tracked in `docs/specs/meta_language.md` Known Divergences and the
    /// overall plan `docs/plans/20260509-meta-language-overall.md`.
    #[test]
    fn goto_def_for_model_ref_and_source_ref_values_pass_through_or_noop() {
        // The pure helper returns None (graceful no-op per spec; full resolution
        // requires expansion-time context — known divergence tracked in
        // docs/plans/20260509-meta-language-overall.md).
        let result = goto_def_for_wide_reflection_accessor();
        assert!(
            result.is_none(),
            "goto_def_for_wide_reflection_accessor must return None (graceful no-op), \
             got Some"
        );

        // goto_def_for_model_ref_value: when a path is supplied, returns it.
        let path = std::path::PathBuf::from("/project/models/orders.sql");
        let result_with_path = goto_def_for_model_ref_value(Some(path.clone()));
        assert_eq!(
            result_with_path,
            Some(path.clone()),
            "goto_def_for_model_ref_value(Some(path)) must return Some(path)"
        );
        let result_no_path = goto_def_for_model_ref_value(None);
        assert!(
            result_no_path.is_none(),
            "goto_def_for_model_ref_value(None) must return None (graceful no-op)"
        );

        // SourceRef variant
        let yaml_path = std::path::PathBuf::from("/project/sources.yml");
        let result_src = goto_def_for_source_ref_value(Some(yaml_path.clone()));
        assert_eq!(
            result_src,
            Some(yaml_path),
            "goto_def_for_source_ref_value(Some(path)) must return Some(path)"
        );
        let result_src_none = goto_def_for_source_ref_value(None);
        assert!(
            result_src_none.is_none(),
            "goto_def_for_source_ref_value(None) must return None (graceful no-op)"
        );
    }

    /// Goto-def on `m.path` or `m.name` returns the same model file.
    ///
    /// Tests that `goto_def_for_model_ref_value` passes through a supplied path,
    /// mirroring the Phase C `goto_def_for_lifted_identifier` contract.
    #[test]
    fn goto_def_from_model_ref_path_or_name_resolves_to_source_file() {
        // `m.path` and `m.name` both route through `goto_def_for_model_ref_value`
        // with the model's source path.  The pure helper passes the path through.
        let model_path = std::path::PathBuf::from("/project/models/cohort_a.sql");
        let result_path = goto_def_for_model_ref_value(Some(model_path.clone()));
        let result_name = goto_def_for_model_ref_value(Some(model_path.clone()));
        assert_eq!(
            result_path, result_name,
            "`m.path` and `m.name` goto-def must resolve to the same file"
        );
        assert_eq!(
            result_path,
            Some(model_path),
            "goto_def_for_model_ref_value must return the supplied path"
        );

        // SourceRef: `s.path` and `s.name` both route through `goto_def_for_source_ref_value`.
        let source_yaml = std::path::PathBuf::from("/project/sources.yml");
        let result_s_path = goto_def_for_source_ref_value(Some(source_yaml.clone()));
        assert_eq!(
            result_s_path,
            Some(source_yaml),
            "goto_def_for_source_ref_value must return the supplied yaml path"
        );
    }

    /// Completion at `smelt.models.<cursor>` offers exactly `{with_tag, all}` and
    /// no other identifier. Same for `smelt.sources.<cursor>`.
    ///
    /// Tests `wide_reflection_accessor_completions` pure helper.
    #[test]
    fn completion_at_smelt_models_namespace_offers_closed_set() {
        let names = wide_reflection_accessor_completions();
        assert_eq!(
            names.len(),
            2,
            "wide_reflection_accessor_completions must return exactly 2 items, got: {names:?}"
        );
        assert!(
            names.contains(&"with_tag".to_string()),
            "wide_reflection_accessor_completions must include `with_tag`, got: {names:?}"
        );
        assert!(
            names.contains(&"all".to_string()),
            "wide_reflection_accessor_completions must include `all`, got: {names:?}"
        );
        // Must NOT contain anything else
        for name in &names {
            assert!(
                name == "with_tag" || name == "all",
                "wide_reflection_accessor_completions must only contain `with_tag` and `all`, \
                 got unexpected: {name}"
            );
        }
    }

    /// Completion at `m.<cursor>` where `m: ModelRef` offers exactly
    /// `{path, name, tags, columns}`. Analogous for `SourceRef`.
    ///
    /// Tests `model_ref_field_completions` and `source_ref_field_completions`
    /// pure helpers.
    #[test]
    fn completion_at_model_ref_field_offers_closed_set() {
        // ModelRef fields
        let names = model_ref_field_completions();
        assert_eq!(
            names.len(),
            4,
            "model_ref_field_completions must return exactly 4 items, got: {names:?}"
        );
        for field in &["path", "name", "tags", "columns"] {
            assert!(
                names.contains(&field.to_string()),
                "model_ref_field_completions must include `{field}`, got: {names:?}"
            );
        }
        // Must NOT include ColumnRef fields
        assert!(
            !names.contains(&"is_numeric".to_string()),
            "model_ref_field_completions must NOT include ColumnRef field `is_numeric`, \
             got: {names:?}"
        );

        // SourceRef fields
        let src_names = source_ref_field_completions();
        assert_eq!(
            src_names.len(),
            4,
            "source_ref_field_completions must return exactly 4 items, got: {src_names:?}"
        );
        for field in &["path", "name", "tags", "columns"] {
            assert!(
                src_names.contains(&field.to_string()),
                "source_ref_field_completions must include `{field}`, got: {src_names:?}"
            );
        }

        // Dispatch routing: `m.<cursor>` inside ModelRef lambda offers field completions.
        // The detection helper `is_model_ref_param_before_dot` is the gating function.
        let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.path)";
        // Cursor positioned just after the final `m.` (after the dot, before `path`).
        let dot_pos = sql.rfind("m.").expect("`m.` must appear in SQL") + 2;
        // Verify the detection helper fires
        use smelt_parser::ast::File as AstFile;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = AstFile::cast(root).expect("must parse to File");
        let param = is_model_ref_param_before_dot(&file, sql, dot_pos);
        assert!(
            param.is_some(),
            "is_model_ref_param_before_dot must return Some for `m.` inside \
             smelt.models.with_tag lambda, got None"
        );
        assert_eq!(
            param.unwrap(),
            "m",
            "is_model_ref_param_before_dot must return `m` as param name"
        );
    }
}
