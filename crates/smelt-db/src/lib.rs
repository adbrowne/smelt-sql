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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rowan::TextRange;
use salsa::{Accumulator, Setter};
use serde::Deserialize;
use smelt_parser::{self, File as AstFile, RefCall, TableRef};
use smelt_types::signatures::{extract_function_signatures, FunctionSig};
use smelt_types::{parse_type, DataType, TypedColumn};

pub mod code_actions;
pub mod references;
pub mod schema;
pub mod type_inference;
pub mod yaml_edits;

pub use schema::{
    Column, ColumnConstraint, ColumnSource, FunctionInput, FunctionOutput, InputConstraint,
    ModelFunctionType, ModelSchema, ResolvedSchema, RowExtension, TypedField,
};
pub use type_inference::{
    infer_cte_columns, infer_expression_type, infer_select_column_types,
    walk_expression_columns_with_visitor, walk_select_columns_with_visitor, TypeContext,
};

// Source types re-exported from smelt-core
pub use smelt_core::{SeedInfo, SourceColumnDef, SourceDef, SourceTableDef, SourcesConfig};

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
    pub fn set_project_input(&mut self, root: PathBuf, sources_yaml: String) -> ProjectInput {
        let existing = self.projects.read().unwrap().get(&root).copied();
        match existing {
            Some(project) => {
                project.set_sources_yaml(self).to(sources_yaml);
                project
            }
            None => {
                let project = ProjectInput::new(self, root.clone(), sources_yaml);
                self.projects.write().unwrap().insert(root, project);
                project
            }
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
    pub fn set_workspace(&mut self, files: Vec<SourceFile>, projects: Vec<ProjectInput>) {
        match Workspace::try_get(self) {
            Some(ws) => {
                ws.set_files(self).to(files);
                ws.set_projects(self).to(projects);
            }
            None => {
                Workspace::new(self, files, projects);
            }
        }
    }

    /// Convenience accessor: the workspace singleton, creating it empty if missing.
    pub fn workspace(&mut self) -> Workspace {
        match Workspace::try_get(self) {
            Some(ws) => ws,
            None => Workspace::new(self, Vec::new(), Vec::new()),
        }
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

/// Position in a file (line, column)
pub type Position = smelt_parser::ast::Position;

/// Range in a file (start, end)
pub type Range = smelt_parser::ast::Range;

/// Diagnostic codes for pattern-matching in code actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    ParseError,
    InvalidModel,
    UndefinedModelRef,
    UndefinedSource,
    CannotInferType,
    UndeclaredColumn,
    TypeMismatch,
    CircularDependency,
    UnsupportedConstruct,
    YamlParseError,
    SourceTypeError,
    MalformedSource,
    AmbiguousColumn,
    UnknownCastType,
    UnrecognizedFunction,
    /// Emitted when two `smelt.define` declarations share a function name.
    /// Anchored at the *second* (sorted-by-path) declaration's name span; the
    /// first declaration wins. Introduced in Phase 3 of smelt-functions.
    DuplicateFunctionDefinition,
}

/// Structured metadata attached to diagnostics for code actions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticData {
    UndefinedRef {
        model_name: String,
    },
    UndefinedSource {
        source_name: String,
        table_name: String,
    },
    CannotInferType {
        column_name: String,
    },
    UndeclaredColumn {
        qualifier: Option<String>,
        column_name: String,
    },
    TypeMismatch {
        column_name: String,
        ref_name: String,
        actual_type: String,
        expected_type: String,
    },
}

/// Represents a diagnostic (error, warning, info)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub range: Range,
    pub code: Option<DiagnosticCode>,
    pub data: Option<DiagnosticData>,
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

// ============================================================================
// Syntax queries
// ============================================================================

#[salsa::tracked(returns(ref))]
pub fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> smelt_parser::Parse {
    let text = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text);
    smelt_parser::parse(&clean_text)
}

#[salsa::tracked]
pub fn parse_model(db: &dyn salsa::Database, file: SourceFile) -> Option<Arc<Model>> {
    let path = file.path(db).clone();
    // Extract model name: from virtual path suffix (multi-model) or file stem (single-model)
    let path_str = path.to_str().unwrap_or("");
    let (model_name, source_path) = if let Some((file_part, name)) = path_str.rsplit_once("::") {
        (name.to_string(), PathBuf::from(file_part))
    } else {
        (path.file_stem()?.to_str()?.to_string(), path.clone())
    };

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    ast.select_stmt()?;

    Some(Arc::new(Model {
        name: model_name,
        path,
        source_path,
    }))
}

#[salsa::tracked]
pub fn model_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<RefLocation>> {
    let parse = parse_file(db, file);
    let text = file.text(db);
    let syntax = parse.syntax();

    if let Some(ast) = AstFile::cast(syntax) {
        let refs: Vec<RefLocation> = ast
            .refs()
            .filter_map(|ref_call| {
                let name = ref_call.model_name()?;
                let text_range = ref_call.name_range().unwrap_or(ref_call.range());
                let range = smelt_parser::ast::text_range_to_range(text, text_range);
                Some(RefLocation { name, range })
            })
            .collect();

        Arc::new(refs)
    } else {
        Arc::new(Vec::new())
    }
}

