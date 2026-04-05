//! Integration tests for smelt-lsp
//!
//! These tests verify LSP functionality by directly testing the smelt-db
//! Database queries that power the LSP features.

use std::path::PathBuf;
use std::sync::Arc;

use smelt_db::{
    yaml_edits::{find_source_column_yaml_rename, find_source_table_yaml_rename},
    Database, DiagnosticSeverity, Inputs, Schema, Semantic, Syntax, TypeChecking,
};
use smelt_parser::is_valid_sql_identifier;
use tempfile::TempDir;

/// Result of a model rename operation
#[derive(Default)]
struct RenameModelResult {
    /// Text edits: (file_path, start_line, start_col, end_line, end_col)
    text_edits: Vec<(PathBuf, u32, u32, u32, u32)>,
    /// File rename: (old_path, new_path)
    file_rename: Option<(PathBuf, PathBuf)>,
    /// Whether a name conflict was detected
    conflict: bool,
}

/// Result of a column rename operation
#[derive(Default)]
struct RenameColumnResult {
    /// Text edits within the same model: (start_line, start_col, end_line, end_col)
    local_edits: Vec<(u32, u32, u32, u32)>,
    /// Text edits in other files: (file_path, start_line, start_col, end_line, end_col)
    cross_file_edits: Vec<(PathBuf, u32, u32, u32, u32)>,
    /// YAML edit: (line_number, old_line_text, new_line_text)
    yaml_edit: Option<(u32, String, String)>,
    /// Error message if rename is rejected
    error: Option<String>,
}

/// Result of a source table rename operation
#[derive(Default)]
struct RenameSourceResult {
    /// Text edits in SQL files: (file_path, start_line, start_col, end_line, end_col)
    sql_edits: Vec<(PathBuf, u32, u32, u32, u32)>,
    /// YAML edit: (line_number, old_line_text, new_line_text)
    yaml_edit: Option<(u32, String, String)>,
}

/// Unified code action representation for handler-level testing.
/// Captures the title and kind (quickfix, refactor.extract, refactor.inline) that the
/// LSP handler would return, without depending on LSP protocol types.
#[derive(Debug, Clone)]
struct HandlerCodeAction {
    title: String,
    kind: String,
}

/// Test workspace that simulates a smelt project
struct TestWorkspace {
    #[allow(dead_code)]
    temp_dir: TempDir,
    db: Database,
    models_dir: PathBuf,
    model_files: Vec<PathBuf>,
}

impl TestWorkspace {
    /// Create a new empty test workspace
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let models_dir = temp_dir.path().join("models");
        std::fs::create_dir_all(&models_dir).expect("Failed to create models directory");

        let project_root = temp_dir.path().to_path_buf();
        let mut db = Database::default();
        db.set_all_files(Arc::new(Vec::new()));
        db.set_project_sources_yaml(project_root.clone(), Arc::new(String::new()));
        db.set_all_project_roots(Arc::new(vec![project_root]));

