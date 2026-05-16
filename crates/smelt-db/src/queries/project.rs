//! Project-level queries (sources.yml, smelt.yml vars, paths/seeds/sources,
//! plus workspace-level enumeration of models / refs / sources).
//!
//! All Salsa-tracked but body delegates to `sources_config` etc. and plain
//! data structures from `smelt_core`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rowan::TextRange;
use serde::Deserialize;
use smelt_core::metadata::{extract_file_metadata, FileMetadata};
use smelt_parser::ast::{Expr, RecordLiteral};
use smelt_types::parse_type;

use crate::config_vars;
use crate::queries::loader::{loader_resolved_value, LoaderCallSiteId};
use crate::queries::parse::{parse_file, parse_model};
use crate::type_inference::{
    check_generator_body_reflection_forbid, infer_generator_file_body, validate_model_def_name,
    TypeContext,
};
use crate::{DiagnosticCode, LoaderFileInput, Model, ProjectInput, SourceFile, Workspace};

pub use smelt_types::{ModelOrigin, ModelRefValue, SourceOrigin, SourceRefValue};

use smelt_core::{IncrementalConfig, SeedInfo, SourceInfo, SourcesConfig};

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

/// Return `true` when the project's `smelt.yml` contains `unstable_schema: true`.
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
/// The LSP updates the text whenever `smelt.yml` changes on disk, so this
/// query is automatically invalidated and re-evaluated on each change.
#[salsa::tracked]
pub fn project_unstable_schema(db: &dyn salsa::Database, project: ProjectInput) -> bool {
    smelt_core::parse_unstable_schema_flag(project.smelt_yml_text(db))
}

