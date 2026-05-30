//! Phase 5 (meta-language E1) — schema-typed config loader parsers and validator.
//!
//! This module provides:
//! - [`parse_yaml`]: parse YAML text into a [`ParsedConfigFile`], retaining per-row spans.
//! - [`parse_json`]: parse JSON text into a [`ParsedConfigFile`], retaining byte-offset spans.
//! - [`validate_against_schema`]: pure schema-validation function that converts a
//!   [`ParsedConfigFile`] to a typed [`MetaValue`] plus a list of diagnostics.
//! - [`is_admissible_loader_schema`]: check whether a `SmeltType` is an admissible
//!   loader schema (record, `List<record>`, `Map<Text, record>`).
//!
//! **Pure-function rule (CLAUDE.md):** this module contains no Salsa imports. All Salsa
//! wrappers live in `lib.rs`. The functions here take `&str` / `&SmeltType` and return
//! plain Rust values.

use std::collections::BTreeMap;

use smelt_types::signatures::SmeltType;
use smelt_types::DataType;
use smelt_types::TypeConstraint;

// ============================================================================
// Public types
// ============================================================================

/// A file format recognised by the loader family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Yaml,
    Json,
}

impl ConfigFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigFormat::Yaml => "YAML",
            ConfigFormat::Json => "JSON",
        }
    }
}

/// A source span inside a loaded file: 0-indexed line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpan {
    pub line: usize,
    pub column: usize,
}

impl FileSpan {
    pub fn zero() -> Self {
        FileSpan { line: 0, column: 0 }
    }
}

impl std::fmt::Display for FileSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 1-indexed for human-readable messages.
        write!(f, "line {}", self.line + 1)
    }
}

/// A parsed YAML/JSON value node with its source span.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedNode {
    /// A null scalar.
    Null { span: FileSpan },
    /// A boolean scalar.
    Bool { value: bool, span: FileSpan },
    /// An integer scalar.
    Integer { value: i64, span: FileSpan },
    /// A floating-point scalar.
    Float { value: f64, span: FileSpan },
    /// A string scalar.
    String { value: String, span: FileSpan },
    /// A sequence of nodes.
    Sequence {
        items: Vec<ParsedNode>,
        span: FileSpan,
    },
    /// A mapping of (key_str, key_span, value_node) entries, in insertion order.
    Mapping {
        entries: Vec<(String, FileSpan, ParsedNode)>,
        span: FileSpan,
    },
}

impl ParsedNode {
    /// The source span of this node's opening position.
    pub fn span(&self) -> FileSpan {
        match self {
            ParsedNode::Null { span }
            | ParsedNode::Bool { span, .. }
            | ParsedNode::Integer { span, .. }
            | ParsedNode::Float { span, .. }
            | ParsedNode::String { span, .. }
            | ParsedNode::Sequence { span, .. }
            | ParsedNode::Mapping { span, .. } => *span,
        }
    }

    /// Human-readable shape name for diagnostics.
    pub fn shape_name(&self) -> &'static str {
        match self {
            ParsedNode::Null { .. } => "null",
            ParsedNode::Bool { .. } => "boolean",
            ParsedNode::Integer { .. } => "integer",
            ParsedNode::Float { .. } => "float",
            ParsedNode::String { .. } => "string",
            ParsedNode::Sequence { .. } => "sequence",
            ParsedNode::Mapping { .. } => "mapping",
        }
    }
}

/// A parse error from the YAML/JSON parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigParseError {
    pub message: String,
    pub span: FileSpan,
}

/// A format-agnostic intermediate carrying per-row spans. Produced by
/// [`parse_yaml`] / [`parse_json`].
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedConfigFile {
    pub format: ConfigFormat,
    pub root: ParsedNode,
    /// Workspace-relative path of the file (for error messages).
    pub path: String,
}

// ============================================================================
// A meta-world value produced by validation
// ============================================================================

/// A typed meta-world value produced by [`validate_against_schema`].
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    /// A `Text` scalar.
    Text(String),
    /// An `Integer` scalar.
    Integer(i64),
    /// A `Float` scalar.
    Float(f64),
    /// A `Boolean` scalar.
    Boolean(bool),
    /// A `Record` value: ordered fields.
    Record(BTreeMap<String, MetaValue>),
    /// A `List` value: ordered elements.
    List(Vec<MetaValue>),
    /// A `Map` value: ordered key→value pairs.
    Map(BTreeMap<String, MetaValue>),
    /// Used when validation produced errors for this node.
    Error,
}

// ============================================================================
// Validation diagnostic
// ============================================================================

/// Severity of a loader validation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderDiagnosticSeverity {
    Error,
    Warning,
}

/// A diagnostic produced by [`validate_against_schema`].
#[derive(Debug, Clone, PartialEq)]
pub struct LoaderDiagnostic {
    /// The diagnostic code.
    pub code: crate::DiagnosticCode,
    /// Severity (most are Error; `ConfigLoaderNullCoercion` is Warning).
    pub severity: LoaderDiagnosticSeverity,
    /// Pre-rendered message string.
    pub message: String,
    /// Primary span inside the loaded file (row of the offending node).
    pub primary_span: FileSpan,
    /// Optional secondary span — the loader call site in the smelt model file.
    /// Callers should fill this in from the call-site span before emitting the
    /// diagnostic to the LSP accumulator.
    pub secondary_call_span: Option<rowan::TextRange>,
}

/// The result of a validation run.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// The typed meta-world value (may be `MetaValue::Error` for error nodes).
    pub value: MetaValue,
    /// Diagnostics produced during validation.
    pub diagnostics: Vec<LoaderDiagnostic>,
}

// ============================================================================
// YAML parser (marked-yaml for span retention)
// ============================================================================

/// Parse YAML text into a [`ParsedConfigFile`].
///
/// Uses `marked-yaml` for line/column span retention through validation.
/// Tries mapping-at-root first; if that fails due to a sequence top-level,
/// retries with `toplevel_sequence()`. This allows both `{key: val}` and
/// `- item` top-level YAML files.
///
/// Pure function — no Salsa dependency, no I/O.
pub fn parse_yaml(text: &str, path: &str) -> Result<ParsedConfigFile, ConfigParseError> {
    // First attempt: mapping at top level (the common case for record/map schemas).
    let mapping_result =
        marked_yaml::parse_yaml_with_options(0, text, marked_yaml::LoaderOptions::default());

    let node = match mapping_result {
        Ok(node) => node,
        Err(e) => {
            let msg = e.to_string();
            // If the error is "Top level must be a mapping", retry as sequence.
            if msg.contains("Top level must be a mapping") {
                let seq_result = marked_yaml::parse_yaml_with_options(
                    0,
                    text,
                    marked_yaml::LoaderOptions::default().toplevel_sequence(),
                );
                match seq_result {
                    Ok(node) => node,
                    Err(e2) => {
                        let msg2 = e2.to_string();
                        let span = extract_span_from_marked_error(&msg2);
                        return Err(ConfigParseError {
                            message: msg2,
                            span,
                        });
                    }
                }
            } else {
                let span = extract_span_from_marked_error(&msg);
                return Err(ConfigParseError { message: msg, span });
            }
        }
    };

    let root = convert_marked_node(&node);
    Ok(ParsedConfigFile {
        format: ConfigFormat::Yaml,
        root,
        path: path.to_string(),
    })
}