        Self {
            temp_dir,
            db,
            models_dir,
            model_files: Vec::new(),
        }
    }

    fn project_root(&self) -> PathBuf {
        self.temp_dir.path().to_path_buf()
    }

    /// Add a model file to the workspace
    fn add_model(&mut self, name: &str, sql: &str) {
        let path = self.models_dir.join(format!("{}.sql", name));

        // Write file to disk (for realistic testing)
        std::fs::write(&path, sql).expect("Failed to write model file");

        // Update database
        self.db
            .set_file_text(path.clone(), Arc::new(sql.to_string()));
        self.db
            .set_file_project_root(path.clone(), self.project_root());
        self.model_files.push(path);
        self.db.set_all_files(Arc::new(self.model_files.clone()));
    }

    /// Update an existing model file
    fn update_model(&mut self, name: &str, sql: &str) {
        let path = self.models_dir.join(format!("{}.sql", name));

        // Write file to disk
        std::fs::write(&path, sql).expect("Failed to write model file");

        // Update database
        self.db.set_file_text(path, Arc::new(sql.to_string()));
    }

    /// Set sources.yml content
    fn set_sources_yml(&mut self, content: &str) {
        let path = self.temp_dir.path().join("sources.yml");
        std::fs::write(&path, content).expect("Failed to write sources.yml");
        self.db
            .set_project_sources_yaml(self.project_root(), Arc::new(content.to_string()));
    }

    /// Get the path for a model
    fn model_path(&self, name: &str) -> PathBuf {
        self.models_dir.join(format!("{}.sql", name))
    }

    /// Simulate the full code_action handler: diagnostic-based actions via generate_all_code_actions
    /// plus cursor-based CTE refactorings (extract/inline). Returns a unified representation.
    fn handler_code_actions_at(&self, model: &str, line: u32, col: u32) -> Vec<HandlerCodeAction> {
        use smelt_db::code_actions::{
            find_extract_cte_suggestion, find_inline_cte_suggestion, CodeActionKind,
        };

        let path = self.model_path(model);
        let mut all_diags = (*self.db.file_diagnostics(path.clone())).clone();
        all_diags.extend((*self.db.type_diagnostics(path.clone())).clone());

        let text = self.db.file_text(path);
        let sources_yml_content = (*self.db.project_sources_yaml(self.project_root())).clone();

        // Filter diagnostics to those overlapping the cursor position
        let matching: Vec<_> = all_diags
            .into_iter()
            .filter(|d| {
                let r = &d.range;
                (r.start.line < line || (r.start.line == line && r.start.column <= col))
                    && (r.end.line > line || (r.end.line == line && r.end.column >= col))
            })
            .collect();

        let mut actions = Vec::new();

        // Diagnostic-based actions
        for diag in &matching {
            for kind in
                smelt_db::code_actions::generate_all_code_actions(diag, &text, &sources_yml_content)
            {
                let action = match kind {
                    CodeActionKind::TextEdit(s) => HandlerCodeAction {
                        title: s.title,
                        kind: "quickfix".to_string(),
                    },
                    CodeActionKind::CreateModel(s) => HandlerCodeAction {
                        title: s.title,
                        kind: "quickfix".to_string(),
                    },
                    CodeActionKind::YamlEdit(s) => HandlerCodeAction {
                        title: s.title,
                        kind: "quickfix".to_string(),
                    },
                };
                actions.push(action);
            }
        }

        // Cursor-based CTE refactorings
        if let Some(result) = find_extract_cte_suggestion(&text, line, col) {
            actions.push(HandlerCodeAction {
                title: result.title,
                kind: "refactor.extract".to_string(),
            });
        }
        if let Some(result) = find_inline_cte_suggestion(&text, line, col) {
            actions.push(HandlerCodeAction {
                title: result.title,
                kind: "refactor.inline".to_string(),
            });
        }

        actions
    }

    /// Get code actions at a position in a model.
    /// Collects diagnostics overlapping the position and generates code actions from them.
    fn code_actions_at(
        &self,
        model: &str,
        line: u32,
        col: u32,
    ) -> Vec<smelt_db::code_actions::CodeActionSuggestion> {
        let path = self.model_path(model);
        let mut all_diags = (*self.db.file_diagnostics(path.clone())).clone();
        all_diags.extend((*self.db.type_diagnostics(path.clone())).clone());

        let text = self.db.file_text(path);

        // Filter diagnostics to those overlapping the cursor position
        let matching: Vec<_> = all_diags
            .into_iter()
            .filter(|d| {
                let r = &d.range;
                (r.start.line < line || (r.start.line == line && r.start.column <= col))
                    && (r.end.line > line || (r.end.line == line && r.end.column >= col))
            })
            .collect();

        let mut actions = Vec::new();
        for diag in &matching {
            actions.extend(smelt_db::code_actions::generate_code_actions(diag, &text));
        }
        actions
    }

    /// Get all code action kinds (text edits, create model, YAML edits) at a position.
    /// Uses generate_all_code_actions which handles UndefinedModelRef, UndefinedSource, etc.
    fn all_code_actions_at(
        &self,
        model: &str,
        line: u32,
        col: u32,
    ) -> Vec<smelt_db::code_actions::CodeActionKind> {
        let path = self.model_path(model);
        let mut all_diags = (*self.db.file_diagnostics(path.clone())).clone();
        all_diags.extend((*self.db.type_diagnostics(path.clone())).clone());

        let text = self.db.file_text(path);
        let sources_yml_content = (*self.db.project_sources_yaml(self.project_root())).clone();

        // Filter diagnostics to those overlapping the cursor position
        let matching: Vec<_> = all_diags
            .into_iter()
            .filter(|d| {
                let r = &d.range;
                (r.start.line < line || (r.start.line == line && r.start.column <= col))
                    && (r.end.line > line || (r.end.line == line && r.end.column >= col))
            })
            .collect();

        let mut actions = Vec::new();
        for diag in &matching {
            actions.extend(smelt_db::code_actions::generate_all_code_actions(
                diag,
                &text,
                &sources_yml_content,
            ));
        }
        actions
    }

    /// Find all references to a symbol at a position.
    /// Uses symbol_at_cursor + pure reference functions from smelt-db.
    #[allow(dead_code)]
    fn references_for(&self, model: &str, line: u32, col: u32) -> Vec<(PathBuf, (u32, u32))> {
        use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

        let path = self.model_path(model);
        let text = self.db.file_text(path.clone());
        let parse = self.db.parse_file(path.clone());
        let syntax = parse.syntax();

        let file = match smelt_parser::ast::File::cast(syntax) {
            Some(f) => f,
            None => return vec![],
        };

        let offset = position_to_offset(&text, line, col);
        match symbol_at_cursor(&file, &text, offset) {
            Some(SymbolAtCursor::RefCall { name }) => {
                let all_file_refs: Vec<_> = self
                    .db
                    .all_files()
                    .iter()
                    .map(|p| (p.clone(), (*self.db.model_refs(p.clone())).clone()))
                    .collect();
                smelt_db::references::find_model_references(&name, &all_file_refs)
                    .into_iter()
                    .map(|(p, r)| (p, (r.start.line, r.start.column)))
                    .collect()
            }
            Some(SymbolAtCursor::SourceCall {
                source_name,
                table_name,
            }) => {
                let qualified = format!("{}.{}", source_name, table_name);
                let all_file_sources: Vec<_> = self
                    .db
                    .all_files()
                    .iter()
                    .map(|p| (p.clone(), (*self.db.model_sources(p.clone())).clone()))
                    .collect();
                smelt_db::references::find_source_references(&qualified, &all_file_sources)
                    .into_iter()
                    .map(|(p, r)| (p, (r.start.line, r.start.column)))
                    .collect()
            }
            Some(SymbolAtCursor::CteDefinition { name })
            | Some(SymbolAtCursor::CteReference { name }) => {
                let cte_refs = smelt_db::references::find_cte_references(&file, &text, &name);
                cte_refs
                    .into_iter()
                    .map(|text_range| {
                        let r = smelt_parser::ast::text_range_to_range(&text, text_range);
                        (path.clone(), (r.start.line, r.start.column))
                    })
                    .collect()
            }
            _ => vec![],
        }
    }

    /// Check if a position is renamable and return the range of the symbol.
    /// Returns None if the cursor is not on a renamable symbol.
    fn prepare_rename(&self, model: &str, line: u32, col: u32) -> Option<(u32, u32, u32, u32)> {
        use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

        let path = self.model_path(model);
        let text = self.db.file_text(path.clone());
        let parse = self.db.parse_file(path);
        let syntax = parse.syntax();

        let file = smelt_parser::ast::File::cast(syntax)?;
        let offset = position_to_offset(&text, line, col);

        match symbol_at_cursor(&file, &text, offset) {
            Some(SymbolAtCursor::CteDefinition { name })
            | Some(SymbolAtCursor::CteReference { name }) => {
                // Find the CTE definition range for prepareRename
                let select_stmt = file.select_stmt()?;
                let with_clause = select_stmt.with_clause()?;
                for cte in with_clause.ctes() {
                    if cte.name().as_deref() == Some(&name) {
                        let name_range = cte.name_range()?;
                        let r = smelt_parser::ast::text_range_to_range(&text, name_range);
                        return Some((r.start.line, r.start.column, r.end.line, r.end.column));
                    }
                }
                None
            }
            Some(SymbolAtCursor::RefCall { name }) => {
                // For ref calls, return the content range (inside quotes)
                for ref_call in file.refs() {
                    if ref_call.model_name().as_deref() == Some(name.as_str()) {
                        if let Some(content_range) = ref_call.content_range() {
                            let r = smelt_parser::ast::text_range_to_range(&text, content_range);
                            return Some((r.start.line, r.start.column, r.end.line, r.end.column));
                        }
                    }
                }
                None
            }
            Some(SymbolAtCursor::SourceCall {
                source_name,
                table_name,
            }) => {
                // For source calls, return the table_name_range (just the table part)
                for source_call in file.sources() {
                    if source_call.source_name().as_deref() == Some(source_name.as_str())
                        && source_call.table_name().as_deref() == Some(table_name.as_str())
                    {
                        if let Some(tn_range) = source_call.table_name_range() {
                            let r = smelt_parser::ast::text_range_to_range(&text, tn_range);
                            return Some((r.start.line, r.start.column, r.end.line, r.end.column));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Rename a model across the project.
    /// Returns a RenameModelResult with text edits across files and the file rename info.
    fn rename_model(&self, model: &str, line: u32, col: u32, new_name: &str) -> RenameModelResult {
        use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

        let path = self.model_path(model);
        let text = self.db.file_text(path.clone());
        let parse = self.db.parse_file(path.clone());
        let syntax = parse.syntax();

        let file = match smelt_parser::ast::File::cast(syntax) {
            Some(f) => f,
            None => return RenameModelResult::default(),
        };

        let offset = position_to_offset(&text, line, col);
        let model_name = match symbol_at_cursor(&file, &text, offset) {
            Some(SymbolAtCursor::RefCall { name }) => name,
            _ => return RenameModelResult::default(),
        };

        // Check for conflict: does a model with new_name already exist?
        let new_path = self.models_dir.join(format!("{}.sql", new_name));
        if self.model_files.contains(&new_path) {
            return RenameModelResult {
                conflict: true,
                ..Default::default()
            };
        }

        // Collect all ref edits across files
        let all_file_refs: Vec<_> = self
            .db
            .all_files()
            .iter()
            .map(|p| (p.clone(), (*self.db.model_refs(p.clone())).clone()))
            .collect();
        let ref_locations =
            smelt_db::references::find_model_references(&model_name, &all_file_refs);

        // For each ref location, we need the content range (inside quotes)
        let mut text_edits: Vec<(PathBuf, u32, u32, u32, u32)> = Vec::new();
        for (ref_path, _) in &ref_locations {
            let ref_text = self.db.file_text(ref_path.clone());
            let ref_parse = self.db.parse_file(ref_path.clone());
            let ref_syntax = ref_parse.syntax();
            if let Some(ref_file) = smelt_parser::ast::File::cast(ref_syntax) {
                for ref_call in ref_file.refs() {
                    if ref_call.model_name().as_deref() == Some(&model_name) {
                        if let Some(content_range) = ref_call.content_range() {
                            let r =
                                smelt_parser::ast::text_range_to_range(&ref_text, content_range);
                            text_edits.push((
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

        // File rename
        let old_path = self.models_dir.join(format!("{}.sql", model_name));
        let file_rename = if self.model_files.contains(&old_path) {
            Some((old_path, new_path))
        } else {
            None
        };

        RenameModelResult {
            text_edits,
            file_rename,
            conflict: false,
        }
    }

    /// Rename a CTE symbol at a position.
    /// Returns a list of (start_line, start_col, end_line, end_col, new_text) edits.
    fn rename_cte(
        &self,
        model: &str,
        line: u32,
        col: u32,
        new_name: &str,
    ) -> Vec<(u32, u32, u32, u32, String)> {
        use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

        let path = self.model_path(model);
        let text = self.db.file_text(path.clone());
        let parse = self.db.parse_file(path);
        let syntax = parse.syntax();

        let file = match smelt_parser::ast::File::cast(syntax) {
            Some(f) => f,
            None => return vec![],
        };

        let offset = position_to_offset(&text, line, col);
        let cte_name = match symbol_at_cursor(&file, &text, offset) {
            Some(SymbolAtCursor::CteDefinition { name })
            | Some(SymbolAtCursor::CteReference { name }) => name,
            _ => return vec![],
        };

        let refs = smelt_db::references::find_cte_references(&file, &text, &cte_name);
        refs.into_iter()
            .map(|text_range| {
                let r = smelt_parser::ast::text_range_to_range(&text, text_range);
                (
                    r.start.line,
                    r.start.column,
                    r.end.line,
                    r.end.column,
                    new_name.to_string(),
                )
            })
            .collect()
    }

    /// Rename a source table across the project.
    /// Returns a RenameSourceResult with SQL text edits and an optional YAML edit.
    fn rename_source(
        &self,
        model: &str,
        line: u32,
        col: u32,
        new_table_name: &str,
    ) -> RenameSourceResult {
        use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

        let path = self.model_path(model);
        let text = self.db.file_text(path.clone());
        let parse = self.db.parse_file(path);
        let syntax = parse.syntax();

        let file = match smelt_parser::ast::File::cast(syntax) {
            Some(f) => f,
            None => return RenameSourceResult::default(),
        };

        let offset = position_to_offset(&text, line, col);
        let (source_name, old_table_name) = match symbol_at_cursor(&file, &text, offset) {
            Some(SymbolAtCursor::SourceCall {
                source_name,
                table_name,
            }) => (source_name, table_name),
            _ => return RenameSourceResult::default(),
        };

        let qualified = format!("{}.{}", source_name, old_table_name);

        // Find all source() call sites across the project
        let all_file_sources: Vec<_> = self
            .db
            .all_files()
            .iter()
            .map(|p| (p.clone(), (*self.db.model_sources(p.clone())).clone()))
            .collect();
        let source_locations =
            smelt_db::references::find_source_references(&qualified, &all_file_sources);

        // For each source location, get the table_name_range (inside quotes, after the dot)
        let mut sql_edits = Vec::new();
        for (ref_path, _) in &source_locations {
            let ref_text = self.db.file_text(ref_path.clone());
            let ref_parse = self.db.parse_file(ref_path.clone());
            let ref_syntax = ref_parse.syntax();
            if let Some(ref_file) = smelt_parser::ast::File::cast(ref_syntax) {
                for source_call in ref_file.sources() {
                    if source_call.source_name().as_deref() == Some(&source_name)
                        && source_call.table_name().as_deref() == Some(&old_table_name)
                    {
                        if let Some(tn_range) = source_call.table_name_range() {
                            let r = smelt_parser::ast::text_range_to_range(&ref_text, tn_range);
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

        // Find and update the YAML entry
        let sources_yml = (*self.db.project_sources_yaml(self.project_root())).clone();
        let yaml_edit = find_source_table_yaml_rename(
            &sources_yml,
            &source_name,
            &old_table_name,
            new_table_name,
        );

        RenameSourceResult {
            sql_edits,
            yaml_edit,
        }
    }

    /// Rename a column at a position.
    /// Finds all references to the column within the current file (local) and across the project.
    /// Returns a RenameColumnResult with local edits, cross-file edits, and optional YAML edits.
    fn rename_column(
        &self,
        model: &str,
        line: u32,
        col: u32,
        new_name: &str,
    ) -> RenameColumnResult {
        use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

        let path = self.model_path(model);
        let text = self.db.file_text(path.clone());
        let parse = self.db.parse_file(path.clone());
        let syntax = parse.syntax();

        let file = match smelt_parser::ast::File::cast(syntax) {
            Some(f) => f,
            None => return RenameColumnResult::default(),
        };

        let offset = position_to_offset(&text, line, col);
        let (qualifier, column_name) = match symbol_at_cursor(&file, &text, offset) {
            Some(SymbolAtCursor::ColumnRef { qualifier, name }) => (qualifier, name),
            _ => return RenameColumnResult::default(),
        };

        // Validate new name
        if !is_valid_sql_identifier(new_name) {
            return RenameColumnResult {
                error: Some(format!("'{}' is not a valid SQL identifier", new_name)),
                ..Default::default()
            };
        }

        // Find all column references in the current file
        let local_refs = smelt_db::references::find_column_references_in_file(
            &file,
            &column_name,
            qualifier.as_deref(),
        );

        let mut local_edits: Vec<(u32, u32, u32, u32)> = local_refs
            .iter()
            .map(|r| {
                let range = smelt_parser::ast::text_range_to_range(&text, r.name_range);
                (
                    range.start.line,
                    range.start.column,
                    range.end.line,
                    range.end.column,
                )
            })
            .collect();

        // Also include the column definition in the SELECT list if it exists
        if let Some(def_range) =
            smelt_db::references::find_column_definition_in_select(&file, &column_name)
        {
            let range = smelt_parser::ast::text_range_to_range(&text, def_range);
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

        // Sort and dedup
        local_edits.sort();
        local_edits.dedup();

        // Cross-file column tracing via schema lineage
        let mut cross_file_edits = Vec::new();
        let schema = self.db.model_schema(path.clone());

        // Check if this column comes from an upstream model (trace upstream)
        if let Some(col) = schema.columns.iter().find(|c| c.name == column_name) {
            if let smelt_db::schema::ColumnSource::FromModel {
                model_name,
                column_name: upstream_col,
            } = &col.source
            {
                if upstream_col == &column_name {
                    // Find the upstream model file and its column references
                    for upstream_path in self.db.all_files().iter() {
                        let up_name = upstream_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        if up_name == model_name {
                            let up_text = self.db.file_text(upstream_path.clone());
                            let up_parse = self.db.parse_file(upstream_path.clone());
                            let up_syntax = up_parse.syntax();
                            if let Some(up_file) = smelt_parser::ast::File::cast(up_syntax) {
                                // Find column definition in upstream SELECT list
                                if let Some(def_range) =
                                    smelt_db::references::find_column_definition_in_select(
                                        &up_file,
                                        &column_name,
                                    )
                                {
                                    let r =
                                        smelt_parser::ast::text_range_to_range(&up_text, def_range);
                                    cross_file_edits.push((
                                        upstream_path.clone(),
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
            }
        }

        // Trace downstream transitively: BFS through the model graph.
        // A model is "downstream" if it refs a model that exposes this column
        // (either explicitly or through SELECT *).
        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // BFS: models whose column `column_name` should be renamed in downstream consumers
        let mut models_exposing_column: Vec<String> = vec![model_name.clone()];
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        visited.insert(model_name.clone());

        let mut depth = 0;
        while depth < 10 {
            let mut next_batch = Vec::new();
            for exposing_model in &models_exposing_column {
                for downstream_path in self.db.all_files().iter() {
                    if *downstream_path == path {
                        continue;
                    }
                    let down_name = downstream_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if visited.contains(&down_name) {
                        continue;
                    }

                    let down_refs = self.db.model_refs(downstream_path.clone());
                    let refs_exposing = down_refs.iter().any(|r| r.name == *exposing_model);
                    if !refs_exposing {
                        continue;
                    }

                    let down_text = self.db.file_text(downstream_path.clone());
                    let down_parse = self.db.parse_file(downstream_path.clone());
                    let down_syntax = down_parse.syntax();
                    if let Some(down_file) = smelt_parser::ast::File::cast(down_syntax) {
                        // Find explicit column references
                        let down_refs_found = smelt_db::references::find_column_references_in_file(
                            &down_file,
                            &column_name,
                            None,
                        );
                        for col_ref in &down_refs_found {
                            let r = smelt_parser::ast::text_range_to_range(
                                &down_text,
                                col_ref.name_range,
                            );
                            cross_file_edits.push((
                                downstream_path.clone(),
                                r.start.line,
                                r.start.column,
                                r.end.line,
                                r.end.column,
                            ));
                        }

                        // Check if this model passes the column through via SELECT *
                        // (RowExtension). If so, add to next BFS batch.
                        let down_schema = self.db.model_schema(downstream_path.clone());
                        let has_passthrough = down_schema
                            .row_extensions
                            .iter()
                            .any(|ext| ext.ref_name == *exposing_model);
                        if has_passthrough {
                            next_batch.push(down_name.clone());
                        }

                        visited.insert(down_name);
                    }
                }
            }
            if next_batch.is_empty() {
                break;
            }
            models_exposing_column = next_batch;
            depth += 1;
        }

        // Source column: check if column comes from a source and update YAML
        let yaml_edit = self.find_source_column_yaml_rename(&column_name, new_name);

        RenameColumnResult {
            local_edits,
            cross_file_edits,
            yaml_edit,
            error: None,
        }
    }

    /// Find a source column in sources.yml and produce the rename edit.
    fn find_source_column_yaml_rename(
        &self,
        old_column_name: &str,
        new_column_name: &str,
    ) -> Option<(u32, String, String)> {
        let sources_yml = (*self.db.project_sources_yaml(self.project_root())).clone();
        find_source_column_yaml_rename(&sources_yml, old_column_name, new_column_name)
    }
}

// =============================================================================
// Diagnostic Tests
// =============================================================================

mod diagnostics {
    use super::*;

    #[test]
    fn test_parse_error_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELEC * FROM table"); // Typo in SELECT

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert!(!diags.is_empty(), "Expected parse error diagnostic");
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn test_undefined_ref_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.ref('nonexistent')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert!(diags[0].message.contains("Undefined model"));
        assert!(diags[0].message.contains("nonexistent"));
    }

    #[test]
    fn test_valid_ref_produces_no_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 as id");
        ws.add_model("downstream", "SELECT * FROM smelt.ref('upstream')");

        let diags = ws.db.file_diagnostics(ws.model_path("downstream"));

        assert!(
            diags.is_empty(),
            "Expected no diagnostics for valid ref, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_undefined_source_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.source('raw.nonexistent')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert!(diags[0].message.contains("Undefined source"));
    }

    #[test]
    fn test_valid_source_produces_no_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
"#,
        );
        ws.add_model("model", "SELECT * FROM smelt.source('raw.users')");

        let diags = ws.db.file_diagnostics(ws.model_path("model"));

        assert!(
            diags.is_empty(),
            "Expected no diagnostics for valid source, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_diagnostic_position_accuracy() {
        let mut ws = TestWorkspace::new();
        // Position the ref on line 2 (0-indexed)
        ws.add_model(
            "broken",
            "-- Comment\nSELECT *\nFROM smelt.ref('nonexistent')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        // The ref is on line 2 (0-indexed)
        assert_eq!(diags[0].range.start.line, 2);
        assert_eq!(diags[0].range.end.line, 2);
    }

    #[test]
    fn test_multiple_errors_in_file() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "broken",
            "SELECT * FROM smelt.ref('missing1') CROSS JOIN smelt.ref('missing2')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 2, "Expected 2 undefined ref diagnostics");
        assert!(diags.iter().any(|d| d.message.contains("missing1")));
        assert!(diags.iter().any(|d| d.message.contains("missing2")));
    }

    #[test]
    fn test_diagnostic_has_code_undefined_ref() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.ref('nonexistent')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].code,
            Some(smelt_db::DiagnosticCode::UndefinedModelRef),
            "Undefined ref diagnostic should have UndefinedModelRef code"
        );
    }

    #[test]
    fn test_diagnostic_has_code_type_mismatch() {
        use smelt_db::DiagnosticCode;

        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model(
            "stg_users",
            "SELECT id, name FROM smelt.source('raw.users')",
        );
        // SUM(name) expects numeric but name is VARCHAR — triggers cross-model type mismatch
        ws.add_model(
            "bad_sum",
            "SELECT SUM(name) as total FROM smelt.ref('stg_users')",
        );

        // type_diagnostics checks cross-model type mismatches
        let diags = ws.db.type_diagnostics(ws.model_path("bad_sum"));

        let type_mismatch = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::TypeMismatch));
        assert!(
            type_mismatch.is_some(),
            "Expected a TypeMismatch diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_diagnostic_has_data_undefined_ref() {
        use smelt_db::DiagnosticData;

        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.ref('my_model')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].data,
            Some(DiagnosticData::UndefinedRef {
                model_name: "my_model".to_string(),
            }),
            "Undefined ref diagnostic should have UndefinedRef data with model name"
        );
    }

    #[test]
    fn test_diagnostic_has_data_undeclared_column() {
        use smelt_db::DiagnosticData;

        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        ws.add_model(
            "model",
            "SELECT nonexistent_col FROM smelt.source('raw.users')",
        );

        // Undeclared column checks are in type_diagnostics
        let diags = ws.db.type_diagnostics(ws.model_path("model"));

        let undeclared = diags
            .iter()
            .find(|d| matches!(&d.data, Some(DiagnosticData::UndeclaredColumn { .. })));
        assert!(
            undeclared.is_some(),
            "Expected an UndeclaredColumn diagnostic data, got: {:?}",
            diags
        );
        if let Some(DiagnosticData::UndeclaredColumn { column_name, .. }) =
            &undeclared.unwrap().data
        {
            assert_eq!(column_name, "nonexistent_col");
        }
    }
}

// =============================================================================
// Go-to-Definition Tests
// =============================================================================

mod goto_definition {
    use super::*;

    #[test]
    fn test_resolve_ref_to_model_path() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");
        ws.add_model("orders", "SELECT * FROM smelt.ref('users')");

        let resolved = ws.db.resolve_ref("users".to_string());

        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap(), ws.model_path("users"));
    }

    #[test]
    fn test_resolve_nonexistent_ref_returns_none() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");

        let resolved = ws.db.resolve_ref("nonexistent".to_string());

        assert!(resolved.is_none());
    }

    #[test]
    fn test_model_refs_extracts_ref_calls() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream1", "SELECT 1 as id");
        ws.add_model("upstream2", "SELECT 2 as id");
        ws.add_model(
            "downstream",
            "SELECT * FROM smelt.ref('upstream1') CROSS JOIN smelt.ref('upstream2')",
        );

        let refs = ws.db.model_refs(ws.model_path("downstream"));

        assert_eq!(refs.len(), 2);
        let ref_names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"upstream1"));
        assert!(ref_names.contains(&"upstream2"));
    }
}

// =============================================================================
// Hover Tests (Schema Information)
// =============================================================================

mod hover {
    use super::*;

    #[test]
    fn test_model_schema_extracts_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT id, name, email FROM raw.users");

        let schema = ws.db.model_schema(ws.model_path("users"));

        assert_eq!(schema.columns.len(), 3);
        let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"name"));
        assert!(col_names.contains(&"email"));
    }

    #[test]
    fn test_model_schema_with_aliases() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "stats",
            "SELECT user_id, COUNT(*) as total_count FROM events GROUP BY user_id",
        );

        let schema = ws.db.model_schema(ws.model_path("stats"));

        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].name, "user_id");
        assert_eq!(schema.columns[1].name, "total_count");
        assert_eq!(schema.columns[1].alias, Some("total_count".to_string()));
    }

    #[test]
    fn test_typed_schema_infers_literal_types() {
        let mut ws = TestWorkspace::new();
        ws.add_model("literals", "SELECT 42 as num, 'hello' as str FROM dual");

        let schema = ws.db.typed_model_schema(ws.model_path("literals"));

        assert_eq!(schema.columns.len(), 2);

        // First column should be numeric
        assert!(schema.columns[0].data_type.is_some());

        // Second column should be text
        assert!(schema.columns[1].data_type.is_some());
        assert_eq!(
            schema.columns[1].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::Text
        );
    }

    #[test]
    fn test_typed_schema_infers_aggregate_types() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "aggs",
            "SELECT COUNT(*) as cnt, SUM(amount) as total FROM orders",
        );

        let schema = ws.db.typed_model_schema(ws.model_path("aggs"));

        assert_eq!(schema.columns.len(), 2);

        // COUNT should be BigInt
        assert!(schema.columns[0].data_type.is_some());
        assert_eq!(
            schema.columns[0].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::BigInt
        );
    }

    #[test]
    fn test_available_columns_includes_upstream() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT id, name, email FROM raw.users");
        ws.add_model("orders", "SELECT user_id FROM smelt.ref('users')");

        let available = ws.db.available_columns(ws.model_path("orders"));

        // Should include columns from both the current model and upstream
        let col_names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"user_id")); // From current model
        assert!(col_names.contains(&"id")); // From upstream
        assert!(col_names.contains(&"name")); // From upstream
        assert!(col_names.contains(&"email")); // From upstream
    }
}

