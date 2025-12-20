/// Salsa database for incremental compilation
///
/// This module defines the Salsa queries that power the LSP and optimizer.
/// Salsa automatically handles incremental recomputation when inputs change.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
    /// Parse a file and extract model definitions
    /// Returns None if file doesn't contain a valid model
    fn parse_model(&self, path: PathBuf) -> Option<Arc<Model>>;

    /// Extract all ref() calls from a model
    fn model_refs(&self, path: PathBuf) -> Arc<Vec<String>>;

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

/// The main database that combines all query groups
#[salsa::database(InputsStorage, SyntaxStorage, SemanticStorage)]
#[derive(Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
}

impl salsa::Database for Database {}

// Query implementations

fn parse_model(db: &dyn Syntax, path: PathBuf) -> Option<Arc<Model>> {
    let text = db.file_text(path.clone());

    // Very simple parser for now - just look for {{ ref() }} patterns
    // TODO: Replace with proper Rowan-based parser

    // Extract model name from file path (e.g., models/users.sql -> users)
    let model_name = path
        .file_stem()?
        .to_str()?
        .to_string();

    // Check if file contains SQL (very naive check)
    if !text.contains("SELECT") && !text.contains("select") {
        return None;
    }

    Some(Arc::new(Model {
        name: model_name,
        path: path.clone(),
    }))
}

fn model_refs(db: &dyn Syntax, path: PathBuf) -> Arc<Vec<String>> {
    let text = db.file_text(path);

    // Extract {{ ref('...') }} patterns
    // Very naive regex-like parsing for now
    let mut refs = Vec::new();
    let text_str = text.as_str();

    let mut pos = 0;
    while let Some(start) = text_str[pos..].find("{{ ref('") {
        let abs_start = pos + start + 8; // After "{{ ref('"

        if let Some(end) = text_str[abs_start..].find("')") {
            let ref_name = &text_str[abs_start..abs_start + end];
            refs.push(ref_name.to_string());
            pos = abs_start + end + 2;
        } else {
            break;
        }
    }

    Arc::new(refs)
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

    // Check if model is valid
    if db.parse_model(path.clone()).is_none() {
        // Only report error if file is supposed to be a model (in models/ directory)
        if path.to_str().map(|s| s.contains("models/")).unwrap_or(false) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "File does not contain a valid SQL query".to_string(),
                line: 0,
                column: 0,
            });
        }
        return Arc::new(diagnostics);
    }

    // Check for undefined refs
    let refs = db.model_refs(path.clone());
    for ref_name in refs.iter() {
        if db.resolve_ref(ref_name.clone()).is_none() {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined model reference: '{}'", ref_name),
                line: 0, // TODO: Track actual line numbers
                column: 0,
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

/// Represents a diagnostic (error, warning, info)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}