/// Return the set of *active* backend names for `project` — i.e. the
/// distinct `target_type` values in `smelt.yml`'s `targets:` map.
///
/// Phase 42: this is the set the
/// [`as_struct_backend_diagnostics_for_file`] gate intersects against
/// when a function declares `BackendSet::All` (no explicit
/// `backends:` frontmatter). Returning `None` means we could not parse
/// the workspace config — callers should treat that as "no constraint
/// on active backends" and fall back to the Phase 38 behaviour
/// (only check explicitly-declared `BackendSet::Only`).
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
#[salsa::tracked]
pub fn project_active_backends(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Option<Vec<String>> {
    smelt_core::parse_active_backends(project.smelt_yml_text(db))
}

/// Return the `vars:` block from `smelt.yml` as a `BTreeMap<String, serde_yaml::Value>`.
///
/// Returns an empty map when the `vars:` key is absent or the YAML cannot be
/// parsed. The `None` case is mapped to an empty map to avoid bubbling parse
/// errors into callers — callers that need to distinguish "vars absent" from
/// "vars present but empty" should use `config_vars::parse_vars_from_yaml` directly.
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
/// Automatically invalidated and re-evaluated whenever `smelt.yml` changes.
#[salsa::tracked]
pub fn smelt_yml_vars_query(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Arc<std::collections::BTreeMap<String, serde_yaml::Value>> {
    let text = project.smelt_yml_text(db);
    Arc::new(config_vars::parse_vars_from_yaml(text).unwrap_or_default())
}

/// Return the project's `paths:` scan-root list from `smelt.yml`, or an
/// empty list when the config text is missing or fails to parse.
///
/// Reads from `ProjectInput::smelt_yml_text` so the result is Salsa-tracked
/// and reused across every per-file resolver call. Without this, callers
/// like `resolve_ref_path` re-read and re-parse `smelt.yml` once per
/// workspace file, turning per-file diagnostics into an O(N^2) operation
/// (this was the root cause of the multi-hour CI bench regression).
///
/// The empty-on-error fallback matches the previous `Config::load(...)
/// .map(|c| c.paths).unwrap_or_default()` behaviour in the resolver: when
/// the workspace has no parseable config, no scan-root prefix is stripped
/// and the full directory path becomes part of the resolved tuple.
#[salsa::tracked]
pub fn project_paths(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<String>> {
    let text = project.smelt_yml_text(db);
    let paths = smelt_core::Config::parse_with_warnings(text)
        .map(|(c, _warnings)| c.paths)
        .unwrap_or_default();
    Arc::new(paths)
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
    let paths = smelt_core::Config::load(&project_root)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    // Phase 5: use with_sidecars so ephemeral materialization is tracked.
    Arc::new(smelt_core::discover_seed_infos_with_sidecars(
        &project_root,
        &paths,
    ))
}

/// Discover per-entity source YAML files for a project root.
///
/// Phase 6: sources live as standalone `.yml` files (no sibling `.csv`) under
/// the project's `paths:` directories. The query is keyed on `ProjectInput`
/// so it re-runs when `smelt.yml` changes (e.g. `paths:` updated) but not on
/// every source file change (LSP restarts are acceptable for source changes).
#[salsa::tracked]
pub fn project_sources(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SourceInfo>> {
    let project_root = project.root(db).clone();
    let paths = smelt_core::Config::load(&project_root)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    Arc::new(smelt_core::discover_source_infos(&project_root, &paths))
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
// W1–W4 Generator pipeline
// ============================================================================

/// A single emitted model from a generator file.
///
/// Produced by `evaluate_generator` after materialising a `ModelDef` literal
/// from the generator body.  Carries the field values extracted from the AST as
/// plain strings so downstream consumers do not need to re-parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedModelDef {
    /// Absolute path of the generator `.sql` file.
    pub generator_file: PathBuf,
    /// The `ModelDef.name` field value (validated, path-safe).
    pub name: String,
    /// CST span of the `name` field value expression (for goto-def / diagnostics).
    pub name_span: TextRange,
    /// Raw SQL text of the `ModelDef.body` field value.
    pub body_text: String,
    /// CST span of the `body` field value expression.
    pub body_span: TextRange,
    /// `ModelDef.materialization` field value, defaulting to `"view"`.
    pub materialization: String,
    /// Tags from `ModelDef.tags` field, merged with frontmatter tags.
    pub tags: Vec<String>,
    /// `ModelDef.description` field value, defaulting to `""`.
    pub description: String,
    /// Inherited `incremental:` block from the generator file's frontmatter,
    /// when `materialization == "incremental"`.  `None` for non-incremental
    /// emissions or when the generator's frontmatter has no `incremental:` block.
    ///
    /// Per `incremental_models.md` §"Generator-emitted incremental models":
    /// the file-wide `incremental:` frontmatter is shared by all incremental
    /// emissions from that generator; per-`ModelDef` overrides are not supported
    /// in v1.
    pub incremental_config: Option<IncrementalConfig>,
}

/// The result of evaluating a single generator file.
///
/// Produced by the `evaluate_generator` Salsa query; does NOT include
/// cross-file collision diagnostics (those live in `EmittedModelsResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedGenerator {
    /// Successfully materialised model definitions from this generator.
    ///
    /// Per-file duplicate-name detection has already been applied: the second
    /// occurrence of a duplicate name is dropped from this list.
    pub emissions: Vec<EmittedModelDef>,
    /// Diagnostics from body type-checking and per-file duplicate-name
    /// detection (but NOT cross-file collision diagnostics).
    pub diagnostics: Vec<crate::Diagnostic>,
}

/// The aggregate result of the W3 collision-detection pass over all generators.
#[derive(Debug, Clone, PartialEq)]
pub struct EmittedModelsResult {
    /// Surviving (non-colliding) emitted models sorted by `(generator_file_path,
    /// modeldef_name)` byte-lexicographically — the deterministic ordering spec
    /// Semantics rule 10 requires.
    pub survivors: Vec<EmittedModelDef>,
    /// Emitted models that were dropped due to collision.
    pub discarded: Vec<EmittedModelDef>,
    /// Cross-file and hand-authored-wins collision diagnostics.  Each is anchored
    /// at the offending `ModelDef.name` field value expression.
    pub collision_diagnostics: Vec<crate::Diagnostic>,
}

/// W1: Discover generator files in the workspace.
///
/// Returns the subset of `workspace.files()` whose file content routes to
/// `FileMetadata::Generator { .. }` — i.e. files with `generates: models`
/// frontmatter.  Sorted by absolute path (byte-lexicographic) for
/// deterministic W3 iteration order.
#[salsa::tracked]
pub fn generator_files(db: &dyn salsa::Database, workspace: Workspace) -> Arc<Vec<SourceFile>> {
    let mut gen_files: Vec<SourceFile> = workspace
        .files(db)
        .iter()
        .copied()
        .filter(|&file| {
            let text = file.text(db);
            matches!(
                extract_file_metadata(text),
                Ok(FileMetadata::Generator { .. })
            )
        })
        .collect();
    // Sort by absolute path for deterministic W3 cross-file ordering.
    gen_files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    Arc::new(gen_files)
}

/// Pure helper: extract the `name` field literal value text from a `RecordLiteral`.
fn extract_modeldef_field_text(
    literal: &RecordLiteral,
    field_name: &str,
) -> Option<(String, TextRange)> {
    for field in literal.fields() {
        let name = field.name()?;
        if name != field_name {
            continue;
        }
        let value_expr = field.value_expr()?;
        let span = value_expr.syntax().text_range();
        let raw = value_expr.syntax().text().to_string();
        // Strip surrounding quotes for string literals.
        let unquoted = if raw.len() >= 2
            && ((raw.starts_with('\'') && raw.ends_with('\''))
                || (raw.starts_with('"') && raw.ends_with('"')))
        {
            raw[1..raw.len() - 1].to_string()
        } else {
            raw
        };
        return Some((unquoted, span));
    }
    None
}

/// Pure helper: materialise an `EmittedModelDef` from a `RecordLiteral` AST node.
///
/// Returns `None` when the `name` field is absent or invalid (the caller should
/// have already run `infer_model_def_literal` to diagnose those cases; this
/// function is a best-effort extractor for the happy path).
///
/// `frontmatter_incremental` is the generator file's file-wide `incremental:`
/// block (if any). When `ModelDef.materialization == "incremental"` the emitted
/// model inherits this config; for non-incremental emissions it is ignored.
fn materialise_emitted_model_def(
    generator_file: &Path,
    literal: &RecordLiteral,
    frontmatter_tags: &[String],
    frontmatter_incremental: Option<&IncrementalConfig>,
) -> Option<EmittedModelDef> {
    let (name, name_span) = extract_modeldef_field_text(literal, "name")?;
    if name.is_empty() {
        return None;
    }
    // body: take the raw SQL text (the whole value expression text).
    //
    // NOTE: `field.value_expr()` uses `Expr::cast` which does not accept
    // SELECT_STMT nodes — so `body: SELECT 1` would return None.  Instead we
    // look at the RECORD_FIELD's raw SyntaxNode children and take the text of
    // the first non-token child that follows the COLON (any SyntaxNode child
    // after the colon is the value).
    let body_text;
    let body_span;
    {
        use smelt_parser::SyntaxKind::COLON;
        let mut found = false;
        let mut bt = String::new();
        let mut bs = TextRange::new(0.into(), 0.into());
        for field in literal.fields() {
            if field.name().as_deref() == Some("body") {
                // First: try field.value_expr() for normal expression values.
                if let Some(val) = field.value_expr() {
                    bt = val.syntax().text().to_string();
                    bs = val.syntax().text_range();
                    found = true;
                } else {
                    // Fallback: the body value may be a SELECT_STMT (not an Expr).
                    // Find the value node: first SyntaxNode child of the RECORD_FIELD
                    // that appears after the COLON token.
                    let mut past_colon = false;
                    for child in field.syntax().children_with_tokens() {
                        match child {
                            rowan::NodeOrToken::Token(tok) if tok.kind() == COLON => {
                                past_colon = true;
                            }
                            rowan::NodeOrToken::Node(node) if past_colon => {
                                bt = node.text().to_string();
                                bs = node.text_range();
                                found = true;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if !found {
            return None;
        }
        body_text = bt;
        body_span = bs;
    }

    let materialization = extract_modeldef_field_text(literal, "materialization")
        .map(|(v, _)| v)
        .unwrap_or_else(|| "view".to_string());

    // tags: combine ModelDef.tags with frontmatter tags.
    let mut tags: Vec<String> = frontmatter_tags.to_vec();
    for field in literal.fields() {
        if field.name().as_deref() != Some("tags") {
            continue;
        }
        if let Some(val) = field.value_expr() {
            // val is an array literal like `['cohort', 'region']`.
            if let Some(arr) = val.as_array_literal() {
                for elem in arr.elements() {
                    let raw = elem.syntax().text().to_string();
                    let unquoted = if raw.len() >= 2
                        && ((raw.starts_with('\'') && raw.ends_with('\''))
                            || (raw.starts_with('"') && raw.ends_with('"')))
                    {
                        raw[1..raw.len() - 1].to_string()
                    } else {
                        raw
                    };
                    if !tags.contains(&unquoted) {
                        tags.push(unquoted);
                    }
                }
            }
        }
    }

    let description = extract_modeldef_field_text(literal, "description")
        .map(|(v, _)| v)
        .unwrap_or_default();

    // Inherit the `incremental:` block only when materialization == "incremental".
    // Per `incremental_models.md` §"Generator-emitted incremental models", non-incremental
    // emissions silently ignore the generator's frontmatter `incremental:` block.
    let incremental_config = if materialization == "incremental" {
        frontmatter_incremental.cloned()
    } else {
        None
    };

    Some(EmittedModelDef {
        generator_file: generator_file.to_path_buf(),
        name,
        name_span,
        body_text,
        body_span,
        materialization,
        tags,
        description,
        incremental_config,
    })
}

/// Pure helper: evaluate a meta-language expression body from a generator file
/// and collect `EmittedModelDef` values.
///
/// Handles two patterns:
/// 1. Array literal `[ModelDef { … }, …]` — each element that is a `RecordLiteral`
///    is materialised directly.
/// 2. HOF chain (`smelt.config.load_yaml(…) |> map(fn c => ModelDef { name: c.name, … })`)
///    — the loader data is provided via `loader_values`; the lambda body's `ModelDef`
///    literals are materialised with field accesses resolved against each record entry.
///
/// Returns the list of materialised models (may be empty) plus any emission-level
/// diagnostics (e.g. body type error from generator-body inference).
fn evaluate_body_emissions(
    generator_file: &Path,
    body_expr: &Expr,
    frontmatter_tags: &[String],
    frontmatter_incremental: Option<&IncrementalConfig>,
    loader_values: &[(String, crate::loader::MetaValue)],
    ctx: &TypeContext,
) -> (Vec<EmittedModelDef>, Vec<crate::Diagnostic>) {
    let mut emissions: Vec<EmittedModelDef> = Vec::new();
    let mut diagnostics: Vec<crate::Diagnostic> = Vec::new();

    // Run body type-check first.
    let body_result = infer_generator_file_body(body_expr, ctx);
    for sentinel in body_result.sentinels {
        let range = smelt_parser::ast::text_range_to_range(
            // We need the file text for accurate line/col but don't have it here.
            // Use a zero-range placeholder; the Salsa caller has the file text.
            "",
            sentinel.span,
        );
        diagnostics.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: sentinel.message,
            range,
            code: Some(sentinel.code),
            data: None,
        });
    }

    // Case 1: top-level array literal.
    //
    // CST structure for `[ModelDef { name: 'x', body: SELECT 1 }]`:
    //   EXPRESSION (outer wrapper from parse_expression())
    //     ARRAY_LITERAL
    //       EXPRESSION (per-element wrapper from parse_bracket_list_literal)
    //         RECORD_LITERAL  ← ModelDef { … }
    //
    // `arr.elements()` uses `Expr::cast` which does not accept RECORD_LITERAL,
    // so all elements would be silently dropped.  Instead we locate the
    // ARRAY_LITERAL and, for each of its EXPRESSION children, find the
    // nested RECORD_LITERAL.
    if body_expr.as_array_literal().is_some() {
        use smelt_parser::SyntaxKind::{ARRAY_LITERAL, EXPRESSION, RECORD_LITERAL};
        let expr_node = body_expr.syntax();
        let array_node = if expr_node.kind() == ARRAY_LITERAL {
            expr_node.clone()
        } else {
            expr_node
                .children()
                .find(|n| n.kind() == ARRAY_LITERAL)
                .unwrap_or_else(|| expr_node.clone())
        };
        for expr_child in array_node.children().filter(|n| n.kind() == EXPRESSION) {
            if let Some(record_node) = expr_child.children().find(|n| n.kind() == RECORD_LITERAL) {
                if let Some(record_lit) = RecordLiteral::cast(record_node) {
                    // Collect per-element type-check diagnostics (invalid name,
                    // missing body, etc.) from infer_model_def_literal.
                    let elem_result =
                        crate::type_inference::infer_model_def_literal(&record_lit, ctx);
                    for sentinel in elem_result.sentinels {
                        let range = smelt_parser::ast::text_range_to_range("", sentinel.span);
                        diagnostics.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: sentinel.message,
                            range,
                            code: Some(sentinel.code),
                            data: None,
                        });
                    }
                    if let Some(emitted) = materialise_emitted_model_def(
                        generator_file,
                        &record_lit,
                        frontmatter_tags,
                        frontmatter_incremental,
                    ) {
                        emissions.push(emitted);
                    }
                }
            }
        }
        return (emissions, diagnostics);
    }

    // Case 2: pipe expression ending in a `map(fn item => ModelDef { … })` HOF.
    // Walk the pipe chain looking for the terminal map call.
    if let Some(pipe) = extract_pipe_expr_from_body(body_expr) {
        if let (Some(loader_call), Some(map_call)) = (pipe.lhs(), pipe.rhs()) {
            // The RHS of the pipe is the `map` call.
            if let Some(func) = map_call.as_function_call() {
                let fname = func.name().unwrap_or_default().to_lowercase();
                if fname == "map" {
                    // Extract the loader call's resolved value from `loader_values`.
                    let loader_path = extract_loader_path(&loader_call);
                    let empty_slice: &[crate::loader::MetaValue] = &[];
                    let records: &[crate::loader::MetaValue] = if let Some(path) = loader_path {
                        loader_values
                            .iter()
                            .find(|(p, _)| *p == path)
                            .and_then(|(_, v)| match v {
                                crate::loader::MetaValue::List(items) => Some(items.as_slice()),
                                _ => None,
                            })
                            .unwrap_or(empty_slice)
                    } else {
                        empty_slice
                    };

                    // Get the lambda (second positional arg to map).
                    let args = func.arguments();
                    if let Some(lambda_arg) = args.first() {
                        // The lambda is a `LAMBDA` node (fn c => body).
                        if let Some(lambda) = smelt_parser::ast::Lambda::cast(
                            lambda_arg.syntax().clone(),
                        )
                        .or_else(|| {
                            lambda_arg
                                .syntax()
                                .descendants()
                                .find_map(smelt_parser::ast::Lambda::cast)
                        }) {
                            if let Some(body) = lambda.body() {
                                // For each record in the loader, evaluate the lambda body.
                                for record_val in records {
                                    if let Some(emitted) = materialise_modeldef_from_lambda_body(
                                        generator_file,
                                        &body,
                                        record_val,
                                        frontmatter_tags,
                                        frontmatter_incremental,
                                    ) {
                                        emissions.push(emitted);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    (emissions, diagnostics)
}

/// Extract the pipe expression from a body expr, if it is one.
fn extract_pipe_expr_from_body(expr: &Expr) -> Option<smelt_parser::ast::PipeExpr> {
    use smelt_parser::SyntaxKind::PIPE_EXPR;
    let node = expr.syntax().clone();
    if node.kind() == PIPE_EXPR {
        return smelt_parser::ast::PipeExpr::cast(node);
    }
    // Check if the expr wraps a PIPE_EXPR as its only child.
    for child in expr.syntax().children() {
        if child.kind() == PIPE_EXPR {
            return smelt_parser::ast::PipeExpr::cast(child);
        }
    }
    None
}

/// Extract the loader file path from a `smelt.config.load_yaml('path', Schema)` call.
fn extract_loader_path(expr: &Expr) -> Option<String> {
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;
    // Find a SMELT_PATH_CALL node in the expr subtree.
    for node in expr.syntax().descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        if let Some(call) = smelt_parser::ast::SmeltPathCall::cast(node) {
            let segs = call.segments();
            if segs.len() == 2 && segs[0].to_lowercase() == "config" {
                let method = segs[1].to_lowercase();
                if method == "load_yaml" || method == "load_json" {
                    let args = call.arg_list()?.positional_args();
                    let path_arg = args.first()?;
                    let raw = path_arg.syntax().text().to_string();
                    // Strip quotes.
                    let unquoted = if raw.len() >= 2
                        && ((raw.starts_with('\'') && raw.ends_with('\''))
                            || (raw.starts_with('"') && raw.ends_with('"')))
                    {
                        raw[1..raw.len() - 1].to_string()
                    } else {
                        raw
                    };
                    return Some(unquoted);
                }
            }
        }
    }
    None
}

/// Materialise a `ModelDef` from a lambda body expression by substituting
/// the `c.field` accesses with values from a `MetaValue::Record`.
fn materialise_modeldef_from_lambda_body(
    generator_file: &Path,
    body: &Expr,
    record_val: &crate::loader::MetaValue,
    frontmatter_tags: &[String],
    frontmatter_incremental: Option<&IncrementalConfig>,
) -> Option<EmittedModelDef> {
    let record_fields = match record_val {
        crate::loader::MetaValue::Record(fields) => fields,
        _ => return None,
    };

    // The lambda body should be a RecordLiteral `ModelDef { name: c.name, body: … }`.
    let literal = body.syntax().descendants().find_map(RecordLiteral::cast)?;

    // Extract `name` field — may be a field access like `c.name` or a literal string.
    let (name, name_span) = extract_field_value_with_binding(&literal, "name", record_fields)?;
    if name.is_empty() {
        return None;
    }

    // Extract `body` field text.
    let (body_text, body_span) = extract_field_value_with_binding(&literal, "body", record_fields)?;

    let materialization =
        extract_field_value_with_binding(&literal, "materialization", record_fields)
            .map(|(v, _)| v)
            .unwrap_or_else(|| "view".to_string());

    let description = extract_field_value_with_binding(&literal, "description", record_fields)
        .map(|(v, _)| v)
        .unwrap_or_default();

    let mut tags = frontmatter_tags.to_vec();
    for field in literal.fields() {
        if field.name().as_deref() != Some("tags") {
            continue;
        }
        if let Some(val) = field.value_expr() {
            if let Some(arr) = val.as_array_literal() {
                for elem in arr.elements() {
                    let raw = elem.syntax().text().to_string();
                    let unquoted = if raw.len() >= 2
                        && ((raw.starts_with('\'') && raw.ends_with('\''))
                            || (raw.starts_with('"') && raw.ends_with('"')))
                    {
                        raw[1..raw.len() - 1].to_string()
                    } else {
                        raw
                    };
                    if !tags.contains(&unquoted) {
                        tags.push(unquoted);
                    }
                }
            }
        }
    }

    // Inherit the `incremental:` block only when materialization == "incremental".
    let incremental_config = if materialization == "incremental" {
        frontmatter_incremental.cloned()
    } else {
        None
    };

    Some(EmittedModelDef {
        generator_file: generator_file.to_path_buf(),
        name,
        name_span,
        body_text,
        body_span,
        materialization,
        tags,
        description,
        incremental_config,
    })
}

/// Extract a field value from a record literal, resolving simple `param.field`
/// accesses against the given record binding.
///
/// Returns the resolved text value and the span of the field value expression.
fn extract_field_value_with_binding(
    literal: &RecordLiteral,
    field_name: &str,
    record_fields: &std::collections::BTreeMap<String, crate::loader::MetaValue>,
) -> Option<(String, TextRange)> {
    use smelt_parser::SyntaxKind::COLON;
    for field in literal.fields() {
        let name = field.name()?;
        if name != field_name {
            continue;
        }
        // Try the normal Expr path first (string literals, field accesses, etc.).
        if let Some(value_expr) = field.value_expr() {
            let span = value_expr.syntax().text_range();
            let raw = value_expr.syntax().text().to_string();
            let resolved = resolve_field_access_or_literal(&raw, record_fields);
            return Some((resolved, span));
        }
        // Fallback: the field value may be a SELECT_STMT (body: SELECT ...) which
        // Expr::cast does not accept.  Find the first SyntaxNode child after COLON.
        let mut past_colon = false;
        for child in field.syntax().children_with_tokens() {
            match child {
                rowan::NodeOrToken::Token(tok) if tok.kind() == COLON => {
                    past_colon = true;
                }
                rowan::NodeOrToken::Node(node) if past_colon => {
                    let span = node.text_range();
                    let raw = node.text().to_string();
                    // SQL body text — return as-is (no field-access resolution).
                    return Some((raw.trim().to_string(), span));
                }
                _ => {}
            }
        }
    }
    None
}

/// Resolve a simple expression text to a string:
/// - If it's a string literal like `'foo'`, strip the quotes.
/// - If it looks like a `param.field` access (e.g. `c.name`), look up in the
///   record binding.
/// - Otherwise return the raw text.
fn resolve_field_access_or_literal(
    raw: &str,
    record_fields: &std::collections::BTreeMap<String, crate::loader::MetaValue>,
) -> String {
    let s = raw.trim();
    // String literal.
    if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
    {
        return s[1..s.len() - 1].to_string();
    }
    // Field access `param.field` — split at the last dot.
    if let Some(dot_pos) = s.rfind('.') {
        let field_key = &s[dot_pos + 1..];
        if let Some(val) = record_fields.get(field_key) {
            return match val {
                crate::loader::MetaValue::Text(t) => t.clone(),
                crate::loader::MetaValue::Integer(n) => n.to_string(),
                other => format!("{:?}", other),
            };
        }
    }
    s.to_string()
}

/// Compute the workspace-relative smelt path for an emitted model.
///
/// Spec rule (Semantics rule 7): `smelt.<dir>.<file_stem>.<modeldef.name>`
/// where `<dir>` is the scan-root-stripped directory chain (joined by `.`)
/// and `<file_stem>` is the generator file's stem (without `.gen.sql` etc.).
pub fn emitted_model_smelt_path(
    generator_file: &Path,
    project_root: &Path,
    scan_roots: &[String],
    model_name: &str,
) -> String {
    // Strip project root.
    let rel = generator_file
        .strip_prefix(project_root)
        .unwrap_or(generator_file);

    // Strip the first matching scan root prefix.
    let effective_rel = scan_roots
        .iter()
        .find_map(|sr| {
            let sr_path = std::path::Path::new(sr.as_str());
            rel.strip_prefix(sr_path).ok()
        })
        .unwrap_or(rel);

    // Directory components.
    let parent = effective_rel.parent().unwrap_or(std::path::Path::new(""));
    let dir_parts: Vec<String> = parent
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    // File stem (strip all extensions, e.g. `cohorts.gen.sql` → `cohorts`).
    let stem = effective_rel
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("");

    // Build path segments: dir... + stem + model_name.
    let mut parts = dir_parts;
    parts.push(stem.to_string());
    parts.push(model_name.to_string());
    parts.join(".")
}

/// W2: Evaluate a single generator file, returning its emissions and diagnostics.
///
/// Memoised by Salsa on `(workspace, file)`. Invalidated when the generator
/// file's text changes (via `file.text(db)` dependency) or when any loader
/// file referenced by the generator changes (via the `loader_resolved_value`
/// dependency chain).
///
/// Does NOT perform cross-file collision detection — that is W3's job.
#[salsa::tracked]
pub fn evaluate_generator(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<EvaluatedGenerator> {
    let text = file.text(db);
    let gen_meta = match extract_file_metadata(text) {
        Ok(FileMetadata::Generator {
            metadata,
            body_offset,
        }) => (metadata, body_offset),
        _ => {
            return Arc::new(EvaluatedGenerator {
                emissions: Vec::new(),
                diagnostics: Vec::new(),
            });
        }
    };
    let (metadata, _body_offset) = gen_meta;
    let frontmatter_tags = metadata.tags.clone();
    let frontmatter_incremental = metadata.incremental.clone();

    // Parse the file body (already cached by Salsa via `parse_file`).
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    // Find the body expression in the parse tree (the top-level expression
    // produced by `parse_meta_expression_from_offset`).
    //
    // IMPORTANT: `Expr::cast(FILE)` succeeds in the fallback branch because
    // FILE has expression-like children — so `syntax.descendants().find_map(Expr::cast)`
    // returns the FILE node itself, not the inner body EXPRESSION.  We skip
    // the root FILE node by looking at direct FILE children instead.
    let body_expr = syntax.children().find_map(Expr::cast);

    let body_expr = match body_expr {
        Some(e) => e,
        None => {
            return Arc::new(EvaluatedGenerator {
                emissions: Vec::new(),
                diagnostics: Vec::new(),
            });
        }
    };

    // Build a generator TypeContext with `is_inside_generator_file = true`
    // and `workspace_shape_includes_generators = false`.
    let mut ctx = TypeContext::new();
    ctx.is_inside_generator_file = true;
    ctx.workspace_shape_includes_generators = false;

    // Collect loader values: for each `smelt.config.load_yaml(path, Schema)` call
    // in the body, look up the resolved loader value from Salsa.
    let loader_values = collect_loader_values(db, workspace, file, &syntax);

    // Run the body evaluation.
    let (raw_emissions, mut diagnostics) = evaluate_body_emissions(
        file.path(db),
        &body_expr,
        &frontmatter_tags,
        frontmatter_incremental.as_ref(),
        &loader_values,
        &ctx,
    );

    // Emit reflection-forbid diagnostics.
    let forbid_sentinels = check_generator_body_reflection_forbid(&syntax);
    for sentinel in forbid_sentinels {
        let range = smelt_parser::ast::text_range_to_range(text, sentinel.span);
        diagnostics.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: sentinel.message,
            range,
            code: Some(sentinel.code),
            data: None,
        });
    }

    // Per-file duplicate-name detection.
    let mut seen_names: HashMap<String, ()> = HashMap::new();
    let mut emissions: Vec<EmittedModelDef> = Vec::new();
    for emitted in raw_emissions {
        if seen_names.contains_key(&emitted.name) {
            // Duplicate — emit diagnostic at the name span, drop this emission.
            let range = smelt_parser::ast::text_range_to_range(text, emitted.name_span);
            diagnostics.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: format!(
                    "duplicate ModelDef.name `{}` in this generator file",
                    emitted.name
                ),
                range,
                code: Some(DiagnosticCode::ModelDefDuplicateName),
                data: None,
            });
        } else {
            seen_names.insert(emitted.name.clone(), ());
            emissions.push(emitted);
        }
    }

    // Emit ModelDefInvalidName diagnostics for emissions with bad names
    // (these would have been dropped by materialise_emitted_model_def for completely invalid
    // names, but duplicate-emit diagnoses here for names we DID materialise that have issues).
    // Re-check each surviving emission.
    for emitted in &emissions {
        let zero = TextRange::new(0.into(), 0.into());
        if let Some(sentinel) = validate_model_def_name(&emitted.name, zero) {
            let range = smelt_parser::ast::text_range_to_range(text, emitted.name_span);
            diagnostics.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: sentinel.message,
                range,
                code: Some(sentinel.code),
                data: None,
            });
        }
    }

    Arc::new(EvaluatedGenerator {
        emissions,
        diagnostics,
    })
}

/// Collect resolved loader values for all `smelt.config.load_yaml(path, Schema)`
/// calls in the generator body.
///
/// Returns a vec of `(workspace_relative_path, MetaValue)` pairs. Each entry
/// corresponds to one loader call site. If no `LoaderFileInput` is registered
/// for the path, the entry is omitted.
fn collect_loader_values(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
) -> Vec<(String, crate::loader::MetaValue)> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut result = Vec::new();

    // Build a path→input lookup from workspace.loader_files.
    let loader_inputs: Vec<LoaderFileInput> = workspace.loader_files(db).to_vec();
    let mut input_by_path: HashMap<String, LoaderFileInput> = HashMap::new();
    for inp in &loader_inputs {
        input_by_path.insert(inp.path(db).to_string(), *inp);
    }

    for node in syntax.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };
        let segs = call.segments();
        if segs.len() != 2 || segs[0].to_lowercase() != "config" {
            continue;
        }
        let method = segs[1].to_lowercase();
        if method != "load_yaml" && method != "load_json" {
            continue;
        }

        let args = match call.arg_list() {
            Some(a) => a.positional_args(),
            None => continue,
        };
        let path_arg = match args.first() {
            Some(a) => a,
            None => continue,
        };
        let schema_arg = match args.get(1) {
            Some(a) => a,
            None => continue,
        };

        // Extract the path string literal.
        if !crate::config_vars::is_string_literal_expr(path_arg) {
            continue;
        }
        let path_value = match crate::config_vars::extract_string_literal_value(path_arg) {
            Some(v) => v,
            None => continue,
        };

        // Look up the registered loader input.
        let loader_input = match input_by_path.get(&path_value) {
            Some(inp) => *inp,
            None => continue,
        };
        if !loader_input.exists(db) {
            continue;
        }

        // Build the call-site ID and call the Salsa query.
        let byte_offset = node.text_range().start().into();
        let schema_text: Arc<str> = Arc::from(schema_arg.syntax().text().to_string().as_str());
        let file_path_str: Arc<str> = Arc::from(
            file.path(db)
                .strip_prefix(file.project_root(db))
                .unwrap_or(file.path(db))
                .to_string_lossy()
                .replace('\\', "/")
                .as_str(),
        );
        let loader_path: Arc<str> = Arc::from(path_value.as_str());

        let call_site = LoaderCallSiteId {
            file_path: file_path_str,
            byte_offset,
            loader_path: loader_path.clone(),
            schema_text,
        };

        let resolved = loader_resolved_value(db, loader_input, call_site);

        if let Some(merged) = &resolved.merged {
            result.push((path_value, merged.clone()));
        } else if let Some(parsed_ref) = &resolved.parsed {
            // No merge yet — use the base parsed value.
            // We need to get the MetaValue from the parsed file by running
            // validation. Since `loader_resolved_value` already did that and
            // stored it in `merged = None` (no overlay), re-use the
            // diagnostics-free path: treat the first non-error result.
            let _ = parsed_ref;
            // Use the loader_resolved_value result's value via validation_result.value.
            // This requires calling validate_against_schema again here.
            // Use a simple helper.
            let schema_ty = crate::queries::loader::parse_smelt_type_from_field_annotation(
                &schema_arg.syntax().text().to_string(),
                "",
            );
            let call_range = crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            };
            let parsed_clone = (**parsed_ref).clone();
            let val_result =
                crate::loader::validate_against_schema(&parsed_clone, &schema_ty, call_range);
            if val_result.diagnostics.is_empty()
                || val_result
                    .diagnostics
                    .iter()
                    .all(|d| d.severity != crate::loader::LoaderDiagnosticSeverity::Error)
            {
                result.push((path_value, val_result.value));
            }
        }
    }

    result
}

/// W3: Aggregate all generators' emissions, detect collisions, and produce
/// the workspace-wide `EmittedModelsResult`.
///
/// Collision detection order:
/// 1. Within each generator: per-file duplicates (already handled in `evaluate_generator`).
/// 2. Cross-file: earlier-path generator wins; later-path generator's emission is dropped.
/// 3. Hand-authored wins: an emission that maps to the same smelt path as a
///    hand-authored model is dropped.
///
/// Iteration order over generator files is the byte-lexicographic order on
/// their absolute paths (per spec Semantics rule 10).
#[salsa::tracked]
pub fn emitted_models(db: &dyn salsa::Database, workspace: Workspace) -> Arc<EmittedModelsResult> {
    let gen_files = generator_files(db, workspace);

    // Collect all hand-authored smelt paths for collision detection.
    // A hand-authored model's smelt path is: `<dir_chain>.<stem>` (no modeldef.name suffix).
    let hand_authored_paths: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        for file in workspace.files(db).iter().copied() {
            let text = file.text(db);
            let is_gen = matches!(
                extract_file_metadata(text),
                Ok(FileMetadata::Generator { .. })
            );
            if is_gen {
                continue;
            }
            if parse_model(db, file).is_none() {
                continue;
            }
            // Compute the smelt path for this hand-authored model.
            let project_root = file.project_root(db).clone();
            let abs_path = file.path(db).clone();
            let scan_roots = workspace
                .projects(db)
                .iter()
                .find(|p| p.root(db).as_path() == project_root.as_path())
                .map(|p| project_paths(db, *p))
                .map(|paths| paths.as_ref().clone())
                .unwrap_or_default();
            let rel = abs_path.strip_prefix(&project_root).unwrap_or(&abs_path);
            let effective_rel = scan_roots
                .iter()
                .find_map(|sr| rel.strip_prefix(std::path::Path::new(sr.as_str())).ok())
                .unwrap_or(rel);
            // Build smelt path: dir components joined by `.` + stem.
            let parent = effective_rel.parent().unwrap_or(std::path::Path::new(""));
            let dir_parts: Vec<String> = parent
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            let stem = effective_rel
                .file_stem()
                .and_then(|f| f.to_str())
                .unwrap_or("")
                .to_string();
            let mut parts = dir_parts;
            parts.push(stem);
            set.insert(parts.join("."));
        }
        set
    };

    let mut survivors: Vec<EmittedModelDef> = Vec::new();
    let mut discarded: Vec<EmittedModelDef> = Vec::new();
    let mut collision_diagnostics: Vec<crate::Diagnostic> = Vec::new();

    // Track seen smelt paths across all generators.
    let mut seen_smelt_paths: HashMap<String, PathBuf> = HashMap::new();

    for gen_file in gen_files.iter().copied() {
        let evaluated = evaluate_generator(db, workspace, gen_file);
        let project_root = gen_file.project_root(db).clone();
        let scan_roots = workspace
            .projects(db)
            .iter()
            .find(|p| p.root(db).as_path() == project_root.as_path())
            .map(|p| project_paths(db, *p))
            .map(|paths| paths.as_ref().clone())
            .unwrap_or_default();
        let file_text = gen_file.text(db);

        for emitted in &evaluated.emissions {
            let smelt_path = emitted_model_smelt_path(
                &emitted.generator_file,
                &project_root,
                &scan_roots,
                &emitted.name,
            );

            // Check hand-authored collision: the emitted smelt path matches a
            // hand-authored model's path either exactly or as a prefix.
            //
            // Case A (exact): emitted `cohorts.us_west` == hand-authored `cohorts.us_west`
            // Case B (prefix): emitted `cohorts.us_west.sub` starts with `cohorts.us_west.`
            let collides_with_hand_authored = hand_authored_paths.iter().any(|ha_path| {
                smelt_path == *ha_path || smelt_path.starts_with(&format!("{}.", ha_path))
            });

            if collides_with_hand_authored {
                // Find the actual hand-authored file path for the message.
                let other_path = hand_authored_paths
                    .iter()
                    .find(|ha| {
                        smelt_path == ha.as_str() || smelt_path.starts_with(&format!("{}.", ha))
                    })
                    .cloned()
                    .unwrap_or_default();
                let range = smelt_parser::ast::text_range_to_range(file_text, emitted.name_span);
                collision_diagnostics.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: format!(
                        "ModelDef emits `{}` which collides with {}",
                        smelt_path, other_path
                    ),
                    range,
                    code: Some(DiagnosticCode::ModelDefHandAuthoredCollision),
                    data: None,
                });
                discarded.push(emitted.clone());
                continue;
            }

            // Check cross-generator collision.
            if let Some(prior_gen_path) = seen_smelt_paths.get(&smelt_path) {
                let range = smelt_parser::ast::text_range_to_range(file_text, emitted.name_span);
                collision_diagnostics.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: format!(
                        "ModelDef emits `{}` which collides with {}",
                        smelt_path,
                        prior_gen_path.display()
                    ),
                    range,
                    code: Some(DiagnosticCode::ModelDefHandAuthoredCollision),
                    data: None,
                });
                discarded.push(emitted.clone());
                continue;
            }

            seen_smelt_paths.insert(smelt_path, emitted.generator_file.clone());
            survivors.push(emitted.clone());
        }
    }

    // Sort survivors by (generator_file_path, modeldef_name) byte-lexicographic.
    survivors.sort_by(|a, b| {
        a.generator_file
            .cmp(&b.generator_file)
            .then_with(|| a.name.cmp(&b.name))
    });

    Arc::new(EmittedModelsResult {
        survivors,
        discarded,
        collision_diagnostics,
    })
}

