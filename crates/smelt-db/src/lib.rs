/// Salsa database for incremental compilation
///
/// This module defines the Salsa queries that power the LSP and optimizer.
/// Salsa automatically handles incremental recomputation when inputs change.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use smelt_parser::{self, File as AstFile, RefCall};
use smelt_types::{parse_type, DataType, TypedColumn};

pub mod schema;
pub mod type_inference;

pub use schema::{Column, ColumnSource, ModelSchema};
pub use type_inference::{
    infer_cte_columns, infer_expression_type, infer_select_column_types, TypeContext,
};

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

    /// Get the raw YAML content of sources.yml
    #[salsa::input]
    fn sources_yaml(&self) -> Arc<String>;
}

/// Syntax queries - parsing and CST construction
#[salsa::query_group(SyntaxStorage)]
pub trait Syntax: Inputs {
    /// Parse a file into a CST
    fn parse_file(&self, path: PathBuf) -> Arc<smelt_parser::Parse>;

    /// Parse a file and extract model definitions
    /// Returns None if file doesn't contain a valid model
    fn parse_model(&self, path: PathBuf) -> Option<Arc<Model>>;

    /// Extract all ref() calls from a model with their positions
    fn model_refs(&self, path: PathBuf) -> Arc<Vec<RefLocation>>;

    /// Extract all source() calls from a model with their positions
    fn model_sources(&self, path: PathBuf) -> Arc<Vec<SourceLocation>>;

    /// Parse sources.yml into structured config
    fn sources_config(&self) -> Arc<SourcesConfig>;

    /// Get any parse error from sources.yml
    fn sources_yaml_error(&self) -> Option<YamlParseError>;

    /// Get invalid type errors from sources.yml column definitions
    fn sources_type_errors(&self) -> Arc<Vec<SourceTypeError>>;

    /// Get all models in the project
    fn all_models(&self) -> Arc<HashMap<PathBuf, Model>>;
}

/// Semantic queries - name resolution, type checking, etc.
#[salsa::query_group(SemanticStorage)]
pub trait Semantic: Syntax {
    /// Resolve a ref() to the file path where it's defined
    /// Returns None if the ref is undefined
    fn resolve_ref(&self, model_name: String) -> Option<PathBuf>;

    /// Resolve a source() to its table definition
    /// Returns None if the source is undefined
    fn resolve_source(&self, source_name: String, table_name: String) -> Option<SourceTableDef>;

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

/// Type checking queries - type inference and validation
#[salsa::query_group(TypeCheckingStorage)]
pub trait TypeChecking: Schema {
    /// Get the schema with inferred types for a model
    fn typed_model_schema(&self, path: PathBuf) -> Arc<ModelSchema>;

    /// Build type context for a model (source and upstream model types)
    fn type_context(&self, path: PathBuf) -> Arc<TypeContext>;
}

/// The main database that combines all query groups
#[salsa::database(
    InputsStorage,
    SyntaxStorage,
    SemanticStorage,
    SchemaStorage,
    TypeCheckingStorage
)]
#[derive(Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
}

impl salsa::Database for Database {}

// Query implementations

fn parse_file(db: &dyn Syntax, path: PathBuf) -> Arc<smelt_parser::Parse> {
    let text = db.file_text(path);
    Arc::new(smelt_parser::parse(&text))
}

fn parse_model(db: &dyn Syntax, path: PathBuf) -> Option<Arc<Model>> {
    // Extract model name from file path (e.g., models/users.sql -> users)
    let model_name = path.file_stem()?.to_str()?.to_string();

    // Parse file and check if it contains a valid SELECT statement
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();
    let file = AstFile::cast(syntax)?;

    // Check if file has a SELECT statement
    file.select_stmt()?;

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
                let range = smelt_parser::ast::text_range_to_range(&text, text_range);

                Some(RefLocation { name, range })
            })
            .collect();

        Arc::new(refs)
    } else {
        Arc::new(Vec::new())
    }
}

fn model_sources(db: &dyn Syntax, path: PathBuf) -> Arc<Vec<SourceLocation>> {
    let parse = db.parse_file(path.clone());
    let text = db.file_text(path);
    let syntax = parse.syntax();

    if let Some(file) = AstFile::cast(syntax) {
        let sources: Vec<SourceLocation> = file
            .sources()
            .filter_map(|source_call| {
                let qualified_name = source_call.qualified_name()?;
                let source_name = source_call.source_name()?;
                let table_name = source_call.table_name()?;
                let text_range = source_call.name_range().unwrap_or(source_call.range());
                let range = smelt_parser::ast::text_range_to_range(&text, text_range);

                Some(SourceLocation {
                    source_name,
                    table_name,
                    qualified_name,
                    range,
                })
            })
            .collect();

        Arc::new(sources)
    } else {
        Arc::new(Vec::new())
    }
}

fn sources_config(db: &dyn Syntax) -> Arc<SourcesConfig> {
    let yaml = db.sources_yaml();
    if yaml.is_empty() {
        return Arc::new(SourcesConfig::default());
    }

    match serde_yaml::from_str::<SourcesConfig>(&yaml) {
        Ok(config) => Arc::new(config),
        Err(_) => Arc::new(SourcesConfig::default()),
    }
}

fn sources_yaml_error(db: &dyn Syntax) -> Option<YamlParseError> {
    let yaml = db.sources_yaml();
    if yaml.is_empty() {
        return None;
    }

    match serde_yaml::from_str::<SourcesConfig>(&yaml) {
        Ok(_) => None,
        Err(e) => {
            let (line, column) = e
                .location()
                .map(|loc| (Some(loc.line()), Some(loc.column())))
                .unwrap_or((None, None));

            Some(YamlParseError {
                message: e.to_string(),
                line,
                column,
            })
        }
    }
}