// =============================================================================
// Incremental Update Tests
// =============================================================================

mod incremental {
    use super::*;

    #[test]
    fn test_fixing_error_clears_diagnostic() {
        let mut ws = TestWorkspace::new();

        // Start with a broken model
        ws.add_model("model", "SELECT * FROM smelt.ref('missing')");
        let diags1 = ws.db.file_diagnostics(ws.model_path("model"));
        assert_eq!(diags1.len(), 1, "Should have undefined ref error");

        // Add the missing model
        ws.add_model("missing", "SELECT 1 as id");

        // Check diagnostics are cleared
        let diags2 = ws.db.file_diagnostics(ws.model_path("model"));
        assert!(
            diags2.is_empty(),
            "Diagnostics should be cleared after adding missing model"
        );
    }

    #[test]
    fn test_updating_model_triggers_reparse() {
        let mut ws = TestWorkspace::new();

        // Create a model with one column
        ws.add_model("model", "SELECT id FROM users");
        let schema1 = ws.db.model_schema(ws.model_path("model"));
        assert_eq!(schema1.columns.len(), 1);

        // Update to have two columns
        ws.update_model("model", "SELECT id, name FROM users");
        let schema2 = ws.db.model_schema(ws.model_path("model"));
        assert_eq!(schema2.columns.len(), 2);
    }

    #[test]
    fn test_adding_model_makes_it_available_for_refs() {
        let mut ws = TestWorkspace::new();

        // Create a downstream model first (with broken ref)
        ws.add_model("downstream", "SELECT * FROM smelt.ref('upstream')");
        assert!(
            ws.db.resolve_ref("upstream".to_string()).is_none(),
            "Upstream should not exist yet"
        );

        // Now add the upstream model
        ws.add_model("upstream", "SELECT 1 as id");

        // Verify ref now resolves
        assert!(
            ws.db.resolve_ref("upstream".to_string()).is_some(),
            "Upstream should now resolve"
        );
    }

    #[test]
    fn test_sources_yml_changes_affect_diagnostics() {
        let mut ws = TestWorkspace::new();

        // Model referencing a source that doesn't exist yet
        ws.add_model("model", "SELECT * FROM smelt.source('raw.users')");
        let diags1 = ws.db.file_diagnostics(ws.model_path("model"));
        assert_eq!(diags1.len(), 1, "Should have undefined source error");

        // Add the source
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
"#,
        );

        // Diagnostics should clear
        let diags2 = ws.db.file_diagnostics(ws.model_path("model"));
        assert!(
            diags2.is_empty(),
            "Diagnostics should clear after adding source"
        );
    }
}

// =============================================================================
// Source Resolution Tests
// =============================================================================

mod sources {
    use super::*;

    #[test]
    fn test_resolve_source_with_columns() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR(255)
"#,
        );

        let resolved =
            ws.db
                .resolve_source(ws.project_root(), "raw".to_string(), "users".to_string());

        assert!(resolved.is_some());
        let table = resolved.unwrap();
        assert_eq!(table.name, "users");
        assert_eq!(table.columns.len(), 2);
    }

    #[test]
    fn test_sources_config_parses_nested_format() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    database: analytics
    schema: public
    tables:
      users:
        columns:
          - name: id
      events:
        columns:
          - name: event_id
"#,
        );

        let config = ws.db.sources_config(ws.project_root());

        assert_eq!(config.sources.len(), 1);
        let raw = &config.sources[0];
        assert_eq!(raw.name, "raw");
        assert_eq!(raw.database, Some("analytics".to_string()));
        assert_eq!(raw.schema, Some("public".to_string()));
        assert_eq!(raw.tables.len(), 2);
    }

    #[test]
    fn test_model_sources_extracts_source_calls() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "model",
            "SELECT * FROM smelt.source('raw.users') CROSS JOIN smelt.source('raw.events')",
        );

        let sources = ws.db.model_sources(ws.model_path("model"));

        assert_eq!(sources.len(), 2);
        let qualified_names: Vec<&str> =
            sources.iter().map(|s| s.qualified_name.as_str()).collect();
        assert!(qualified_names.contains(&"raw.users"));
        assert!(qualified_names.contains(&"raw.events"));
    }

    #[test]
    fn test_malformed_source_without_dot_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        // Source call without dot separator (e.g., 'foo' instead of 'raw.users')
        ws.add_model("model", "SELECT * FROM smelt.source('foo')");

        let diags = ws.db.file_diagnostics(ws.model_path("model"));

        // Should produce an error for malformed/undefined source
        assert!(
            !diags.is_empty(),
            "Expected diagnostic for malformed source 'foo' without dot separator"
        );
    }
}

// =============================================================================
// Completion Tests (Alias Autocomplete)
// =============================================================================

mod completion {
    use super::*;

    #[test]
    fn test_type_context_registers_explicit_source_alias() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR
"#,
        );
        ws.add_model("model", "SELECT t.id FROM smelt.source('raw.users') AS t");

        // The type context should register the alias 't' -> 'raw.users'
        let ctx = ws.db.type_context(ws.model_path("model"));

        // Verify the alias is registered by looking up a column through it
        let col = ctx.lookup_column(Some("t"), "id");
        assert!(col.is_some(), "Should find column 'id' via alias 't'");
    }

    #[test]
    fn test_type_context_registers_implicit_source_alias() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        ws.add_model("model", "SELECT t.id FROM smelt.source('raw.users') t");

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Should find column via implicit alias
        let col = ctx.lookup_column(Some("t"), "id");
        assert!(
            col.is_some(),
            "Should find column 'id' via implicit alias 't'"
        );
    }

    #[test]
    fn test_type_context_registers_table_name_as_fallback_alias() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        // Using table name 'users' as qualifier (implicit alias by table name)
        ws.add_model("model", "SELECT users.id FROM smelt.source('raw.users')");

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Should find column via table name
        let col = ctx.lookup_column(Some("users"), "id");
        assert!(
            col.is_some(),
            "Should find column 'id' via table name 'users'"
        );
    }

    #[test]
    fn test_source_columns_available_for_completion() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR
          - name: created_at
            type: TIMESTAMP