fn extract_span_from_marked_error(msg: &str) -> FileSpan {
    // marked-yaml error messages often contain "line X" or "X:Y" patterns.
    // Best-effort: scan for the first integer that looks like a line number.
    // If nothing found, default to line 0.
    if let Some(pos) = msg.find("line ") {
        let rest = &msg[pos + 5..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if let Ok(n) = rest[..end].parse::<usize>() {
            return FileSpan {
                line: n.saturating_sub(1),
                column: 0,
            };
        }
    }
    FileSpan::zero()
}

fn convert_marked_node(node: &marked_yaml::Node) -> ParsedNode {
    use marked_yaml::Node;
    match node {
        Node::Scalar(scalar) => convert_scalar(scalar),
        Node::Mapping(mapping) => {
            let span = marker_opt_to_file_span(mapping.span().start());
            let mut entries = Vec::new();
            for (key, value) in mapping.iter() {
                let key_span = marker_opt_to_file_span(key.span().start());
                let key_str = key.as_str().to_string();
                let val_node = convert_marked_node(value);
                entries.push((key_str, key_span, val_node));
            }
            ParsedNode::Mapping { entries, span }
        }
        Node::Sequence(sequence) => {
            let span = marker_opt_to_file_span(sequence.span().start());
            let items: Vec<ParsedNode> = sequence.iter().map(convert_marked_node).collect();
            ParsedNode::Sequence { items, span }
        }
    }
}

fn convert_scalar(scalar: &marked_yaml::types::MarkedScalarNode) -> ParsedNode {
    let span = marker_opt_to_file_span(scalar.span().start());
    let s = scalar.as_str();

    // YAML 1.2 core schema: null, bool, int, float, string.
    // Check in order: null → bool → int → float → string.
    if s.is_empty() || s == "null" || s == "~" {
        return ParsedNode::Null { span };
    }
    if s == "true" || s == "false" {
        return ParsedNode::Bool {
            value: s == "true",
            span,
        };
    }
    // Integer: decimal, 0x hex, 0o octal (YAML 1.2 core)
    if let Some(i) = parse_yaml_integer(s) {
        return ParsedNode::Integer { value: i, span };
    }
    // Float: .inf, -.inf, .nan, standard float
    if let Some(f) = parse_yaml_float(s) {
        return ParsedNode::Float { value: f, span };
    }
    // Otherwise: string
    ParsedNode::String {
        value: s.to_string(),
        span,
    }
}

fn parse_yaml_integer(s: &str) -> Option<i64> {
    if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16).ok()
    } else if s.starts_with("0o") || s.starts_with("0O") {
        i64::from_str_radix(&s[2..], 8).ok()
    } else {
        s.parse::<i64>().ok()
    }
}

fn parse_yaml_float(s: &str) -> Option<f64> {
    match s {
        ".inf" | ".Inf" | ".INF" => Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
        _ => s.parse::<f64>().ok(),
    }
}

/// Convert an `Option<&Marker>` to a `FileSpan` (0-indexed).
///
/// `marked-yaml` Markers use 1-indexed line and column numbers.
fn marker_opt_to_file_span(marker: Option<&marked_yaml::Marker>) -> FileSpan {
    match marker {
        Some(m) => FileSpan {
            line: m.line().saturating_sub(1),
            column: m.column().saturating_sub(1),
        },
        None => FileSpan::zero(),
    }
}

// ============================================================================
// JSON parser (serde_json with best-effort span tracking)
// ============================================================================

/// Parse JSON text into a [`ParsedConfigFile`].
///
/// Uses `serde_json`. Because `serde_json` does not provide per-value spans
/// during deserialization, we use the error line/column from parse failures
/// and fall back to `FileSpan::zero()` for all successfully-parsed values.
/// This is an acceptable tradeoff: JSON parse errors are the primary use-case
/// for span anchoring; schema-validation diagnostics carry the JSON's line 0.
///
/// Pure function — no Salsa dependency, no I/O.
pub fn parse_json(text: &str, path: &str) -> Result<ParsedConfigFile, ConfigParseError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        // serde_json error has .line() and .column() (1-indexed).
        let line = e.line().saturating_sub(1);
        let column = e.column().saturating_sub(1);
        ConfigParseError {
            message: e.to_string(),
            span: FileSpan { line, column },
        }
    })?;

    let root = convert_json_value(&value, FileSpan::zero());
    Ok(ParsedConfigFile {
        format: ConfigFormat::Json,
        root,
        path: path.to_string(),
    })
}

fn convert_json_value(value: &serde_json::Value, span: FileSpan) -> ParsedNode {
    match value {
        serde_json::Value::Null => ParsedNode::Null { span },
        serde_json::Value::Bool(b) => ParsedNode::Bool { value: *b, span },
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ParsedNode::Integer { value: i, span }
            } else if let Some(f) = n.as_f64() {
                ParsedNode::Float { value: f, span }
            } else {
                ParsedNode::String {
                    value: n.to_string(),
                    span,
                }
            }
        }
        serde_json::Value::String(s) => ParsedNode::String {
            value: s.clone(),
            span,
        },
        serde_json::Value::Array(arr) => {
            let items: Vec<ParsedNode> = arr.iter().map(|v| convert_json_value(v, span)).collect();
            ParsedNode::Sequence { items, span }
        }
        serde_json::Value::Object(obj) => {
            let entries: Vec<(String, FileSpan, ParsedNode)> = obj
                .iter()
                .map(|(k, v)| (k.clone(), span, convert_json_value(v, span)))
                .collect();
            ParsedNode::Mapping { entries, span }
        }
    }
}

// ============================================================================
// Schema admissibility check
// ============================================================================