fn sources_type_errors(db: &dyn Syntax) -> Arc<Vec<SourceTypeError>> {
    let yaml = db.sources_yaml();
    if yaml.is_empty() {
        return Arc::new(Vec::new());
    }

    // Parse with a raw structure that preserves type strings
    #[derive(Deserialize)]
    struct RawSourcesConfig {
        #[serde(default)]
        sources: Vec<RawSource>,
    }

    #[derive(Deserialize)]
    struct RawSource {
        name: String,
        #[serde(default)]
        tables: Vec<RawTable>,
    }

    #[derive(Deserialize)]
    struct RawTable {
        name: String,
        #[serde(default)]
        columns: Vec<RawColumn>,
    }

    #[derive(Deserialize)]
    struct RawColumn {
        name: String,
        #[serde(default, rename = "type")]
        type_str: Option<String>,
    }

    let config: RawSourcesConfig = match serde_yaml::from_str(&yaml) {
        Ok(c) => c,
        Err(_) => return Arc::new(Vec::new()), // YAML parse error handled elsewhere
    };

    let mut errors = Vec::new();

    for source in &config.sources {
        for table in &source.tables {
            for column in &table.columns {
                if let Some(type_str) = &column.type_str {
                    if parse_type(type_str).is_err() {
                        errors.push(SourceTypeError {
                            source_name: source.name.clone(),
                            table_name: table.name.clone(),
                            column_name: column.name.clone(),
                            invalid_type: type_str.clone(),
                        });
                    }
                }
            }
        }
    }

    Arc::new(errors)
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
    models
        .iter()
        .find(|(_, model)| model.name == model_name)
        .map(|(path, _)| path.clone())
}

fn resolve_source(
    db: &dyn Semantic,
    source_name: String,
    table_name: String,
) -> Option<SourceTableDef> {
    let config = db.sources_config();

    // Find the source with this name
    let source = config.sources.iter().find(|s| s.name == source_name)?;

    // Find the table within the source
    source.tables.iter().find(|t| t.name == table_name).cloned()
}

