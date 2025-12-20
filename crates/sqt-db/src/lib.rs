/// Salsa database for incremental compilation
///
/// This module defines the Salsa queries that power the LSP and optimizer.
/// Salsa automatically handles incremental recomputation when inputs change.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use sqt_parser::{self, File as AstFile};

pub mod schema;
pub use schema::{Column, ColumnSource, ModelSchema};

/// Input queries - these are set by the LSP when files change
#[salsa::query_group(InputsStorage)]
pub trait Inputs {
    /// Get the text content of a file
    /// This is an input query - set by LSP when file changes
    #[salsa::input]
    fn file_text(&self, path: PathBuf) -> Arc<String>;

    /// Get all file paths in the project
    #[salsa::input]
    fn all_files(&self) -> Arc<Vec<PathBuf>>;
}

/// Syntax queries - parsing and CST construction
#[salsa::query_group(SyntaxStorage)]
pub trait Syntax: Inputs {
    /// Parse a file into a CST
    fn parse_file(&self, path: PathBuf) -> Arc<sqt_parser::Parse>;

    /// Parse a file and extract model definitions
    /// Returns None if file doesn't contain a valid model
    fn parse_model(&self, path: PathBuf) -> Option<Arc<Model>>;

    /// Extract all ref() calls from a model with their positions
    fn model_refs(&self, path: PathBuf) -> Arc<Vec<RefLocation>>;

    /// Get all models in the project
    fn all_models(&self) -> Arc<HashMap<PathBuf, Model>>;
}

/// Semantic queries - name resolution, type checking, etc.
#[salsa::query_group(SemanticStorage)]
pub trait Semantic: Syntax {
    /// Resolve a ref() to the file path where it's defined
    /// Returns None if the ref is undefined
    fn resolve_ref(&self, model_name: String) -> Option<PathBuf>;

    /// Get all diagnostics for a file
    fn file_diagnostics(&self, path: PathBuf) -> Arc<Vec<Diagnostic>>;
}

/// Schema queries - column tracking and inference
#[salsa::query_group(SchemaStorage)]
pub trait Schema: Semantic {
    /// Extract the output schema from a model
    fn model_schema(&self, path: PathBuf) -> Arc<ModelSchema>;

    /// Get available columns at a specific position in a file
    /// (for autocomplete context)
    fn available_columns(&self, path: PathBuf) -> Arc<Vec<Column>>;
}

/// The main database that combines all query groups
#[salsa::database(InputsStorage, SyntaxStorage, SemanticStorage, SchemaStorage)]
#[derive(Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
}

impl salsa::Database for Database {}

// Query implementations

fn parse_file(db: &dyn Syntax, path: PathBuf) -> Arc<sqt_parser::Parse> {
    let text = db.file_text(path);
    Arc::new(sqt_parser::parse(&text))
}

fn parse_model(db: &dyn Syntax, path: PathBuf) -> Option<Arc<Model>> {
    // Extract model name from file path (e.g., models/users.sql -> users)
    let model_name = path
        .file_stem()?
        .to_str()?
        .to_string();

    // Parse file and check if it contains a valid SELECT statement
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();
    let file = AstFile::cast(syntax)?;

    // Check if file has a SELECT statement
    if file.select_stmt().is_none() {
        return None;
    }

    Some(Arc::new(Model {
        name: model_name,
        path: path.clone(),
    }))
}

fn model_refs(db: &dyn Syntax, path: PathBuf) -> Arc<Vec<RefLocation>> {
    let parse = db.parse_file(path.clone());
    let text = db.file_text(path);
    let syntax = parse.syntax();

    // Use AST to extract all ref calls with positions
    if let Some(file) = AstFile::cast(syntax) {
        let refs: Vec<RefLocation> = file
            .refs()
            .filter_map(|ref_call| {
                let name = ref_call.model_name()?;
                let text_range = ref_call.name_range().unwrap_or(ref_call.range());
                let range = sqt_parser::ast::text_range_to_range(&text, text_range);

                Some(RefLocation { name, range })
            })
            .collect();

        Arc::new(refs)
    } else {
        Arc::new(Vec::new())
    }
}

fn all_models(db: &dyn Syntax) -> Arc<HashMap<PathBuf, Model>> {
    let files = db.all_files();
    let mut models = HashMap::new();

    for path in files.iter() {
        if let Some(model) = db.parse_model(path.clone()) {
            models.insert(path.clone(), (*model).clone());
        }
    }

    Arc::new(models)
}

fn resolve_ref(db: &dyn Semantic, model_name: String) -> Option<PathBuf> {
    let models = db.all_models();

    // Find the model with this name
    models.iter()
        .find(|(_, model)| model.name == model_name)
        .map(|(path, _)| path.clone())
}