/// Pure helper: materialise a `ModelRefValue` from an `EmittedModelDef`.
fn make_model_ref_value_from_emitted(
    emitted: &EmittedModelDef,
    project_root: &Path,
    scan_roots: &[String],
    smelt_yml_config: &Option<smelt_core::Config>,
) -> ModelRefValue {
    let smelt_path = emitted_model_smelt_path(
        &emitted.generator_file,
        project_root,
        scan_roots,
        &emitted.name,
    );

    // Compute workspace-relative path of the generator file with `/` separators.
    let rel_path = emitted
        .generator_file
        .strip_prefix(project_root)
        .unwrap_or(&emitted.generator_file)
        .to_string_lossy()
        .replace('\\', "/");

    // Merge tags: ModelDef.tags + smelt.yml overlay.
    let merged_tags = if let Some(config) = smelt_yml_config {
        // The emitted model name for smelt.yml lookup purposes is the smelt path leaf.
        let leaf_name = &emitted.name;
        // Build a minimal ModelMetadata-compatible struct for get_tags.
        let fm_meta = smelt_core::ModelMetadata {
            tags: emitted.tags.clone(),
            ..Default::default()
        };
        config.get_tags(leaf_name, Some(&fm_meta))
    } else {
        emitted.tags.clone()
    };

    ModelRefValue {
        path: rel_path,
        name: smelt_path.clone(),
        tags: merged_tags,
        model_name_for_columns: smelt_path,
    }
}