#[salsa::tracked]
pub fn model_sources(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<SourceLocation>> {
    let parse = parse_file(db, file);
    let text = file.text(db);
    let syntax = parse.syntax();

    if let Some(ast) = AstFile::cast(syntax) {
        let sources: Vec<SourceLocation> = ast
            .sources()
            .filter_map(|source_call| {
                let qualified_name = source_call.qualified_name()?;
                let source_name = source_call.source_name()?;
                let table_name = source_call.table_name()?;
                let text_range = source_call.name_range().unwrap_or(source_call.range());
                let range = smelt_parser::ast::text_range_to_range(text, text_range);
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

#[salsa::tracked]
pub fn sources_config(db: &dyn salsa::Database, project: ProjectInput) -> Arc<SourcesConfig> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return Arc::new(SourcesConfig::default());
    }
    match serde_yaml::from_str::<SourcesConfig>(yaml) {
        Ok(config) => Arc::new(config),
        Err(_) => Arc::new(SourcesConfig::default()),
    }
}

/// Discover seed CSV files for a project root and infer their column types.
///
/// Reads from disk (not a tracked Salsa input) — seeds that change on disk
/// require a tool restart to be detected. The query is keyed on `ProjectInput`
/// so it's recomputed when the project's sources_yaml changes, but not when
/// CSV files change.
#[salsa::tracked]
pub fn project_seeds(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SeedInfo>> {
    let project_root = project.root(db).clone();
    let seed_paths = smelt_core::Config::load(&project_root)
        .map(|c| c.seed_paths)
        .unwrap_or_else(|_| vec!["seeds".to_string()]);
    Arc::new(smelt_core::discover_seed_infos(&project_root, &seed_paths))
}

#[salsa::tracked]
pub fn sources_yaml_error(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Option<YamlParseError> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return None;
    }
    match serde_yaml::from_str::<SourcesConfig>(yaml) {
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

#[salsa::tracked]
pub fn sources_type_errors(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Arc<Vec<SourceTypeError>> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return Arc::new(Vec::new());
    }

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

    let config: RawSourcesConfig = match serde_yaml::from_str(yaml) {
        Ok(c) => c,
        Err(_) => return Arc::new(Vec::new()),
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

#[salsa::tracked]
pub fn all_models(db: &dyn salsa::Database, workspace: Workspace) -> Arc<HashMap<PathBuf, Model>> {
    let mut models = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        if let Some(model) = parse_model(db, file) {
            models.insert(file.path(db).clone(), (*model).clone());
        }
    }
    Arc::new(models)
}

// ============================================================================
// Function signature index (Phase 3, smelt-functions Step 1)
// ============================================================================
//
// Per §20H of `docs/research/20260413-smelt-functions.md`, signature lookups
// (used by downstream type-checking) must not be invalidated by edits to a
// function *body*. Split:
//   - `file_signature_inputs` / `functions_in_file` — signatures only. Its
//     return value is content-equal across body-only edits, so Salsa's
//     by-value backdating stops the re-run cascade at the boundary.
//   - `function_body` — CST of the body expression, re-computed on any edit
//     but independent of the signature query.
//
// All of these are thin wrappers over the pure
// `smelt_types::signatures::extract_function_signatures` function — per the
// pure-function rule in CLAUDE.md.

/// Extract function signatures from a single file. Pure-function wrapper
/// around `smelt_types::signatures::extract_function_signatures`.
///
/// This query's output only changes when *signature* tokens change. Body
/// edits do not affect the returned `Vec<FunctionSig>`, so Salsa's durability
/// check prevents downstream consumers from re-running. This is the §20H
/// invalidation hinge.
#[salsa::tracked]
pub fn file_signature_inputs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<FunctionSig>> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw);
    if let Some(ast) = AstFile::cast(syntax) {
        Arc::new(extract_function_signatures(&ast, &clean_text))
    } else {
        Arc::new(Vec::new())
    }
}

/// All function signatures declared in `file`, in declaration order.
///
/// Exposed as a distinct public name from `file_signature_inputs` per the
/// plan; internally it is the same query.
#[salsa::tracked]
pub fn functions_in_file(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<FunctionSig>> {
    file_signature_inputs(db, file)
}

/// Look up a single function's signature by name within one file.
///
/// Memoized by `(file, name)`. Re-uses `file_signature_inputs` so edits to
/// other declarations in the same file don't necessarily invalidate this
/// lookup either (though Salsa's current implementation cannot detect that
/// granularity — it still goes through `file_signature_inputs`'s output).
#[salsa::tracked]
pub fn function_signature(
    db: &dyn salsa::Database,
    file: SourceFile,
    name: String,
) -> Option<Arc<FunctionSig>> {
    let sigs = file_signature_inputs(db, file);
    sigs.iter()
        .find(|s| s.name == name)
        .map(|s| Arc::new(s.clone()))
}

/// Byte range of a function body in the stripped source text.
///
/// Rowan's `SyntaxNode` is `!Send`, so we cannot store it in a Salsa tracked
/// output directly. Instead, this query returns the byte range of the body
/// within the parsed (frontmatter-stripped) source. Callers can re-parse or
/// re-read the CST via `parse_file` and locate the body using this range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyRange {
    /// Inclusive start byte offset into the stripped source.
    pub start: u32,
    /// Exclusive end byte offset into the stripped source.
    pub end: u32,
}

/// Byte range of `name`'s body in `file`'s stripped source text, if any.
///
/// Depends directly on `parse_file` — not on `file_signature_inputs` — so
/// that body-only edits invalidate this query without invalidating the
/// signature query. (A body edit changes the `Parse` output, which changes
/// the body's text range if body length changed, and re-parsing anyway
/// — in practice this query re-computes on any file edit. The invariant
/// that matters is the asymmetric direction: `function_signature`
/// is *not* invalidated by body edits.)
#[salsa::tracked]
pub fn function_body(
    db: &dyn salsa::Database,
    file: SourceFile,
    name: String,
) -> Option<BodyRange> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    for define in ast.defines() {
        if define.name().as_deref() == Some(name.as_str()) {
            let body = define.body()?;
            let range = body.syntax().text_range();
            return Some(BodyRange {
                start: u32::from(range.start()),
                end: u32::from(range.end()),
            });
        }
    }
    None
}

/// Resolve a function name to the first matching `FunctionSig` in the
/// workspace. Files are enumerated in sorted-by-path order for deterministic
/// diagnostics (the first file declares; later files collide).
#[salsa::tracked]
pub fn resolve_function(
    db: &dyn salsa::Database,
    workspace: Workspace,
    name: String,
) -> Option<Arc<FunctionSig>> {
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in files {
        let sigs = file_signature_inputs(db, f);
        if let Some(sig) = sigs.iter().find(|s| s.name == name) {
            return Some(Arc::new(sig.clone()));
        }
    }
    None
}

/// Workspace-wide duplicate-function-name diagnostics. Each returned tuple is
/// `(path, diagnostic)` where `path` is the offending file and `diagnostic`
/// points at the colliding `DEFINE_NAME` span inside that file.
///
/// Iteration is sorted-by-path so the "first declaration wins, later ones
/// emit diagnostics" rule is deterministic.
#[salsa::tracked]
pub fn workspace_function_diagnostics(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<Vec<(PathBuf, Diagnostic)>> {
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));

    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut diagnostics: Vec<(PathBuf, Diagnostic)> = Vec::new();

    for f in files {
        let path = f.path(db).clone();
        let sigs = file_signature_inputs(db, f);
        for sig in sigs.iter() {
            if let Some(first_path) = seen.get(&sig.name) {
                diagnostics.push((
                    path.clone(),
                    Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Function `{}` is already defined in {}",
                            sig.name,
                            first_path.display()
                        ),
                        range: sig.name_range,
                        code: Some(DiagnosticCode::DuplicateFunctionDefinition),
                        data: None,
                    },
                ));
            } else {
                seen.insert(sig.name.clone(), path.clone());
            }
        }
    }

    Arc::new(diagnostics)
}