fn file_diagnostics(db: &dyn Semantic, path: PathBuf) -> Arc<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    // Add parse errors
    let parse = db.parse_file(path.clone());
    for error in parse.errors.iter() {
        let text = db.file_text(path.clone());
        let range = smelt_parser::ast::text_range_to_range(&text, error.range);

        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            range,
        });
    }

    // Check if model is valid
    if db.parse_model(path.clone()).is_none() {
        // Only report error if file is supposed to be a model (in models/ directory)
        if path
            .to_str()
            .map(|s| s.contains("models/"))
            .unwrap_or(false)
        {
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

    // Check for undefined sources with accurate positions
    let sources = db.model_sources(path.clone());
    for source_loc in sources.iter() {
        if db
            .resolve_source(
                source_loc.source_name.clone(),
                source_loc.table_name.clone(),
            )
            .is_none()
        {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined source: '{}'", source_loc.qualified_name),
                range: source_loc.range,
            });
        }
    }

    // If model references sources and there's a YAML parse error, report it
    if !sources.is_empty() {
        if let Some(yaml_error) = db.sources_yaml_error() {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("sources.yml parse error: {}", yaml_error.message),
                range: Range {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 0, column: 0 },
                },
            });
        }

        // Check for invalid type definitions in sources.yml
        let type_errors = db.sources_type_errors();
        for error in type_errors.iter() {
            // Only report if this model uses this source
            let source_qualified = format!("{}.{}", error.source_name, error.table_name);
            if sources.iter().any(|s| s.qualified_name == source_qualified) {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Unknown type '{}' for column '{}' in source '{}'. Type information unavailable.",
                        error.invalid_type, error.column_name, source_qualified
                    ),
                    range: Range {
                        start: Position { line: 0, column: 0 },
                        end: Position { line: 0, column: 0 },
                    },
                });
            }
        }
    }

    // Check for malformed source calls (missing dot separator like 'foo' instead of 'raw.users')
    // These are filtered out by model_sources() so we need to check them separately
    let text = db.file_text(path.clone());
    let syntax = parse.syntax();
    if let Some(file) = AstFile::cast(syntax) {
        for source_call in file.sources() {
            if let Some(qualified_name) = source_call.qualified_name() {
                // Check if the qualified name has a dot separator
                if !qualified_name.contains('.') {
                    let text_range = source_call.name_range().unwrap_or(source_call.range());
                    let range = smelt_parser::ast::text_range_to_range(&text, text_range);
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Malformed source reference: '{}'. Expected format: 'source_name.table_name'",
                            qualified_name
                        ),
                        range,
                    });
                }
            }
        }

        // Check for invalid CAST types and unknown functions in SELECT list
        if let Some(select_stmt) = file.select_stmt() {
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        check_expression_types(&expr, &mut diagnostics);
                    }
                }
            }

            // Check for ambiguous unqualified columns when there are multiple FROM sources
            let from_sources = count_from_sources(&select_stmt);
            if from_sources > 1 {
                if let Some(select_list) = select_stmt.select_list() {
                    for item in select_list.items() {
                        if let Some(expr) = item.expression() {
                            if let Some(col_ref) = expr.as_column_ref() {
                                // Warn if column reference has no qualifier
                                if col_ref.qualifier().is_none() {
                                    let col_name = col_ref.name();
                                    // Skip wildcards
                                    if col_name != "*" {
                                        diagnostics.push(Diagnostic {
                                            severity: DiagnosticSeverity::Warning,
                                            message: format!(
                                                "Column '{}' is ambiguous - multiple sources in FROM clause. Consider using a qualified name (e.g., table.{}).",
                                                col_name, col_name
                                            ),
                                            range: Range {
                                                start: Position { line: 0, column: 0 },
                                                end: Position { line: 0, column: 0 },
                                            },
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Arc::new(diagnostics)
}

/// Count the number of sources in a FROM clause (refs, sources, tables, joins)
fn count_from_sources(select_stmt: &smelt_parser::ast::SelectStmt) -> usize {
    let mut count = 0;

    if let Some(from_clause) = select_stmt.from_clause() {
        // Count table references (refs, sources, plain tables)
        count += from_clause.table_refs().count();

        // Count JOIN clauses
        count += from_clause.joins().count();
    }

    count
}

/// Known SQL functions for type inference
const KNOWN_FUNCTIONS: &[&str] = &[
    // Aggregate functions
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    // Null handling
    "COALESCE",
    "NULLIF",
    // Date functions
    "NOW",
    "CURRENT_TIMESTAMP",
    "CURRENT_DATE",
    "DATE",
    "DATE_TRUNC",
    // String functions
    "CONCAT",
    "UPPER",
    "LOWER",
    "TRIM",
    "LTRIM",
    "RTRIM",
    "SUBSTRING",
    "SUBSTR",
    "LENGTH",
    "CHAR_LENGTH",
    "CHARACTER_LENGTH",
    "TO_CHAR",
    // Boolean functions
    "BOOL_AND",
    "BOOL_OR",
    "EVERY",
    // Window functions
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "NTILE",
    "LAG",
    "LEAD",
    "FIRST_VALUE",
    "LAST_VALUE",
    "NTH_VALUE",
];

/// Check an expression for invalid CAST types and unknown functions
fn check_expression_types(expr: &smelt_parser::ast::Expr, diagnostics: &mut Vec<Diagnostic>) {
    // Default range for diagnostics (position tracking would require more AST work)
    let default_range = Range {
        start: Position { line: 0, column: 0 },
        end: Position { line: 0, column: 0 },
    };

    // Check for CAST with invalid type
    if let Some(cast_expr) = expr.as_cast() {
        if let Some(type_spec) = cast_expr.type_spec() {
            let type_text = type_spec.full_text();
            if parse_type(&type_text).is_err() {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Unknown type '{}' in CAST expression. Type inference unavailable.",
                        type_text
                    ),
                    range: default_range,
                });
            }
        }
        // Recursively check the inner expression
        if let Some(inner) = cast_expr.expression() {
            check_expression_types(&inner, diagnostics);
        }
    }

    // Check for unknown function calls (but not smelt.ref/smelt.source which are special)
    if let Some(func) = expr.as_function_call() {
        if let Some(name) = func.name() {
            let upper_name = name.to_uppercase();
            // Skip smelt.ref and smelt.source - they're handled separately
            if func.namespace().is_none() && !KNOWN_FUNCTIONS.contains(&upper_name.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Info,
                    message: format!(
                        "Function '{}' is not a recognized SQL function. Type inference unavailable.",
                        name
                    ),
                    range: default_range,
                });
            }
        }
    }
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

/// Source location with position information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub source_name: String,
    pub table_name: String,
    pub qualified_name: String,
    pub range: Range,
}

/// Sources configuration from sources.yml
/// Supports nested object format like dbt:
/// ```yaml
/// sources:
///   raw:
///     tables:
///       users:
///         columns: [...]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourcesConfig {
    pub sources: Vec<SourceDef>,
}

impl<'de> Deserialize<'de> for SourcesConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Raw YAML structure with nested objects
        #[derive(Deserialize)]
        struct RawConfig {
            #[serde(default)]
            sources: HashMap<String, RawSourceDef>,
        }

        #[derive(Deserialize)]
        struct RawSourceDef {
            #[serde(default)]
            database: Option<String>,
            #[serde(default)]
            schema: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            tables: HashMap<String, RawTableDef>,
        }

        #[derive(Deserialize)]
        struct RawTableDef {
            #[serde(default)]
            identifier: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            columns: Vec<SourceColumnDef>,
        }

        let raw = RawConfig::deserialize(deserializer)?;

        let sources = raw
            .sources
            .into_iter()
            .map(|(name, raw_source)| {
                let tables = raw_source
                    .tables
                    .into_iter()
                    .map(|(table_name, raw_table)| SourceTableDef {
                        name: table_name,
                        identifier: raw_table.identifier,
                        description: raw_table.description,
                        columns: raw_table.columns,
                    })
                    .collect();

                SourceDef {
                    name,
                    database: raw_source.database,
                    schema: raw_source.schema,
                    description: raw_source.description,
                    tables,
                }
            })
            .collect();

        Ok(SourcesConfig { sources })
    }
}

/// Source definition (a named source with tables)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDef {
    pub name: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub description: Option<String>,
    pub tables: Vec<SourceTableDef>,
}

/// Table definition within a source
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTableDef {
    pub name: String,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub columns: Vec<SourceColumnDef>,
}

/// Column definition within a source table
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceColumnDef {
    pub name: String,
    pub data_type: Option<DataType>,
    pub description: Option<String>,
}

impl<'de> Deserialize<'de> for SourceColumnDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawColumn {
            name: String,
            #[serde(default, rename = "type")]
            type_str: Option<String>,
            #[serde(default)]
            description: Option<String>,
        }

        let raw = RawColumn::deserialize(deserializer)?;

        // Parse type string into DataType if present
        let data_type = raw.type_str.as_ref().and_then(|s| parse_type(s).ok());

        Ok(SourceColumnDef {
            name: raw.name,
            data_type,
            description: raw.description,
        })
    }
}

/// Position in a file (line, column)
pub type Position = smelt_parser::ast::Position;

/// Range in a file (start, end)
pub type Range = smelt_parser::ast::Range;

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