/// Returns `true` if `schema` is an admissible loader schema.
///
/// Admissible shapes:
/// - Inline or named record type (`Record { .. }`)
/// - `List<S>` where `S` is an admissible record-rooted schema
/// - `Map<Text, S>` where `S` is an admissible record-rooted schema
///
/// Everything else (bare scalars, `Lambda`, `ColumnRef`, `ModelRef`,
/// `SourceRef`, `TableExpr`, etc.) is forbidden.
///
/// Pure function — no Salsa dependency.
pub fn is_admissible_loader_schema(schema: &SmeltType) -> bool {
    match schema {
        SmeltType::Record { .. } => true,
        SmeltType::List(inner) => is_admissible_loader_schema(inner),
        SmeltType::Map { key, value } => {
            matches!(
                key.as_ref(),
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ) && is_admissible_loader_schema(value)
        }
        _ => false,
    }
}

// ============================================================================
// Schema validation
// ============================================================================

/// Validate a [`ParsedConfigFile`] against a [`SmeltType`] schema.
///
/// The schema must be an admissible loader schema (checked by the caller via
/// [`is_admissible_loader_schema`]); the caller emits `ConfigLoaderSchemaForbidden`
/// and short-circuits when the schema is inadmissible.
///
/// Returns a [`ValidationResult`] carrying the typed value and any diagnostics.
/// Content-validation diagnostics carry `call_site_range` as the secondary frame.
///
/// Pure function — no Salsa dependency, no I/O.
pub fn validate_against_schema(
    parsed: &ParsedConfigFile,
    schema: &SmeltType,
    call_site_range: rowan::TextRange,
) -> ValidationResult {
    let mut diags: Vec<LoaderDiagnostic> = Vec::new();
    let value = validate_node(
        &parsed.root,
        schema,
        None,
        parsed,
        call_site_range,
        &mut diags,
    );
    ValidationResult {
        value,
        diagnostics: diags,
    }
}

/// Validate a parsed config file against `schema` in **partial** mode.
///
/// Identical to [`validate_against_schema`] except that missing record fields
/// are *not* emitted as diagnostics. This is the correct mode for overlay files:
/// the overlay provides replacements for a subset of the base's fields; absent
/// fields are taken from the base, so a field missing from the overlay is not an error.
///
/// Pure function — no Salsa dependency, no I/O.
pub fn validate_against_schema_partial(
    parsed: &ParsedConfigFile,
    schema: &SmeltType,
    call_site_range: rowan::TextRange,
) -> ValidationResult {
    let mut diags: Vec<LoaderDiagnostic> = Vec::new();
    let value = validate_node_partial(
        &parsed.root,
        schema,
        None,
        parsed,
        call_site_range,
        &mut diags,
    );
    ValidationResult {
        value,
        diagnostics: diags,
    }
}

/// Dispatch to the appropriate validator (partial-mode: no missing-field errors).
fn validate_node_partial(
    node: &ParsedNode,
    schema: &SmeltType,
    field_name: Option<&str>,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    match schema {
        SmeltType::Record { fields, .. } => {
            validate_record_node_partial(node, fields, schema, file, call_site_range, diags)
        }
        SmeltType::List(elem_ty) => {
            // List-root overlay replaces base entirely — no partial semantics needed for
            // the list items themselves; validate normally.
            validate_list_node(node, elem_ty, schema, file, call_site_range, diags)
        }
        SmeltType::Map {
            key: _,
            value: val_ty,
        } => validate_map_node(node, val_ty, schema, file, call_site_range, diags),
        SmeltType::Expr(TypeConstraint::Concrete(dt)) => {
            let fname = field_name.unwrap_or("<value>");
            validate_scalar_field(fname, node, dt, file, call_site_range, diags)
        }
        _ => MetaValue::Error,
    }
}

/// Validate a record mapping in partial mode: validates fields that ARE present
/// and checks for unknown fields, but does NOT emit `ConfigLoaderRequiredFieldMissing`
/// for absent fields.
fn validate_record_node_partial(
    node: &ParsedNode,
    fields: &BTreeMap<String, SmeltType>,
    schema: &SmeltType,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    let schema_name = schema_display_name(schema);
    match node {
        ParsedNode::Mapping { entries, .. } => {
            let mut result: BTreeMap<String, MetaValue> = BTreeMap::new();
            let known: std::collections::HashSet<&str> =
                fields.keys().map(|s| s.as_str()).collect();
            let mut schema_fields: Vec<&str> = fields.keys().map(|s| s.as_str()).collect();
            schema_fields.sort_unstable();

            for (key, key_span, val_node) in entries {
                if !known.contains(key.as_str()) {
                    // Unknown field — same error as strict mode.
                    let schema_field_list = schema_fields.join(", ");
                    diags.push(content_diag(
                        crate::DiagnosticCode::ConfigLoaderUnknownField,
                        LoaderDiagnosticSeverity::Error,
                        crate::meta_loader_diagnostic_message(
                            crate::DiagnosticCode::ConfigLoaderUnknownField,
                            None,
                            None,
                            None,
                            None,
                            Some(key),
                            Some(&schema_field_list),
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        *key_span,
                        call_site_range,
                    ));
                } else {
                    let field_ty = &fields[key.as_str()];
                    let val = validate_node_partial(
                        val_node,
                        field_ty,
                        Some(key.as_str()),
                        file,
                        call_site_range,
                        diags,
                    );
                    result.insert(key.clone(), val);
                }
            }
            // NOTE: No missing-field check in partial mode.
            MetaValue::Record(result)
        }
        other => {
            diags.push(content_diag(
                crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                LoaderDiagnosticSeverity::Error,
                crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&schema_name),
                    None,
                    Some("mapping"),
                    Some(other.shape_name()),
                    None,
                    None,
                    None,
                ),
                other.span(),
                call_site_range,
            ));
            MetaValue::Error
        }
    }
}

/// Dispatch to the appropriate validator based on the schema type.
///
/// `field_name` is `Some("field_name")` when we're validating a record field
/// (used for `ConfigLoaderTypeMismatch` messages); `None` for top-level or
/// list/map element contexts.
fn validate_node(
    node: &ParsedNode,
    schema: &SmeltType,
    field_name: Option<&str>,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    match schema {
        SmeltType::Record { fields, .. } => {
            validate_record_node(node, fields, schema, file, call_site_range, diags)
        }
        SmeltType::List(elem_ty) => {
            validate_list_node(node, elem_ty, schema, file, call_site_range, diags)
        }
        SmeltType::Map {
            key: _,
            value: val_ty,
        } => validate_map_node(node, val_ty, schema, file, call_site_range, diags),
        SmeltType::Expr(TypeConstraint::Concrete(dt)) => {
            let fname = field_name.unwrap_or("<value>");
            validate_scalar_field(fname, node, dt, file, call_site_range, diags)
        }
        _ => {
            // Caller should have emitted ConfigLoaderSchemaForbidden before reaching here.
            MetaValue::Error
        }
    }
}