fn file_diagnostics(db: &dyn Semantic, path: PathBuf) -> Arc<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    // Add parse errors
    let parse = db.parse_file(path.clone());
    for error in parse.errors.iter() {
        let text = db.file_text(path.clone());
        let range = sqt_parser::ast::text_range_to_range(&text, error.range);

        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            range,
        });
    }

    // Check if model is valid
    if db.parse_model(path.clone()).is_none() {
        // Only report error if file is supposed to be a model (in models/ directory)
        if path.to_str().map(|s| s.contains("models/")).unwrap_or(false) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "File does not contain a valid SQL query".to_string(),
                range: Range {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 0, column: 0 },
                },
            });
        }
        return Arc::new(diagnostics);
    }

    // Check for undefined refs with accurate positions
    let refs = db.model_refs(path.clone());
    for ref_loc in refs.iter() {
        if db.resolve_ref(ref_loc.name.clone()).is_none() {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined model reference: '{}'", ref_loc.name),
                range: ref_loc.range,
            });
        }
    }

    Arc::new(diagnostics)
}

/// Represents a model (SQL file in models/ directory)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub name: String,
    pub path: PathBuf,
}

/// Reference location with position information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLocation {
    pub name: String,
    pub range: Range,
}

/// Position in a file (line, column)
pub type Position = sqt_parser::ast::Position;

/// Range in a file (start, end)
pub type Range = sqt_parser::ast::Range;

/// Represents a diagnostic (error, warning, info)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub range: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

// Schema query implementations

fn model_schema(db: &dyn Schema, path: PathBuf) -> Arc<ModelSchema> {
    // Parse the model
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();
    let text = db.file_text(path.clone());

    let file = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return Arc::new(ModelSchema::empty()),
    };

    // Get refs from FROM clause to determine sources
    let from_refs: Vec<String> = if let Some(from_clause) = select_stmt.from_clause() {
        from_clause
            .table_refs()
            .filter_map(|table_ref| {
                table_ref
                    .template_expr()
                    .and_then(|t| t.ref_call())
                    .and_then(|r| r.model_name())
            })
            .collect()
    } else {
        Vec::new()
    };

    // Extract columns from select list
    let mut columns = Vec::new();

    for item in select_list.items() {
        // Handle SELECT *
        if let Some(expr) = item.expression() {
            if expr.text().trim() == "*" {
                // Wildcard - need to expand from source(s)
                for ref_name in &from_refs {
                    columns.push(Column {
                        name: "*".to_string(),
                        alias: None,
                        source: ColumnSource::Wildcard {
                            model_name: ref_name.clone(),
                        },
                        expression: "*".to_string(),
                        range: item.range(),
                    });
                }
                continue;
            }
        }

        // Regular column
        let name = match item.column_name() {
            Some(n) => n,
            None => continue, // Skip if we can't determine name
        };

        let alias = item.alias();
        let expression = item.expression().map(|e| e.text()).unwrap_or_default();

        // Determine source
        let source = if let Some(expr) = item.expression() {
            // Check for function calls first (before column refs)
            if expr.as_function_call().is_some() {
                // Functions like COUNT, SUM, etc. are computed
                ColumnSource::Computed
            } else if let Some(col_ref) = expr.as_column_ref() {
                // Simple column reference - try to trace to upstream model
                let column_name = col_ref.name().to_string();

                // If there's exactly one ref, assume it's from that model
                if from_refs.len() == 1 {
                    ColumnSource::FromModel {
                        model_name: from_refs[0].clone(),
                        column_name,
                    }
                } else if from_refs.is_empty() {
                    // No refs - external table
                    ColumnSource::ExternalTable {
                        table_name: col_ref.qualifier().unwrap_or("unknown").to_string(),
                    }
                } else {
                    // Multiple refs - need qualifier to determine source
                    if let Some(_qualifier) = col_ref.qualifier() {
                        // Check if qualifier matches a ref
                        // For now, mark as Unknown - would need alias resolution
                        ColumnSource::Unknown
                    } else {
                        ColumnSource::Unknown
                    }
                }
            } else {
                // Complex expression (binary op, etc.)
                ColumnSource::Computed
            }
        } else {
            ColumnSource::Unknown
        };

        columns.push(Column {
            name,
            alias,
            source,
            expression,
            range: item.range(),
        });
    }

    Arc::new(ModelSchema { columns })
}