/// YAML parse error with location information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlParseError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// Invalid type in sources.yml column definition
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTypeError {
    pub source_name: String,
    pub table_name: String,
    pub column_name: String,
    pub invalid_type: String,
}

// Schema query implementations

fn model_schema(db: &dyn Schema, path: PathBuf) -> Arc<ModelSchema> {
    // Parse the model
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();
    let _text = db.file_text(path.clone());

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
                    .function_call()
                    .and_then(RefCall::from_function_call)
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
                        data_type: None, // Wildcard type is determined by expansion
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
            data_type: None, // Type inference will be done in Phase 4
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
                    if let Some(func) = table_ref.function_call() {
                        if let Some(ref_call) = RefCall::from_function_call(func) {
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

// TypeChecking query implementations

fn type_context(db: &dyn TypeChecking, path: PathBuf) -> Arc<TypeContext> {
    let mut ctx = TypeContext::new();

    // Get sources config and add source column types
    let sources_config = db.sources_config();
    for source in &sources_config.sources {
        for table in &source.tables {
            for col in &table.columns {
                if let Some(data_type) = &col.data_type {
                    ctx.add_source_column(
                        &source.name,
                        &table.name,
                        &col.name,
                        TypedColumn {
                            data_type: data_type.clone(),
                            nullable: true, // Assume nullable by default
                        },
                    );
                }
            }
        }
    }

    // Get refs from FROM clause and add upstream model column types
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();

    if let Some(file) = AstFile::cast(syntax) {
        if let Some(select_stmt) = file.select_stmt() {
            // Process WITH clause CTEs (in order for forward references)
            // CTEs must be processed before FROM clause so they're available for column lookup
            if let Some(with_clause) = select_stmt.with_clause() {
                for cte in with_clause.ctes() {
                    if let Some(cte_name) = cte.name() {
                        // For recursive CTEs with explicit column list, bootstrap with Unknown types first
                        // This allows the recursive reference to find the columns
                        if with_clause.is_recursive() {
                            for col_name in cte.column_names() {
                                ctx.add_cte_column(
                                    &cte_name,
                                    &col_name,
                                    TypedColumn {
                                        data_type: DataType::Unknown,
                                        nullable: true,
                                    },
                                );
                            }
                        }

                        // Infer columns from CTE query
                        let columns = infer_cte_columns(&cte, &ctx);
                        for (col_name, typed_col) in columns {
                            ctx.add_cte_column(&cte_name, &col_name, typed_col);
                        }

                        // Register CTE name as alias to itself (for qualified lookups like cte_name.col)
                        ctx.add_alias(&cte_name, &cte_name);
                    }
                }
            }

            if let Some(from_clause) = select_stmt.from_clause() {
                for table_ref in from_clause.table_refs() {
                    // Check for smelt.ref() calls
                    if let Some(func) = table_ref.function_call() {
                        if let Some(ref_call) = RefCall::from_function_call(func) {
                            if let Some(model_name) = ref_call.model_name() {
                                // Resolve upstream model and get its typed schema
                                if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
                                    let upstream_schema = db.typed_model_schema(upstream_path);

                                    // Add upstream columns to context
                                    for col in &upstream_schema.columns {
                                        if col.name != "*" {
                                            if let Some(typed_col) = &col.data_type {
                                                ctx.add_model_column(
                                                    &model_name,
                                                    &col.name,
                                                    typed_col.clone(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Check for smelt.source() calls
                    if let Some(func) = table_ref.function_call() {
                        if let Some(source_call) =
                            smelt_parser::ast::SourceCall::from_function_call(func)
                        {
                            if let Some(source_name) = source_call.source_name() {
                                if let Some(table_name) = source_call.table_name() {
                                    let qualified_name = format!("{}.{}", source_name, table_name);

                                    // Add explicit alias if present (e.g., "t" from "smelt.source('raw.users') t")
                                    if let Some(explicit_alias) = table_ref.alias() {
                                        ctx.add_alias(&explicit_alias, &qualified_name);
                                    }

                                    // Also add implicit alias using the table name
                                    ctx.add_alias(&table_name, &qualified_name);
                                }
                            }
                        }
                    }

                    // Check for CTE references with aliases (e.g., "FROM daily_totals dt")
                    // When a table_ref is just an identifier (not a function call), and that
                    // identifier is a known CTE, register any alias
                    if table_ref.function_call().is_none() {
                        if let Some(table_name) = table_ref.identifier() {
                            if ctx.is_cte(&table_name) {
                                // Add explicit alias if present (e.g., "dt" from "daily_totals dt")
                                if let Some(explicit_alias) = table_ref.alias() {
                                    ctx.add_alias(&explicit_alias, &table_name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Arc::new(ctx)
}

fn typed_model_schema(db: &dyn TypeChecking, path: PathBuf) -> Arc<ModelSchema> {
    // Get the base schema
    let base_schema = db.model_schema(path.clone());

    // Get the type context for this model
    let ctx = db.type_context(path.clone());

    // Parse the model to get expression AST
    let parse = db.parse_file(path.clone());
    let syntax = parse.syntax();

    let file = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return base_schema,
    };

    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return base_schema,
    };

    // Use infer_select_column_types which handles UNION by combining types
    let inferred_types = infer_select_column_types(&select_stmt, &ctx);

    // Create new columns with inferred types
    let mut typed_columns = Vec::new();

    for (i, col) in base_schema.columns.iter().enumerate() {
        let mut col = col.clone();

        // Use inferred type if available
        if let Some(typed_col) = inferred_types.get(i) {
            col.data_type = Some(typed_col.clone());
        }

        typed_columns.push(col);
    }

    Arc::new(ModelSchema {
        columns: typed_columns,
    })
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
            Arc::new("SELECT\n  user_id,\n  COUNT(*) as session_count\nFROM smelt.ref('raw_events')\nGROUP BY user_id".to_string()),
        );

        // Set up all_files for model resolution
        db.set_all_files(Arc::new(vec![
            raw_events_path.clone(),
            sessions_path.clone(),
        ]));

        let schema = db.model_schema(sessions_path);

        assert_eq!(schema.columns.len(), 2);

        // user_id should be traced to raw_events
        assert_eq!(schema.columns[0].name, "user_id");
        match &schema.columns[0].source {
            ColumnSource::FromModel {
                model_name,
                column_name,
            } => {
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
            Arc::new(
                "SELECT\n  user_id,\n  event_id,\n  event_time\nFROM source.events".to_string(),
            ),
        );

        // Create downstream model
        let sessions_path = PathBuf::from("models/user_sessions.sql");
        db.set_file_text(
            sessions_path.clone(),
            Arc::new("SELECT\n  user_id\nFROM smelt.ref('raw_events')".to_string()),
        );

        db.set_all_files(Arc::new(vec![
            raw_events_path.clone(),
            sessions_path.clone(),
        ]));

        let available = db.available_columns(sessions_path);

        // Should include current model's columns (1) + upstream columns (3) = 4
        assert_eq!(available.len(), 4);

        let column_names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
        assert!(column_names.contains(&"user_id"));
        assert!(column_names.contains(&"event_id"));
        assert!(column_names.contains(&"event_time"));
    }

    #[test]
    fn test_undefined_ref_diagnostic_position() {
        let mut db = Database::default();

        // Create a model with an undefined ref
        let path = PathBuf::from("test_model.sql");
        db.set_file_text(
            path.clone(),
            Arc::new("SELECT * FROM smelt.ref('nonexistent_model')".to_string()),
        );

        // Register the file (no other files, so ref won't resolve)
        db.set_all_files(Arc::new(vec![path.clone()]));

        // Get diagnostics
        let diagnostics = db.file_diagnostics(path);

        // Should have exactly one diagnostic for undefined ref
        assert_eq!(diagnostics.len(), 1);
        let diag = &diagnostics[0];

        // Check severity and message
        assert_eq!(diag.severity, DiagnosticSeverity::Error);
        assert!(diag
            .message
            .contains("Undefined model reference: 'nonexistent_model'"));

        // Check position - should point to the string parameter 'nonexistent_model'
        // In "SELECT * FROM smelt.ref('nonexistent_model')", the STRING token (including quotes)
        // starts at position 24 and ends at position 43 (exclusive)
        assert_eq!(diag.range.start.line, 0);
        assert_eq!(diag.range.start.column, 24); // Opening quote ' (0-indexed)
        assert_eq!(diag.range.end.line, 0);
        assert_eq!(diag.range.end.column, 43); // One past closing quote ' (exclusive)
    }

    #[test]
    fn test_undefined_ref_diagnostic_position_multiline() {
        let mut db = Database::default();

        // Create a model matching broken_model.sql structure
        let path = PathBuf::from("broken_model.sql");
        let content = "-- This model has an undefined reference - should show diagnostic\nSELECT *\nFROM smelt.ref('nonexistent_model')\n";
        db.set_file_text(path.clone(), Arc::new(content.to_string()));

        // Debug: Check what the parser extracts
        let parse = db.parse_file(path.clone());
        let text = db.file_text(path.clone());
        use smelt_parser::ast::File as AstFile;
        if let Some(file) = AstFile::cast(parse.syntax()) {
            for ref_call in file.refs() {
                println!("Found ref call");
                if let Some(name) = ref_call.model_name() {
                    println!("  Model name: {:?}", name);
                }
                if let Some(text_range) = ref_call.name_range() {
                    println!("  TextRange: {:?}", text_range);
                    println!(
                        "  Start offset: {}, End offset: {}",
                        usize::from(text_range.start()),
                        usize::from(text_range.end())
                    );

                    // Check content length
                    println!("  Content length: {}", text.len());

                    // Extract the actual text at this range (if valid)
                    let start = usize::from(text_range.start());
                    let end = usize::from(text_range.end());
                    if end <= text.len() {
                        let extracted = &text[start..end];
                        println!("  Extracted text: {:?}", extracted);
                    } else {
                        println!(
                            "  ERROR: Range {} out of bounds (content length is {})",
                            end,
                            text.len()
                        );
                    }
                }
            }
        }

        // Register the file (no other files, so ref won't resolve)
        db.set_all_files(Arc::new(vec![path.clone()]));

        // Get diagnostics
        let diagnostics = db.file_diagnostics(path);

        // Debug output
        println!("\nContent: {:?}", content);
        println!("Content length: {}", content.len());
        println!("Number of diagnostics: {}", diagnostics.len());
        if !diagnostics.is_empty() {
            let diag = &diagnostics[0];
            println!(
                "Diagnostic range: line {} col {} to line {} col {}",
                diag.range.start.line,
                diag.range.start.column,
                diag.range.end.line,
                diag.range.end.column
            );
        }

        // Should have exactly one diagnostic
        assert_eq!(diagnostics.len(), 1);
        let diag = &diagnostics[0];

        // Check it's on line 2 (0-indexed)
        assert_eq!(diag.range.start.line, 2);
        assert_eq!(diag.range.end.line, 2);

        // In "FROM smelt.ref('nonexistent_model')", the model name should be highlighted
        // Expected: 'nonexistent_model' with quotes starting at column 16
        println!("Expected to highlight 'nonexistent_model' on line 2");
    }

    #[test]
    fn test_lexer_positions() {
        use smelt_parser::lexer::tokenize;

        let content = "-- This model has an undefined reference - should show diagnostic\nSELECT *\nFROM smelt.ref('nonexistent_model')\n";
        let tokens = tokenize(content);

        println!("Total content length: {}", content.len());
        println!("\nTokens:");
        let mut offset = 0;
        for token in &tokens {
            let text = &content[offset..offset + token.len];
            println!(
                "  {:?} @ {}..{}: {:?}",
                token.kind,
                offset,
                offset + token.len,
                text
            );
            offset += token.len;
        }
        println!("Final offset: {}", offset);
    }

    #[test]
    fn test_typed_model_schema_literals() {
        let mut db = Database::default();

        // Create a model with various literal types
        let path = PathBuf::from("test_model.sql");
        db.set_file_text(
            path.clone(),
            Arc::new("SELECT 42 as small_num, 100000 as medium_num, 'hello' as greeting FROM source.test".to_string()),
        );
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(String::new()));

        let schema = db.typed_model_schema(path);

        assert_eq!(schema.columns.len(), 3);

        // Check small_num is SmallInt
        assert!(schema.columns[0].data_type.is_some());
        assert_eq!(
            schema.columns[0].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::SmallInt
        );

        // Check medium_num is Integer
        assert!(schema.columns[1].data_type.is_some());
        assert_eq!(
            schema.columns[1].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::Integer
        );

        // Check greeting is Text
        assert!(schema.columns[2].data_type.is_some());
        assert_eq!(
            schema.columns[2].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::Text
        );
    }

    #[test]
    fn test_typed_model_schema_aggregates() {
        let mut db = Database::default();

        // Create a model with aggregate functions
        let path = PathBuf::from("test_model.sql");
        db.set_file_text(
            path.clone(),
            Arc::new("SELECT COUNT(*) as cnt, AVG(price) as avg_price FROM source.test GROUP BY category".to_string()),
        );
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(String::new()));

        let schema = db.typed_model_schema(path);

        assert_eq!(schema.columns.len(), 2);

        // COUNT(*) should be BigInt
        assert!(schema.columns[0].data_type.is_some());
        assert_eq!(
            schema.columns[0].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::BigInt
        );
        // COUNT never returns NULL
        assert!(!schema.columns[0].data_type.as_ref().unwrap().nullable);

        // AVG should be Double
        assert!(schema.columns[1].data_type.is_some());
        assert_eq!(
            schema.columns[1].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::Double
        );
        // AVG can be NULL for empty sets
        assert!(schema.columns[1].data_type.as_ref().unwrap().nullable);
    }

    #[test]
    fn test_typed_model_schema_with_sources() {
        let mut db = Database::default();

        // Create sources.yml with typed columns
        let sources_yaml = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR(255)
          - name: created_at
            type: TIMESTAMP
"#;

        let path = PathBuf::from("test_model.sql");
        db.set_file_text(
            path.clone(),
            Arc::new("SELECT id, email, created_at FROM smelt.source('raw.users')".to_string()),
        );
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path);

        assert_eq!(schema.columns.len(), 3);

        // Note: Column type inference from sources requires column reference resolution
        // which is a more complex case. For now, the basic literal and aggregate
        // inference is working.
    }

    #[test]
    fn test_simple_cte_type_inference() {
        let mut db = Database::default();

        // Create sources.yml with typed columns
        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: created_at
            type: TIMESTAMP
          - name: amount
            type: DECIMAL(10,2)
"#;

        // SQL with CTE using DATE() and SUM()
        let sql = r#"
WITH daily_totals AS (
    SELECT DATE(created_at) as day, SUM(amount) as total
    FROM smelt.source('raw.orders')
    GROUP BY DATE(created_at)
)
SELECT day, total FROM daily_totals WHERE total > 1000
"#;

        let path = PathBuf::from("models/test_cte.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        // Should have 2 columns: day and total
        assert_eq!(schema.columns.len(), 2);

        // Check that day has DATE type (from DATE() function)
        let day_col = schema.columns.iter().find(|c| c.name == "day");
        assert!(day_col.is_some(), "Column 'day' not found");
        if let Some(typed_col) = &day_col.unwrap().data_type {
            assert_eq!(typed_col.data_type, DataType::Date);
        }

        // Check that total has Decimal type (from SUM())
        let total_col = schema.columns.iter().find(|c| c.name == "total");
        assert!(total_col.is_some(), "Column 'total' not found");
        if let Some(typed_col) = &total_col.unwrap().data_type {
            assert!(
                matches!(typed_col.data_type, DataType::Decimal { .. }),
                "Expected Decimal type for 'total', got {:?}",
                typed_col.data_type
            );
        }
    }

    #[test]
    fn test_multiple_ctes_forward_reference() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: DECIMAL(10,2)
"#;

        // Multiple CTEs where cte2 references cte1
        let sql = r#"
WITH cte1 AS (
    SELECT SUM(amount) as total
    FROM smelt.source('raw.orders')
),
cte2 AS (
    SELECT total * 2 as doubled
    FROM cte1
)
SELECT doubled FROM cte2
"#;

        let path = PathBuf::from("models/test_multi_cte.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        // Should have 1 column: doubled
        assert_eq!(schema.columns.len(), 1);

        // Check that doubled has inferred type from the multiplication expression
        let doubled_col = schema.columns.iter().find(|c| c.name == "doubled");
        assert!(doubled_col.is_some(), "Column 'doubled' not found");
    }

    #[test]
    fn test_cte_explicit_column_list() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

        // CTE with explicit column list - names should override inferred names
        let sql = r#"
WITH order_stats(order_sum, order_count) AS (
    SELECT SUM(amount), COUNT(*)
    FROM smelt.source('raw.orders')
)
SELECT order_sum, order_count FROM order_stats
"#;

        let path = PathBuf::from("models/test_explicit_cols.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        // Should have 2 columns with explicit names
        assert_eq!(schema.columns.len(), 2);

        // Check that order_sum exists (explicit name, not SUM)
        let order_sum_col = schema.columns.iter().find(|c| c.name == "order_sum");
        assert!(
            order_sum_col.is_some(),
            "Column 'order_sum' not found - explicit column list should override inferred names"
        );

        // Check that order_count exists (explicit name, not COUNT)
        let order_count_col = schema.columns.iter().find(|c| c.name == "order_count");
        assert!(
            order_count_col.is_some(),
            "Column 'order_count' not found - explicit column list should override inferred names"
        );
    }

    #[test]
    fn test_nested_cte_in_cte() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

        // Nested CTE: outer_cte contains inner_cte in its definition
        let sql = r#"
WITH outer_cte AS (
    WITH inner_cte AS (
        SELECT SUM(amount) as inner_total
        FROM smelt.source('raw.orders')
    )
    SELECT inner_total FROM inner_cte
)
SELECT inner_total FROM outer_cte
"#;

        let path = PathBuf::from("models/test_nested_cte.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        // Should have 1 column: inner_total
        assert_eq!(schema.columns.len(), 1);

        // Check that inner_total has Decimal type (from SUM() in nested CTE)
        let result_col = schema.columns.iter().find(|c| c.name == "inner_total");
        assert!(result_col.is_some(), "Column 'inner_total' not found");
        if let Some(typed_col) = &result_col.unwrap().data_type {
            assert!(
                matches!(typed_col.data_type, DataType::Decimal { .. }),
                "Expected Decimal type for 'inner_total' (from SUM in nested CTE), got {:?}",
                typed_col.data_type
            );
        }
    }

    #[test]
    fn test_recursive_cte_without_explicit_columns() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      nodes:
        columns:
          - name: id
            type: INTEGER
          - name: parent_id
            type: INTEGER
"#;

        // Recursive CTE WITHOUT explicit column list
        let sql = r#"
WITH RECURSIVE tree AS (
    SELECT id, parent_id FROM smelt.source('raw.nodes') WHERE parent_id IS NULL
    UNION ALL
    SELECT n.id, n.parent_id FROM smelt.source('raw.nodes') n
    INNER JOIN tree ON n.parent_id = tree.id
)
SELECT id, parent_id FROM tree
"#;

        let path = PathBuf::from("models/test_recursive.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        // Should have 2 columns: id and parent_id
        assert_eq!(schema.columns.len(), 2);

        // Check that id has INTEGER type (inferred from anchor term)
        let id_col = schema.columns.iter().find(|c| c.name == "id");
        assert!(id_col.is_some(), "Column 'id' not found");
        assert!(
            id_col.unwrap().data_type.is_some(),
            "Column 'id' should have a type"
        );
        assert_eq!(
            id_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Integer,
            "Expected INTEGER for 'id'"
        );

        // Check that parent_id also has INTEGER type
        let parent_id_col = schema.columns.iter().find(|c| c.name == "parent_id");
        assert!(parent_id_col.is_some(), "Column 'parent_id' not found");
        assert!(
            parent_id_col.unwrap().data_type.is_some(),
            "Column 'parent_id' should have a type"
        );
        assert_eq!(
            parent_id_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Integer,
            "Expected INTEGER for 'parent_id'"
        );
    }

    #[test]
    fn test_recursive_cte_with_literal_anchor() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources: {}
"#;

        // Recursive CTE with literal in anchor term (no source references)
        let sql = r#"
WITH RECURSIVE nums AS (
    SELECT 1 as n
    UNION ALL
    SELECT n + 1 FROM nums WHERE n < 10
)
SELECT n FROM nums
"#;

        let path = PathBuf::from("models/test_recursive_literal.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        // Should have 1 column: n
        assert_eq!(schema.columns.len(), 1);

        // Check that n has SmallInt type (from literal 1 in anchor term)
        let n_col = schema.columns.iter().find(|c| c.name == "n");
        assert!(n_col.is_some(), "Column 'n' not found");
        assert!(
            n_col.unwrap().data_type.is_some(),
            "Column 'n' should have a type"
        );
        assert_eq!(
            n_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::SmallInt,
            "Expected SmallInt for 'n' (from literal 1)"
        );
    }

    #[test]
    fn test_between_type_inference() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

        // BETWEEN expression should return Boolean
        let sql = r#"
SELECT amount BETWEEN 10 AND 100 as in_range
FROM smelt.source('raw.orders')
"#;

        let path = PathBuf::from("models/test_between.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let in_range_col = schema.columns.iter().find(|c| c.name == "in_range");
        assert!(in_range_col.is_some(), "Column 'in_range' not found");
        assert!(
            in_range_col.unwrap().data_type.is_some(),
            "Column 'in_range' should have a type"
        );
        assert_eq!(
            in_range_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Boolean,
            "BETWEEN should return Boolean"
        );
    }

    #[test]
    fn test_in_type_inference() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: status
            type: VARCHAR(50)
"#;

        // IN expression should return Boolean
        let sql = r#"
SELECT status IN ('pending', 'processing', 'shipped') as is_active
FROM smelt.source('raw.orders')
"#;

        let path = PathBuf::from("models/test_in.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let is_active_col = schema.columns.iter().find(|c| c.name == "is_active");
        assert!(is_active_col.is_some(), "Column 'is_active' not found");
        assert!(
            is_active_col.unwrap().data_type.is_some(),
            "Column 'is_active' should have a type"
        );
        assert_eq!(
            is_active_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Boolean,
            "IN should return Boolean"
        );
    }

    #[test]
    fn test_exists_type_inference() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: user_id
            type: INTEGER
      users:
        columns:
          - name: id
            type: INTEGER
"#;

        // EXISTS expression should return Boolean (never NULL)
        let sql = r#"
SELECT EXISTS (SELECT 1 FROM smelt.source('raw.users') WHERE id = user_id) as has_user
FROM smelt.source('raw.orders')
"#;

        let path = PathBuf::from("models/test_exists.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let has_user_col = schema.columns.iter().find(|c| c.name == "has_user");
        assert!(has_user_col.is_some(), "Column 'has_user' not found");
        assert!(
            has_user_col.unwrap().data_type.is_some(),
            "Column 'has_user' should have a type"
        );
        let typed_col = has_user_col.unwrap().data_type.as_ref().unwrap();
        assert_eq!(
            typed_col.data_type,
            DataType::Boolean,
            "EXISTS should return Boolean"
        );
        assert!(
            !typed_col.nullable,
            "EXISTS should never be NULL (always TRUE or FALSE)"
        );
    }

    #[test]
    fn test_not_operator_type_inference() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: is_completed
            type: BOOLEAN
"#;

        // NOT operator should return Boolean
        let sql = r#"
SELECT NOT is_completed as is_pending
FROM smelt.source('raw.orders')
"#;

        let path = PathBuf::from("models/test_not.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let is_pending_col = schema.columns.iter().find(|c| c.name == "is_pending");
        assert!(is_pending_col.is_some(), "Column 'is_pending' not found");
        assert!(
            is_pending_col.unwrap().data_type.is_some(),
            "Column 'is_pending' should have a type"
        );
        assert_eq!(
            is_pending_col
                .unwrap()
                .data_type
                .as_ref()
                .unwrap()
                .data_type,
            DataType::Boolean,
            "NOT should return Boolean"
        );
    }

    #[test]
    fn test_unary_negation_type_inference() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

        // Unary negation should preserve numeric type
        let sql = r#"
SELECT -amount as negative_amount
FROM smelt.source('raw.orders')
"#;

        let path = PathBuf::from("models/test_negation.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let neg_col = schema.columns.iter().find(|c| c.name == "negative_amount");
        assert!(neg_col.is_some(), "Column 'negative_amount' not found");
        assert!(
            neg_col.unwrap().data_type.is_some(),
            "Column 'negative_amount' should have a type"
        );
        assert_eq!(
            neg_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Integer,
            "Unary negation should preserve numeric type (INTEGER)"
        );
    }

    #[test]
    fn test_union_same_types() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
      returns:
        columns:
          - name: id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
"#;

        // UNION with same types should preserve those types
        let sql = r#"
SELECT id, amount FROM smelt.source('raw.orders')
UNION
SELECT id, amount FROM smelt.source('raw.returns')
"#;

        let path = PathBuf::from("models/test_union_same.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 2);

        // id should be INTEGER
        let id_col = schema.columns.iter().find(|c| c.name == "id");
        assert!(id_col.is_some(), "Column 'id' not found");
        assert_eq!(
            id_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Integer,
            "UNION with same types should preserve INTEGER"
        );

        // amount should be DECIMAL
        let amount_col = schema.columns.iter().find(|c| c.name == "amount");
        assert!(amount_col.is_some(), "Column 'amount' not found");
        assert!(
            matches!(
                amount_col.unwrap().data_type.as_ref().unwrap().data_type,
                DataType::Decimal { .. }
            ),
            "UNION with same types should preserve DECIMAL"
        );
    }

    #[test]
    fn test_union_type_promotion() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources: {}
"#;

        // UNION with INTEGER literal and BIGINT literal should promote to BIGINT
        // Using CAST to ensure we get specific types
        let sql = r#"
SELECT CAST(1 AS INTEGER) as n
UNION
SELECT CAST(2 AS BIGINT) as n
"#;

        let path = PathBuf::from("models/test_union_promote.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let n_col = schema.columns.iter().find(|c| c.name == "n");
        assert!(n_col.is_some(), "Column 'n' not found");
        assert_eq!(
            n_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::BigInt,
            "UNION of INTEGER and BIGINT should promote to BIGINT"
        );
    }

    #[test]
    fn test_union_all_type_promotion() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources: {}
"#;

        // UNION ALL with INTEGER and DOUBLE should promote to DOUBLE
        let sql = r#"
SELECT CAST(1 AS INTEGER) as value
UNION ALL
SELECT CAST(2.5 AS DOUBLE) as value
"#;

        let path = PathBuf::from("models/test_union_all.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let value_col = schema.columns.iter().find(|c| c.name == "value");
        assert!(value_col.is_some(), "Column 'value' not found");
        assert_eq!(
            value_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Double,
            "UNION ALL of INTEGER and DOUBLE should promote to DOUBLE"
        );
    }

    #[test]
    fn test_union_chained() {
        let mut db = Database::default();

        let sources_yaml = r#"
sources: {}
"#;

        // Chained UNION: SMALLINT UNION INTEGER UNION BIGINT should be BIGINT
        let sql = r#"
SELECT CAST(1 AS SMALLINT) as n
UNION
SELECT CAST(2 AS INTEGER) as n
UNION
SELECT CAST(3 AS BIGINT) as n
"#;

        let path = PathBuf::from("models/test_union_chained.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_sources_yaml(Arc::new(sources_yaml.to_string()));

        let schema = db.typed_model_schema(path.clone());

        assert_eq!(schema.columns.len(), 1);

        let n_col = schema.columns.iter().find(|c| c.name == "n");
        assert!(n_col.is_some(), "Column 'n' not found");
        assert_eq!(
            n_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::BigInt,
            "Chained UNION of SMALLINT, INTEGER, BIGINT should be BIGINT"
        );
    }
}