"#,
        );
        ws.add_model(
            "model",
            "SELECT t.id, t.email FROM smelt.source('raw.users') AS t",
        );

        // Get sources config and verify columns are available
        let config = ws.db.sources_config(ws.project_root());
        let raw = config.sources.iter().find(|s| s.name == "raw").unwrap();
        let users = raw.tables.iter().find(|t| t.name == "users").unwrap();

        assert_eq!(users.columns.len(), 3);
        let col_names: Vec<&str> = users.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"email"));
        assert!(col_names.contains(&"created_at"));
    }

    #[test]
    fn test_model_columns_available_for_ref_alias() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT id, name, email FROM raw_users");
        ws.add_model(
            "downstream",
            "SELECT u.id, u.name FROM smelt.ref('users') AS u",
        );

        // The upstream model schema should have the columns
        let schema = ws.db.model_schema(ws.model_path("users"));
        assert_eq!(schema.columns.len(), 3);

        let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"name"));
        assert!(col_names.contains(&"email"));
    }

    #[test]
    fn test_join_aliases_both_registered() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
      orders:
        columns:
          - name: order_id
            type: INTEGER
          - name: user_id
            type: INTEGER
"#,
        );
        ws.add_model(
            "model",
            "SELECT u.id, o.order_id FROM smelt.source('raw.users') u JOIN smelt.source('raw.orders') o ON u.id = o.user_id",
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Both aliases should be registered
        let user_col = ctx.lookup_column(Some("u"), "id");
        assert!(user_col.is_some(), "Should find 'id' via alias 'u'");

        let order_col = ctx.lookup_column(Some("o"), "order_id");
        assert!(order_col.is_some(), "Should find 'order_id' via alias 'o'");
    }

    #[test]
    fn test_cte_names_available_for_from_completion() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
          - name: created_at
            type: TIMESTAMP
"#,
        );
        ws.add_model(
            "model",
            r#"WITH daily_totals AS (
    SELECT DATE(created_at) as day, SUM(amount) as total
    FROM smelt.source('raw.orders')
    GROUP BY DATE(created_at)
)
SELECT day, total FROM daily_totals WHERE total > 1000"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // CTE name should be registered
        assert!(ctx.is_cte("daily_totals"));

        // CTE columns should be available
        let columns = ctx.cte_columns("daily_totals");
        assert!(!columns.is_empty(), "CTE should have inferred columns");

        let col_names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
        assert!(col_names.contains(&"day"), "Should have 'day' column");
        assert!(col_names.contains(&"total"), "Should have 'total' column");
    }

    #[test]
    fn test_multiple_ctes_available_for_completion() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
      orders:
        columns:
          - name: user_id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
"#,
        );
        ws.add_model(
            "model",
            r#"WITH
active_users AS (
    SELECT id, name FROM smelt.source('raw.users')
),
user_orders AS (
    SELECT user_id, SUM(amount) as total FROM smelt.source('raw.orders') GROUP BY user_id
)
SELECT u.id, u.name, o.total
FROM active_users u
JOIN user_orders o ON u.id = o.user_id"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Both CTEs should be registered
        assert!(ctx.is_cte("active_users"));
        assert!(ctx.is_cte("user_orders"));

        // active_users columns
        let au_columns = ctx.cte_columns("active_users");
        let au_names: Vec<&str> = au_columns.iter().map(|(name, _)| *name).collect();
        assert!(au_names.contains(&"id"));
        assert!(au_names.contains(&"name"));

        // user_orders columns
        let uo_columns = ctx.cte_columns("user_orders");
        let uo_names: Vec<&str> = uo_columns.iter().map(|(name, _)| *name).collect();
        assert!(uo_names.contains(&"user_id"));
        assert!(uo_names.contains(&"total"));
    }

    #[test]
    fn test_python_model_ref_resolution() {
        // Simulate the LSP registering a Python model's generated SQL.
        // The virtual path must use `<name>.sql` so parse_model derives the correct name.
        let mut ws = TestWorkspace::new();

        // Register the Python model's SQL under the correct virtual path
        let virtual_path = ws.models_dir.join("combined_events.sql");
        let sql = "SELECT event_type, COUNT(*) as cnt FROM raw_events GROUP BY event_type";
        std::fs::write(&virtual_path, sql).expect("write virtual sql");
        ws.db
            .set_file_text(virtual_path.clone(), Arc::new(sql.to_string()));
        ws.db
            .set_file_project_root(virtual_path.clone(), ws.project_root());
        ws.model_files.push(virtual_path);
        ws.db.set_all_files(Arc::new(ws.model_files.clone()));

        // Add a SQL model that references the Python model
        ws.add_model(
            "event_summary",
            "SELECT * FROM smelt.ref('combined_events')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("event_summary"));
        assert!(
            diags.is_empty(),
            "Expected no diagnostics when Python model is registered with <name>.sql path, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_py_gen_prefix_breaks_ref_resolution() {
        // Regression guard: the old __py_gen__ naming scheme causes ref resolution failure
        let mut ws = TestWorkspace::new();

        // Register the Python model's SQL under the OLD broken virtual path
        let virtual_path = ws.models_dir.join("__py_gen__combined_events.sql");
        let sql = "SELECT event_type, COUNT(*) as cnt FROM raw_events GROUP BY event_type";
        std::fs::write(&virtual_path, sql).expect("write virtual sql");
        ws.db
            .set_file_text(virtual_path.clone(), Arc::new(sql.to_string()));
        ws.db
            .set_file_project_root(virtual_path.clone(), ws.project_root());
        ws.model_files.push(virtual_path);
        ws.db.set_all_files(Arc::new(ws.model_files.clone()));

        // A SQL model referencing 'combined_events' should NOT resolve
        ws.add_model(
            "event_summary",
            "SELECT * FROM smelt.ref('combined_events')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("event_summary"));
        assert!(
            !diags.is_empty(),
            "Expected undefined ref diagnostic with __py_gen__ prefix path"
        );
        assert!(diags[0].message.contains("Undefined model"));
    }

    #[test]
    fn test_recursive_cte_available_for_completion() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "model",
            r#"WITH RECURSIVE counter(n) AS (
    SELECT 1
    UNION ALL
    SELECT n + 1 FROM counter WHERE n < 10
)
SELECT n FROM counter"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Recursive CTE should be registered
        assert!(ctx.is_cte("counter"));

        // Should have explicit column from column list
        let columns = ctx.cte_columns("counter");
        let col_names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
        assert!(
            col_names.contains(&"n"),
            "Should have 'n' column from explicit list"
        );
    }
}

// =============================================================================
// Goto-Definition Extended Tests
// =============================================================================

mod goto_definition_extended {
    use super::*;

    #[test]
    fn test_source_resolves_to_table_def() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model("model", "SELECT id FROM smelt.source('raw.users')");

        let resolved =
            ws.db
                .resolve_source(ws.project_root(), "raw".to_string(), "users".to_string());
        assert!(resolved.is_some());
        let table_def = resolved.unwrap();
        assert_eq!(table_def.name, "users");
        assert_eq!(table_def.columns.len(), 2);
    }

    #[test]
    fn test_model_schema_columns_have_ranges() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 AS user_id, 'hello' AS user_name");

        let schema = ws.db.model_schema(ws.model_path("upstream"));
        assert_eq!(schema.columns.len(), 2);

        // Each column should have a non-zero range
        for col in &schema.columns {
            let start: usize = col.range.start().into();
            let end: usize = col.range.end().into();
            assert!(
                end > start,
                "Column '{}' should have a valid range",
                col.name
            );
        }

        assert_eq!(schema.columns[0].name, "user_id");
        assert_eq!(schema.columns[1].name, "user_name");
    }

    #[test]
    fn test_column_traced_through_single_ref() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model("orders", "SELECT user_id FROM smelt.ref('users')");

        // The downstream model's schema should have user_id with FromModel source
        let schema = ws.db.model_schema(ws.model_path("orders"));
        assert_eq!(schema.columns.len(), 1);
        assert_eq!(schema.columns[0].name, "user_id");

        // The upstream model should have user_id with a valid range
        let upstream_schema = ws.db.model_schema(ws.model_path("users"));
        let user_id_col = upstream_schema.find_column("user_id");
        assert!(user_id_col.is_some(), "Upstream should have user_id column");
    }

    #[test]
    fn test_wildcard_model_has_row_extensions() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id");
        ws.add_model("passthrough", "SELECT * FROM smelt.ref('users')");

        let schema = ws.db.model_schema(ws.model_path("passthrough"));
        // SELECT * should create row extensions, not explicit columns
        assert!(
            !schema.row_extensions.is_empty(),
            "Should have row extensions for SELECT *"
        );
        assert_eq!(schema.row_extensions[0].ref_name, "users");
    }

    #[test]
    fn test_resolved_schema_expands_wildcards() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model("passthrough", "SELECT * FROM smelt.ref('users')");

        let resolved = ws.db.resolved_model_schema(ws.model_path("passthrough"));
        assert!(resolved.is_fully_resolved);
        assert_eq!(resolved.columns.len(), 2);

        let col_names: Vec<&str> = resolved.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"user_id"));
        assert!(col_names.contains(&"user_name"));
    }

    #[test]
    fn test_column_through_wildcard_chain() {
        let mut ws = TestWorkspace::new();
        ws.add_model("base", "SELECT 1 AS col_a, 2 AS col_b");
        ws.add_model("middle", "SELECT * FROM smelt.ref('base')");
        ws.add_model("top", "SELECT col_a FROM smelt.ref('middle')");

        // The 'base' model should have col_a as an explicit column
        let base_schema = ws.db.model_schema(ws.model_path("base"));
        assert!(base_schema.find_column("col_a").is_some());

        // 'middle' has wildcard, so resolved schema should include col_a
        let middle_resolved = ws.db.resolved_model_schema(ws.model_path("middle"));
        let col_names: Vec<&str> = middle_resolved
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(col_names.contains(&"col_a"));

        // 'top' explicitly selects col_a
        let top_schema = ws.db.model_schema(ws.model_path("top"));
        assert_eq!(top_schema.columns.len(), 1);
        assert_eq!(top_schema.columns[0].name, "col_a");
    }

    #[test]
    fn test_cte_columns_available_in_context() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "model",
            r#"WITH totals AS (
    SELECT 1 AS total_count, 2 AS total_amount
)
SELECT total_count FROM totals"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));
        assert!(ctx.is_cte("totals"));

        let columns = ctx.cte_columns("totals");
        let col_names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
        assert!(col_names.contains(&"total_count"));
        assert!(col_names.contains(&"total_amount"));
    }

    #[test]
    fn test_source_column_available_in_context() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: INTEGER
"#,
        );
        ws.add_model("model", "SELECT event_id FROM smelt.source('raw.events')");

        let ctx = ws.db.type_context(ws.model_path("model"));
        // Should be able to look up source columns
        let result = ctx.lookup_column(Some("events"), "event_id");
        assert!(result.is_some(), "Should find event_id in source context");
    }

    #[test]
    fn test_model_sources_extracts_source_call_info() {
        let mut ws = TestWorkspace::new();
        ws.add_model("model", "SELECT event_id FROM smelt.source('raw.events') e");

        let sources = ws.db.model_sources(ws.model_path("model"));
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_name, "raw");
        assert_eq!(sources[0].table_name, "events");
        assert_eq!(sources[0].qualified_name, "raw.events");
    }

    #[test]
    fn test_cte_with_wildcard_resolves_upstream_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model(
            "model",
            r#"WITH user_cte AS (
    SELECT * FROM smelt.ref('users')
)
SELECT user_id FROM user_cte"#,
        );

        // The CTE should expose upstream columns through wildcard
        let ctx = ws.db.type_context(ws.model_path("model"));
        assert!(ctx.is_cte("user_cte"), "user_cte should be recognized");

        // The final model should resolve user_id
        let result = ctx.lookup_column(None, "user_id");
        assert!(result.is_some(), "Should find user_id through CTE wildcard");
    }

    #[test]
    fn test_cte_with_explicit_and_wildcard_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model("base", "SELECT 1 AS col_a, 2 AS col_b");
        ws.add_model(
            "model",
            r#"WITH enriched AS (
    SELECT *, 3 AS col_c FROM smelt.ref('base')
)
SELECT col_a, col_c FROM enriched"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));
        assert!(ctx.is_cte("enriched"));

        // col_c is explicit in the CTE
        let col_c = ctx.lookup_column(Some("enriched"), "col_c");
        assert!(col_c.is_some(), "Should find explicit col_c in CTE");

        // col_a comes through the wildcard
        let col_a = ctx.lookup_column(Some("enriched"), "col_a");
        assert!(
            col_a.is_some(),
            "Should find col_a through CTE wildcard from upstream"
        );
    }

    #[test]
    fn test_table_alias_in_type_context() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id");
        ws.add_model("model", "SELECT u.user_id FROM smelt.ref('users') AS u");

        let ctx = ws.db.type_context(ws.model_path("model"));
        // Alias 'u' should resolve to 'users'
        let resolved = ctx.resolve_alias("u");
        assert_eq!(resolved, Some("users".to_string()));

        // Should find user_id through the alias
        let result = ctx.lookup_column(Some("u"), "user_id");
        assert!(result.is_some(), "Should find user_id through alias 'u'");
    }
}

