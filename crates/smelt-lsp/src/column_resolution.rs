//! Column tracing for goto-definition and hover.
//!
//! Resolves a column reference at the cursor through CTE/source/model chains
//! to its definition location(s). Used by `Backend::goto_definition` and
//! hover handlers.

use std::path::PathBuf;

use smelt_db::{Database, Workspace};
use smelt_parser::ast::File as AstFile;
use smelt_types::TypedColumn;

use crate::db_helpers::{file_project_root, lookup_file, lookup_project, resolve_ref_path};

pub(crate) fn format_type(typed_col: &TypedColumn) -> String {
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
pub(crate) struct ColumnDefLocation {
    pub(crate) path: PathBuf,
    pub(crate) line: u32,
    pub(crate) col: u32,
    pub(crate) end_line: u32,
    pub(crate) end_col: u32,
}

/// Resolve a column reference to its definition location(s).
///
/// Traces through wildcard (`SELECT *`) chains until finding an explicit column definition.
/// Returns multiple locations for ambiguous (unqualified) columns.
pub(crate) fn resolve_column_definitions(
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
    let project_root = file_project_root(db, current_path);

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
        &project_root,
        effective_qualifier,
        column_name,
        &ctx,
        &mut locations,
    );

    // Check model columns (from smelt.ref() sources)
    find_column_in_models(
        db,
        current_path,
        &project_root,
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
    project_root: &std::path::Path,
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
                        let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                            &text,
                            item.range(),
                        );
                        locations.push(ColumnDefLocation {
                            path: current_path.to_path_buf(),
                            line: pr.start.line,
                            col: pr.start.character,
                            end_line: pr.end.line,
                            end_col: pr.end.character,
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
                        let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                            &text,
                            item.range(),
                        );
                        locations.push(ColumnDefLocation {
                            path: current_path.to_path_buf(),
                            line: pr.start.line,
                            col: pr.start.character,
                            end_line: pr.end.line,
                            end_col: pr.end.character,
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
                                if let Some(upstream_path) =
                                    resolve_ref_path(db, project_root, &model_name)
                                {
                                    find_column_in_model_chain(
                                        db,
                                        &upstream_path,
                                        project_root,
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
    project_root: &std::path::Path,
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
        if let Some(upstream_path) = resolve_ref_path(db, project_root, model_name) {
            if find_column_in_model_chain(
                db,
                &upstream_path,
                project_root,
                column_name,
                10,
                locations,
            ) && qualifier.is_some()
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
    project_root: &std::path::Path,
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
            let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(&text, col.range);
            locations.push(ColumnDefLocation {
                path: model_path.to_path_buf(),
                line: pr.start.line,
                col: pr.start.character,
                end_line: pr.end.line,
                end_col: pr.end.character,
            });
            return true;
        }
    }

    // If not found in explicit columns, check wildcard extensions
    for ext in &schema.row_extensions {
        if let Some(upstream_path) = resolve_ref_path(db, project_root, &ext.ref_name) {
            if find_column_in_model_chain(
                db,
                &upstream_path,
                project_root,
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
pub(crate) fn collect_from_model_names(db: &Database, path: &std::path::Path) -> Vec<String> {
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
pub(crate) fn trace_upstream_column(
    db: &Database,
    all_files: &[PathBuf],
    project_root: &std::path::Path,
    model_name: &str,
    column_name: &str,
    edits: &mut Vec<(PathBuf, u32, u32, u32, u32)>,
) {
    for upstream_path in all_files.iter() {
        // Project isolation: only match files under the caller's project root.
        if !upstream_path.starts_with(project_root) {
            continue;
        }
        let up_name = upstream_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if up_name == model_name {
            trace_upstream_column_chain(db, upstream_path, project_root, column_name, 10, edits);
            break;
        }
    }
}

/// Recursively trace a column definition through upstream models,
/// following wildcard (SELECT *) chains.
fn trace_upstream_column_chain(
    db: &Database,
    model_path: &std::path::Path,
    project_root: &std::path::Path,
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
            let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(&up_text, def_range);
            edits.push((
                model_path.to_path_buf(),
                r.start.line,
                r.start.character,
                r.end.line,
                r.end.character,
            ));
            return true;
        }

        // Check wildcard extensions (SELECT *)
        let schema = smelt_db::model_schema(db, up_file_input);
        for ext in &schema.row_extensions {
            if let Some(upstream_path) = resolve_ref_path(db, project_root, &ext.ref_name) {
                if trace_upstream_column_chain(
                    db,
                    &upstream_path,
                    project_root,
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
pub(crate) fn build_python_context(all_files: &[PathBuf], config: &smelt_core::Config) -> String {
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