fn available_columns(db: &dyn Schema, path: PathBuf) -> Arc<Vec<Column>> {
    // Get the schema of this model
    let schema = db.model_schema(path.clone());
    let mut available = schema.columns.clone();

    // Get refs in FROM clause and add their columns
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();

    if let Some(file) = AstFile::cast(syntax) {
        if let Some(select_stmt) = file.select_stmt() {
            if let Some(from_clause) = select_stmt.from_clause() {
                for table_ref in from_clause.table_refs() {
                    if let Some(template) = table_ref.template_expr() {
                        if let Some(ref_call) = template.ref_call() {
                            if let Some(model_name) = ref_call.model_name() {
                                // Resolve upstream model schema
                                if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
                                    let upstream_schema = db.model_schema(upstream_path);

                                    // Add upstream columns to available list
                                    for col in upstream_schema.columns.iter() {
                                        // Skip wildcards
                                        if col.name == "*" {
                                            continue;
                                        }
                                        available.push(col.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Arc::new(available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_schema_extraction_simple_columns() {
        let mut db = Database::default();

        // Create a simple model with no aliases
        let path = PathBuf::from("test_model.sql");
        db.set_file_text(
            path.clone(),
            Arc::new(
                "SELECT\n  event_id,\n  user_id,\n  event_time\nFROM source.events".to_string(),
            ),
        );

        let schema = db.model_schema(path);

        assert_eq!(schema.columns.len(), 3);
        assert_eq!(schema.columns[0].name, "event_id");
        assert_eq!(schema.columns[1].name, "user_id");
        assert_eq!(schema.columns[2].name, "event_time");

        // All should have no alias
        assert!(schema.columns[0].alias.is_none());
        assert!(schema.columns[1].alias.is_none());
        assert!(schema.columns[2].alias.is_none());
    }

    #[test]
    fn test_schema_extraction_with_aliases() {
        let mut db = Database::default();

        let path = PathBuf::from("test_model.sql");
        db.set_file_text(
            path.clone(),
            Arc::new("SELECT\n  user_id,\n  COUNT(*) as event_count\nFROM source.events\nGROUP BY user_id".to_string()),
        );

        let schema = db.model_schema(path);

        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].name, "user_id");
        assert!(schema.columns[0].alias.is_none());

        assert_eq!(schema.columns[1].name, "event_count");
        assert_eq!(schema.columns[1].alias, Some("event_count".to_string()));
        assert!(schema.columns[1].expression.contains("COUNT"));
    }

    #[test]
    fn test_schema_extraction_from_ref() {
        let mut db = Database::default();

        // Create upstream model
        let raw_events_path = PathBuf::from("models/raw_events.sql");
        db.set_file_text(
            raw_events_path.clone(),
            Arc::new("SELECT\n  user_id,\n  event_id\nFROM source.events".to_string()),
        );

        // Create downstream model that refs upstream
        let sessions_path = PathBuf::from("models/user_sessions.sql");
        db.set_file_text(
            sessions_path.clone(),
            Arc::new("SELECT\n  user_id,\n  COUNT(*) as session_count\nFROM {{ ref('raw_events') }}\nGROUP BY user_id".to_string()),
        );

        // Set up all_files for model resolution
        db.set_all_files(Arc::new(vec![raw_events_path.clone(), sessions_path.clone()]));

        let schema = db.model_schema(sessions_path);

        assert_eq!(schema.columns.len(), 2);

        // user_id should be traced to raw_events
        assert_eq!(schema.columns[0].name, "user_id");
        match &schema.columns[0].source {
            ColumnSource::FromModel { model_name, column_name } => {
                assert_eq!(model_name, "raw_events");
                assert_eq!(column_name, "user_id");
            }
            _ => panic!("Expected FromModel source"),
        }

        // COUNT(*) should be Computed
        assert_eq!(schema.columns[1].name, "session_count");
        assert_eq!(schema.columns[1].alias, Some("session_count".to_string()));
        match schema.columns[1].source {
            ColumnSource::Computed => {}
            _ => panic!("Expected Computed source"),
        }
    }

    #[test]
    fn test_available_columns_includes_upstream() {
        let mut db = Database::default();

        // Create upstream model
        let raw_events_path = PathBuf::from("models/raw_events.sql");
        db.set_file_text(
            raw_events_path.clone(),
            Arc::new("SELECT\n  user_id,\n  event_id,\n  event_time\nFROM source.events".to_string()),
        );

        // Create downstream model
        let sessions_path = PathBuf::from("models/user_sessions.sql");
        db.set_file_text(
            sessions_path.clone(),
            Arc::new("SELECT\n  user_id\nFROM {{ ref('raw_events') }}".to_string()),
        );

        db.set_all_files(Arc::new(vec![raw_events_path.clone(), sessions_path.clone()]));

        let available = db.available_columns(sessions_path);

        // Should include current model's columns (1) + upstream columns (3) = 4
        assert_eq!(available.len(), 4);

        let column_names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
        assert!(column_names.contains(&"user_id"));
        assert!(column_names.contains(&"event_id"));
        assert!(column_names.contains(&"event_time"));
    }
}