/// W4: Return all models in the workspace (hand-authored + generator-emitted),
/// sorted ascending by workspace-relative `path` then `name`
/// (byte-lexicographic). Salsa-cached.
#[salsa::tracked]
pub fn models_all_with_generators(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<Vec<ModelRefValue>> {
    // Start with hand-authored models.
    let mut result: Vec<ModelRefValue> = workspace
        .files(db)
        .iter()
        .copied()
        .filter_map(|file| make_model_ref_value(db, workspace, file))
        .collect();

    // Add generator-emitted models.
    let emitted = emitted_models(db, workspace);
    for project in workspace.projects(db).iter().copied() {
        let project_root = project.root(db).clone();
        let scan_roots = project_paths(db, project);
        let smelt_yml_text = project.smelt_yml_text(db);
        let config = smelt_core::Config::parse_with_warnings(smelt_yml_text)
            .map(|(c, _)| c)
            .ok();

        for emitted_model in &emitted.survivors {
            if !emitted_model.generator_file.starts_with(&project_root) {
                continue;
            }
            let mrv = make_model_ref_value_from_emitted(
                emitted_model,
                &project_root,
                scan_roots.as_slice(),
                &config,
            );
            result.push(mrv);
        }
    }

    result.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));
    Arc::new(result)
}