// =============================================================================
// Column Goto-Definition Tests
// =============================================================================
//
// These tests verify the expression-finding logic used by the goto-definition
// handler. The handler walks AST descendants to find the tightest Expr at the
// cursor, then calls as_column_ref() to extract the column reference.

mod column_goto_definition {
    use super::*;
    use smelt_parser::ast::{Expr, File as AstFile};

    /// Helper: find the tightest Expr at a byte offset, using the same logic
    /// as the goto-definition handler in main.rs.
    fn find_expr_at_offset(file: &AstFile, cursor_offset: usize) -> Option<Expr> {
        let mut best_expr: Option<Expr> = None;
        let mut best_len = usize::MAX;

        for node in file.syntax().descendants() {
            if let Some(expr) = Expr::cast(node) {
                let range = expr.text_range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();
                let len = end - start;

                if cursor_offset >= start && cursor_offset <= end && len <= best_len {
                    best_len = len;
                    best_expr = Some(expr);
                }
            }
        }

        best_expr
    }

    #[test]
    fn test_bare_column_in_select_resolves_column_ref() {
        // This is the core bug: bare `event_timestamp` in SELECT has the same
        // text range for SELECT_ITEM and inner EXPRESSION nodes. The fix
        // (len <= best_len) ensures the deeper EXPRESSION is selected.
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_timestamp
            type: TIMESTAMP
          - name: user_id
            type: INTEGER
"#,
        );
        ws.add_model(
            "model",
            "SELECT\n    event_timestamp\nFROM smelt.source('raw.events')",
        );

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        // Cursor on "event_timestamp" (byte offset within the identifier)
        let text = "SELECT\n    event_timestamp\nFROM smelt.source('raw.events')";
        let col_start = text.find("event_timestamp").unwrap();
        let cursor = col_start + 5; // middle of the identifier

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find an expression at cursor");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Expression should resolve to a ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "event_timestamp");
        assert!(col_ref.qualifier().is_none());
    }

    #[test]
    fn test_qualified_column_in_select_resolves_column_ref() {
        // Open question 1: qualified columns like e.event_timestamp
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_timestamp
            type: TIMESTAMP
"#,
        );
        ws.add_model(
            "model",
            "SELECT\n    e.event_timestamp\nFROM smelt.source('raw.events') e",
        );

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        let text = "SELECT\n    e.event_timestamp\nFROM smelt.source('raw.events') e";
        // Cursor on the column name part (after the dot)
        let col_start = text.find("event_timestamp").unwrap();
        let cursor = col_start + 3;

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find an expression at cursor");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Qualified expression should resolve to a ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "event_timestamp");
        assert_eq!(col_ref.qualifier(), Some("e"));
    }

    #[test]
    fn test_column_from_ref_model_resolves() {
        // Open question 2: columns from smelt.ref() models
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model("model", "SELECT\n    user_id\nFROM smelt.ref('users')");

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        let text = "SELECT\n    user_id\nFROM smelt.ref('users')";
        let col_start = text.find("user_id").unwrap();
        let cursor = col_start + 3;

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find an expression at cursor");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Column from ref model should resolve to ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "user_id");
        assert!(col_ref.qualifier().is_none());

        // Also verify the column can be found in the upstream model schema
        let upstream_schema = ws.db.model_schema(ws.model_path("users"));
        assert!(
            upstream_schema.find_column("user_id").is_some(),
            "Upstream model should have user_id column"
        );
    }

    #[test]
    fn test_column_in_where_clause_resolves() {
        // Open question 3: columns in WHERE should work because the parent
        // node (WHERE_CLAUSE) has a larger range than the EXPRESSION
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: is_active
            type: BOOLEAN
"#,
        );
        ws.add_model(
            "model",
            "SELECT event_id FROM smelt.source('raw.events') WHERE is_active",
        );

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        let text = "SELECT event_id FROM smelt.source('raw.events') WHERE is_active";
        let col_start = text.find("is_active").unwrap();
        let cursor = col_start + 3;

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find expression in WHERE clause");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Column in WHERE clause should resolve to ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "is_active");
        assert!(col_ref.qualifier().is_none());
    }
}

// =============================================================================
// Symbol At Cursor Tests (Phase 1)
// =============================================================================

mod symbol_at_cursor {
    use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

    /// Helper: parse text and call symbol_at_cursor at (line, col)
    fn resolve(sql: &str, line: u32, col: u32) -> Option<SymbolAtCursor> {
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let offset = position_to_offset(sql, line, col);
        symbol_at_cursor(&file, sql, offset)
    }

    #[test]
    fn test_symbol_at_cursor_ref_call() {
        let sql = "SELECT * FROM smelt.ref('users')";
        // Cursor inside the ref call (on 'users')
        let sym = resolve(sql, 0, 25);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::RefCall {
                name: "users".to_string()
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_source_call() {
        let sql = "SELECT * FROM smelt.source('raw.users')";
        // Cursor inside the source call
        let sym = resolve(sql, 0, 30);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::SourceCall {
                source_name: "raw".to_string(),
                table_name: "users".to_string(),
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_cte_reference() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM cte1";
        // Cursor on "cte1" in FROM clause (line 1, col 14)
        let sym = resolve(sql, 1, 14);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::CteReference {
                name: "cte1".to_string()
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_cte_definition() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM cte1";
        // Cursor on "cte1" in WITH clause (line 0, col 5)
        let sym = resolve(sql, 0, 5);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::CteDefinition {
                name: "cte1".to_string()
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_column_ref() {
        let sql = "SELECT t.user_id FROM orders t";
        // Cursor on "user_id" part of "t.user_id" (col 9)
        let sym = resolve(sql, 0, 9);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::ColumnRef {
                qualifier: Some("t".to_string()),
                name: "user_id".to_string(),
            })
        );
    }
}

// =============================================================================
// AST Range Helper Tests (Phase 1)
// =============================================================================

mod ast_range_helpers {
    #[test]
    fn test_cte_name_range() {
        let sql = "WITH my_cte AS (SELECT 1 as id)\nSELECT * FROM my_cte";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let select_stmt = file.select_stmt().unwrap();
        let with_clause = select_stmt.with_clause().unwrap();
        let cte = with_clause.ctes().next().unwrap();

        let name_range = cte.name_range();
        assert!(name_range.is_some(), "CTE should have a name range");
        let range = name_range.unwrap();
        let name_text = &sql[usize::from(range.start())..usize::from(range.end())];
        assert_eq!(name_text, "my_cte");
    }

    #[test]
    fn test_ref_content_range() {
        let sql = "SELECT * FROM smelt.ref('my_model')";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let ref_call = file.refs().next().unwrap();

        let content_range = ref_call.content_range();
        assert!(
            content_range.is_some(),
            "RefCall should have a content range"
        );
        let range = content_range.unwrap();
        let content_text = &sql[usize::from(range.start())..usize::from(range.end())];
        assert_eq!(
            content_text, "my_model",
            "content_range should exclude quotes"
        );
    }

    #[test]
    fn test_source_table_name_range() {
        let sql = "SELECT * FROM smelt.source('raw.users')";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let source_call = file.sources().next().unwrap();

        let table_range = source_call.table_name_range();
        assert!(
            table_range.is_some(),
            "SourceCall should have a table_name_range"
        );
        let range = table_range.unwrap();
        let table_text = &sql[usize::from(range.start())..usize::from(range.end())];
        assert_eq!(
            table_text, "users",
            "table_name_range should cover just the table name"
        );
    }
}

// =============================================================================
// Find References Tests (Phase 2)
// =============================================================================

mod find_references {
    use super::*;
    use smelt_db::references::{
        find_cte_references, find_model_references, find_source_references,
    };

    #[test]
    fn test_find_model_references_single_file() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");
        ws.add_model("orders", "SELECT * FROM smelt.ref('users')");

        let all_file_refs: Vec<_> = ws
            .db
            .all_files()
            .iter()
            .map(|p| (p.clone(), (*ws.db.model_refs(p.clone())).clone()))
            .collect();

        let refs = find_model_references("users", &all_file_refs);
        assert_eq!(refs.len(), 1, "Expected 1 reference to 'users'");
        assert_eq!(refs[0].0, ws.model_path("orders"));
    }

    #[test]
    fn test_find_model_references_multiple_files() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");
        ws.add_model("a", "SELECT * FROM smelt.ref('users')");
        ws.add_model("b", "SELECT * FROM smelt.ref('users')");
        ws.add_model("c", "SELECT * FROM smelt.ref('users')");

        let all_file_refs: Vec<_> = ws
            .db
            .all_files()
            .iter()
            .map(|p| (p.clone(), (*ws.db.model_refs(p.clone())).clone()))
            .collect();

        let refs = find_model_references("users", &all_file_refs);
        assert_eq!(refs.len(), 3, "Expected 3 references to 'users'");
    }

    #[test]
    fn test_find_model_references_unreferenced() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");
        ws.add_model("orders", "SELECT 1 as id");

        let all_file_refs: Vec<_> = ws
            .db
            .all_files()
            .iter()
            .map(|p| (p.clone(), (*ws.db.model_refs(p.clone())).clone()))
            .collect();

        let refs = find_model_references("users", &all_file_refs);
        assert!(refs.is_empty(), "Expected no references to 'users'");
    }

    #[test]
    fn test_find_source_references() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
"#,
        );
        ws.add_model("a", "SELECT * FROM smelt.source('raw.users')");
        ws.add_model("b", "SELECT * FROM smelt.source('raw.users')");

        let all_file_sources: Vec<_> = ws
            .db
            .all_files()
            .iter()
            .map(|p| (p.clone(), (*ws.db.model_sources(p.clone())).clone()))
            .collect();

        let refs = find_source_references("raw.users", &all_file_sources);
        assert_eq!(refs.len(), 2, "Expected 2 references to 'raw.users'");
    }

    #[test]
    fn test_find_cte_references_in_from() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM cte1";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();

        let refs = find_cte_references(&file, sql, "cte1");
        // Should find the reference in FROM
        let has_from_ref = refs.iter().any(|r| {
            let text = &sql[usize::from(r.start())..usize::from(r.end())];
            text == "cte1"
        });
        assert!(has_from_ref, "Should find CTE reference in FROM clause");
    }

    #[test]
    fn test_find_cte_references_in_join() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM other INNER JOIN cte1 ON other.id = cte1.id";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();

        let refs = find_cte_references(&file, sql, "cte1");
        // Should find the reference in JOIN
        assert!(!refs.is_empty(), "Should find CTE reference in JOIN clause");
    }

    #[test]
    fn test_find_cte_references_as_qualifier() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT cte1.id FROM cte1";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();

        let refs = find_cte_references(&file, sql, "cte1");
        // Should find the qualifier usage AND the FROM reference
        assert!(
            refs.len() >= 2,
            "Should find CTE as qualifier and in FROM, got {}",
            refs.len()
        );
    }

    #[test]
    fn test_find_cte_references_includes_definition() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM cte1";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();

        let refs = find_cte_references(&file, sql, "cte1");
        // Should include the definition site in WITH clause
        let has_def = refs.iter().any(|r| {
            // The definition is at offset 5 ("WITH cte1")
            let start: usize = r.start().into();
            start < 10 // definition is near the beginning
        });
        assert!(has_def, "Should include CTE definition site in references");
    }
}

// =============================================================================
// Code Action Tests
// =============================================================================

mod code_actions {
    use super::*;

    #[test]
    fn test_code_action_cast_for_type_mismatch() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model(
            "stg_users",
            "SELECT id, name FROM smelt.source('raw.users')",
        );
        // SUM(name) expects numeric but name is VARCHAR — type mismatch
        ws.add_model(
            "bad_sum",
            "SELECT SUM(name) as total FROM smelt.ref('stg_users')",
        );

        // The diagnostic is on "name" in SUM(name) — find its position
        // "SELECT SUM(name) as total FROM smelt.ref('stg_users')"
        //             ^--- line 0, col ~11
        let actions = ws.code_actions_at("bad_sum", 0, 11);

        // Should offer CAST to the expected numeric type
        let cast_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.title.contains("CAST"))
            .collect();
        assert!(
            !cast_actions.is_empty(),
            "Expected CAST code action for type mismatch, got: {:?}",
            actions
        );
        // The action should suggest casting to the expected type
        assert!(
            cast_actions
                .iter()
                .any(|a| a.title.contains("CAST") && a.new_text.contains("CAST(")),
            "CAST action should contain CAST() in the replacement text, got: {:?}",
            cast_actions
        );
    }

    #[test]
    fn test_code_action_cast_for_unknown_type() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: id
            type: INTEGER
          - name: payload