/// Filter `workspace_function_diagnostics` to a single file.
pub fn duplicate_function_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let target = file.path(db);
    workspace_function_diagnostics(db, workspace)
        .iter()
        .filter(|(p, _)| p == target)
        .map(|(_, d)| d.clone())
        .collect()
}

// ============================================================================
// Semantic queries
// ============================================================================

/// Resolve a `smelt.ref(name)` call to the `SourceFile` that defines it.
#[salsa::tracked]
pub fn resolve_ref(
    db: &dyn salsa::Database,
    workspace: Workspace,
    model_name: String,
) -> Option<SourceFile> {
    for file in workspace.files(db).iter().copied() {
        if let Some(model) = parse_model(db, file) {
            if model.name == model_name {
                return Some(file);
            }
        }
    }
    None
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
// Diagnostics (accumulator-based)
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

#[salsa::tracked]
pub fn check_file_diagnostics(db: &dyn salsa::Database, workspace: Workspace, file: SourceFile) {
    let path = file.path(db);
    let text = file.text(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    // Parse errors
    let parse = parse_file(db, file);
    for error in parse.errors.iter() {
        let range = smelt_parser::ast::text_range_to_range(text, error.range);
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

    // Check if model is valid
    if parse_model(db, file).is_none() {
        let path_str = path.to_str().unwrap_or("");
        let is_virtual_submodel = path_str.contains("::");
        if !is_virtual_submodel && path_str.contains("models/") {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "File does not contain a valid SQL query".to_string(),
                range: Range {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 0, column: 0 },
                },
                code: Some(DiagnosticCode::InvalidModel),
                data: None,
            })
            .accumulate(db);
        }
        return;
    }

    // Undefined refs
    let refs = model_refs(db, file);
    for ref_loc in refs.iter() {
        if resolve_ref(db, workspace, ref_loc.name.clone()).is_none()
            && !is_known_seed(db, workspace, &ref_loc.name)
        {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined model reference: '{}'", ref_loc.name),
                range: ref_loc.range,
                code: Some(DiagnosticCode::UndefinedModelRef),
                data: Some(DiagnosticData::UndefinedRef {
                    model_name: ref_loc.name.clone(),
                }),
            })
            .accumulate(db);
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
                    range: Range {
                        start: Position { line: 0, column: 0 },
                        end: Position { line: 0, column: 0 },
                    },
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
                        range: Range {
                            start: Position { line: 0, column: 0 },
                            end: Position { line: 0, column: 0 },
                        },
                        code: Some(DiagnosticCode::SourceTypeError),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Unsupported constructs + malformed sources + CAST / unknown fn / ambiguous column
    check_unsupported_constructs(&parse.syntax(), text, db);

    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        for source_call in ast.sources() {
            if let Some(qualified_name) = source_call.qualified_name() {
                if !qualified_name.contains('.') {
                    let text_range = source_call.name_range().unwrap_or(source_call.range());
                    let range = smelt_parser::ast::text_range_to_range(text, text_range);
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Malformed source reference: '{}'. Expected format: 'source_name.table_name'",
                            qualified_name
                        ),
                        range,
                        code: Some(DiagnosticCode::MalformedSource),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }

        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        check_expression_types(&expr, db);
                    }
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
                                            range: Range {
                                                start: Position { line: 0, column: 0 },
                                                end: Position { line: 0, column: 0 },
                                            },
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
    }
}

/// Resolve a project root path to a `ProjectInput` via the workspace.
fn find_project(
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

/// True if `name` matches a discovered seed in any project in the workspace.
/// Seeds aren't part of the file-based `resolve_ref` graph; this helper lets
/// the "Undefined model reference" diagnostic exclude seed names.
fn is_known_seed(db: &dyn salsa::Database, workspace: Workspace, name: &str) -> bool {
    workspace
        .projects(db)
        .iter()
        .copied()
        .any(|p| project_seeds(db, p).iter().any(|s| s.name == name))
}

fn count_from_sources(select_stmt: &smelt_parser::ast::SelectStmt) -> usize {
    let mut count = 0;
    if let Some(from_clause) = select_stmt.from_clause() {
        count += from_clause.table_refs().count();
        count += from_clause.joins().count();
    }
    count
}

/// Check an expression for invalid CAST types and unknown functions; push
/// diagnostics into the accumulator.
fn check_expression_types(expr: &smelt_parser::ast::Expr, db: &dyn salsa::Database) {
    let default_range = Range {
        start: Position { line: 0, column: 0 },
        end: Position { line: 0, column: 0 },
    };

    if let Some(cast_expr) = expr.as_cast() {
        if let Some(type_spec) = cast_expr.type_spec() {
            let type_text = type_spec.full_text();
            if parse_type(&type_text).is_err() {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Unknown type '{}' in CAST expression. Type inference unavailable.",
                        type_text
                    ),
                    range: default_range,
                    code: Some(DiagnosticCode::UnknownCastType),
                    data: None,
                })
                .accumulate(db);
            }
        }
        if let Some(inner) = cast_expr.expression() {
            check_expression_types(&inner, db);
        }
    }

    if let Some(func) = expr.as_function_call() {
        if let Some(name) = func.name() {
            let upper_name = name.to_uppercase();
            if func.namespace().is_none()
                && smelt_types::SqlFunction::from_name(&upper_name).is_none()
            {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Function '{}' is not a recognized SQL function. Type inference unavailable.",
                        name
                    ),
                    range: default_range,
                    code: Some(DiagnosticCode::UnrecognizedFunction),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }
}