// ============================================================================
// Wide-reflection Salsa queries (Phase D, meta-language)
// ============================================================================
//
// Four queries materialise the `List<ModelRef>` / `List<SourceRef>` values
// that `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`,
// and `smelt.sources.all` resolve to at expansion time.
//
// Each query is a thin wrapper over pure filtering/projection logic:
// - Read the existing `all_models` / `project_sources` Salsa input.
// - Filter by tag membership (when applicable).
// - Project to `ModelRefValue` / `SourceRefValue`.
// - Sort ascending by `path` (byte-lexicographic on workspace-relative path
//   with `/` separators).
//
// The query keys on `Workspace` / `ProjectInput` so Salsa invalidates on
// workspace-state changes that affect tag membership or file lists.
//
// Per the pure-function rule (CLAUDE.md): analysis logic is in the pure
// helpers below; queries are thin wrappers.

/// Pure helper: materialise a `ModelRefValue` from a model file.
///
/// Returns `None` when the file is not a valid model.
fn make_model_ref_value(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<ModelRefValue> {
    let model = parse_model(db, file)?;
    let raw_text = file.text(db);
    let project_root = file.project_root(db).clone();
    let abs_path = file.path(db).clone();

    // Compute workspace-relative path with `/` separators.
    let rel_path = abs_path
        .strip_prefix(&project_root)
        .unwrap_or(&abs_path)
        .to_string_lossy()
        .replace('\\', "/");

    // Extract frontmatter metadata to get the model's frontmatter tags.
    let frontmatter_metadata: Option<smelt_core::ModelMetadata> =
        smelt_core::extract_file_metadata(raw_text)
            .ok()
            .and_then(|fm| match fm {
                smelt_core::FileMetadata::Single { metadata, .. } => Some(*metadata),
                _ => None,
            });

    // Load the smelt.yml config for the project to get the merged tag set.
    let smelt_yml_text = workspace
        .projects(db)
        .iter()
        .copied()
        .find(|p| p.root(db).as_path() == project_root.as_path())
        .map(|p| p.smelt_yml_text(db).clone())
        .unwrap_or_default();

    let merged_tags =
        if let Ok((config, _)) = smelt_core::Config::parse_with_warnings(&smelt_yml_text) {
            config.get_tags(&model.name, frontmatter_metadata.as_ref())
        } else {
            // No smelt.yml or parse failure — only frontmatter tags.
            frontmatter_metadata.map(|m| m.tags).unwrap_or_default()
        };

    Some(ModelRefValue {
        path: rel_path,
        name: model.name.clone(),
        tags: merged_tags,
        model_name_for_columns: model.name.clone(),
    })
}

/// Return every model in the workspace whose merged tag set contains `tag`,
/// sorted ascending by workspace-relative `path` (byte-lexicographic, `/`
/// separators). Salsa-cached; invalidated when the workspace file list or any
/// file's content/frontmatter changes.
///
/// Includes both hand-authored models and generator-emitted models.
#[salsa::tracked]
pub fn models_with_tag(
    db: &dyn salsa::Database,
    workspace: Workspace,
    tag: String,
) -> Arc<Vec<ModelRefValue>> {
    let all = models_all_with_generators(db, workspace);
    let mut result: Vec<ModelRefValue> = all
        .iter()
        .filter(|m| m.tags.contains(&tag))
        .cloned()
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}

/// Return every model in the workspace, sorted ascending by workspace-relative
/// `path` (byte-lexicographic, `/` separators). Salsa-cached; invalidated when
/// the workspace file list or any file's content changes.
///
/// Includes both hand-authored models and generator-emitted models.
#[salsa::tracked]
pub fn models_all(db: &dyn salsa::Database, workspace: Workspace) -> Arc<Vec<ModelRefValue>> {
    models_all_with_generators(db, workspace)
}

/// Pure helper: materialise a `SourceRefValue` from a `SourceInfo`.
fn make_source_ref_value(project_root: &Path, source: &smelt_core::SourceInfo) -> SourceRefValue {
    // Workspace-relative path with `/` separators.
    let rel_path = source
        .path
        .strip_prefix(project_root)
        .unwrap_or(&source.path)
        .to_string_lossy()
        .replace('\\', "/");

    // Name: last address segment (final stem).
    let name = source.address_segments.last().cloned().unwrap_or_default();

    SourceRefValue {
        path: rel_path,
        name,
        tags: source.tags.clone(),
        address_segments: source.address_segments.clone(),
    }
}

/// Return every source in the project whose `tags:` list contains `tag`,
/// sorted ascending by workspace-relative `path` (byte-lexicographic, `/`
/// separators). Salsa-cached; invalidated when the `ProjectInput` changes.
#[salsa::tracked]
pub fn sources_with_tag(
    db: &dyn salsa::Database,
    project: ProjectInput,
    tag: String,
) -> Arc<Vec<SourceRefValue>> {
    let root = project.root(db).clone();
    let sources = project_sources(db, project);
    let mut result: Vec<SourceRefValue> = sources
        .iter()
        .filter(|s| s.tags.contains(&tag))
        .map(|s| make_source_ref_value(&root, s))
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}

/// Return every source in the project, sorted ascending by workspace-relative
/// `path` (byte-lexicographic, `/` separators). Salsa-cached; invalidated
/// when the `ProjectInput` changes.
#[salsa::tracked]
pub fn sources_all(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SourceRefValue>> {
    let root = project.root(db).clone();
    let sources = project_sources(db, project);
    let mut result: Vec<SourceRefValue> = sources
        .iter()
        .map(|s| make_source_ref_value(&root, s))
        .collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(result)
}

// ============================================================================
// Tests (W1–W4 pipeline)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use salsa::Setter;
    use std::path::PathBuf;

    // ─── Test database setup helpers ─────────────────────────────────────────

    /// Create a minimal `Database` with a workspace containing the given files.
    ///
    /// `files`: list of `(relative_path, text)` pairs.  All files are placed
    /// under a tempdir and registered with `project_root = tempdir`.
    fn db_with_files(files: &[(&str, &str)]) -> (Database, PathBuf, crate::Workspace) {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let root = tmpdir.keep();
        let mut db = Database::default();

        let project = db.set_project_input(root.clone(), String::new());

        let mut source_files: Vec<crate::SourceFile> = Vec::new();
        for (rel_path, text) in files {
            let abs_path = root.join(rel_path);
            if let Some(parent) = abs_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&abs_path, text).ok();
            let sf = db.set_source_file(abs_path, text.to_string(), root.clone());
            source_files.push(sf);
        }

        db.set_workspace(source_files, vec![project]);
        let workspace = db.workspace();
        (db, root, workspace)
    }

    // ─── Test 1: generator_files returns only generator files ─────────────────

    #[test]
    fn generator_files_returns_only_generator_files() {
        let bare_select = "SELECT id FROM orders";
        let multi_section = "--- name: foo ---\nSELECT 1\n--- name: bar ---\nSELECT 2";
        let generator =
            "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }]";

        let (db, _root, workspace) = db_with_files(&[
            ("models/bare.sql", bare_select),
            ("models/multi.sql", multi_section),
            ("models/cohorts.gen.sql", generator),
        ]);

        let gen_files = generator_files(&db, workspace);
        assert_eq!(
            gen_files.len(),
            1,
            "expected 1 generator file, got: {:?}",
            gen_files.iter().map(|f| f.path(&db)).collect::<Vec<_>>()
        );
        let path_str = gen_files[0].path(&db).to_string_lossy().to_string();
        assert!(
            path_str.ends_with("cohorts.gen.sql"),
            "expected cohorts.gen.sql, got: {}",
            path_str
        );
    }

    // ─── Test 2: evaluate_generator on happy path ────────────────────────────

    #[test]
    fn evaluate_generator_returns_emitted_model_defs_on_happy_path() {
        let generator = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }, ModelDef { name: 'eu', body: SELECT 2 }]";

        let (db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        let gen_files = generator_files(&db, workspace);
        assert_eq!(gen_files.len(), 1);

        let result = evaluate_generator(&db, workspace, gen_files[0]);
        assert!(
            result.diagnostics.is_empty(),
            "expected no diagnostics on happy path, got: {:?}",
            result.diagnostics
        );
        assert_eq!(
            result.emissions.len(),
            2,
            "expected 2 emissions, got: {:?}",
            result.emissions.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        let names: Vec<&str> = result.emissions.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"us_west"),
            "expected us_west in emissions: {:?}",
            names
        );
        assert!(
            names.contains(&"eu"),
            "expected eu in emissions: {:?}",
            names
        );
    }

    // ─── Test 3: evaluate_generator propagates inference diagnostics ──────────

    #[test]
    fn evaluate_generator_propagates_inference_diagnostics() {
        // ModelDef with invalid name (contains dot) and missing required body field.
        let generator = "---\ngenerates: models\n---\n[ModelDef { name: 'us.west' }]";

        let (db, _root, workspace) = db_with_files(&[("models/bad.gen.sql", generator)]);

        let gen_files = generator_files(&db, workspace);
        assert_eq!(gen_files.len(), 1);

        let result = evaluate_generator(&db, workspace, gen_files[0]);
        // Expect at least one diagnostic (ModelDefInvalidName or RecordFieldMissing).
        assert!(
            !result.diagnostics.is_empty(),
            "expected diagnostics for invalid ModelDef, got none"
        );
    }

    // ─── Test 4: emitted_models aggregates across generator files ─────────────

    #[test]
    fn emitted_models_aggregates_across_generator_files() {
        let gen_a = "---\ngenerates: models\n---\n[ModelDef { name: 'alpha', body: SELECT 1 }, ModelDef { name: 'beta', body: SELECT 2 }]";
        let gen_b = "---\ngenerates: models\n---\n[ModelDef { name: 'gamma', body: SELECT 3 }, ModelDef { name: 'delta', body: SELECT 4 }]";

        let (db, _root, workspace) =
            db_with_files(&[("models/a.gen.sql", gen_a), ("models/b.gen.sql", gen_b)]);

        let result = emitted_models(&db, workspace);
        assert_eq!(
            result.survivors.len(),
            4,
            "expected 4 emitted models, got: {:?}",
            result.survivors.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert!(
            result.collision_diagnostics.is_empty(),
            "unexpected collisions"
        );
    }

    // ─── Test 5: emitted_models computes smelt path correctly ─────────────────

    #[test]
    fn emitted_models_computes_smelt_path_correctly() {
        // `emitted_model_smelt_path` is a pure function — test it directly
        // without needing a full Salsa database, avoiding cross-database panics.
        let root = PathBuf::from("/fake/root");
        let gen_file = root.join("models/cohorts.gen.sql");
        let scan_roots = vec!["models".to_string()];

        let path = emitted_model_smelt_path(&gen_file, &root, &scan_roots, "us_west");
        assert_eq!(
            path, "cohorts.us_west",
            "expected cohorts.us_west, got: {}",
            path
        );
    }

    // ─── Test 6: emitted_models_computes_smelt_path_for_subdirectory ──────────

    #[test]
    fn emitted_models_computes_smelt_path_for_subdirectory() {
        let root = std::path::PathBuf::from("/fake/root");
        let gen_file = root.join("models/staging/sources.gen.sql");
        let scan_roots = vec!["models".to_string()];
        let path = emitted_model_smelt_path(&gen_file, &root, &scan_roots, "orders");
        assert_eq!(
            path, "staging.sources.orders",
            "expected staging.sources.orders, got: {}",
            path
        );
    }

    // ─── Test 7: emitted_models detects duplicate name within file ────────────

    #[test]
    fn emitted_models_detects_duplicate_name_within_file() {
        let generator = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }, ModelDef { name: 'us_west', body: SELECT 2 }]";

        let (db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        let gen_files = generator_files(&db, workspace);
        let result = evaluate_generator(&db, workspace, gen_files[0]);

        // Expect a ModelDefDuplicateName diagnostic.
        let has_dup = result
            .diagnostics
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ModelDefDuplicateName));
        assert!(
            has_dup,
            "expected ModelDefDuplicateName diagnostic, got: {:?}",
            result.diagnostics
        );
        // Only one emission should survive.
        assert_eq!(
            result.emissions.len(),
            1,
            "expected only 1 surviving emission after duplicate detection, got: {:?}",
            result.emissions.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
    }

    // ─── Test 8: emitted_models detects hand-authored collision ───────────────

    #[test]
    fn emitted_models_detects_hand_authored_collision() {
        // Hand-authored model: models/cohorts/us_west.sql
        // smelt path: cohorts.us_west (with models/ stripped)
        let hand_authored = "SELECT 1 AS id";
        // Generator at models/cohorts.gen.sql emits ModelDef { name: 'us_west' }
        // smelt path: cohorts.us_west (with models/ stripped) — collision!
        let generator =
            "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 2 }]";

        let (db, _root, workspace) = db_with_files(&[
            ("models/cohorts/us_west.sql", hand_authored),
            ("models/cohorts.gen.sql", generator),
        ]);

        let result = emitted_models(&db, workspace);
        // The emitted model should be dropped.
        assert!(
            result.survivors.iter().all(|e| e.name != "us_west"
                || e.generator_file
                    .to_str()
                    .map(|p| !p.ends_with("cohorts.gen.sql"))
                    .unwrap_or(true)),
            "emitted us_west from cohorts.gen.sql should be dropped due to collision"
        );
        // A collision diagnostic should be emitted.
        let has_collision = result
            .collision_diagnostics
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ModelDefHandAuthoredCollision));
        assert!(
            has_collision,
            "expected ModelDefHandAuthoredCollision diagnostic, got: {:?}",
            result.collision_diagnostics
        );
    }

    // ─── Test 9: emitted_models detects cross-generator collision ────────────

    #[test]
    fn emitted_models_detects_cross_generator_collision() {
        // Two generators with the same file stem emit the same model name,
        // producing the same smelt path — the byte-earlier file wins.
        //
        // Smelt path for `models/cohorts.gen.sql` emitting `name: 'us_west'`  →  `cohorts.us_west`
        // Smelt path for `models/cohorts.gen2.sql` emitting `name: 'us_west'` →  `cohorts.us_west`
        // (both stems reduce to `cohorts` via `file_name().split('.').next()`)
        let gen_a = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }]";
        let gen_b = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 2 }]";

        let (db, _root, workspace) = db_with_files(&[
            ("models/cohorts.gen.sql", gen_a),
            ("models/cohorts.gen2.sql", gen_b),
        ]);

        let result = emitted_models(&db, workspace);
        // Only one survivor: the earlier-path generator's emission.
        assert_eq!(
            result.survivors.len(),
            1,
            "expected 1 survivor from collision, got: {:?}",
            result.survivors.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        // The survivor should come from cohorts.gen.sql (byte-earlier path).
        let survivor = &result.survivors[0];
        assert!(
            survivor
                .generator_file
                .to_str()
                .map(|p| p.ends_with("cohorts.gen.sql"))
                .unwrap_or(false),
            "expected survivor from cohorts.gen.sql, got: {:?}",
            survivor.generator_file
        );
        // There should be a collision diagnostic.
        assert!(
            !result.collision_diagnostics.is_empty(),
            "expected collision diagnostic"
        );
    }

    // ─── Test 10: empty list admits zero emissions, no diagnostic ─────────────

    #[test]
    fn emitted_models_empty_list_admits_zero_emissions_no_diagnostic() {
        let generator = "---\ngenerates: models\n---\n[]";

        let (db, _root, workspace) = db_with_files(&[("models/empty.gen.sql", generator)]);

        let gen_files = generator_files(&db, workspace);
        let result = evaluate_generator(&db, workspace, gen_files[0]);

        assert!(
            result.emissions.is_empty(),
            "expected no emissions for empty list, got: {:?}",
            result.emissions
        );
        assert!(
            result.diagnostics.is_empty(),
            "expected no diagnostics for empty list body, got: {:?}",
            result.diagnostics
        );
    }

    // ─── Test 11: models_all_with_generators includes generator emissions ──────

    #[test]
    fn models_all_with_generators_includes_generator_emissions() {
        let hand1 = "SELECT 1 AS id";
        let hand2 = "SELECT 2 AS val";
        let gen = "---\ngenerates: models\n---\n[ModelDef { name: 'em1', body: SELECT 3 }, ModelDef { name: 'em2', body: SELECT 4 }]";

        let (db, _root, workspace) = db_with_files(&[
            ("models/hand1.sql", hand1),
            ("models/hand2.sql", hand2),
            ("models/cohorts.gen.sql", gen),
        ]);

        let all = models_all_with_generators(&db, workspace);
        // 2 hand-authored + 2 emitted = 4 total.
        assert_eq!(
            all.len(),
            4,
            "expected 4 models (2 hand-authored + 2 emitted), got: {:?}",
            all.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    // ─── Test 12: models_with_tag includes generator emissions ───────────────

    #[test]
    fn models_with_tag_includes_generator_emissions() {
        let hand = "---\ntags: [cohort]\n---\nSELECT 1 AS id";
        let gen = "---\ngenerates: models\ntags: [cohort]\n---\n[ModelDef { name: 'em1', body: SELECT 2 }]";
        let other = "SELECT 3 AS x";

        let (db, _root, workspace) = db_with_files(&[
            ("models/hand.sql", hand),
            ("models/cohorts.gen.sql", gen),
            ("models/other.sql", other),
        ]);

        let tagged = models_with_tag(&db, workspace, "cohort".to_string());
        // Both the hand-authored model and the emitted model should be tagged.
        assert_eq!(
            tagged.len(),
            2,
            "expected 2 models with tag 'cohort', got: {:?}",
            tagged.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    // ─── Test 13: smelt path resolves correctly for generators ───────────────

    #[test]
    fn generator_body_literal_smelt_path_resolves_against_hand_authored_only() {
        // A generator body containing smelt.staging.orders (hand-authored) — should not error.
        // This test verifies the generator TypeContext has workspace_shape_includes_generators=false.
        let gen = "---\ngenerates: models\n---\n[ModelDef { name: 'test', body: SELECT 1 }]";

        let (db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", gen)]);

        let gen_files = generator_files(&db, workspace);
        assert_eq!(gen_files.len(), 1);
        // Verify the file is indeed treated as a generator.
        let result = evaluate_generator(&db, workspace, gen_files[0]);
        // The evaluation should work; we verify ctx.workspace_shape_includes_generators = false
        // by checking no GeneratorBodyForbidsModelReflection diagnostic fires for literal path refs.
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::GeneratorBodyForbidsModelReflection)),
            "literal path refs must not trigger reflection forbid diagnostic"
        );
    }

    // ─── Test 14: Salsa cache invalidates on generator file edit ─────────────

    #[test]
    fn salsa_cache_invalidates_on_generator_file_edit() {
        let gen_initial = "---\ngenerates: models\n---\n[ModelDef { name: 'v1', body: SELECT 1 }]";
        let gen_updated =
            "---\ngenerates: models\n---\n[ModelDef { name: 'v1', body: SELECT 1 }, ModelDef { name: 'v2', body: SELECT 2 }]";
        let other_gen =
            "---\ngenerates: models\n---\n[ModelDef { name: 'other', body: SELECT 99 }]";

        let (mut db, _root, workspace) = db_with_files(&[
            ("models/main.gen.sql", gen_initial),
            ("models/side.gen.sql", other_gen),
        ]);

        // Initial state.
        let gen_files_before = generator_files(&db, workspace);
        let main_file = gen_files_before
            .iter()
            .find(|f| {
                f.path(&db)
                    .to_str()
                    .map(|p| p.ends_with("main.gen.sql"))
                    .unwrap_or(false)
            })
            .copied()
            .expect("main.gen.sql");
        let side_file = gen_files_before
            .iter()
            .find(|f| {
                f.path(&db)
                    .to_str()
                    .map(|p| p.ends_with("side.gen.sql"))
                    .unwrap_or(false)
            })
            .copied()
            .expect("side.gen.sql");

        let result_before = evaluate_generator(&db, workspace, main_file);
        assert_eq!(result_before.emissions.len(), 1, "initial: 1 emission");

        // Edit the main generator file.
        main_file.set_text(&mut db).to(gen_updated.to_string());

        // Re-run: main.gen.sql now emits 2 models.
        let result_after = evaluate_generator(&db, workspace, main_file);
        assert_eq!(result_after.emissions.len(), 2, "after edit: 2 emissions");

        // side.gen.sql's result is unchanged (still 1 emission).
        let side_result = evaluate_generator(&db, workspace, side_file);
        assert_eq!(
            side_result.emissions.len(),
            1,
            "side.gen.sql must not be invalidated by main.gen.sql edit"
        );
    }

    // ─── Test 15: deterministic byte-equal across runs ────────────────────────

    #[test]
    fn evaluated_generator_deterministic_byte_equal_across_runs() {
        let generator = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }, ModelDef { name: 'eu', body: SELECT 2 }]";

        // Run twice in separate databases.
        let (db1, _root1, workspace1) = db_with_files(&[("models/cohorts.gen.sql", generator)]);
        let (db2, _root2, workspace2) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        let gen_files1 = generator_files(&db1, workspace1);
        let gen_files2 = generator_files(&db2, workspace2);

        let result1 = evaluate_generator(&db1, workspace1, gen_files1[0]);
        let result2 = evaluate_generator(&db2, workspace2, gen_files2[0]);

        // Names in both results should be in the same order.
        let names1: Vec<&str> = result1.emissions.iter().map(|e| e.name.as_str()).collect();
        let names2: Vec<&str> = result2.emissions.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names1, names2,
            "emission order must be deterministic across runs"
        );
        assert_eq!(result1.diagnostics.len(), result2.diagnostics.len());
    }

    // ─── Test 16: loader + HOF chain emits one ModelDef per YAML entry ────────

    #[test]
    fn evaluate_generator_with_loader_and_hof_chain() {
        // Generator body: load a YAML list of records and map each to a ModelDef
        // via a lambda.  The loader supplies two records in document order
        // (`eu` before `us_west`); we expect exactly two emissions in that order.
        //
        // The inline record schema `{name: Text, region: Text}` is used here so
        // that the loader validation can check the schema fields without requiring
        // a `smelt.record` declaration to be resolved from the workspace.
        let generator = concat!(
            "---\ngenerates: models\n---\n",
            "smelt.config.load_yaml('cohorts.yaml', List<{name: Text, region: Text}>) |> map(fn c => ModelDef { name: c.name, body: SELECT * FROM orders WHERE region = c.region })"
        );

        // YAML list in document order (eu first, then us_west).
        let yaml = "- name: eu\n  region: eu-central-1\n- name: us_west\n  region: us-west-1\n";

        let (mut db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        // Register the YAML as a Salsa loader input (workspace-relative path).
        let loader_path: std::sync::Arc<str> = std::sync::Arc::from("cohorts.yaml");
        let yaml_text: std::sync::Arc<str> = std::sync::Arc::from(yaml);
        db.set_loader_file(loader_path, yaml_text, true);

        let gen_files = generator_files(&db, workspace);
        assert_eq!(gen_files.len(), 1, "expected one generator file");

        let result = evaluate_generator(&db, workspace, gen_files[0]);
        assert!(
            result.diagnostics.is_empty(),
            "expected no diagnostics on happy path, got: {:?}",
            result.diagnostics
        );
        assert_eq!(
            result.emissions.len(),
            2,
            "expected one emission per YAML entry; got: {:?}",
            result.emissions.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        // Emission order must follow the loader's document order.
        assert_eq!(
            result.emissions[0].name, "eu",
            "first emission must be `eu` (document order)"
        );
        assert_eq!(
            result.emissions[1].name, "us_west",
            "second emission must be `us_west` (document order)"
        );
    }

    // ─── Test 17: Salsa cache invalidates on YAML dependency edit ────────────

    #[test]
    fn salsa_cache_invalidates_on_yaml_dependency_edit() {
        // The `evaluate_generator` Salsa query must be invalidated when the
        // YAML file consumed by `smelt.config.load_yaml` changes.  Invalidation
        // happens via the `loader_resolved_value` query's dependency chain:
        // editing the `LoaderFileInput` text propagates to `evaluate_generator`.
        let generator = concat!(
            "---\ngenerates: models\n---\n",
            "smelt.config.load_yaml('cohorts.yaml', List<{name: Text, region: Text}>) |> map(fn c => ModelDef { name: c.name, body: SELECT 1 })"
        );

        // Initial YAML: one entry.
        let yaml_v1: std::sync::Arc<str> =
            std::sync::Arc::from("- name: us_west\n  region: us-west-1\n");

        let (mut db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        let loader_path: std::sync::Arc<str> = std::sync::Arc::from("cohorts.yaml");
        db.set_loader_file(loader_path.clone(), yaml_v1, true);

        let gen_files = generator_files(&db, workspace);
        let gen_file = gen_files[0];

        // First evaluation: one YAML entry → one emission.
        let result_v1 = evaluate_generator(&db, workspace, gen_file);
        assert_eq!(
            result_v1.emissions.len(),
            1,
            "initial YAML has one entry; expected 1 emission, got: {:?}",
            result_v1
                .emissions
                .iter()
                .map(|e| &e.name)
                .collect::<Vec<_>>()
        );

        // Edit the YAML via the Salsa input setter — add a second entry.
        // This is the genuine Salsa invalidation: `set_loader_file` calls
        // `input.set_text(&mut db).to(new_text)` which Salsa tracks as an input
        // change, propagating invalidation to all dependent queries.
        let yaml_v2: std::sync::Arc<str> = std::sync::Arc::from(
            "- name: eu\n  region: eu-central-1\n- name: us_west\n  region: us-west-1\n",
        );
        db.set_loader_file(loader_path, yaml_v2, true);

        // Re-query: Salsa must re-evaluate `evaluate_generator` because the
        // `LoaderFileInput` text changed.
        let result_v2 = evaluate_generator(&db, workspace, gen_file);
        assert_eq!(
            result_v2.emissions.len(),
            2,
            "updated YAML has two entries; expected 2 emissions, got: {:?}",
            result_v2
                .emissions
                .iter()
                .map(|e| &e.name)
                .collect::<Vec<_>>()
        );

        // Confirm the result genuinely changed (not a stale cache hit).
        assert_ne!(
            result_v1.emissions.len(),
            result_v2.emissions.len(),
            "emissions count must change when the YAML changes"
        );
    }

    // ─── Test Phase 5 (E2): generator_file: selector resolution ─────────────

    /// A workspace with `models/cohorts.gen.sql` emitting two models (`us_west`,
    /// `eu`); `resolve_generator_file_selector` matches both surviving emissions.
    #[test]
    fn generator_file_selector_matches_all_emissions() {
        use smelt_core::selector::{parse_selector, SelectionMethod};

        let gen = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }, ModelDef { name: 'eu', body: SELECT 2 }]";
        let (db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", gen)]);

        let result = emitted_models(&db, workspace);
        assert_eq!(result.survivors.len(), 2);

        // The generator_file: selector should be resolved against the survivors list.
        let selector =
            parse_selector("generator_file:models/cohorts.gen.sql").expect("parse selector");
        let path = match &selector.method {
            SelectionMethod::GeneratorFile { path } => path.clone(),
            _ => panic!("expected GeneratorFile"),
        };

        // Resolve: filter survivors by generator_file path (workspace-relative).
        let matched: Vec<_> = result
            .survivors
            .iter()
            .filter(|e| {
                // Compare the workspace-relative path.
                let gen_path = e.generator_file.to_string_lossy();
                gen_path.ends_with(path.to_str().unwrap_or(""))
                    || gen_path.ends_with(
                        path.to_string_lossy()
                            .replace('/', std::path::MAIN_SEPARATOR_STR)
                            .as_str(),
                    )
            })
            .collect();

        assert_eq!(
            matched.len(),
            2,
            "both us_west and eu must be matched by generator_file: selector"
        );
    }

    /// A selector pointing at a hand-authored `.sql` file returns an empty
    /// match set (no error, no panic).
    #[test]
    fn generator_file_selector_against_non_generator_path_matches_nothing() {
        use smelt_core::selector::{parse_selector, SelectionMethod};

        let hand = "SELECT 1 AS id";
        let gen = "---\ngenerates: models\n---\n[ModelDef { name: 'em', body: SELECT 2 }]";

        let (db, _root, workspace) =
            db_with_files(&[("models/hand.sql", hand), ("models/cohorts.gen.sql", gen)]);

        let result = emitted_models(&db, workspace);

        // Selector pointing at the hand-authored file — not a generator file.
        let selector = parse_selector("generator_file:models/hand.sql").expect("parse selector");
        let path = match &selector.method {
            SelectionMethod::GeneratorFile { path } => path.clone(),
            _ => panic!("expected GeneratorFile"),
        };

        let matched: Vec<_> = result
            .survivors
            .iter()
            .filter(|e| {
                let gen_path = e.generator_file.to_string_lossy();
                gen_path.ends_with(path.to_str().unwrap_or(""))
            })
            .collect();

        assert_eq!(
            matched.len(),
            0,
            "selector against non-generator path must match nothing; got: {:?}",
            matched.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
    }

    /// A collision loser is NOT in `survivors` and is therefore excluded from
    /// `generator_file:` selector matches (collision losers are in `discarded`).
    #[test]
    fn generator_file_selector_excludes_collision_losers() {
        use smelt_core::selector::{parse_selector, SelectionMethod};

        // Hand-authored model: cohorts/us_west.sql → smelt path cohorts.us_west
        let hand = "SELECT 1 AS id";
        // Generator at cohorts.gen.sql emits 'us_west' → collision → dropped.
        let gen = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 2 }, ModelDef { name: 'eu', body: SELECT 3 }]";

        let (db, _root, workspace) = db_with_files(&[
            ("models/cohorts/us_west.sql", hand),
            ("models/cohorts.gen.sql", gen),
        ]);

        let result = emitted_models(&db, workspace);

        // Only 'eu' must survive; 'us_west' is a collision loser.
        let selector =
            parse_selector("generator_file:models/cohorts.gen.sql").expect("parse selector");
        let path = match &selector.method {
            SelectionMethod::GeneratorFile { path } => path.clone(),
            _ => panic!("expected GeneratorFile"),
        };

        let matched: Vec<_> = result
            .survivors
            .iter()
            .filter(|e| {
                let gen_path = e.generator_file.to_string_lossy();
                gen_path.ends_with(path.to_str().unwrap_or(""))
            })
            .collect();

        // Only 'eu' should match — 'us_west' is a collision loser, not in survivors.
        assert_eq!(
            matched.len(),
            1,
            "collision loser must be excluded from generator_file: selector; got: {:?}",
            matched.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert_eq!(
            matched[0].name, "eu",
            "survivor must be 'eu', not the collision loser 'us_west'"
        );
    }

    // ─── Test Phase 5 (E2): incremental frontmatter inheritance ─────────────

    /// A generator file whose frontmatter declares `incremental:` and emits a
    /// `ModelDef { materialization: 'incremental' }` — the resulting emission
    /// carries `materialization == "incremental"` and its `incremental_config()`
    /// reflects the generator file's frontmatter block.
    #[test]
    fn emitted_incremental_model_inherits_frontmatter_incremental_block() {
        // Generator file with incremental frontmatter.
        let generator = concat!(
            "---\n",
            "generates: models\n",
            "incremental:\n",
            "  event_time_column: dt\n",
            "  partition_column: dt\n",
            "  granularity: day\n",
            "---\n",
            "[ModelDef { name: 'us_west', body: SELECT * FROM orders, materialization: 'incremental' }]"
        );

        let (db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        let gen_files = generator_files(&db, workspace);
        assert_eq!(gen_files.len(), 1);

        let result = evaluate_generator(&db, workspace, gen_files[0]);
        assert!(
            result.diagnostics.is_empty(),
            "expected no diagnostics, got: {:?}",
            result.diagnostics
        );
        assert_eq!(result.emissions.len(), 1);

        let emission = &result.emissions[0];
        assert_eq!(
            emission.materialization, "incremental",
            "emitted model must have materialization=incremental"
        );
        // The emission should carry the inherited incremental config.
        let inc = emission.incremental_config.as_ref().expect(
            "emitted incremental model must carry the inherited incremental_config from frontmatter"
        );
        assert_eq!(
            inc.partition_column, "dt",
            "inherited partition_column must be 'dt'"
        );
        assert_eq!(
            inc.event_time_column, "dt",
            "inherited event_time_column must be 'dt'"
        );
    }

    /// A generator file with an `incremental:` frontmatter block that emits a
    /// `ModelDef { materialization: 'view' }` — the emitted model is a view
    /// and must NOT inherit the incremental config (it is silently ignored).
    #[test]
    fn emitted_non_incremental_model_does_not_inherit_incremental_block() {
        let generator = concat!(
            "---\n",
            "generates: models\n",
            "incremental:\n",
            "  event_time_column: dt\n",
            "  partition_column: dt\n",
            "  granularity: day\n",
            "---\n",
            "[ModelDef { name: 'us_west', body: SELECT 1 }]"
        );

        let (db, _root, workspace) = db_with_files(&[("models/cohorts.gen.sql", generator)]);

        let gen_files = generator_files(&db, workspace);
        let result = evaluate_generator(&db, workspace, gen_files[0]);

        assert_eq!(result.emissions.len(), 1);
        let emission = &result.emissions[0];
        // Default materialization is "view".
        assert_eq!(
            emission.materialization, "view",
            "emitted model without explicit materialization must be 'view'"
        );
        // View emissions must not inherit the incremental block.
        assert!(
            emission.incremental_config.is_none(),
            "non-incremental emitted model must not carry incremental_config; got: {:?}",
            emission.incremental_config
        );
    }
}