"#,
        );
        // payload has no type declared — CannotInferType
        ws.add_model(
            "stg_events",
            "SELECT id, payload FROM smelt.source('raw.events')",
        );

        // The diagnostic is on "payload" — find its position
        // "SELECT id, payload FROM smelt.source('raw.events')"
        //            ^--- line 0, col ~11
        let actions = ws.code_actions_at("stg_events", 0, 13);

        // Should offer multiple CAST options (VARCHAR, INTEGER, TIMESTAMP, etc.)
        let cast_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.title.contains("CAST"))
            .collect();
        assert!(
            cast_actions.len() >= 3,
            "Expected multiple CAST options for unknown type, got {} actions: {:?}",
            cast_actions.len(),
            cast_actions
        );
        // Should include common types
        let titles: Vec<String> = cast_actions.iter().map(|a| a.title.clone()).collect();
        assert!(
            titles.iter().any(|t| t.contains("VARCHAR")),
            "Should include VARCHAR option, got: {:?}",
            titles
        );
        assert!(
            titles.iter().any(|t| t.contains("INTEGER")),
            "Should include INTEGER option, got: {:?}",
            titles
        );
    }

    #[test]
    fn test_code_action_cast_wraps_expression() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model(
            "stg_users",
            "SELECT id, name FROM smelt.source('raw.users')",
        );
        // SUM(name) — the CAST should wrap "name", not just the column identifier
        ws.add_model(
            "bad_sum",
            "SELECT SUM(name) as total FROM smelt.ref('stg_users')",
        );

        let actions = ws.code_actions_at("bad_sum", 0, 11);

        let cast_action = actions.iter().find(|a| a.title.contains("CAST"));
        assert!(cast_action.is_some(), "Expected CAST action");
        let action = cast_action.unwrap();
        // The replacement text should wrap the expression with CAST()
        assert!(
            action.new_text.starts_with("CAST(")
                && action.new_text.contains(" AS ")
                && action.new_text.ends_with(")"),
            "CAST action should wrap expression: CAST(expr AS type), got: '{}'",
            action.new_text
        );
    }

    #[test]
    fn test_no_code_action_on_valid_code() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model("valid", "SELECT id, name FROM smelt.source('raw.users')");

        // Position on a valid column — should have no code actions
        let actions = ws.code_actions_at("valid", 0, 8);
        assert!(
            actions.is_empty(),
            "Expected no code actions on valid code, got: {:?}",
            actions
        );
    }
}

// =============================================================================
// Phase 4: Quick-Fix Code Actions — Create Model, Add Source/Column
// =============================================================================

mod code_actions_create_add {
    use super::*;
    use smelt_db::code_actions::CodeActionKind;

    #[test]
    fn test_code_action_create_missing_model() {
        let mut ws = TestWorkspace::new();
        ws.add_model("downstream", "SELECT * FROM smelt.ref('missing_model')");

        // Diagnostic covers the ref name range: 'missing_model' is at col 24..39
        // Cursor on the ref name content
        let actions = ws.all_code_actions_at("downstream", 0, 30);

        let create_actions: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, CodeActionKind::CreateModel(_)))
            .collect();
        assert!(
            !create_actions.is_empty(),
            "Expected CreateModel code action for undefined ref, got: {:?}",
            actions
        );

        if let CodeActionKind::CreateModel(suggestion) = &create_actions[0] {
            assert_eq!(suggestion.model_name, "missing_model");
            assert!(
                suggestion.title.contains("missing_model"),
                "Title should mention model name: '{}'",
                suggestion.title
            );
            assert!(
                suggestion.skeleton_sql.contains("SELECT"),
                "Skeleton SQL should contain SELECT: '{}'",
                suggestion.skeleton_sql
            );
        }
    }

    #[test]
    fn test_code_action_add_source_to_yaml() {
        let mut ws = TestWorkspace::new();
        // Source 'raw' exists but table 'orders' doesn't
        ws.set_sources_yml(
            r#"sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        ws.add_model("stg_orders", "SELECT * FROM smelt.source('raw.orders')");

        // Diagnostic covers the source ref range — cursor inside the quoted part
        let actions = ws.all_code_actions_at("stg_orders", 0, 32);

        let yaml_actions: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, CodeActionKind::YamlEdit(_)))
            .collect();
        assert!(
            !yaml_actions.is_empty(),
            "Expected YamlEdit code action to add table 'orders', got: {:?}",
            actions
        );

        if let CodeActionKind::YamlEdit(suggestion) = &yaml_actions[0] {
            assert!(
                suggestion.title.contains("orders"),
                "Title should mention table name: '{}'",
                suggestion.title
            );
            // The new lines should include the table key
            let joined = suggestion.new_lines.join("\n");
            assert!(
                joined.contains("orders:"),
                "New lines should contain 'orders:': '{}'",
                joined
            );
        }
    }

    #[test]
    fn test_code_action_add_source_new_section() {
        let mut ws = TestWorkspace::new();
        // No 'analytics' source exists — need full source block
        ws.set_sources_yml(
            r#"sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        ws.add_model(
            "stg_events",
            "SELECT * FROM smelt.source('analytics.events')",
        );

        // Cursor inside the quoted source reference
        let actions = ws.all_code_actions_at("stg_events", 0, 35);

        let yaml_actions: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, CodeActionKind::YamlEdit(_)))
            .collect();
        assert!(
            !yaml_actions.is_empty(),
            "Expected YamlEdit code action to add source 'analytics', got: {:?}",
            actions
        );

        if let CodeActionKind::YamlEdit(suggestion) = &yaml_actions[0] {
            let joined = suggestion.new_lines.join("\n");
            assert!(
                joined.contains("analytics:"),
                "Should include source name: '{}'",
                joined
            );
            assert!(
                joined.contains("events:"),
                "Should include table name: '{}'",
                joined
            );
            assert!(
                joined.contains("tables:"),
                "Should include tables key: '{}'",
                joined
            );
        }
    }

    #[test]
    fn test_code_action_add_column_to_yaml() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        // Use a qualified column reference (users.email) so UndeclaredColumn
        // gets qualifier = Some("users")
        ws.add_model(
            "stg_users",
            "SELECT users.email FROM smelt.source('raw.users') AS users",
        );

        // 'users.email' — the 'email' part triggers UndeclaredColumn with qualifier 'users'
        // "SELECT users.email FROM ..."
        //         ^--- col 7
        let actions = ws.all_code_actions_at("stg_users", 0, 13);

        let yaml_actions: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, CodeActionKind::YamlEdit(_)))
            .collect();
        assert!(
            !yaml_actions.is_empty(),
            "Expected YamlEdit code action to add column 'email', got: {:?}",
            actions
        );

        if let CodeActionKind::YamlEdit(suggestion) = &yaml_actions[0] {
            assert!(
                suggestion.title.contains("email"),
                "Title should mention column name: '{}'",
                suggestion.title
            );
            let joined = suggestion.new_lines.join("\n");
            assert!(
                joined.contains("- name: email"),
                "New lines should add the column: '{}'",
                joined
            );
        }
    }

    #[test]
    fn test_yaml_insertion_preserves_structure() {
        let mut ws = TestWorkspace::new();
        let original_yml = r#"sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
      orders:
        columns:
          - name: order_id
            type: INTEGER
"#;
        ws.set_sources_yml(original_yml);
        // Reference a new table 'products' that doesn't exist
        ws.add_model("stg_products", "SELECT * FROM smelt.source('raw.products')");

        // Cursor inside the quoted source reference
        let actions = ws.all_code_actions_at("stg_products", 0, 32);

        let yaml_actions: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, CodeActionKind::YamlEdit(_)))
            .collect();
        assert!(
            !yaml_actions.is_empty(),
            "Expected YamlEdit for new table 'products'"
        );

        if let CodeActionKind::YamlEdit(suggestion) = &yaml_actions[0] {
            // Verify indentation is consistent (6 spaces for table key)
            let table_line = suggestion
                .new_lines
                .iter()
                .find(|l| l.contains("products:"))
                .expect("Should have products: line");
            let indent = table_line.len() - table_line.trim_start().len();
            assert_eq!(
                indent, 6,
                "Table key should be indented 6 spaces, got {}",
                indent
            );

            // Verify the insert point is after existing tables content
            // (not in the middle of the users or orders block)
            let line_count = original_yml.lines().count();
            assert!(
                suggestion.insert_after_line < line_count,
                "Insert point {} should be within the file ({}  lines)",
                suggestion.insert_after_line,
                line_count
            );
        }
    }

    // ===== Phase 13: Handler-level code action tests =====

    #[test]
    fn test_handler_code_action_create_model() {
        let mut ws = TestWorkspace::new();
        ws.add_model("downstream", "SELECT * FROM smelt.ref('nonexistent')");

        // Cursor inside the ref call (on 'nonexistent')
        let actions = ws.handler_code_actions_at("downstream", 0, 30);

        let create_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.title.contains("Create") && a.kind == "quickfix")
            .collect();
        assert!(
            !create_actions.is_empty(),
            "Handler should return 'Create model' quickfix for undefined ref, got: {:?}",
            actions
        );
    }

    #[test]
    fn test_handler_code_action_yaml_add_source() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        ws.add_model(
            "stg_orders",
            "SELECT * FROM smelt.source('raw.missing_table')",
        );

        // Cursor inside the source call
        let actions = ws.handler_code_actions_at("stg_orders", 0, 32);

        let yaml_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.title.contains("Add table") && a.kind == "quickfix")
            .collect();
        assert!(
            !yaml_actions.is_empty(),
            "Handler should return 'Add table' quickfix for undefined source, got: {:?}",
            actions
        );
    }

    #[test]
    fn test_handler_code_action_yaml_add_column() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
        );
        ws.add_model(
            "model_col",
            "SELECT users.nonexistent_col FROM smelt.source('raw.users') AS users",
        );

        // Cursor on 'users.nonexistent_col' — need to find the right position
        // 'users.nonexistent_col' starts at col 7
        let actions = ws.handler_code_actions_at("model_col", 0, 13);

        let col_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.title.contains("Add column") && a.kind == "quickfix")
            .collect();
        assert!(
            !col_actions.is_empty(),
            "Handler should return 'Add column' quickfix for undeclared column, got: {:?}",
            actions
        );
    }

    #[test]
    fn test_handler_code_action_extract_cte() {
        let mut ws = TestWorkspace::new();
        // Model with a subquery in FROM
        ws.add_model(
            "subq_model",
            "SELECT t.id FROM (SELECT id FROM smelt.ref('upstream')) t",
        );
        // Also add upstream so the ref is valid
        ws.add_model("upstream", "SELECT 1 AS id");

        // Cursor inside the subquery — col 18 is inside "(SELECT id FROM ...)"
        let actions = ws.handler_code_actions_at("subq_model", 0, 22);

        let extract_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.kind == "refactor.extract" && a.title.contains("Extract"))
            .collect();
        assert!(
            !extract_actions.is_empty(),
            "Handler should return 'Extract CTE' refactoring for subquery, got: {:?}",
            actions
        );
    }

    #[test]
    fn test_handler_code_action_inline_cte() {
        let mut ws = TestWorkspace::new();
        // Model with a CTE used exactly once
        ws.add_model(
            "cte_model",
            "WITH sub AS (SELECT 1 AS id)\nSELECT sub.id FROM sub",
        );

        // Cursor on the CTE definition name 'sub' (line 0, col 5)
        let actions = ws.handler_code_actions_at("cte_model", 0, 5);

        let inline_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.kind == "refactor.inline" && a.title.contains("Inline"))
            .collect();
        assert!(
            !inline_actions.is_empty(),
            "Handler should return 'Inline CTE' refactoring for single-use CTE, got: {:?}",
            actions
        );
    }
}

// =============================================================================
// Rename CTE Tests (Phase 5)
// =============================================================================

mod rename_cte {
    use super::*;