fn check_unsupported_constructs(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    text: &str,
    db: &dyn salsa::Database,
) {
    use smelt_parser::SyntaxKind::{PIVOT_CLAUSE, UNPIVOT_CLAUSE};

    for node in syntax.descendants() {
        let (kind_name, node_range) = match node.kind() {
            PIVOT_CLAUSE => ("PIVOT", node.text_range()),
            UNPIVOT_CLAUSE => ("UNPIVOT", node.text_range()),
            _ => continue,
        };
        let range = smelt_parser::ast::text_range_to_range(text, node_range);
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "{} is not supported \u{2014} output columns depend on data values and cannot be determined at compile time",
                kind_name
            ),
            range,
            code: Some(DiagnosticCode::UnsupportedConstruct),
            data: None,
        })
        .accumulate(db);
    }
}

// ============================================================================
// Schema queries
// ============================================================================

#[salsa::tracked]
pub fn model_schema(db: &dyn salsa::Database, file: SourceFile) -> Arc<ModelSchema> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return Arc::new(ModelSchema::empty()),
    };

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

    let mut columns = Vec::new();
    let mut row_extensions = Vec::new();

    for item in select_list.items() {
        if item.is_wildcard() {
            for ref_name in &from_refs {
                row_extensions.push(schema::RowExtension {
                    ref_name: ref_name.clone(),
                    excluded_columns: vec![],
                    range: item.range(),
                });
            }
            continue;
        }

        let name = match item.column_name() {
            Some(n) => n,
            None => continue,
        };

        let alias = item.alias();
        let expression = item.expression().map(|e| e.text()).unwrap_or_default();

        let source = if let Some(expr) = item.expression() {
            if expr.as_function_call().is_some() {
                ColumnSource::Computed
            } else if let Some(col_ref) = expr.as_column_ref() {
                let column_name = col_ref.name().to_string();
                if from_refs.len() == 1 {
                    ColumnSource::FromModel {
                        model_name: from_refs[0].clone(),
                        column_name,
                    }
                } else if from_refs.is_empty() {
                    ColumnSource::ExternalTable {
                        table_name: col_ref.qualifier().unwrap_or("unknown").to_string(),
                    }
                } else {
                    ColumnSource::Unknown
                }
            } else {
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
            data_type: None,
        });
    }

    if !row_extensions.is_empty() {
        let explicit_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        for ext in &mut row_extensions {
            ext.excluded_columns = explicit_names.clone();
        }
    }

    Arc::new(ModelSchema {
        columns,
        row_extensions,
        input_constraints: vec![],
    })
}