fn validate_record_node(
    node: &ParsedNode,
    fields: &BTreeMap<String, SmeltType>,
    schema: &SmeltType,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    let schema_name = schema_display_name(schema);
    match node {
        ParsedNode::Mapping {
            entries,
            span: map_span,
        } => {
            let mut result: BTreeMap<String, MetaValue> = BTreeMap::new();
            let known: std::collections::HashSet<&str> =
                fields.keys().map(|s| s.as_str()).collect();
            // Sort schema fields for deterministic "expected one of" messages.
            let mut schema_fields: Vec<&str> = fields.keys().map(|s| s.as_str()).collect();
            schema_fields.sort_unstable();

            // Validate each entry in the mapping.
            for (key, key_span, val_node) in entries {
                if !known.contains(key.as_str()) {
                    diags.push(content_diag(
                        crate::DiagnosticCode::ConfigLoaderUnknownField,
                        LoaderDiagnosticSeverity::Error,
                        crate::meta_loader_diagnostic_message(
                            crate::DiagnosticCode::ConfigLoaderUnknownField,
                            None,
                            None,
                            None,
                            None,
                            Some(key),
                            Some(&schema_fields.join(", ")),
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        *key_span,
                        call_site_range,
                    ));
                } else {
                    let field_ty = &fields[key.as_str()];
                    let val = validate_node(
                        val_node,
                        field_ty,
                        Some(key.as_str()),
                        file,
                        call_site_range,
                        diags,
                    );
                    result.insert(key.clone(), val);
                }
            }

            // Check for missing required fields (anchored at the mapping's own span).
            let mut missing_fields: Vec<&str> = fields
                .keys()
                .filter(|field_name| !entries.iter().any(|(k, _, _)| k == *field_name))
                .map(|s| s.as_str())
                .collect();
            missing_fields.sort_unstable();
            for field_name in missing_fields {
                diags.push(content_diag(
                    crate::DiagnosticCode::ConfigLoaderRequiredFieldMissing,
                    LoaderDiagnosticSeverity::Error,
                    crate::meta_loader_diagnostic_message(
                        crate::DiagnosticCode::ConfigLoaderRequiredFieldMissing,
                        None,
                        None,
                        None,
                        None,
                        Some(field_name),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                    *map_span,
                    call_site_range,
                ));
            }
            MetaValue::Record(result)
        }
        other => {
            diags.push(content_diag(
                crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                LoaderDiagnosticSeverity::Error,
                crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&schema_name),
                    None,
                    Some("mapping"),
                    Some(other.shape_name()),
                    None,
                    None,
                    None,
                ),
                other.span(),
                call_site_range,
            ));
            MetaValue::Error
        }
    }
}

fn validate_list_node(
    node: &ParsedNode,
    elem_ty: &SmeltType,
    schema: &SmeltType,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    let schema_name = schema_display_name(schema);
    match node {
        ParsedNode::Sequence { items, .. } => {
            let mut result = Vec::new();
            for item in items {
                let val = validate_node(item, elem_ty, None, file, call_site_range, diags);
                result.push(val);
            }
            MetaValue::List(result)
        }
        other => {
            diags.push(content_diag(
                crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                LoaderDiagnosticSeverity::Error,
                crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&schema_name),
                    None,
                    Some("sequence"),
                    Some(other.shape_name()),
                    None,
                    None,
                    None,
                ),
                other.span(),
                call_site_range,
            ));
            MetaValue::Error
        }
    }
}

fn validate_map_node(
    node: &ParsedNode,
    val_ty: &SmeltType,
    schema: &SmeltType,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    let schema_name = schema_display_name(schema);
    match node {
        ParsedNode::Mapping { entries, span: _ } => {
            let mut result: BTreeMap<String, MetaValue> = BTreeMap::new();
            let mut seen_keys: BTreeMap<String, FileSpan> = BTreeMap::new();
            for (key, key_span, val_node) in entries {
                if let Some(first_span) = seen_keys.get(key) {
                    diags.push(content_diag(
                        crate::DiagnosticCode::ConfigLoaderDuplicateMapKey,
                        LoaderDiagnosticSeverity::Error,
                        crate::meta_loader_diagnostic_message(
                            crate::DiagnosticCode::ConfigLoaderDuplicateMapKey,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            Some(key),
                            Some(&key_span.to_string()),
                            Some(&first_span.to_string()),
                        ),
                        *key_span,
                        call_site_range,
                    ));
                } else {
                    seen_keys.insert(key.clone(), *key_span);
                    let val = validate_node(val_node, val_ty, None, file, call_site_range, diags);
                    result.insert(key.clone(), val);
                }
            }
            MetaValue::Map(result)
        }
        other => {
            diags.push(content_diag(
                crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                LoaderDiagnosticSeverity::Error,
                crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderRootShapeMismatch,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&schema_name),
                    None,
                    Some("mapping"),
                    Some(other.shape_name()),
                    None,
                    None,
                    None,
                ),
                other.span(),
                call_site_range,
            ));
            MetaValue::Error
        }
    }
}