    #[test]
    fn test_prepare_rename_cte_definition() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "cte_model",
            "WITH my_cte AS (\n  SELECT 1 AS id\n)\nSELECT * FROM my_cte",
        );

        // Cursor on 'my_cte' in WITH clause (line 0, col 5 = inside 'my_cte')
        let result = ws.prepare_rename("cte_model", 0, 5);
        assert!(result.is_some(), "Should be renamable on CTE definition");
        let (sl, sc, el, ec) = result.unwrap();
        assert_eq!(sl, 0);
        assert_eq!(sc, 5);
        assert_eq!(el, 0);
        assert_eq!(ec, 11); // 'my_cte' is 6 chars: 5..11
    }

    #[test]
    fn test_prepare_rename_cte_reference() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "cte_model",
            "WITH my_cte AS (\n  SELECT 1 AS id\n)\nSELECT * FROM my_cte",
        );

        // Cursor on 'my_cte' in FROM clause (line 3, col 14 = inside 'my_cte')
        let result = ws.prepare_rename("cte_model", 3, 14);
        assert!(result.is_some(), "Should be renamable on CTE reference");
        // prepareRename returns the definition range (the canonical location)
        let (sl, sc, _el, _ec) = result.unwrap();
        assert_eq!(sl, 0, "Definition is on line 0");
        assert_eq!(sc, 5, "Definition starts at col 5");
    }

    #[test]
    fn test_prepare_rename_rejects_keyword() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "cte_model",
            "WITH my_cte AS (\n  SELECT 1 AS id\n)\nSELECT * FROM my_cte",
        );

        // Cursor on 'SELECT' keyword (line 3, col 0)
        let result = ws.prepare_rename("cte_model", 3, 0);
        assert!(result.is_none(), "Should not be renamable on SQL keyword");
    }

    #[test]
    fn test_rename_cte_updates_definition_and_references() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "cte_model",
            "WITH my_cte AS (\n  SELECT 1 AS id\n)\nSELECT * FROM my_cte",
        );

        // Rename from CTE definition (line 0, col 5)
        let edits = ws.rename_cte("cte_model", 0, 5, "renamed_cte");
        assert_eq!(
            edits.len(),
            2,
            "Should have 2 edits: definition + reference"
        );

        // Both edits should use the new name
        for (_, _, _, _, new_text) in &edits {
            assert_eq!(new_text, "renamed_cte");
        }

        // One edit should be at definition (line 0, col 5)
        assert!(
            edits.iter().any(|(sl, sc, _, _, _)| *sl == 0 && *sc == 5),
            "Should have edit at CTE definition"
        );
        // One edit should be at reference in FROM (line 3, col 14)
        assert!(
            edits.iter().any(|(sl, sc, _, _, _)| *sl == 3 && *sc == 14),
            "Should have edit at CTE reference in FROM"
        );
    }

    #[test]
    fn test_rename_cte_with_qualified_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "cte_qual",
            "WITH base AS (\n  SELECT 1 AS id, 'a' AS name\n)\nSELECT base.id, base.name FROM base",
        );

        // Rename CTE 'base' from definition
        let edits = ws.rename_cte("cte_qual", 0, 5, "src");

        // Should have edits for: definition, base.id qualifier, base.name qualifier, FROM base
        assert!(
            edits.len() >= 4,
            "Expected at least 4 edits (def + 2 qualifiers + FROM ref), got {}",
            edits.len()
        );

        // Verify all edits have the new name
        for (_, _, _, _, new_text) in &edits {
            assert_eq!(new_text, "src");
        }
    }

    #[test]
    fn test_rename_cte_validates_identifier() {
        // This test verifies that invalid SQL identifiers are rejected.
        // The validation is a pure function.
        assert!(is_valid_sql_identifier("my_cte"));
        assert!(is_valid_sql_identifier("_private"));
        assert!(is_valid_sql_identifier("CTE1"));
        assert!(!is_valid_sql_identifier("123bad"));
        assert!(!is_valid_sql_identifier("has space"));
        assert!(!is_valid_sql_identifier("has-dash"));
        assert!(!is_valid_sql_identifier(""));
    }
}

// =============================================================================
// Model Rename Tests
// =============================================================================

mod rename_model {
    use super::*;

    #[test]
    fn test_prepare_rename_model_from_ref() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 AS id");
        ws.add_model("downstream", "SELECT id FROM smelt.ref('upstream')");

        // Cursor on 'upstream' inside ref() — col 24 is inside the string content
        let result = ws.prepare_rename("downstream", 0, 24);
        assert!(
            result.is_some(),
            "prepare_rename should return a range for ref() calls"
        );
        let (sl, sc, el, ec) = result.unwrap();
        // Should return the content range inside quotes (excluding quotes)
        assert_eq!(sl, 0);
        assert_eq!(el, 0);
        // The content 'upstream' is 8 chars
        assert_eq!(ec - sc, 8, "range should span the model name 'upstream'");
    }

    #[test]
    fn test_rename_model_updates_all_refs() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS id, 'alice' AS name");
        ws.add_model("orders", "SELECT id FROM smelt.ref('users')");
        ws.add_model("payments", "SELECT id FROM smelt.ref('users')");
        ws.add_model(
            "report",
            "SELECT id FROM smelt.ref('users') CROSS JOIN smelt.ref('orders')",
        );

        // Cursor inside ref('users') in orders model
        let result = ws.rename_model("orders", 0, 30, "customers");
        assert!(!result.conflict);
        // Should have 3 edits: orders, payments, report all reference 'users'
        assert_eq!(
            result.text_edits.len(),
            3,
            "should update all 3 files referencing 'users'"
        );
    }

    #[test]
    fn test_rename_model_includes_file_rename() {
        let mut ws = TestWorkspace::new();
        ws.add_model("old_name", "SELECT 1 AS id");
        ws.add_model("consumer", "SELECT id FROM smelt.ref('old_name')");

        let result = ws.rename_model("consumer", 0, 30, "new_name");
        assert!(!result.conflict);
        // Should include a file rename operation
        assert!(
            result.file_rename.is_some(),
            "should include a file rename for old_name.sql -> new_name.sql"
        );
        let (old_path, new_path) = result.file_rename.unwrap();
        assert!(
            old_path.ends_with("old_name.sql"),
            "old path should be old_name.sql"
        );
        assert!(
            new_path.ends_with("new_name.sql"),
            "new path should be new_name.sql"
        );
    }

    #[test]
    fn test_rename_model_ref_content_range() {
        let mut ws = TestWorkspace::new();
        ws.add_model("my_model", "SELECT 1 AS id");
        ws.add_model("consumer", "SELECT id FROM smelt.ref('my_model')");

        let result = ws.rename_model("consumer", 0, 30, "renamed_model");
        assert!(!result.conflict);
        assert_eq!(result.text_edits.len(), 1);

        // The edit should only replace the content inside quotes, not the quotes
        let (_, sl, sc, el, ec) = &result.text_edits[0];
        let text = "SELECT id FROM smelt.ref('my_model')";
        // 'my_model' starts at col 25 (after the opening quote) and ends at col 33
        let content_start = text.find("my_model").unwrap() as u32;
        let content_end = content_start + "my_model".len() as u32;
        assert_eq!(*sl, 0);
        assert_eq!(*sc, content_start, "edit start should be inside the quotes");
        assert_eq!(*el, 0);
        assert_eq!(*ec, content_end, "edit end should be inside the quotes");
    }

    #[test]
    fn test_rename_model_no_conflict() {
        let mut ws = TestWorkspace::new();
        ws.add_model("existing", "SELECT 1 AS id");
        ws.add_model("other", "SELECT 1 AS id");
        ws.add_model("consumer", "SELECT id FROM smelt.ref('other')");

        // Try to rename 'other' to 'existing' — should detect conflict
        let result = ws.rename_model("consumer", 0, 30, "existing");
        assert!(
            result.conflict,
            "should reject rename to existing model name"
        );
    }
}

// =============================================================================
// Rename Source Tests
// =============================================================================

mod rename_source {
    use super::*;

    #[test]
    fn test_prepare_rename_source_from_call() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            "version: 1\n\nsources:\n  raw:\n    tables:\n      users:\n        columns:\n          - name: id\n            type: INTEGER\n",
        );
        ws.add_model("staging", "SELECT id FROM smelt.source('raw.users')");

        // Cursor on 'users' inside source('raw.users') — find col for 'users'
        let text = "SELECT id FROM smelt.source('raw.users')";
        let users_start = text.find("users')").unwrap() as u32;
        let result = ws.prepare_rename("staging", 0, users_start);
        assert!(
            result.is_some(),
            "prepare_rename should return a range for source() table name"
        );
        let (sl, sc, el, ec) = result.unwrap();
        assert_eq!(sl, 0);
        assert_eq!(el, 0);
        assert_eq!(ec - sc, 5, "range should span 'users' (5 chars)");
    }

    #[test]
    fn test_rename_source_table_updates_all_calls() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            "version: 1\n\nsources:\n  raw:\n    tables:\n      users:\n        columns:\n          - name: id\n            type: INTEGER\n",
        );
        ws.add_model("staging_a", "SELECT id FROM smelt.source('raw.users')");
        ws.add_model("staging_b", "SELECT id FROM smelt.source('raw.users')");

        // Cursor on 'users' in staging_a
        let text = "SELECT id FROM smelt.source('raw.users')";
        let users_start = text.find("users')").unwrap() as u32;
        let result = ws.rename_source("staging_a", 0, users_start, "customers");

        // Both files should get text edits
        assert_eq!(
            result.sql_edits.len(),
            2,
            "should have edits for both files referencing raw.users"
        );
    }

    #[test]
    fn test_rename_source_table_updates_yaml() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            "version: 1\n\nsources:\n  raw:\n    tables:\n      users:\n        columns:\n          - name: id\n            type: INTEGER\n",
        );
        ws.add_model("staging", "SELECT id FROM smelt.source('raw.users')");

        let text = "SELECT id FROM smelt.source('raw.users')";
        let users_start = text.find("users')").unwrap() as u32;
        let result = ws.rename_source("staging", 0, users_start, "customers");

        // YAML should have a rename edit
        assert!(
            result.yaml_edit.is_some(),
            "should produce a YAML edit for the table key"
        );
        let (line_num, old_line, new_line) = result.yaml_edit.unwrap();
        assert!(
            old_line.contains("users:"),
            "old line should contain 'users:'"
        );
        assert!(
            new_line.contains("customers:"),
            "new line should contain 'customers:'"
        );
        // Line 5 is "      users:" (0-indexed)
        assert_eq!(line_num, 5, "YAML table key should be on line 5");
    }

    #[test]
    fn test_rename_source_table_yaml_preserves_columns() {
        let yaml = "version: 1\n\nsources:\n  raw:\n    tables:\n      users:\n        columns:\n          - name: id\n            type: INTEGER\n          - name: email\n            type: VARCHAR\n";
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(yaml);
        ws.add_model("staging", "SELECT id FROM smelt.source('raw.users')");

        let text = "SELECT id FROM smelt.source('raw.users')";
        let users_start = text.find("users')").unwrap() as u32;
        let result = ws.rename_source("staging", 0, users_start, "customers");

        // The YAML edit should only change the table key line, not touch columns
        assert!(result.yaml_edit.is_some());
        let (_, old_line, new_line) = result.yaml_edit.unwrap();
        // Only the table key line changes
        assert!(old_line.contains("users:"));
        assert!(new_line.contains("customers:"));
        // Verify columns are NOT in the edit (only the key line changes)
        assert!(
            !old_line.contains("columns"),
            "edit should not include column lines"
        );
        assert!(
            !new_line.contains("columns"),
            "edit should not include column lines"
        );
    }
}

// =============================================================================
// Rename Column Tests
// =============================================================================

mod rename_column {
    use super::*;

    #[test]
    fn test_rename_column_single_model() {
        // Rename `user_id` in SELECT list should also update WHERE and GROUP BY
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "users",
            "SELECT user_id, COUNT(*) AS cnt FROM smelt.ref('raw_events') WHERE user_id > 0 GROUP BY user_id",
        );
        ws.add_model("raw_events", "SELECT 1 AS user_id, 2 AS event_type");

        // Cursor on `user_id` in SELECT (col 7 = 'u' of first user_id)
        let result = ws.rename_column("users", 0, 7, "account_id");
        assert!(result.error.is_none(), "should not return error");