#[salsa::tracked]
pub fn available_columns(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<Vec<Column>> {
    let schema = model_schema(db, file);
    let mut available = schema.columns.clone();

    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(from_clause) = select_stmt.from_clause() {
                for table_ref in from_clause.table_refs() {
                    if let Some(func) = table_ref.function_call() {
                        if let Some(ref_call) = RefCall::from_function_call(func) {
                            if let Some(model_name) = ref_call.model_name() {
                                if let Some(upstream) =
                                    resolve_ref(db, workspace, model_name.clone())
                                {
                                    let upstream_schema = model_schema(db, upstream);
                                    for col in upstream_schema.columns.iter() {
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

// ============================================================================
// Type checking queries (with cycle recovery via cycle_initial)
// ============================================================================

fn typed_model_schema_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<ModelSchema> {
    Arc::new(ModelSchema::empty())
}

fn type_context_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<TypeContext> {
    Arc::new(TypeContext::new())
}

fn resolved_model_schema_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<ResolvedSchema> {
    Arc::new(ResolvedSchema {
        columns: vec![],
        is_fully_resolved: true,
        unresolved_extensions: vec![],
    })
}

/// Provider for upstream `smelt.ref()` schema lookups, used by the pure
/// `build_type_context` function.
///
/// The Salsa version uses [`SalsaRefSchemaProvider`] (delegates to the new
/// 0.26-API free functions `resolve_ref` + `resolved_model_schema`).
/// The CLI batch compiler uses [`StaticRefSchemaProvider`], which is fully
/// pure and takes pre-computed maps.
pub trait RefSchemaProvider {
    /// Returns the typed columns for the model named `model_name`, if known.
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>>;
    /// Returns the typed columns for the seed named `seed_name`, if known.
    /// Seeds and model refs are looked up separately because the type-context
    /// loop wants to distinguish them (CSV files don't participate in
    /// SELECT * schema resolution, etc.).
    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>>;
}

/// `RefSchemaProvider` impl that delegates to the Salsa database. Used by the
/// `type_context()` Salsa query so the LSP keeps benefiting from
/// incremental recomputation.
pub struct SalsaRefSchemaProvider<'a> {
    db: &'a dyn salsa::Database,
    workspace: Workspace,
}

impl<'a> SalsaRefSchemaProvider<'a> {
    pub fn new(db: &'a dyn salsa::Database, workspace: Workspace) -> Self {
        Self { db, workspace }
    }
}

impl RefSchemaProvider for SalsaRefSchemaProvider<'_> {
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        let upstream = resolve_ref(self.db, self.workspace, model_name.to_string())?;
        let resolved = resolved_model_schema(self.db, self.workspace, upstream);
        Some(
            resolved
                .columns
                .iter()
                .map(|col| {
                    let typed_col = col.data_type.clone().unwrap_or(TypedColumn {
                        data_type: DataType::Unknown,
                        nullable: true,
                    });
                    (col.name.clone(), typed_col)
                })
                .collect(),
        )
    }

    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        // Seeds aren't part of the file-based resolve_ref graph; walk all
        // projects in the workspace and look in each project's seeds.
        for project in self.workspace.projects(self.db).iter().copied() {
            for seed in project_seeds(self.db, project).iter() {
                if seed.name == seed_name {
                    return Some(
                        seed.columns
                            .iter()
                            .map(|(name, dt)| {
                                (
                                    name.clone(),
                                    TypedColumn {
                                        data_type: dt.clone(),
                                        nullable: true,
                                    },
                                )
                            })
                            .collect(),
                    );
                }
            }
        }
        None
    }
}

/// Fully pure `RefSchemaProvider` for batch compilation (CLI, planner). Holds
/// pre-computed maps of model and seed schemas so it can answer lookups
/// without touching Salsa.
pub struct StaticRefSchemaProvider<'a> {
    pub models: &'a HashMap<String, Vec<(String, TypedColumn)>>,
    pub seeds: &'a HashMap<String, Vec<(String, TypedColumn)>>,
}

impl RefSchemaProvider for StaticRefSchemaProvider<'_> {
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        self.models.get(model_name).cloned()
    }

    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        self.seeds.get(seed_name).cloned()
    }
}

/// Pure (Salsa-free) builder for a `TypeContext` from a parsed AST and the
/// surrounding source/seed/model schemas.
///
/// This is the canonical builder; the `type_context()` Salsa query is a thin
/// wrapper that gathers Salsa inputs (sources_config, parsed file, upstream
/// schemas) and delegates here. The CLI batch compiler uses
/// `StaticRefSchemaProvider` to call this directly without a `Database`.
///
/// See CLAUDE.md "Pure Function Rule" for why this matters.
pub fn build_type_context(
    file: &AstFile,
    sources_config: &SourcesConfig,
    refs: &dyn RefSchemaProvider,
) -> TypeContext {
    let mut ctx = TypeContext::new();

    // Source columns from sources.yml.
    for source in &sources_config.sources {
        for table in &source.tables {
            for col in &table.columns {
                let data_type = col.data_type.clone().unwrap_or(DataType::Unknown);
                ctx.add_source_column(
                    &source.name,
                    &table.name,
                    &col.name,
                    TypedColumn {
                        data_type,
                        nullable: true,
                    },
                );
            }
        }
    }

    if let Some(select_stmt) = file.select_stmt() {
        // Process WITH clause CTEs first (CTEs shadow outer scope).
        if let Some(with_clause) = select_stmt.with_clause() {
            for cte in with_clause.ctes() {
                if let Some(cte_name) = cte.name() {
                    // For recursive CTEs with explicit column list, bootstrap
                    // with Unknown types so the recursive reference can find
                    // the columns.
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

                    if let Some(cte_select) = cte.query().and_then(|q| q.select_stmt()) {
                        process_from_clause_pure(&cte_select, refs, &mut ctx);
                    }

                    let columns = infer_cte_columns(&cte, &ctx);
                    for (col_name, typed_col) in columns {
                        ctx.add_cte_column(&cte_name, &col_name, typed_col);
                    }

                    ctx.add_alias(&cte_name, &cte_name);
                }
            }
        }

        process_from_clause_pure(&select_stmt, refs, &mut ctx);
    }

    ctx
}

fn process_from_clause_pure(
    select_stmt: &smelt_parser::ast::SelectStmt,
    refs: &dyn RefSchemaProvider,
    ctx: &mut TypeContext,
) {
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            process_table_ref_pure(&table_ref, refs, ctx);
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                process_table_ref_pure(&table_ref, refs, ctx);
            }
        }
    }
}