/// Validate a scalar field value against its declared `DataType`.
///
/// `field_name` is used in `ConfigLoaderTypeMismatch` messages.
fn validate_scalar_field(
    field_name: &str,
    node: &ParsedNode,
    dt: &DataType,
    file: &ParsedConfigFile,
    call_site_range: rowan::TextRange,
    diags: &mut Vec<LoaderDiagnostic>,
) -> MetaValue {
    let _is_json = file.format == ConfigFormat::Json;
    match dt {
        DataType::Text => match node {
            ParsedNode::String { value, .. } => MetaValue::Text(value.clone()),
            ParsedNode::Null { span } => {
                // null coerces to Text with a ConfigLoaderNullCoercion warning.
                let row_str = span.to_string();
                diags.push(content_diag(
                    crate::DiagnosticCode::ConfigLoaderNullCoercion,
                    LoaderDiagnosticSeverity::Warning,
                    crate::meta_loader_diagnostic_message(
                        crate::DiagnosticCode::ConfigLoaderNullCoercion,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(&row_str),
                        None,
                    ),
                    *span,
                    call_site_range,
                ));
                MetaValue::Text(String::new())
            }
            other => {
                diags.push(type_mismatch_diag(
                    field_name,
                    "Text",
                    other.shape_name(),
                    other.span(),
                    call_site_range,
                ));
                MetaValue::Error
            }
        },
        DataType::Integer => match node {
            ParsedNode::Integer { value, .. } => MetaValue::Integer(*value),
            ParsedNode::Null { span } => {
                // null at Integer: type mismatch (for both YAML and JSON).
                diags.push(type_mismatch_diag(
                    field_name,
                    "Integer",
                    "null",
                    *span,
                    call_site_range,
                ));
                MetaValue::Error
            }
            ParsedNode::Float { span, .. } => {
                // Float at Integer field → type mismatch (spec: Integer accepts
                // YAML integers only; floats with non-zero fractional part emit
                // ConfigLoaderTypeMismatch).
                diags.push(type_mismatch_diag(
                    field_name,
                    "Integer",
                    "float",
                    *span,
                    call_site_range,
                ));
                MetaValue::Error
            }
            ParsedNode::String { value, span } => {
                diags.push(type_mismatch_diag(
                    field_name,
                    "Integer",
                    &format!("string {:?}", value),
                    *span,
                    call_site_range,
                ));
                MetaValue::Error
            }
            other => {
                diags.push(type_mismatch_diag(
                    field_name,
                    "Integer",
                    other.shape_name(),
                    other.span(),
                    call_site_range,
                ));
                MetaValue::Error
            }
        },
        DataType::Float | DataType::Double => match node {
            ParsedNode::Float { value, .. } => MetaValue::Float(*value),
            ParsedNode::Integer { value, .. } => MetaValue::Float(*value as f64),
            other => {
                diags.push(type_mismatch_diag(
                    field_name,
                    "Float",
                    other.shape_name(),
                    other.span(),
                    call_site_range,
                ));
                MetaValue::Error
            }
        },
        DataType::Boolean => match node {
            ParsedNode::Bool { value, .. } => MetaValue::Boolean(*value),
            other => {
                diags.push(type_mismatch_diag(
                    field_name,
                    "Boolean",
                    other.shape_name(),
                    other.span(),
                    call_site_range,
                ));
                MetaValue::Error
            }
        },
        _ => {
            // Other scalar types (Date, Timestamp, Decimal, etc.).
            // Accept string representation (ISO-8601, decimal strings, etc.).
            // Null handling: only Text accepts null coercion; for other types null is a mismatch.
            match node {
                ParsedNode::String { value, .. } => MetaValue::Text(value.clone()),
                ParsedNode::Integer { value, .. } => MetaValue::Integer(*value),
                ParsedNode::Float { value, .. } => MetaValue::Float(*value),
                ParsedNode::Null { span } => {
                    let dt_name = format!("{:?}", dt);
                    diags.push(type_mismatch_diag(
                        field_name,
                        &dt_name,
                        "null",
                        *span,
                        call_site_range,
                    ));
                    MetaValue::Error
                }
                other => {
                    let dt_name = format!("{:?}", dt);
                    diags.push(type_mismatch_diag(
                        field_name,
                        &dt_name,
                        other.shape_name(),
                        other.span(),
                        call_site_range,
                    ));
                    MetaValue::Error
                }
            }
        }
    }
}

// ============================================================================
// Diagnostic helpers
// ============================================================================

fn content_diag(
    code: crate::DiagnosticCode,
    severity: LoaderDiagnosticSeverity,
    message: String,
    primary_span: FileSpan,
    call_site_range: rowan::TextRange,
) -> LoaderDiagnostic {
    LoaderDiagnostic {
        code,
        severity,
        message,
        primary_span,
        secondary_call_span: Some(call_site_range),
    }
}

fn type_mismatch_diag(
    field_name: &str,
    expected: &str,
    actual: &str,
    span: FileSpan,
    call_site_range: rowan::TextRange,
) -> LoaderDiagnostic {
    content_diag(
        crate::DiagnosticCode::ConfigLoaderTypeMismatch,
        LoaderDiagnosticSeverity::Error,
        crate::meta_loader_diagnostic_message(
            crate::DiagnosticCode::ConfigLoaderTypeMismatch,
            None,
            None,
            None,
            None,
            Some(field_name),
            None,
            Some(expected),
            Some(actual),
            None,
            None,
            None,
            None,
            None,
        ),
        span,
        call_site_range,
    )
}

fn schema_display_name(schema: &SmeltType) -> String {
    match schema {
        SmeltType::Record { name: Some(n), .. } => n.clone(),
        SmeltType::Record { .. } => "{record}".to_string(),
        SmeltType::List(inner) => format!("List<{}>", schema_display_name(inner)),
        SmeltType::Map { key, value } => {
            format!(
                "Map<{}, {}>",
                schema_display_name(key),
                schema_display_name(value)
            )
        }
        SmeltType::Expr(TypeConstraint::Concrete(dt)) => format!("{:?}", dt),
        _ => "?".to_string(),
    }
}

// ============================================================================
// Overlay merge (Phase 6)
// ============================================================================