        // Should find 3 local edits: SELECT user_id, WHERE user_id, GROUP BY user_id
        assert!(
            result.local_edits.len() >= 3,
            "expected at least 3 local edits for user_id in SELECT/WHERE/GROUP BY, got {}",
            result.local_edits.len()
        );
    }

    #[test]
    fn test_rename_column_propagates_upstream() {
        // Column from ColumnSource::FromModel should be traced to upstream model
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 AS user_id, 2 AS age");
        ws.add_model("downstream", "SELECT user_id FROM smelt.ref('upstream')");

        // Cursor on `user_id` in downstream SELECT
        let result = ws.rename_column("downstream", 0, 7, "account_id");
        assert!(result.error.is_none());

        // Should find at least 1 local edit (the column ref in downstream)
        assert!(
            !result.local_edits.is_empty(),
            "expected local edits in downstream model"
        );

        // Should find a cross-file edit in the upstream model (the column definition)
        assert!(
            !result.cross_file_edits.is_empty(),
            "expected cross-file edit in upstream model for column definition"
        );
        let upstream_path = ws.model_path("upstream");
        assert!(
            result
                .cross_file_edits
                .iter()
                .any(|(p, _, _, _, _)| *p == upstream_path),
            "cross-file edit should be in upstream model"
        );
    }

    #[test]
    fn test_rename_column_propagates_downstream() {
        // Downstream model using `ref('model').col` gets updated
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 2 AS age");
        ws.add_model("orders", "SELECT user_id FROM smelt.ref('users')");

        // Cursor on `user_id` in the users model SELECT (the definition site)
        let result = ws.rename_column("users", 0, 14, "account_id");
        assert!(result.error.is_none());

        // Should find local edit (user_id definition in SELECT)
        assert!(
            !result.local_edits.is_empty(),
            "expected local edits in users model"
        );

        // Should find cross-file edit in orders model
        let orders_path = ws.model_path("orders");
        assert!(
            result
                .cross_file_edits
                .iter()
                .any(|(p, _, _, _, _)| *p == orders_path),
            "should find cross-file edit in downstream model 'orders'"
        );
    }

    #[test]
    fn test_rename_column_through_select_star() {
        // Upstream renames column, downstream SELECT * is unaffected
        // but downstream explicit col refs are found
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 AS user_id, 2 AS age");
        ws.add_model("passthrough", "SELECT * FROM smelt.ref('upstream')");
        ws.add_model("consumer", "SELECT user_id FROM smelt.ref('passthrough')");

        // Rename user_id in upstream
        let result = ws.rename_column("upstream", 0, 14, "account_id");
        assert!(result.error.is_none());

        // consumer should get a cross-file edit (it explicitly references user_id)
        let consumer_path = ws.model_path("consumer");
        assert!(
            result
                .cross_file_edits
                .iter()
                .any(|(p, _, _, _, _)| *p == consumer_path),
            "consumer using explicit column ref through SELECT * passthrough should be found"
        );
    }

    #[test]
    fn test_rename_column_through_cte_chain() {
        // Column flows through 3 CTEs, all should be renamed
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "cte_chain",
            "WITH
  cte1 AS (SELECT 1 AS user_id),
  cte2 AS (SELECT user_id FROM cte1),
  cte3 AS (SELECT user_id FROM cte2)
SELECT user_id FROM cte3",
        );

        // Cursor on `user_id` in final SELECT (line 4, col 7)
        let result = ws.rename_column("cte_chain", 4, 7, "account_id");
        assert!(result.error.is_none());

        // Should find user_id in cte1 SELECT, cte2 SELECT+FROM, cte3 SELECT+FROM, final SELECT
        // At least 4 unique locations (the definition in cte1 + references in cte2, cte3, final)
        assert!(
            result.local_edits.len() >= 4,
            "expected at least 4 local edits for user_id through CTE chain, got {}",
            result.local_edits.len()
        );
    }

    #[test]
    fn test_rename_column_source_updates_yaml() {
        // Column from source should update sources.yml column name
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            "sources:\n  raw:\n    tables:\n      users:\n        columns:\n          - name: user_id\n            type: INTEGER\n          - name: email\n            type: VARCHAR\n",
        );
        ws.add_model("staging", "SELECT user_id FROM smelt.source('raw.users')");

        // Cursor on `user_id` in SELECT
        let result = ws.rename_column("staging", 0, 7, "account_id");
        assert!(result.error.is_none());

        // Should produce a YAML edit for the column name
        assert!(
            result.yaml_edit.is_some(),
            "should produce YAML edit for source column rename"
        );
        let (_, old_line, new_line) = result.yaml_edit.unwrap();
        assert!(
            old_line.contains("- name: user_id"),
            "old line should contain '- name: user_id'"
        );
        assert!(
            new_line.contains("- name: account_id"),
            "new line should contain '- name: account_id'"
        );
    }

    #[test]
    fn test_rename_column_ambiguous_rejected() {
        // Unqualified column matching multiple sources should be rejected
        let mut ws = TestWorkspace::new();
        ws.add_model("a", "SELECT 1 AS id");
        ws.add_model("b", "SELECT 1 AS id");
        ws.add_model(
            "joined",
            "SELECT id FROM smelt.ref('a') CROSS JOIN smelt.ref('b')",
        );

        // Cursor on unqualified `id` in SELECT — ambiguous between a.id and b.id
        // The rename should still work for local references (it's a valid column name),
        // but won't propagate to both upstreams since it's ambiguous
        let result = ws.rename_column("joined", 0, 7, "identifier");
        // Should have at least the local edit
        assert!(
            !result.local_edits.is_empty() || result.error.is_some(),
            "ambiguous column rename should either produce local edits or an error"
        );
    }
}

// ===== Phase 9: Extract CTE Refactoring =====

#[test]
fn test_extract_cte_from_subquery_in_from() {
    // Subquery in FROM should be extracted into a CTE + reference
    let sql = "SELECT t.id FROM (SELECT id FROM users) t";
    //                           ^ cursor inside subquery (line 0, col 20)
    let result = smelt_db::code_actions::find_extract_cte_suggestion(sql, 0, 20);
    assert!(result.is_some(), "should find extractable subquery in FROM");
    let result = result.unwrap();
    assert!(!result.cte_name.is_empty(), "should generate a CTE name");
    assert!(
        result.edits.len() >= 2,
        "should have at least 2 edits (CTE insertion + subquery replacement)"
    );
    // After applying edits, the result should have a WITH clause and a reference
    let applied = apply_text_edits(sql, &result.edits);
    assert!(
        applied.contains("WITH"),
        "result should contain WITH clause"
    );
    assert!(
        applied.contains("AS (SELECT id FROM users)"),
        "CTE body should contain the subquery"
    );
    // The subquery in FROM should be replaced with just the CTE name
    assert!(
        !applied.contains("FROM (SELECT"),
        "subquery should be replaced with CTE name"
    );
}

#[test]
fn test_extract_cte_from_subquery_in_join() {
    // Subquery in JOIN should be extracted into a CTE + reference
    let sql = "SELECT a.id FROM users a INNER JOIN (SELECT id FROM orders) b ON a.id = b.id";
    //                                               ^ cursor inside subquery (line 0, col 40)
    let result = smelt_db::code_actions::find_extract_cte_suggestion(sql, 0, 40);
    assert!(result.is_some(), "should find extractable subquery in JOIN");
    let result = result.unwrap();
    let applied = apply_text_edits(sql, &result.edits);
    assert!(
        applied.contains("WITH"),
        "result should contain WITH clause"
    );
    assert!(
        applied.contains("AS (SELECT id FROM orders)"),
        "CTE body should contain the subquery"
    );
    assert!(
        !applied.contains("JOIN (SELECT"),
        "subquery in JOIN should be replaced"
    );
}

#[test]
fn test_extract_cte_appends_to_existing_with() {
    // File already has CTEs, new one should be appended
    let sql =
        "WITH existing AS (SELECT 1 AS x)\nSELECT e.x FROM existing e CROSS JOIN (SELECT 2 AS y) f";
    //                                                                             ^ cursor at line 1, col 42
    let result = smelt_db::code_actions::find_extract_cte_suggestion(sql, 1, 42);
    assert!(
        result.is_some(),
        "should find extractable subquery with existing WITH"
    );
    let result = result.unwrap();
    let applied = apply_text_edits(sql, &result.edits);
    // Should have comma-separated CTEs, not two WITH clauses
    let with_count = applied.matches("WITH").count();
    assert_eq!(
        with_count, 1,
        "should have exactly one WITH clause, got: {}",
        applied
    );
    assert!(
        applied.contains("existing AS"),
        "should preserve existing CTE"
    );
    assert!(
        applied.contains("AS (SELECT 2 AS y)"),
        "should append new CTE"
    );
}

#[test]
fn test_extract_cte_creates_with_clause() {
    // File has no CTEs, WITH clause should be created
    let sql = "SELECT t.id FROM (SELECT id FROM users) t WHERE t.id > 0";
    //                            ^ cursor at line 0, col 20
    let result = smelt_db::code_actions::find_extract_cte_suggestion(sql, 0, 20);
    assert!(result.is_some(), "should find extractable subquery");
    let result = result.unwrap();
    let applied = apply_text_edits(sql, &result.edits);
    assert!(
        applied.starts_with("WITH"),
        "should create WITH clause at the start"
    );
    // The WHERE clause should be preserved
    assert!(applied.contains("WHERE"), "should preserve WHERE clause");
}

/// Apply text edits to a string, processing from end to start to maintain positions.
fn apply_text_edits(text: &str, edits: &[smelt_db::code_actions::TextEditSuggestion]) -> String {
    // Convert to byte offsets and sort by position (end to start)
    let mut edits_with_offsets: Vec<_> = edits
        .iter()
        .map(|edit| {
            let start =
                position_to_byte_offset(text, edit.range.start.line, edit.range.start.column);
            let end = position_to_byte_offset(text, edit.range.end.line, edit.range.end.column);
            (start, end, &edit.new_text)
        })
        .collect();
    edits_with_offsets.sort_by(|a, b| b.0.cmp(&a.0)); // reverse order

    let mut result = text.to_string();
    for (start, end, new_text) in edits_with_offsets {
        result.replace_range(start..end, new_text);
    }
    result
}

/// Convert (line, col) to byte offset in text.
fn position_to_byte_offset(text: &str, line: u32, col: u32) -> usize {
    let mut current_line = 0u32;
    let mut current_col = 0u32;
    for (i, ch) in text.char_indices() {
        if current_line == line && current_col == col {
            return i;
        }
        if ch == '\n' {
            current_line += 1;
            current_col = 0;
        } else {
            current_col += 1;
        }
    }
    if current_line == line && current_col == col {
        return text.len();
    }
    text.len()
}

// ===== Phase 10: Inline CTE Refactoring =====

#[test]
fn test_inline_cte_single_reference() {
    // CTE used once in FROM should be inlined as a subquery
    let sql = "WITH sub AS (SELECT id FROM users)\nSELECT sub.id FROM sub";
    //         ^ cursor on CTE definition name (line 0, col 5)
    let result = smelt_db::code_actions::find_inline_cte_suggestion(sql, 0, 5);
    assert!(result.is_some(), "should find inlinable CTE used once");
    let result = result.unwrap();
    assert_eq!(result.cte_name, "sub");
    let applied = apply_text_edits(sql, &result.edits);
    // The WITH clause should be removed entirely
    assert!(
        !applied.contains("WITH"),
        "WITH clause should be removed after inlining last CTE"
    );
    // The CTE reference should be replaced with a subquery
    assert!(
        applied.contains("(SELECT id FROM users)"),
        "reference should be replaced with subquery, got: {}",
        applied
    );
}

#[test]
fn test_inline_cte_removes_with_clause() {
    // When the last CTE is inlined, the entire WITH keyword should be removed
    let sql = "WITH only_cte AS (SELECT 1 AS x)\nSELECT only_cte.x FROM only_cte";
    let result = smelt_db::code_actions::find_inline_cte_suggestion(sql, 0, 5);
    assert!(result.is_some(), "should inline single CTE");
    let result = result.unwrap();
    let applied = apply_text_edits(sql, &result.edits);
    assert!(
        !applied.contains("WITH"),
        "WITH clause should be removed when last CTE is inlined, got: {}",
        applied
    );
    // Should be a valid SELECT statement
    assert!(
        applied.contains("SELECT"),
        "result should still contain SELECT"
    );
}

#[test]
fn test_inline_cte_keeps_other_ctes() {
    // Only the selected CTE is removed, others remain
    let sql =
        "WITH a AS (SELECT 1 AS x),\nb AS (SELECT 2 AS y)\nSELECT a.x, b.y FROM a CROSS JOIN b";
    //                                      ^ cursor on CTE 'b' (line 1, col 0)
    let result = smelt_db::code_actions::find_inline_cte_suggestion(sql, 1, 0);
    assert!(result.is_some(), "should find inlinable CTE 'b'");
    let result = result.unwrap();
    assert_eq!(result.cte_name, "b");
    let applied = apply_text_edits(sql, &result.edits);
    // WITH clause should still exist (CTE 'a' remains)
    assert!(
        applied.contains("WITH"),
        "WITH clause should remain for other CTEs, got: {}",
        applied
    );
    assert!(
        applied.contains("a AS"),
        "CTE 'a' should be preserved, got: {}",
        applied
    );
    // CTE 'b' should be inlined
    assert!(
        !applied.contains("b AS"),
        "CTE 'b' should be removed from WITH, got: {}",
        applied
    );
    assert!(
        applied.contains("(SELECT 2 AS y)"),
        "CTE 'b' body should be inlined as subquery, got: {}",
        applied
    );
}

#[test]
fn test_inline_cte_rejects_multiple_references() {
    // CTE used 3 times should not be inlinable
    let sql = "WITH sub AS (SELECT id FROM users)\nSELECT s1.id FROM sub s1 CROSS JOIN sub s2 CROSS JOIN sub s3";
    let result = smelt_db::code_actions::find_inline_cte_suggestion(sql, 0, 5);
    assert!(
        result.is_none(),
        "should not inline CTE used more than once"
    );
}