fn process_table_ref_pure(
    table_ref: &TableRef,
    refs: &dyn RefSchemaProvider,
    ctx: &mut TypeContext,
) {
    // Check for smelt.ref() calls
    if let Some(func) = table_ref.function_call() {
        if let Some(ref_call) = RefCall::from_function_call(func) {
            if let Some(model_name) = ref_call.model_name() {
                // Try as a seed first (the LSP path tries seeds via the seed
                // CSV check; here we ask the provider both ways).
                if let Some(cols) = refs.seed_columns(&model_name) {
                    for (col_name, typed_col) in cols {
                        ctx.add_model_column(&model_name, &col_name, typed_col);
                    }
                    if let Some(explicit_alias) = table_ref.alias() {
                        ctx.add_alias(&explicit_alias, &model_name);
                    }
                } else if let Some(cols) = refs.resolved_columns(&model_name) {
                    for (col_name, typed_col) in cols {
                        ctx.add_model_column(&model_name, &col_name, typed_col);
                    }
                    if let Some(explicit_alias) = table_ref.alias() {
                        ctx.add_alias(&explicit_alias, &model_name);
                    }
                }
            }
        }
    }

    if let Some(func) = table_ref.function_call() {
        if let Some(source_call) = smelt_parser::ast::SourceCall::from_function_call(func) {
            if let Some(source_name) = source_call.source_name() {
                if let Some(table_name) = source_call.table_name() {
                    let qualified_name = format!("{}.{}", source_name, table_name);

                    if let Some(explicit_alias) = table_ref.alias() {
                        ctx.add_alias(&explicit_alias, &qualified_name);
                    }
                    ctx.add_alias(&table_name, &qualified_name);
                }
            }
        }
    }

    // CTE references with aliases (e.g. "FROM daily_totals dt")
    // OR bare upstream MODEL/seed references (e.g. "FROM main.stg_orders AS o"
    // — produced by the dialect printer after `smelt.ref('stg_orders')` is
    // resolved). Without this branch, the alias `o` is never bound and
    // `o.line_revenue` resolves to Unknown, which silently narrows
    // `SUM(o.line_revenue)` to BIGINT in `_smelt_typed`. See B8.
    if table_ref.function_call().is_none() && table_ref.subquery().is_none() {
        if let Some(raw_name) = table_ref.identifier() {
            // Strip an optional leading schema qualifier (`schema.table`).
            // The dialect printer emits `<schema>.<model_name>`; we want the
            // last segment to look up against the schema provider.
            let table_name = bare_table_name(table_ref).unwrap_or(raw_name.clone());

            if ctx.is_cte(&table_name) {
                if let Some(explicit_alias) = table_ref.alias() {
                    ctx.add_alias(&explicit_alias, &table_name);
                }
            } else if let Some(cols) = refs
                .resolved_columns(&table_name)
                .or_else(|| refs.seed_columns(&table_name))
            {
                for (col_name, typed_col) in cols {
                    ctx.add_model_column(&table_name, &col_name, typed_col);
                }
                // Bind the alias (or the table name itself, so qualified
                // refs like `stg_orders.col` also resolve).
                let bind_to = table_ref.alias().unwrap_or_else(|| table_name.clone());
                ctx.add_alias(&bind_to, &table_name);
            }
        }
    }

    // Subqueries / LATERAL subqueries
    if let Some(subquery) = table_ref.subquery() {
        if let Some(alias) = table_ref.alias() {
            if let Some(select_stmt) = subquery.select_stmt() {
                if let Some(select_list) = select_stmt.select_list() {
                    let mut subquery_ctx = ctx.clone();
                    process_from_clause_pure(&select_stmt, refs, &mut subquery_ctx);

                    let column_types = infer_select_column_types(&select_stmt, &subquery_ctx);

                    for (i, item) in select_list.items().enumerate() {
                        let col_name = if let Some(item_alias) = item.alias() {
                            item_alias
                        } else if let Some(expr) = item.expression() {
                            if let Some(col_ref) = expr.as_column_ref() {
                                col_ref.name().to_string()
                            } else {
                                format!("col{}", i + 1)
                            }
                        } else {
                            format!("col{}", i + 1)
                        };

                        let typed_col = column_types.get(i).cloned().unwrap_or(TypedColumn {
                            data_type: DataType::Unknown,
                            nullable: true,
                        });

                        ctx.add_cte_column(&alias, &col_name, typed_col);
                    }

                    ctx.add_alias(&alias, &alias);
                }
            }
        }
    }
}

/// Extract the table-name segment from a bare `TableRef` like `schema.table`,
/// stripping an optional schema qualifier. Used by `process_table_ref_pure`
/// to look up upstream MODEL/seed schemas after the dialect printer has
/// resolved `smelt.ref('foo')` to `<schema>.foo`.
///
/// Returns `None` for function calls and subqueries (those have their own
/// handling paths).
fn bare_table_name(table_ref: &TableRef) -> Option<String> {
    use smelt_parser::SyntaxKind::{AS_KW, DOT, IDENT};

    if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
        return None;
    }

    // Walk tokens and collect the IDENTs that come BEFORE any AS keyword.
    // The last such IDENT (after any DOT segments) is the table name.
    let mut idents: Vec<String> = Vec::new();
    let mut last_was_dot = false;
    let mut started = false;
    for tok in table_ref
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
    {
        match tok.kind() {
            AS_KW => break,
            IDENT => {
                if !started || last_was_dot {
                    idents.push(tok.text().to_string());
                } else {
                    // Implicit alias (no AS keyword): bail out, take what
                    // we have so far.
                    break;
                }
                started = true;
                last_was_dot = false;
            }
            DOT => {
                last_was_dot = true;
            }
            _ => {}
        }
    }

    idents.last().cloned()
}

#[salsa::tracked(cycle_initial = type_context_initial)]
pub fn type_context(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<TypeContext> {
    let project_root = file.project_root(db).clone();
    let sources = match find_project(db, workspace, &project_root) {
        Some(p) => sources_config(db, p),
        None => Arc::new(SourcesConfig::default()),
    };

    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(TypeContext::new()),
    };

    let provider = SalsaRefSchemaProvider::new(db, workspace);
    Arc::new(build_type_context(&ast, &sources, &provider))
}

#[salsa::tracked(cycle_initial = typed_model_schema_initial)]
pub fn typed_model_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<ModelSchema> {
    let base_schema = model_schema(db, file);
    let ctx = type_context(db, workspace, file);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return base_schema,
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return base_schema,
    };

    let inferred_types = infer_select_column_types(&select_stmt, &ctx);

    let mut typed_columns = Vec::new();
    for (i, col) in base_schema.columns.iter().enumerate() {
        let mut col = col.clone();
        if let Some(typed_col) = inferred_types.get(i) {
            col.data_type = Some(typed_col.clone());
        }
        typed_columns.push(col);
    }

    Arc::new(ModelSchema {
        columns: typed_columns,
        row_extensions: base_schema.row_extensions.clone(),
        input_constraints: base_schema.input_constraints.clone(),
    })
}

#[salsa::tracked(cycle_initial = resolved_model_schema_initial)]
pub fn resolved_model_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<ResolvedSchema> {
    let typed_schema = typed_model_schema(db, workspace, file);

    if typed_schema.row_extensions.is_empty() {
        return Arc::new(ResolvedSchema {
            columns: typed_schema.columns.clone(),
            is_fully_resolved: true,
            unresolved_extensions: vec![],
        });
    }

    let mut columns = Vec::new();
    let mut unresolved_extensions = Vec::new();
    let mut is_fully_resolved = true;

    for ext in &typed_schema.row_extensions {
        if let Some(upstream) = resolve_ref(db, workspace, ext.ref_name.clone()) {
            let upstream_resolved = resolved_model_schema(db, workspace, upstream);
            for col in &upstream_resolved.columns {
                if !ext.excluded_columns.contains(&col.name) {
                    columns.push(col.clone());
                }
            }
            if !upstream_resolved.is_fully_resolved {
                is_fully_resolved = false;
                for upstream_ext in &upstream_resolved.unresolved_extensions {
                    unresolved_extensions.push(upstream_ext.clone());
                }
            }
        } else {
            is_fully_resolved = false;
            unresolved_extensions.push(ext.clone());
        }
    }

    for col in &typed_schema.columns {
        columns.push(col.clone());
    }

    Arc::new(ResolvedSchema {
        columns,
        is_fully_resolved,
        unresolved_extensions,
    })
}