/// Merge `overlay` on top of `base` according to the per-target overlay rules
/// for schema `schema`:
///
/// - **Record root**: per-field replace — overlay fields replace base fields;
///   fields absent from the overlay are taken from the base. Nested records
///   are merged recursively.
/// - **List root**: overlay replaces base entirely (no concatenation).
/// - **Map root**: per-key replace — overlay keys replace base values;
///   keys absent from the overlay are taken from the base.
///
/// This is a **pure function** — no I/O, no Salsa. The caller is responsible
/// for parsing both files and validating against the schema before merging.
///
/// `_schema` is used for dispatch (record / list / map); the concrete field
/// types are consulted only for recursive nested-record merging.
pub fn merge_values(base: MetaValue, overlay: MetaValue, schema: &SmeltType) -> MetaValue {
    match schema {
        SmeltType::Record { fields, .. } => {
            // Record root: per-field replace; recurse into nested record fields.
            let base_fields = match base {
                MetaValue::Record(m) => m,
                other => return other, // shouldn't happen if validation passed
            };
            let overlay_fields = match overlay {
                MetaValue::Record(m) => m,
                _ => return MetaValue::Record(base_fields), // fallback
            };
            let mut merged = base_fields;
            for (key, ov_val) in overlay_fields {
                if let Some(field_schema) = fields.get(&key) {
                    let base_val = merged.remove(&key);
                    let merged_field = match base_val {
                        Some(bv) => merge_values(bv, ov_val, field_schema),
                        None => ov_val,
                    };
                    merged.insert(key, merged_field);
                } else {
                    // Overlay key not in schema — should have been caught by validation;
                    // skip silently here.
                }
            }
            MetaValue::Record(merged)
        }
        SmeltType::List(_) => {
            // List root: overlay replaces base entirely.
            overlay
        }
        SmeltType::Map { .. } => {
            // Map root: per-key replace.
            let base_map = match base {
                MetaValue::Map(m) => m,
                other => return other,
            };
            let overlay_map = match overlay {
                MetaValue::Map(m) => m,
                _ => return MetaValue::Map(base_map),
            };
            let mut merged = base_map;
            for (key, ov_val) in overlay_map {
                merged.insert(key, ov_val);
            }
            MetaValue::Map(merged)
        }
        _ => overlay, // scalar / unknown — overlay wins
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::signatures::SmeltType;
    use smelt_types::DataType;
    use smelt_types::TypeConstraint;

    /// Build a zero `TextRange` (placeholder call-site).
    fn zero_range() -> rowan::TextRange {
        rowan::TextRange::empty(rowan::TextSize::from(0))
    }

    /// Build a simple `Cohort = {name: Text, threshold: Integer}` schema.
    fn cohort_schema() -> SmeltType {
        let mut fields = BTreeMap::new();
        fields.insert(
            "name".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        );
        fields.insert(
            "threshold".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
        );
        SmeltType::Record {
            fields,
            name: Some("Cohort".to_string()),
        }
    }

    // ── yaml_parse_record_root_happy_path ───────────────────────────────────

    #[test]
    fn yaml_parse_record_root_happy_path() {
        let yaml = "{name: us_west, threshold: 100}";
        let parsed = parse_yaml(yaml, "cohort.yaml").expect("must parse");
        let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
        assert!(
            result.diagnostics.is_empty(),
            "happy path must have no diagnostics; got: {:?}",
            result.diagnostics
        );
        match &result.value {
            MetaValue::Record(fields) => {
                assert!(fields.contains_key("name"), "name field missing");
                assert!(fields.contains_key("threshold"), "threshold field missing");
            }
            other => panic!("expected Record, got: {:?}", other),
        }
    }

    // ── yaml_parse_record_root_missing_field ────────────────────────────────

    #[test]
    fn yaml_parse_record_root_missing_field() {
        let yaml = "{name: us_west}";
        let parsed = parse_yaml(yaml, "cohort.yaml").expect("must parse");
        let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
        let missing: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderRequiredFieldMissing)
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "missing `threshold` must emit exactly 1 ConfigLoaderRequiredFieldMissing; got: {:?}",
            result.diagnostics
        );
        // Anchored at line 0 (the mapping row — marked-yaml gives line 1 → 0-indexed = 0).
        assert_eq!(
            missing[0].primary_span.line, 0,
            "diagnostic must be anchored at the YAML file's first line (0-indexed)"
        );
    }

    // ── yaml_parse_record_root_unknown_field ────────────────────────────────

    #[test]
    fn yaml_parse_record_root_unknown_field() {
        let yaml = "{name: us_west, threshold: 100, bogus: true}";
        let parsed = parse_yaml(yaml, "cohort.yaml").expect("must parse");
        let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
        let unknown: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderUnknownField)
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "extra `bogus` field must emit exactly 1 ConfigLoaderUnknownField; got: {:?}",
            result.diagnostics
        );
        assert!(
            unknown[0].message.contains("bogus"),
            "diagnostic message must name the offending field; got: {}",
            unknown[0].message
        );
    }

    // ── yaml_parse_record_root_type_mismatch ────────────────────────────────

    #[test]
    fn yaml_parse_record_root_type_mismatch() {
        // 'lots' is a YAML string — but threshold is Integer.
        let yaml = "{name: us_west, threshold: 'lots'}";
        let parsed = parse_yaml(yaml, "cohort.yaml").expect("must parse");
        let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
        let mismatch: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderTypeMismatch)
            .collect();
        assert_eq!(
            mismatch.len(),
            1,
            "string at integer field must emit exactly 1 ConfigLoaderTypeMismatch; got: {:?}",
            result.diagnostics
        );
    }

    // ── yaml_parse_list_root_emits_root_shape_mismatch_when_mapping ─────────

    #[test]
    fn yaml_parse_list_root_emits_root_shape_mismatch_when_mapping() {
        let yaml = "{a: 1}";
        let parsed = parse_yaml(yaml, "cohorts.yaml").expect("must parse");
        let list_schema = SmeltType::List(Box::new(cohort_schema()));
        let result = validate_against_schema(&parsed, &list_schema, zero_range());
        let shape: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderRootShapeMismatch)
            .collect();
        assert_eq!(
            shape.len(),
            1,
            "mapping against List<Cohort> must emit exactly 1 ConfigLoaderRootShapeMismatch; got: {:?}",
            result.diagnostics
        );
    }

    // ── yaml_parse_list_root_validates_each_element ─────────────────────────

    #[test]
    fn yaml_parse_list_root_validates_each_element() {
        // Three elements; the second omits `threshold`.
        let yaml =
            "- {name: us_west, threshold: 100}\n- {name: us_east}\n- {name: eu_west, threshold: 200}\n";
        let parsed = parse_yaml(yaml, "cohorts.yaml").expect("must parse");
        let list_schema = SmeltType::List(Box::new(cohort_schema()));
        let result = validate_against_schema(&parsed, &list_schema, zero_range());
        let missing: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderRequiredFieldMissing)
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "second element missing `threshold` must emit exactly 1 ConfigLoaderRequiredFieldMissing; got: {:?}",
            result.diagnostics
        );
        // The second element starts at line 1 (0-indexed).
        assert_eq!(
            missing[0].primary_span.line, 1,
            "diagnostic must be anchored at the second element's row (line 1, 0-indexed)"
        );
    }

    // ── yaml_parse_map_root_validates_each_entry ────────────────────────────

    #[test]
    fn yaml_parse_map_root_validates_each_entry() {
        // Three map entries; the third's value omits `name`.
        let yaml = "us_west:\n  name: us_west\n  threshold: 100\nus_east:\n  name: us_east\n  threshold: 200\neu_west:\n  threshold: 300\n";
        let parsed = parse_yaml(yaml, "cohorts.yaml").expect("must parse");
        let map_schema = SmeltType::Map {
            key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
            value: Box::new(cohort_schema()),
        };
        let result = validate_against_schema(&parsed, &map_schema, zero_range());
        let missing: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderRequiredFieldMissing)
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "third entry missing `name` must emit exactly 1 ConfigLoaderRequiredFieldMissing; got: {:?}",
            result.diagnostics
        );
        // The YAML (0-indexed lines):
        //   line 0: us_west:
        //   line 1:   name: us_west
        //   line 2:   threshold: 100
        //   line 3: us_east:
        //   line 4:   name: us_east
        //   line 5:   threshold: 200
        //   line 6: eu_west:
        //   line 7:   threshold: 300
        //
        // The `ConfigLoaderRequiredFieldMissing` diagnostic is anchored at the
        // map_span of the eu_west entry's VALUE mapping (the `{threshold: 300}`
        // block), whose first token is on line 7. (marked-yaml anchors the mapping
        // node at its first field, not at the key line.)
        assert_eq!(
            missing[0].primary_span.line, 7,
            "diagnostic must be anchored at the third entry's value mapping row (line 7, 0-indexed)"
        );
    }

    // ── yaml_parse_map_root_emits_duplicate_key ─────────────────────────────

    #[test]
    fn yaml_parse_map_root_emits_duplicate_key() {
        // We build the ParsedNode directly because most YAML parsers silently deduplicate
        // keys (and `marked-yaml` may do the same). The validator must detect duplicates.
        let map_schema = SmeltType::Map {
            key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
            value: Box::new(cohort_schema()),
        };
        let make_cohort_entry = |name: &str, line: usize| ParsedNode::Mapping {
            entries: vec![
                (
                    "name".to_string(),
                    FileSpan { line, column: 2 },
                    ParsedNode::String {
                        value: name.to_string(),
                        span: FileSpan { line, column: 8 },
                    },
                ),
                (
                    "threshold".to_string(),
                    FileSpan {
                        line: line + 1,
                        column: 2,
                    },
                    ParsedNode::Integer {
                        value: 100,
                        span: FileSpan {
                            line: line + 1,
                            column: 13,
                        },
                    },
                ),
            ],
            span: FileSpan { line, column: 0 },
        };
        let root = ParsedNode::Mapping {
            entries: vec![
                (
                    "us_west".to_string(),
                    FileSpan { line: 0, column: 0 },
                    make_cohort_entry("us_west", 0),
                ),
                (
                    "us_west".to_string(),
                    FileSpan { line: 3, column: 0 },
                    make_cohort_entry("us_west2", 3),
                ),
            ],
            span: FileSpan::zero(),
        };
        let file = ParsedConfigFile {
            format: ConfigFormat::Yaml,
            root,
            path: "cohorts.yaml".to_string(),
        };
        let result = validate_against_schema(&file, &map_schema, zero_range());
        let dup: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderDuplicateMapKey)
            .collect();
        assert_eq!(
            dup.len(),
            1,
            "duplicate key `us_west` must emit exactly 1 ConfigLoaderDuplicateMapKey; got: {:?}",
            result.diagnostics
        );
        assert!(
            dup[0].message.contains("us_west"),
            "duplicate-key message must name the key; got: {}",
            dup[0].message
        );
        // Anchored at the second appearance's row (line 3).
        assert_eq!(
            dup[0].primary_span.line, 3,
            "duplicate-key diagnostic must be anchored at the second key's row"
        );
    }

    // ── yaml_parse_invalid_yaml_emits_parse_error ───────────────────────────

    #[test]
    fn yaml_parse_invalid_yaml_emits_parse_error() {
        let bad_yaml = "{name: us_west, threshold: 100"; // missing closing brace
        let result = parse_yaml(bad_yaml, "cohort.yaml");
        assert!(
            result.is_err(),
            "malformed YAML must return Err; got: {:?}",
            result
        );
    }

    // ── yaml_null_coercion_to_text_emits_warning ────────────────────────────

    #[test]
    fn yaml_null_coercion_to_text_emits_warning() {
        let yaml = "{name: null, threshold: 100}";
        let parsed = parse_yaml(yaml, "cohort.yaml").expect("must parse");
        let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
        let null_warn: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderNullCoercion)
            .collect();
        assert_eq!(
            null_warn.len(),
            1,
            "null at Text field must emit exactly 1 ConfigLoaderNullCoercion; got: {:?}",
            result.diagnostics
        );
        assert_eq!(
            null_warn[0].severity,
            LoaderDiagnosticSeverity::Warning,
            "ConfigLoaderNullCoercion must be a warning"
        );
        // The value must coerce to an empty string.
        match &result.value {
            MetaValue::Record(fields) => match fields.get("name") {
                Some(MetaValue::Text(s)) => {
                    assert_eq!(s, "", "null coerces to empty string")
                }
                other => panic!("expected Text(\"\"), got: {:?}", other),
            },
            other => panic!("expected Record, got: {:?}", other),
        }
    }

    // ── json_parse_record_root_happy_path ───────────────────────────────────

    #[test]
    fn json_parse_record_root_happy_path() {
        let json = r#"{"name": "us_west", "threshold": 100}"#;
        let parsed = parse_json(json, "cohort.json").expect("must parse");
        let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
        assert!(
            result.diagnostics.is_empty(),
            "JSON happy path must have no diagnostics; got: {:?}",
            result.diagnostics
        );
        match &result.value {
            MetaValue::Record(fields) => {
                assert!(fields.contains_key("name"), "name field missing");
                assert!(fields.contains_key("threshold"), "threshold field missing");
            }
            other => panic!("expected Record, got: {:?}", other),
        }
    }

    // ── json_parse_trailing_comma_emits_parse_error ─────────────────────────

    #[test]
    fn json_parse_trailing_comma_emits_parse_error() {
        let bad_json = r#"{"name": "us_west", "threshold": 100,}"#;
        let result = parse_json(bad_json, "cohort.json");
        assert!(
            result.is_err(),
            "JSON with trailing comma must return Err; got: {:?}",
            result
        );
    }

    // ── json_null_coerces_only_to_text ───────────────────────────────────────

    #[test]
    fn json_null_coerces_only_to_text() {
        // null at Integer field → ConfigLoaderTypeMismatch
        {
            let json = r#"{"name": "us_west", "threshold": null}"#;
            let parsed = parse_json(json, "cohort.json").expect("must parse");
            let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
            let mismatch: Vec<_> = result
                .diagnostics
                .iter()
                .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderTypeMismatch)
                .collect();
            assert_eq!(
                mismatch.len(),
                1,
                "JSON null at Integer field must emit ConfigLoaderTypeMismatch; got: {:?}",
                result.diagnostics
            );
        }
        // null at Text field → ConfigLoaderNullCoercion (warning)
        {
            let json = r#"{"name": null, "threshold": 100}"#;
            let parsed = parse_json(json, "cohort.json").expect("must parse");
            let result = validate_against_schema(&parsed, &cohort_schema(), zero_range());
            let null_warn: Vec<_> = result
                .diagnostics
                .iter()
                .filter(|d| d.code == crate::DiagnosticCode::ConfigLoaderNullCoercion)
                .collect();
            assert_eq!(
                null_warn.len(),
                1,
                "JSON null at Text field must emit ConfigLoaderNullCoercion; got: {:?}",
                result.diagnostics
            );
            assert_eq!(
                null_warn[0].severity,
                LoaderDiagnosticSeverity::Warning,
                "ConfigLoaderNullCoercion must be a warning"
            );
        }
    }

    // ── Phase 6 overlay merge tests ────────────────────────────────────────

    /// `merge_values` on a record root deep-merges overridden fields and
    /// preserves fields absent from the overlay.
    #[test]
    fn overlay_record_root_deep_merges_overridden_field() {
        // Base: {name: 'us_west', region: 'us-west-2', threshold: 100}
        // Overlay (target=prod): {threshold: 50}
        // Expected: threshold=50, name and region from base.
        let mut base_schema_fields = BTreeMap::new();
        base_schema_fields.insert(
            "name".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        );
        base_schema_fields.insert(
            "region".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        );
        base_schema_fields.insert(
            "threshold".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
        );
        let schema = SmeltType::Record {
            fields: base_schema_fields,
            name: Some("Cohort".to_string()),
        };

        let mut base_fields = BTreeMap::new();
        base_fields.insert("name".to_string(), MetaValue::Text("us_west".to_string()));
        base_fields.insert(
            "region".to_string(),
            MetaValue::Text("us-west-2".to_string()),
        );
        base_fields.insert("threshold".to_string(), MetaValue::Integer(100));
        let base = MetaValue::Record(base_fields);

        let mut overlay_fields = BTreeMap::new();
        overlay_fields.insert("threshold".to_string(), MetaValue::Integer(50));
        let overlay = MetaValue::Record(overlay_fields);

        let merged = merge_values(base, overlay, &schema);
        match merged {
            MetaValue::Record(fields) => {
                assert_eq!(
                    fields.get("threshold"),
                    Some(&MetaValue::Integer(50)),
                    "threshold must be overridden by overlay"
                );
                assert_eq!(
                    fields.get("name"),
                    Some(&MetaValue::Text("us_west".to_string())),
                    "name must come from base"
                );
                assert_eq!(
                    fields.get("region"),
                    Some(&MetaValue::Text("us-west-2".to_string())),
                    "region must come from base"
                );
            }
            other => panic!("expected Record, got: {:?}", other),
        }
    }

    /// `merge_values` on a list root replaces base entirely with overlay.
    #[test]
    fn overlay_list_root_replaces_base() {
        // Base: [{name: a}, {name: b}]
        // Overlay: [{name: c}]
        // Expected: [{name: c}]
        let elem_schema = cohort_schema();
        let list_schema = SmeltType::List(Box::new(elem_schema));

        let mut a_fields = BTreeMap::new();
        a_fields.insert("name".to_string(), MetaValue::Text("a".to_string()));
        a_fields.insert("threshold".to_string(), MetaValue::Integer(1));
        let mut b_fields = BTreeMap::new();
        b_fields.insert("name".to_string(), MetaValue::Text("b".to_string()));
        b_fields.insert("threshold".to_string(), MetaValue::Integer(2));
        let base = MetaValue::List(vec![
            MetaValue::Record(a_fields),
            MetaValue::Record(b_fields),
        ]);

        let mut c_fields = BTreeMap::new();
        c_fields.insert("name".to_string(), MetaValue::Text("c".to_string()));
        c_fields.insert("threshold".to_string(), MetaValue::Integer(3));
        let overlay = MetaValue::List(vec![MetaValue::Record(c_fields)]);

        let merged = merge_values(base, overlay, &list_schema);
        match merged {
            MetaValue::List(items) => {
                assert_eq!(
                    items.len(),
                    1,
                    "merged list must have 1 item (overlay replaces base)"
                );
                match &items[0] {
                    MetaValue::Record(fields) => {
                        assert_eq!(
                            fields.get("name"),
                            Some(&MetaValue::Text("c".to_string())),
                            "overlay item must have name 'c'"
                        );
                    }
                    other => panic!("expected Record item, got: {:?}", other),
                }
            }
            other => panic!("expected List, got: {:?}", other),
        }
    }

    /// `merge_values` on a map root does per-key replace — keys absent from
    /// the overlay are preserved from base.
    #[test]
    fn overlay_map_root_replaces_per_key() {
        // Base: {a: {threshold: 100}, b: {threshold: 200}}
        // Overlay: {a: {threshold: 50}}
        // Expected: {a: {threshold: 50}, b: {threshold: 200}}
        let val_schema = cohort_schema();
        let map_schema = SmeltType::Map {
            key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
            value: Box::new(val_schema),
        };

        let mut a_fields = BTreeMap::new();
        a_fields.insert("name".to_string(), MetaValue::Text("a".to_string()));
        a_fields.insert("threshold".to_string(), MetaValue::Integer(100));
        let mut b_fields = BTreeMap::new();
        b_fields.insert("name".to_string(), MetaValue::Text("b".to_string()));
        b_fields.insert("threshold".to_string(), MetaValue::Integer(200));
        let mut base_map = BTreeMap::new();
        base_map.insert("a".to_string(), MetaValue::Record(a_fields));
        base_map.insert("b".to_string(), MetaValue::Record(b_fields));
        let base = MetaValue::Map(base_map);

        let mut a2_fields = BTreeMap::new();
        a2_fields.insert("name".to_string(), MetaValue::Text("a".to_string()));
        a2_fields.insert("threshold".to_string(), MetaValue::Integer(50));
        let mut overlay_map = BTreeMap::new();
        overlay_map.insert("a".to_string(), MetaValue::Record(a2_fields));
        let overlay = MetaValue::Map(overlay_map);

        let merged = merge_values(base, overlay, &map_schema);
        match merged {
            MetaValue::Map(map) => {
                // 'a' threshold must be overridden.
                match map.get("a") {
                    Some(MetaValue::Record(fields)) => {
                        assert_eq!(
                            fields.get("threshold"),
                            Some(&MetaValue::Integer(50)),
                            "key 'a' threshold must be 50 from overlay"
                        );
                    }
                    other => panic!("expected Record for key 'a', got: {:?}", other),
                }
                // 'b' must be preserved from base.
                match map.get("b") {
                    Some(MetaValue::Record(fields)) => {
                        assert_eq!(
                            fields.get("threshold"),
                            Some(&MetaValue::Integer(200)),
                            "key 'b' threshold must be 200 (from base; absent in overlay)"
                        );
                    }
                    other => panic!("expected Record for key 'b', got: {:?}", other),
                }
            }
            other => panic!("expected Map, got: {:?}", other),
        }
    }
}