#[salsa::tracked]
pub fn model_input_constraints(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<Vec<InputConstraint>> {
    use schema::{ColumnConstraint, InputConstraint};

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ctx = type_context(db, workspace, file);

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(vec![]),
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return Arc::new(vec![]),
    };

    let mut alias_to_ref: HashMap<String, String> = HashMap::new();
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            if let Some(func) = table_ref.function_call() {
                if let Some(ref_call) = RefCall::from_function_call(func.clone()) {
                    if let Some(model_name) = ref_call.model_name() {
                        alias_to_ref.insert(model_name.clone(), model_name.clone());
                        if let Some(alias) = table_ref.alias() {
                            alias_to_ref.insert(alias, model_name);
                        }
                    }
                }
                if let Some(source_call) = smelt_parser::ast::SourceCall::from_function_call(func) {
                    let input_name = source_call
                        .table_name()
                        .or_else(|| source_call.qualified_name())
                        .unwrap_or_default();
                    if !input_name.is_empty() {
                        alias_to_ref.insert(input_name.clone(), input_name.clone());
                        if let Some(qn) = source_call.qualified_name() {
                            if qn != input_name {
                                alias_to_ref.insert(qn, input_name.clone());
                            }
                        }
                        if let Some(alias) = table_ref.alias() {
                            alias_to_ref.insert(alias, input_name);
                        }
                    }
                }
            }
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if let Some(func) = table_ref.function_call() {
                    if let Some(ref_call) = RefCall::from_function_call(func.clone()) {
                        if let Some(model_name) = ref_call.model_name() {
                            alias_to_ref.insert(model_name.clone(), model_name.clone());
                            if let Some(alias) = table_ref.alias() {
                                alias_to_ref.insert(alias, model_name);
                            }
                        }
                    }
                    if let Some(source_call) =
                        smelt_parser::ast::SourceCall::from_function_call(func)
                    {
                        let input_name = source_call
                            .table_name()
                            .or_else(|| source_call.qualified_name())
                            .unwrap_or_default();
                        if !input_name.is_empty() {
                            alias_to_ref.insert(input_name.clone(), input_name.clone());
                            if let Some(qn) = source_call.qualified_name() {
                                if qn != input_name {
                                    alias_to_ref.insert(qn, input_name.clone());
                                }
                            }
                            if let Some(alias) = table_ref.alias() {
                                alias_to_ref.insert(alias, input_name);
                            }
                        }
                    }
                }
            }
        }
    }

    if alias_to_ref.is_empty() {
        return Arc::new(vec![]);
    }

    let mut constraints_map: HashMap<String, HashMap<String, ColumnConstraint>> = HashMap::new();

    let mut record_constraint =
        |ref_name: &str, col_name: &str, expected_type: Option<TypedColumn>, range: TextRange| {
            let entry = constraints_map
                .entry(ref_name.to_string())
                .or_default()
                .entry(col_name.to_string())
                .or_insert_with(|| ColumnConstraint {
                    expected_type: None,
                    usage_sites: vec![],
                });
            if entry.expected_type.is_none() {
                entry.expected_type = expected_type;
            }
            entry.usage_sites.push(range);
        };

    {
        let mut visitor = |qualifier: Option<&str>,
                           col_name: &str,
                           type_hint: Option<&TypedColumn>,
                           range: TextRange| {
            if col_name == "*" {
                return;
            }
            let inferred_type = type_hint.cloned();
            if let Some(q) = qualifier {
                let resolved = ctx.resolve_alias(q).unwrap_or_else(|| q.to_string());
                if let Some(ref_name) = alias_to_ref.get(&resolved) {
                    let final_type =
                        inferred_type.or_else(|| ctx.lookup_column(Some(q), col_name).cloned());
                    record_constraint(ref_name, col_name, final_type, range);
                }
            } else {
                let unique_refs: std::collections::HashSet<&String> =
                    alias_to_ref.values().collect();
                if unique_refs.len() == 1 {
                    let ref_name = alias_to_ref
                        .values()
                        .next()
                        .expect("unique_refs.len() == 1 guarantees at least one value");
                    let final_type =
                        inferred_type.or_else(|| ctx.lookup_column(None, col_name).cloned());
                    record_constraint(ref_name, col_name, final_type, range);
                }
            }
        };

        type_inference::walk_select_columns_with_visitor(&select_stmt, &ctx, None, &mut visitor);
    }

    let constraints: Vec<InputConstraint> = constraints_map
        .into_iter()
        .map(|(ref_name, required_columns)| InputConstraint {
            ref_name,
            required_columns,
        })
        .collect();

    Arc::new(constraints)
}

#[salsa::tracked]
pub fn model_function_type(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<schema::ModelFunctionType> {
    use schema::{FunctionInput, FunctionOutput, TypedField};

    let path = file.path(db);
    let model_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let input_constraints = model_input_constraints(db, workspace, file);

    let mut inputs: Vec<FunctionInput> = input_constraints
        .iter()
        .map(|ic| {
            let mut columns: Vec<TypedField> = ic
                .required_columns
                .iter()
                .map(|(col_name, constraint)| TypedField {
                    name: col_name.clone(),
                    constraint: constraint.expected_type.clone(),
                })
                .collect();
            columns.sort_by(|a, b| a.name.cmp(&b.name));
            FunctionInput {
                ref_name: ic.ref_name.clone(),
                columns,
            }
        })
        .collect();

    inputs.sort_by(|a, b| a.ref_name.cmp(&b.ref_name));

    let typed_schema = typed_model_schema(db, workspace, file);

    let outputs: Vec<FunctionOutput> = typed_schema
        .columns
        .iter()
        .filter(|col| col.name != "*")
        .map(|col| FunctionOutput {
            name: col.name.clone(),
            data_type: col.data_type.clone(),
        })
        .collect();

    let has_wildcard_output = !typed_schema.row_extensions.is_empty();

    Arc::new(schema::ModelFunctionType {
        model_name,
        inputs,
        outputs,
        has_wildcard_output,
    })
}

fn types_compatible(expected: &DataType, actual: &DataType) -> bool {
    if matches!(expected, DataType::Unknown) || matches!(actual, DataType::Unknown) {
        return true;
    }
    if expected == actual {
        return true;
    }
    if expected.is_numeric() && actual.is_numeric() {
        return true;
    }
    if expected.is_string() && actual.is_string() {
        return true;
    }
    if expected.is_temporal() && actual.is_temporal() {
        return true;
    }
    if matches!(expected, DataType::Boolean) && matches!(actual, DataType::Boolean) {
        return true;
    }
    if expected.is_string() {
        return true;
    }
    false
}

#[salsa::tracked]
pub fn check_type_diagnostics(db: &dyn salsa::Database, workspace: Workspace, file: SourceFile) {
    let path = file.path(db);
    if !path
        .to_str()
        .map(|s| s.contains("models/"))
        .unwrap_or(false)
    {
        return;
    }

    if parse_model(db, file).is_none() {
        return;
    }

    let refs = model_refs(db, file);
    let sources = model_sources(db, file);
    if refs.is_empty() && sources.is_empty() {
        return;
    }

    let text = file.text(db);
    let typed_schema = typed_model_schema(db, workspace, file);

    for col in &typed_schema.columns {
        if col.name == "*" {
            continue;
        }

        match &col.data_type {
            Some(typed_col) if matches!(typed_col.data_type, DataType::Unknown) => {
                let range = smelt_parser::ast::text_range_to_range(text, col.range);
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Could not infer type for column '{}'. Consider adding an explicit CAST.",
                        col.name
                    ),
                    range,
                    code: Some(DiagnosticCode::CannotInferType),
                    data: Some(DiagnosticData::CannotInferType {
                        column_name: col.name.clone(),
                    }),
                })
                .accumulate(db);
            }
            None => {
                let range = smelt_parser::ast::text_range_to_range(text, col.range);
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Could not infer type for column '{}'. Consider adding an explicit CAST.",
                        col.name
                    ),
                    range,
                    code: Some(DiagnosticCode::CannotInferType),
                    data: Some(DiagnosticData::CannotInferType {
                        column_name: col.name.clone(),
                    }),
                })
                .accumulate(db);
            }
            _ => {}
        }
    }

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            let ctx = type_context(db, workspace, file);
            let undeclared = type_inference::check_undeclared_columns(&select_stmt, &ctx);
            for info in undeclared {
                let range = smelt_parser::ast::text_range_to_range(text, info.range);
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: info.message,
                    range,
                    code: Some(DiagnosticCode::UndeclaredColumn),
                    data: Some(DiagnosticData::UndeclaredColumn {
                        qualifier: info.qualifier,
                        column_name: info.column_name,
                    }),
                })
                .accumulate(db);
            }
        }
    }

    let input_constraints = model_input_constraints(db, workspace, file);
    for constraint in input_constraints.iter() {
        let upstream = match resolve_ref(db, workspace, constraint.ref_name.clone()) {
            Some(p) => p,
            None => continue,
        };
        let upstream_schema = typed_model_schema(db, workspace, upstream);

        for (col_name, col_constraint) in &constraint.required_columns {
            let expected = match &col_constraint.expected_type {
                Some(t) => t,
                None => continue,
            };
            let upstream_col = upstream_schema.columns.iter().find(|c| c.name == *col_name);
            if let Some(col) = upstream_col {
                if let Some(actual) = &col.data_type {
                    if !types_compatible(&expected.data_type, &actual.data_type) {
                        for site in &col_constraint.usage_sites {
                            let range = smelt_parser::ast::text_range_to_range(text, *site);
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Warning,
                                message: format!(
                                    "Column '{}' from '{}' has type {} but is used where {} is expected",
                                    col_name, constraint.ref_name, actual.data_type, expected.data_type
                                ),
                                range,
                                code: Some(DiagnosticCode::TypeMismatch),
                                data: Some(DiagnosticData::TypeMismatch {
                                    column_name: col_name.clone(),
                                    ref_name: constraint.ref_name.clone(),
                                    actual_type: actual.data_type.to_string(),
                                    expected_type: expected.data_type.to_string(),
                                }),
                            })
                            .accumulate(db);
                        }
                    }
                }
            }
        }
    }

    // Detect cycles: base schema has content but resolved is empty OR all
    // types are Unknown → cycle recovery fired.
    for ref_loc in refs.iter() {
        if let Some(upstream) = resolve_ref(db, workspace, ref_loc.name.clone()) {
            let upstream_base = model_schema(db, upstream);
            let upstream_resolved = resolved_model_schema(db, workspace, upstream);
            let all_unknown = !upstream_resolved.columns.is_empty()
                && upstream_resolved.columns.iter().all(|c| {
                    c.data_type
                        .as_ref()
                        .map(|t| matches!(t.data_type, DataType::Unknown))
                        .unwrap_or(true)
                });
            let has_any_column =
                !upstream_base.columns.is_empty() || !upstream_base.row_extensions.is_empty();
            if has_any_column && (upstream_resolved.columns.is_empty() || all_unknown) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Circular dependency involving model '{}' — type information unavailable",
                        ref_loc.name
                    ),
                    range: ref_loc.range,
                    code: Some(DiagnosticCode::CircularDependency),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod test_harness;

#[cfg(test)]
mod tests;
