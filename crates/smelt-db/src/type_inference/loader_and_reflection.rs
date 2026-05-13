//! Loader call diagnostics (load_yaml/load_json/load_toml), config_var, columns_of, column_ref/model_ref/source_ref field projections, wide-reflection checks.

#![allow(unused_imports)]
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
};
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, RecordRegistry,
    SmeltType, TypeConstraint,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::type_context::TypeContext;
#[allow(unused_imports)]
use super::*;

/// Walk all `SMELT_PATH_CALL` descendants of `root` whose path is `config.var`.
/// For each such call:
/// - If the argument is not a string literal → `ConfigVarNameNotLiteral`.
/// - If the argument is a string literal but `vars_map` does not contain that key
///   → `ConfigVarNotFound`.
/// - If the argument resolves to a YAML `null` value → `ConfigVarNullCoercion` (Warning).
///
/// `vars_map` is the parsed `vars:` block from `smelt.yml` (pass empty map when absent).
/// `text` is the raw source file text (used for span → range conversion; pass `""`
/// in tests where range accuracy is not needed).
///
/// Pure function — no Salsa dependency.
pub fn check_config_var_call_diagnostics(
    root: &smelt_parser::syntax_kind::SyntaxNode,
    vars_map: &std::collections::BTreeMap<String, serde_yaml::Value>,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only handle `smelt.config.var(...)`.
        let segs = call.segments();
        if segs.len() != 2 || segs[0].to_lowercase() != "config" || segs[1].to_lowercase() != "var"
        {
            continue;
        }

        let call_range = to_range(node.text_range());

        // Extract the first positional argument expression.
        let first_arg = call
            .arg_list()
            .and_then(|args| args.positional_args().into_iter().next());

        let Some(arg_expr) = first_arg else {
            // No argument at all — emit ConfigVarNameNotLiteral so the user
            // gets a useful diagnostic rather than a silent no-op.
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_hof_diagnostic_message(
                    crate::DiagnosticCode::ConfigVarNameNotLiteral,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                range: call_range,
                code: Some(crate::DiagnosticCode::ConfigVarNameNotLiteral),
                data: None,
            });
            continue;
        };

        // Check: is the argument a string literal?
        if !crate::config_vars::is_string_literal_expr(&arg_expr) {
            let arg_range = to_range(arg_expr.syntax().text_range());
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_hof_diagnostic_message(
                    crate::DiagnosticCode::ConfigVarNameNotLiteral,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                range: arg_range,
                code: Some(crate::DiagnosticCode::ConfigVarNameNotLiteral),
                data: None,
            });
            continue;
        }

        // Extract the string value.
        let var_name = match crate::config_vars::extract_string_literal_value(&arg_expr) {
            Some(n) => n,
            None => continue, // should not happen if is_string_literal_expr returned true
        };

        // Look up in vars_map.
        match vars_map.get(&var_name) {
            None => {
                diags.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: crate::meta_hof_diagnostic_message(
                        crate::DiagnosticCode::ConfigVarNotFound,
                        None,
                        Some(&var_name),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                    range: call_range,
                    code: Some(crate::DiagnosticCode::ConfigVarNotFound),
                    data: None,
                });
            }
            Some(val) => {
                // Coerce and check for null.
                let (_text_val, warn_name) =
                    crate::config_vars::coerce_yaml_scalar_to_text(val, &var_name);
                if let Some(null_var) = warn_name {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Warning,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::ConfigVarNullCoercion,
                            None,
                            Some(&null_var),
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: call_range,
                        code: Some(crate::DiagnosticCode::ConfigVarNullCoercion),
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

// ============================================================================
// Loader call dispatch (Phase E1 Phase 5)
// ============================================================================

/// Known scalar type names that are forbidden as loader schemas.
///
/// These are valid as field types inside a record body, but NOT as the schema
/// argument to `smelt.config.load_yaml` / `load_json` — the loader requires a
/// record, `List<record>`, or `Map<Text, record>` at the top level.
const KNOWN_SCALAR_TYPE_NAMES: &[&str] = &[
    "Text",
    "Integer",
    "Float",
    "Double",
    "Boolean",
    "Date",
    "Timestamp",
    "Decimal",
    "Blob",
    "Varchar",
    "BigInt",
    "SmallInt",
    "TinyInt",
    "UInt",
    "UBigInt",
];

/// Reflection-witness type names that are forbidden as loader schemas.
///
/// These are meta-language types that refer to compiler objects (columns,
/// models, sources), not to data shapes. Using them as a loader schema is
/// always wrong: the loader needs a record, `List<record>`, or `Map<Text, record>`.
const REFLECTION_WITNESS_TYPE_NAMES: &[&str] = &["ColumnRef", "ModelRef", "SourceRef"];

/// The outcome of a loader path argument check.
#[derive(Debug, Clone)]
pub enum LoaderPathOutcome {
    /// Path is not a string literal — emit `ConfigLoaderPathNotLiteral`.
    NotLiteral { arg_range: crate::Range },
    /// Path contains `\` — emit `ConfigLoaderPathBackslash`.
    Backslash {
        path: String,
        call_range: crate::Range,
    },
    /// Path is absolute, escapes the workspace, or has a scheme prefix —
    /// emit `ConfigLoaderPathEscapesWorkspace`.
    EscapesWorkspace {
        path: String,
        call_range: crate::Range,
    },
    /// Path is valid but the file was not found — emit `ConfigLoaderFileNotFound`.
    FileNotFound {
        path: String,
        arg_range: crate::Range,
    },
    /// Path is valid and the file exists.
    Valid { path: String },
}

/// Check a single loader path literal for well-formedness.
///
/// `path_value` is the unquoted path string.
/// `path_range` is the range of the string-literal token (for path-check diagnostics).
/// `call_range` is the range of the whole call expression (for escape/backslash diagnostics).
/// `file_exists` is a callback that returns `true` when the workspace contains
/// the given workspace-relative path.
///
/// Pure — no Salsa dependency, no I/O.
pub fn check_loader_path(
    path_value: &str,
    path_range: crate::Range,
    call_range: crate::Range,
    file_exists: &dyn Fn(&str) -> bool,
) -> LoaderPathOutcome {
    // 1. Backslash check.
    if path_value.contains('\\') {
        return LoaderPathOutcome::Backslash {
            path: path_value.to_string(),
            call_range,
        };
    }

    // 2. Escape check: absolute, ..-escape, scheme prefix.
    if path_value.starts_with('/')
        || path_value.starts_with("../")
        || path_value == ".."
        || has_scheme_prefix(path_value)
    {
        return LoaderPathOutcome::EscapesWorkspace {
            path: path_value.to_string(),
            call_range,
        };
    }
    // Component-level `..` check (e.g. `a/../../etc/passwd`).
    for component in path_value.split('/') {
        if component == ".." {
            return LoaderPathOutcome::EscapesWorkspace {
                path: path_value.to_string(),
                call_range,
            };
        }
    }

    // 3. File-not-found check.
    if !file_exists(path_value) {
        return LoaderPathOutcome::FileNotFound {
            path: path_value.to_string(),
            arg_range: path_range,
        };
    }

    LoaderPathOutcome::Valid {
        path: path_value.to_string(),
    }
}

fn has_scheme_prefix(path: &str) -> bool {
    // Check for http://, https://, s3://, file://
    for scheme in &["http://", "https://", "s3://", "file://"] {
        if path.starts_with(scheme) {
            return true;
        }
    }
    false
}

/// Determine if the text of a schema argument is an admissible loader schema.
///
/// Returns `true` when the schema text looks like an inline record (`{...}`),
/// a `List<...>`, a `Map<Text, ...>`, or an identifier that is NOT a known
/// scalar type name (i.e. could be a named `smelt.record` declaration).
///
/// Returns `false` for bare scalar names (`Integer`, `Text`, `Float`, etc.)
/// or `Lambda<...>` style types.
///
/// Pure — no Salsa dependency.
pub fn schema_arg_text_is_admissible(schema_text: &str, registry: &RecordRegistry) -> bool {
    let trimmed = schema_text.trim();
    // Inline record: starts with `{`
    if trimmed.starts_with('{') {
        return true;
    }
    // List or Map with angle brackets
    if trimmed.starts_with("List<") || trimmed.starts_with("Map<") {
        return true;
    }
    // Lambda (forbidden)
    if trimmed.starts_with("Lambda<") {
        return false;
    }
    // Bare identifier: check if it's a known scalar, a reflection witness, or a declared record.
    // Known scalars → forbidden.
    if KNOWN_SCALAR_TYPE_NAMES.contains(&trimmed) {
        return false;
    }
    // Reflection witnesses (`ColumnRef`, `ModelRef`, `SourceRef`) → forbidden.
    // These are meta-language types that name compiler objects, not data shapes.
    if REFLECTION_WITNESS_TYPE_NAMES.contains(&trimmed) {
        return false;
    }
    // Unknown identifier: if it's in the registry → admissible (named record).
    // If not in the registry → we can't tell at this point, so tentatively
    // allow it (the upstream type-checker emits UnknownType separately).
    if registry.lookup(trimmed).is_some() {
        return true;
    }
    // Not in registry and not a known scalar or reflection witness — could be a
    // future/unknown name. Treat as unknown-admissible to avoid false positives.
    // The LSP separately emits UnknownType diagnostics.
    true
}

/// Parse a schema argument text (as it appears in source code) into a [`SmeltType`].
///
/// Handles:
/// - Inline record type: `{field: Type, ...}` → `SmeltType::Record { name: None, fields }`
/// - Named record reference: `Cohort` → `SmeltType::Record { name: Some("Cohort"), fields: registry_fields }`
/// - `List<S>`: recursively resolves `S`.
/// - `Map<Text, S>`: recursively resolves `S`.
/// - Forbidden schemas (scalars, reflection witnesses, Lambda): `SmeltType::Unknown`.
///
/// `ctx` is used for named record lookup. Pure — no Salsa dependency, no I/O.
fn parse_schema_arg_to_smelt_type(schema_text: &str, ctx: &TypeContext) -> SmeltType {
    let trimmed = schema_text.trim();

    // Inline record `{ f: T, ... }`
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let body = &trimmed[1..trimmed.len() - 1];
        let mut fields = std::collections::BTreeMap::new();
        for part in body.split(',') {
            let part = part.trim();
            if let Some((name, ty_text)) = part.split_once(':') {
                let name = name.trim().to_string();
                let ty = parse_field_type_text(ty_text.trim());
                fields.insert(name, ty);
            }
        }
        return SmeltType::Record { fields, name: None };
    }

    // List<S>
    if let Some(inner) = trimmed
        .strip_prefix("List<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let inner_ty = parse_schema_arg_to_smelt_type(inner, ctx);
        return SmeltType::List(Box::new(inner_ty));
    }

    // Map<Text, S>
    if let Some(inner) = trimmed
        .strip_prefix("Map<")
        .and_then(|s| s.strip_suffix('>'))
    {
        if let Some((k_str, v_str)) = split_map_type_args(inner) {
            let k_ty = parse_field_type_text(k_str.trim());
            let v_ty = parse_schema_arg_to_smelt_type(v_str.trim(), ctx);
            return SmeltType::Map {
                key: Box::new(k_ty),
                value: Box::new(v_ty),
            };
        }
    }

    // Bare identifier: check for known scalars and reflection witnesses first.
    if KNOWN_SCALAR_TYPE_NAMES.contains(&trimmed)
        || REFLECTION_WITNESS_TYPE_NAMES.contains(&trimmed)
    {
        return SmeltType::Unknown;
    }
    // Lambda → Unknown
    if trimmed.starts_with("Lambda<") {
        return SmeltType::Unknown;
    }

    // Named record reference: try to look up in the registry.
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_alphanumeric() || c == '_') {
        if let Some(decl) = ctx.lookup_record_decl(trimmed) {
            let fields: std::collections::BTreeMap<String, SmeltType> = decl
                .fields
                .iter()
                .map(|(n, ty, _)| (n.clone(), ty.clone()))
                .collect();
            return SmeltType::Record {
                fields,
                name: Some(trimmed.to_string()),
            };
        }
        // Not in registry — synthesise a shell Record with the name (unknown fields).
        return SmeltType::Record {
            fields: std::collections::BTreeMap::new(),
            name: Some(trimmed.to_string()),
        };
    }

    SmeltType::Unknown
}

/// Parse a field type text (scalar or simple type name) into a [`SmeltType`].
///
/// This is the leaf-level parser used by `parse_schema_arg_to_smelt_type` for
/// field types inside inline records and Map value types. It does NOT resolve
/// named records — field types must be concrete scalar types at this level.
fn parse_field_type_text(type_text: &str) -> SmeltType {
    use smelt_types::TypeConstraint;
    use DataType as DT;
    match type_text.trim() {
        "Text" | "Varchar" => SmeltType::Expr(TypeConstraint::Concrete(DT::Text)),
        "Integer" | "Int" | "BigInt" => SmeltType::Expr(TypeConstraint::Concrete(DT::Integer)),
        "Float" | "Double" => SmeltType::Expr(TypeConstraint::Concrete(DT::Float)),
        "Boolean" | "Bool" => SmeltType::Expr(TypeConstraint::Concrete(DT::Boolean)),
        "Date" => SmeltType::Expr(TypeConstraint::Concrete(DT::Date)),
        "Timestamp" => SmeltType::Expr(TypeConstraint::Concrete(DT::Timestamp {
            with_timezone: false,
        })),
        _ => SmeltType::Unknown,
    }
}

/// Split `K, V` from the inner string of a `Map<K, V>` at the first top-level comma.
fn split_map_type_args(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Infer the [`SmeltType`] synthesised by a `smelt.config.load_yaml` or
/// `smelt.config.load_json` call expression.
///
/// Returns `Some(schema_type)` on the happy path (path is a literal, schema is
/// admissible). Returns `None` when the call is not a `load_yaml`/`load_json`
/// call, the path argument is missing, or the schema argument is missing.
///
/// This function is the pure type-synthesis layer; it does not emit diagnostics.
/// The diagnostic layer (`check_loader_call_diagnostics`) handles error emission.
///
/// Per spec §"Loader value materialisation" rule 1: the call's synthesised type
/// is the declared schema's type. `TypedColumn` (the return of `infer_expression_type`)
/// only carries a `DataType` projection (`DataType::Unknown` for meta-types), so
/// this function returns the full `SmeltType` directly.
///
/// Pure — no Salsa dependency, no I/O.
pub fn infer_loader_call_smelt_type(call: &SmeltPathCall, ctx: &TypeContext) -> Option<SmeltType> {
    let segs = call.segments();
    if segs.len() != 2 || segs[0].to_lowercase() != "config" {
        return None;
    }
    let loader_name = segs[1].to_lowercase();
    if loader_name != "load_yaml" && loader_name != "load_json" {
        // load_toml is reserved → Unknown
        return None;
    }

    // Second positional argument: the schema.
    let positional_args: Vec<_> = call
        .arg_list()
        .map(|args| args.positional_args().into_iter().collect())
        .unwrap_or_default();

    let schema_expr = positional_args.get(1)?;
    let schema_text = schema_expr.syntax().text().to_string();
    let smelt_ty = parse_schema_arg_to_smelt_type(&schema_text, ctx);
    if matches!(smelt_ty, SmeltType::Unknown) {
        return None;
    }
    Some(smelt_ty)
}

/// Walk all `SMELT_PATH_CALL` descendants of `root` for
/// `smelt.config.load_yaml`, `smelt.config.load_json`, and
/// `smelt.config.load_toml` calls.
///
/// For each call:
/// - `load_toml`: emit `ConfigLoaderTomlNotYetSupported` at the call expression.
/// - `load_yaml` / `load_json`:
///   - Check path argument (literal, escape, backslash, existence).
///   - Check schema argument (admissibility).
///   - Synthesise return type (the schema's type).
///
/// `file_exists` is a pure callback that returns `true` when a workspace-
/// relative path exists. In the Salsa layer, this is backed by
/// `loader_file_exists`; in tests, it is a closure over a static set.
///
/// `registry` supplies the workspace `RecordRegistry` for named-schema lookup.
///
/// `text` is the raw source file text (used for span → range conversion; pass
/// `""` in tests where range accuracy is not needed).
///
/// Pure — no Salsa dependency.
pub fn check_loader_call_diagnostics(
    root: &smelt_parser::syntax_kind::SyntaxNode,
    file_exists: &dyn Fn(&str) -> bool,
    registry: &RecordRegistry,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        let segs = call.segments();
        // We handle: config.load_yaml, config.load_json, config.load_toml
        if segs.len() != 2 || segs[0].to_lowercase() != "config" {
            continue;
        }
        let loader_name = segs[1].to_lowercase();
        if loader_name != "load_yaml" && loader_name != "load_json" && loader_name != "load_toml" {
            continue;
        }

        let call_range = to_range(node.text_range());

        // ─── load_toml: reserved ─────────────────────────────────────────
        if loader_name == "load_toml" {
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderTomlNotYetSupported,
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
                    None,
                    None,
                ),
                range: call_range,
                code: Some(crate::DiagnosticCode::ConfigLoaderTomlNotYetSupported),
                data: None,
            });
            continue;
        }

        // ─── load_yaml / load_json ────────────────────────────────────────

        // Collect positional arguments.
        let positional_args: Vec<_> = call
            .arg_list()
            .map(|args| args.positional_args().into_iter().collect())
            .unwrap_or_default();

        // ── Argument 1: path ─────────────────────────────────────────────
        let path_arg = positional_args.first();
        match path_arg {
            None => {
                // No path argument: emit PathNotLiteral at the call.
                diags.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: crate::meta_loader_diagnostic_message(
                        crate::DiagnosticCode::ConfigLoaderPathNotLiteral,
                        Some("(missing)"),
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
                        None,
                    ),
                    range: call_range,
                    code: Some(crate::DiagnosticCode::ConfigLoaderPathNotLiteral),
                    data: None,
                });
                continue;
            }
            Some(arg_expr) => {
                // Is the path argument a string literal?
                if !crate::config_vars::is_string_literal_expr(arg_expr) {
                    let arg_range = to_range(arg_expr.syntax().text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_loader_diagnostic_message(
                            crate::DiagnosticCode::ConfigLoaderPathNotLiteral,
                            Some(&arg_expr.syntax().text().to_string()),
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
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::ConfigLoaderPathNotLiteral),
                        data: None,
                    });
                    continue;
                }

                // Extract the string value.
                let path_value = match crate::config_vars::extract_string_literal_value(arg_expr) {
                    Some(v) => v,
                    None => continue,
                };
                let arg_range = to_range(arg_expr.syntax().text_range());

                // Check path well-formedness.
                match check_loader_path(&path_value, arg_range, call_range, file_exists) {
                    LoaderPathOutcome::Backslash { path, .. } => {
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_loader_diagnostic_message(
                                crate::DiagnosticCode::ConfigLoaderPathBackslash,
                                None,
                                Some(&path),
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
                            ),
                            range: arg_range,
                            code: Some(crate::DiagnosticCode::ConfigLoaderPathBackslash),
                            data: None,
                        });
                        continue;
                    }
                    LoaderPathOutcome::EscapesWorkspace { path, .. } => {
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_loader_diagnostic_message(
                                crate::DiagnosticCode::ConfigLoaderPathEscapesWorkspace,
                                None,
                                Some(&path),
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
                            ),
                            range: arg_range,
                            code: Some(crate::DiagnosticCode::ConfigLoaderPathEscapesWorkspace),
                            data: None,
                        });
                        continue;
                    }
                    LoaderPathOutcome::FileNotFound { path, arg_range } => {
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_loader_diagnostic_message(
                                crate::DiagnosticCode::ConfigLoaderFileNotFound,
                                None,
                                Some(&path),
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
                            ),
                            range: arg_range,
                            code: Some(crate::DiagnosticCode::ConfigLoaderFileNotFound),
                            data: None,
                        });
                        continue;
                    }
                    LoaderPathOutcome::NotLiteral { .. } => {
                        // Already handled above — this arm is unreachable since we
                        // checked is_string_literal_expr first.
                        continue;
                    }
                    LoaderPathOutcome::Valid { .. } => {
                        // Path is valid. Continue to schema check.
                    }
                }

                // ── Argument 2: schema ────────────────────────────────────
                let schema_arg = positional_args.get(1);
                if let Some(schema_expr) = schema_arg {
                    let schema_range = to_range(schema_expr.syntax().text_range());
                    let schema_text = schema_expr.syntax().text().to_string();
                    if !schema_arg_text_is_admissible(&schema_text, registry) {
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_loader_diagnostic_message(
                                crate::DiagnosticCode::ConfigLoaderSchemaForbidden,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                Some(&schema_text),
                                None,
                                None,
                                None,
                                None,
                                None,
                            ),
                            range: schema_range,
                            code: Some(crate::DiagnosticCode::ConfigLoaderSchemaForbidden),
                            data: None,
                        });
                    }
                }
                // Schema is admissible (or absent) — no further diagnostics at
                // this level. The Salsa orchestration layer emits validation
                // diagnostics from `loader_resolved_value` after parsing the file.
            }
        }
    }

    diags
}

/// Walk all `SMELT_PATH_CALL` descendants of `select_stmt` whose path is
/// `columns_of`. For each such call emit:
///
/// - [`DiagnosticCode::ColumnsOfNamedArgument`] for every named argument in
///   the arg list (anchored at the named-arg span).
/// - [`DiagnosticCode::ColumnsOfRequiresTableExpr`] when the single positional
///   argument's inferred type is clearly not a `TableExpr` — specifically when
///   the argument is not a smelt-path expression and its synthesised data type
///   is a concrete non-Unknown type (e.g. an integer literal `42`).
///
/// The function always synthesises `List<ColumnRef>` (recoverable) regardless
/// of errors — that is handled by `infer_smelt_path_call_type`.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_columns_of_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only handle `smelt.columns_of(...)`.
        let segs = call.segments();
        if segs.len() != 1 || segs[0].to_lowercase() != "columns_of" {
            continue;
        }

        let arg_list = match call.arg_list() {
            Some(al) => al,
            None => continue,
        };

        // ── Named argument check ────────────────────────────────────────
        // Any NAMED_PARAM in the arg list is invalid; emit one diagnostic
        // per named arg, anchored at the named-arg node span.
        for named in arg_list.named_params() {
            let named_range = to_range(named.text_range());
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_reflection_diagnostic_message(
                    crate::DiagnosticCode::ColumnsOfNamedArgument,
                    None,
                    None,
                ),
                range: named_range,
                code: Some(crate::DiagnosticCode::ColumnsOfNamedArgument),
                data: None,
            });
        }

        // ── Positional argument type check ──────────────────────────────
        // For each positional arg, check if it's assignable to TableExpr.
        // A smelt-path expression is treated as a potential TableExpr;
        // a plain literal or expression with a concrete scalar type is not.
        for pos_arg in arg_list.positional_args() {
            // If the argument is a smelt path call OR a smelt path ref
            // (e.g. `smelt.models.orders` without parens), accept it
            // unconditionally — smelt paths are TableExpr candidates.
            if pos_arg.as_smelt_path_call().is_some() {
                continue;
            }
            // Check for SMELT_PATH_REF (value-form `smelt.<path>` without parens).
            let is_smelt_path_ref = smelt_parser::ast::SmeltPathRef::cast(pos_arg.syntax().clone())
                .is_some()
                || pos_arg
                    .syntax()
                    .children()
                    .any(|n| smelt_parser::ast::SmeltPathRef::cast(n).is_some());
            if is_smelt_path_ref {
                continue;
            }

            // Check if the arg is a bare identifier registered as TableExpr
            // (or a subtype — `ModelRef` / `SourceRef`) in the context.
            // `ModelRef <: TableExpr` and `SourceRef <: TableExpr` per Phase D
            // subtyping rules, so all three are accepted at `smelt.columns_of` call sites.
            let arg_text = pos_arg.text().trim().to_string();
            let is_bare_ident =
                !arg_text.is_empty() && arg_text.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_bare_ident {
                if let Some(smelt_ty) = ctx.lookup_function_param_smelt_type(&arg_text) {
                    if matches!(
                        smelt_ty,
                        smelt_types::signatures::SmeltType::TableExpr(_)
                            | smelt_types::signatures::SmeltType::ModelRef
                            | smelt_types::signatures::SmeltType::SourceRef
                    ) {
                        continue;
                    }
                }
            }

            // For everything else, infer the scalar type. If the type is
            // Unknown (unresolvable identifier — could be a table ref), give
            // the benefit of the doubt. Only concrete non-TableExpr types
            // (e.g. Integer, Text, Boolean) trigger the diagnostic.
            if let Some(tc) = infer_expression_type(&pos_arg, ctx) {
                let is_clearly_non_table =
                    !matches!(tc.data_type, DataType::Unknown | DataType::Null);
                if is_clearly_non_table {
                    let arg_range = to_range(pos_arg.syntax().text_range());
                    let actual_str = tc.data_type.to_string();
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::ColumnsOfRequiresTableExpr,
                            Some(&actual_str),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr),
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

/// Walk all expression descendants of `select_stmt`. For every expression of
/// the form `<qualifier>.<field>` where `<qualifier>` is registered as
/// `SmeltType::ColumnRef` in the context, check that `<field>` is in the
/// closed `COLUMN_REF_FIELDS` set. Unknown fields emit
/// [`DiagnosticCode::ColumnRefFieldUnknown`] anchored at the field-name span.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_column_ref_field_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_types::signatures::column_ref_field;

    let mut diags = Vec::new();
    // Track seen (qualifier, field) pairs to avoid duplicate diagnostics from
    // nested EXPRESSION nodes that wrap the same column reference text.
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        let expr = match smelt_parser::ast::Expr::cast(node.clone()) {
            Some(e) => e,
            None => continue,
        };

        // We're looking for `qualifier.field` expressions where the qualifier
        // is a ColumnRef-typed binding.
        let col_ref = match smelt_parser::ast::ColumnRef::from_expr(&expr) {
            Some(cr) => cr,
            None => continue,
        };

        let qualifier = match col_ref.qualifier() {
            Some(q) => q,
            None => continue, // bare identifier — not a dot access
        };

        // Is the qualifier a ColumnRef-typed binding?
        let is_column_ref = ctx
            .lookup_function_param_smelt_type(qualifier)
            .map(|ty| matches!(ty, smelt_types::signatures::SmeltType::ColumnRef))
            .unwrap_or(false);

        if !is_column_ref {
            continue;
        }

        let field_name = col_ref.name();

        // Check if the field is in the closed field set.
        if column_ref_field(field_name).is_some() {
            continue; // valid field — no diagnostic
        }

        // Deduplicate: the same qualifier.field pair may appear in multiple
        // nested EXPRESSION wrappers during the descendants walk. Only emit
        // one diagnostic per (qualifier, field) pair.
        let key = (qualifier.to_string(), field_name.to_string());
        if !seen.insert(key) {
            continue;
        }

        // Unknown field — emit ColumnRefFieldUnknown anchored at the field token span.
        // Spec invariant: the diagnostic must point at the field name token (the IDENT
        // after the DOT), not the whole expression. We re-scan the expression's syntax
        // children to find the IDENT token that follows the DOT token and use its
        // text_range(). This avoids modifying the parser AST (approach (a)).
        let field_token_range = {
            use smelt_parser::SyntaxKind;
            // Walk all tokens (not just direct children) within the expression node.
            // The DOT and IDENT tokens may be nested under inner expression sub-nodes.
            let mut after_dot = false;
            let mut found: Option<rowan::TextRange> = None;
            for token in node
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
            {
                let kind = token.kind();
                if kind == SyntaxKind::DOT {
                    after_dot = true;
                } else if kind == SyntaxKind::IDENT && after_dot {
                    found = Some(token.text_range());
                    break;
                }
            }
            // Fall back to the whole expression range if the token walk fails
            // (should not happen for well-formed qualifier.field syntax).
            found.unwrap_or_else(|| node.text_range())
        };
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_reflection_diagnostic_message(
                crate::DiagnosticCode::ColumnRefFieldUnknown,
                None,
                Some(field_name),
            ),
            range: to_range(field_token_range),
            code: Some(crate::DiagnosticCode::ColumnRefFieldUnknown),
            data: None,
        });
    }

    diags
}

// ─── Phase D: wide-reflection diagnostics and field projection ───────────────

/// Returns `true` when `expr` is a compile-time-resolvable meta-`Text` value
/// for the purpose of the `with_tag` argument check.
///
/// Accepted as compile-time Text:
/// - Bare string literals (detected by `is_string_literal_expr`).
/// - `smelt.config.var(...)` calls (the result is always a nullable `Varchar`
///   sourced at compile time).
/// - A `ModelRef` or `SourceRef` field projection to a `Text`-typed field
///   (`m.path`, `m.name`, `s.path`, `s.name`) — i.e. expressions whose inferred
///   `SmeltType` (in the meta-type layer) is `Expr<Text>` from a closed meta-record.
///
/// Rejected (returns `false`):
/// - Integer or other non-Text literals (`42`, `true`, etc.).
/// - Runtime function calls that synthesise `Expr<Text>` at SQL evaluation time
///   (e.g. `UPPER('x')`).
/// - Bare column references.
///
/// Pure — no Salsa dependency.
pub fn is_compile_time_text_arg(expr: &Expr, ctx: &TypeContext) -> bool {
    use smelt_types::signatures::{column_ref_field, model_ref_field, source_ref_field, SmeltType};

    // 1. Bare string literal — always accepted.
    if crate::config_vars::is_string_literal_expr(expr) {
        return true;
    }

    // 2. smelt.config.var(...) call — always accepted (compile-time Text).
    if let Some(path_call) = expr.as_smelt_path_call() {
        let segs = path_call.segments();
        if segs.len() == 2
            && segs[0].eq_ignore_ascii_case("config")
            && segs[1].eq_ignore_ascii_case("var")
        {
            return true;
        }
        // wide-reflection accessor calls return List<ModelRef|SourceRef>, not Text.
        return false;
    }

    // 3. A field projection `<binding>.<field>` where the binding is a
    //    ColumnRef, ModelRef, or SourceRef and the field type is Expr<Text>.
    if let Some(col_ref) = smelt_parser::ast::ColumnRef::from_expr(expr) {
        let qualifier = match col_ref.qualifier() {
            Some(q) => q,
            None => return false, // bare identifier — not a qualified field access
        };
        let field = col_ref.name();

        // Is the qualifier a meta-record-typed binding?
        if let Some(ty) = ctx.lookup_function_param_smelt_type(qualifier) {
            let field_ty_opt: Option<&SmeltType> = match ty {
                SmeltType::ColumnRef => column_ref_field(field),
                SmeltType::ModelRef => model_ref_field(field),
                SmeltType::SourceRef => source_ref_field(field),
                _ => None,
            };
            if let Some(field_ty) = field_ty_opt {
                return matches!(
                    field_ty,
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        DataType::Text
                    ))
                );
            }
        }
    }

    false
}

/// Walk all `SMELT_PATH_CALL` descendants of `select_stmt` whose path is
/// `models.<accessor>` or `sources.<accessor>`. For each:
///
/// - Emit [`DiagnosticCode::WideReflectionUnknownAccessor`] when the accessor
///   name is not in the closed set `{with_tag, all}`.
/// - For `with_tag`: emit [`DiagnosticCode::WithTagNamedArgument`] for any named
///   argument; emit [`DiagnosticCode::WithTagRequiresText`] when the positional
///   argument is not compile-time-resolvable Text.
/// - For `all`: emit [`DiagnosticCode::WideReflectionUnexpectedArgument`] for any
///   argument (positional or named).
///
/// Always synthesises the spec'd return type (recoverable) — diagnostics here do
/// not prevent downstream HOF type-checking.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_wide_reflection_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only handle `smelt.models.<accessor>` and `smelt.sources.<accessor>`.
        let segs = call.segments();
        if segs.len() != 2 {
            continue;
        }
        let ns = segs[0].to_lowercase();
        if ns != "models" && ns != "sources" {
            continue;
        }
        let accessor_name = segs[1].as_str();

        // Check closed accessor set.
        let is_with_tag = accessor_name.eq_ignore_ascii_case("with_tag");
        let is_all = accessor_name.eq_ignore_ascii_case("all");

        if !is_with_tag && !is_all {
            // Unknown accessor — emit WideReflectionUnknownAccessor at the accessor token span.
            let accessor_range = find_last_segment_range(&call, text);
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_reflection_diagnostic_message(
                    crate::DiagnosticCode::WideReflectionUnknownAccessor,
                    Some(&ns),
                    Some(accessor_name),
                ),
                range: accessor_range,
                code: Some(crate::DiagnosticCode::WideReflectionUnknownAccessor),
                data: None,
            });
            continue;
        }

        let arg_list = call.arg_list();

        if is_all {
            // `all` accepts no arguments at all.
            if let Some(al) = &arg_list {
                let full_accessor = format!("smelt.{ns}.all");
                for pos_arg in al.positional_args() {
                    let arg_range = to_range(pos_arg.syntax().text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::WideReflectionUnexpectedArgument,
                            Some(&full_accessor),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument),
                        data: None,
                    });
                }
                for named_arg in al.named_params() {
                    let arg_range = to_range(named_arg.text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::WideReflectionUnexpectedArgument,
                            Some(&full_accessor),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument),
                        data: None,
                    });
                }
            }
            continue;
        }

        // is_with_tag: check for named args and compile-time Text argument.
        if let Some(al) = &arg_list {
            // Named arguments are not supported.
            for named_arg in al.named_params() {
                let arg_range = to_range(named_arg.text_range());
                diags.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: crate::meta_reflection_diagnostic_message(
                        crate::DiagnosticCode::WithTagNamedArgument,
                        None,
                        None,
                    ),
                    range: arg_range,
                    code: Some(crate::DiagnosticCode::WithTagNamedArgument),
                    data: None,
                });
            }

            // Positional argument must be compile-time Text.
            for pos_arg in al.positional_args() {
                if !is_compile_time_text_arg(&pos_arg, ctx) {
                    // Get the actual synthesised type string for the message.
                    let actual_str = infer_expression_type(&pos_arg, ctx)
                        .map(|tc| tc.data_type.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let arg_range = to_range(pos_arg.syntax().text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::WithTagRequiresText,
                            Some(&actual_str),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::WithTagRequiresText),
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

/// Extract path segments from a `SmeltPathCall`, including keyword tokens.
///
/// Unlike `SmeltPathCall::segments()`, which only collects `IDENT` tokens,
/// this helper also collects keyword tokens (e.g. `ALL_KW` for `all`) so that
/// accessor names that happen to be SQL keywords are included. The leading
/// `smelt` IDENT token is dropped (same as `segments()`).
///
/// Find the text range of the last path segment in a `SmeltPathCall`.
///
/// For `smelt.models.bogus(...)` returns the span of `bogus`.
/// Falls back to the node's own range when a more precise span cannot be found.
fn find_last_segment_range(call: &smelt_parser::ast::SmeltPathCall, text: &str) -> crate::Range {
    use smelt_parser::SyntaxKind::IDENT;

    // Walk the path's IDENT tokens to find the last one (the accessor name).
    let path_node = call.path();
    let last_ident_range = path_node.and_then(|p| {
        p.syntax()
            .children_with_tokens()
            .filter_map(|it| {
                if let rowan::NodeOrToken::Token(t) = it {
                    if t.kind() == IDENT {
                        return Some(t.text_range());
                    }
                }
                None
            })
            .last()
    });

    let raw_range = last_ident_range.unwrap_or_else(|| call.syntax().text_range());
    if text.is_empty() {
        crate::Range {
            start: smelt_parser::ast::Position { line: 0, column: 0 },
            end: smelt_parser::ast::Position { line: 0, column: 0 },
        }
    } else {
        smelt_parser::ast::text_range_to_range(text, raw_range)
    }
}

/// Walk all expression descendants of `select_stmt`. For every expression of
/// the form `<qualifier>.<field>` where `<qualifier>` is registered as
/// `SmeltType::ModelRef` or `SmeltType::SourceRef` in the context, check that
/// `<field>` is in the closed `MODEL_REF_FIELDS` / `SOURCE_REF_FIELDS` set.
/// Unknown fields emit [`DiagnosticCode::ModelRefFieldUnknown`] /
/// [`DiagnosticCode::SourceRefFieldUnknown`] anchored at the field-name span.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_model_ref_source_ref_field_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_types::signatures::{model_ref_field, source_ref_field, SmeltType};

    let mut diags = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        let expr = match smelt_parser::ast::Expr::cast(node.clone()) {
            Some(e) => e,
            None => continue,
        };

        let col_ref = match smelt_parser::ast::ColumnRef::from_expr(&expr) {
            Some(cr) => cr,
            None => continue,
        };

        let qualifier = match col_ref.qualifier() {
            Some(q) => q,
            None => continue, // bare identifier — not a dot access
        };

        // Is the qualifier a ModelRef or SourceRef-typed binding?
        let binding_ty = ctx.lookup_function_param_smelt_type(qualifier).cloned();

        let (is_model_ref, _is_source_ref) = match &binding_ty {
            Some(SmeltType::ModelRef) => (true, false),
            Some(SmeltType::SourceRef) => (false, true),
            _ => continue,
        };

        let field_name = col_ref.name();

        // Check if the field is in the closed field set.
        let field_known = if is_model_ref {
            model_ref_field(field_name).is_some()
        } else {
            source_ref_field(field_name).is_some()
        };

        if field_known {
            continue; // valid field — no diagnostic
        }

        // Deduplicate: same (qualifier, field) pair may appear in multiple wrappers.
        let key = (qualifier.to_string(), field_name.to_string());
        if !seen.insert(key) {
            continue;
        }

        // Unknown field — emit the appropriate diagnostic anchored at the field token span.
        let field_token_range = {
            use smelt_parser::SyntaxKind::{DOT, IDENT};
            let mut found: Option<rowan::TextRange> = None;
            let mut after_dot = false;
            for child in node.children_with_tokens() {
                match child {
                    rowan::NodeOrToken::Token(t) if t.kind() == DOT => {
                        after_dot = true;
                    }
                    rowan::NodeOrToken::Token(t) if after_dot && t.kind() == IDENT => {
                        found = Some(t.text_range());
                        break;
                    }
                    _ => {}
                }
            }
            found.unwrap_or_else(|| node.text_range())
        };

        let code = if is_model_ref {
            crate::DiagnosticCode::ModelRefFieldUnknown
        } else {
            crate::DiagnosticCode::SourceRefFieldUnknown
        };
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_reflection_diagnostic_message(code, None, Some(field_name)),
            range: to_range(field_token_range),
            code: Some(code),
            data: None,
        });
    }

    diags
}

/// Infer the [`SmeltType`] of a `ModelRef` field projection `<binding>.<field>`.
///
/// Returns `Some(field_type)` when:
///   - `binding_name` is registered in `ctx` as `SmeltType::ModelRef`, AND
///   - `field_name` is in the closed `MODEL_REF_FIELDS` set.
///
/// Returns `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn infer_field_on_model_ref(
    binding_name: &str,
    field_name: &str,
    ctx: &TypeContext,
) -> Option<smelt_types::signatures::SmeltType> {
    use smelt_types::signatures::{model_ref_field, SmeltType};
    let is_model_ref = ctx
        .lookup_function_param_smelt_type(binding_name)
        .map(|ty| matches!(ty, SmeltType::ModelRef))
        .unwrap_or(false);
    if !is_model_ref {
        return None;
    }
    model_ref_field(field_name).cloned()
}

/// Infer the [`SmeltType`] of a `SourceRef` field projection `<binding>.<field>`.
///
/// Returns `Some(field_type)` when:
///   - `binding_name` is registered in `ctx` as `SmeltType::SourceRef`, AND
///   - `field_name` is in the closed `SOURCE_REF_FIELDS` set.
///
/// Returns `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn infer_field_on_source_ref(
    binding_name: &str,
    field_name: &str,
    ctx: &TypeContext,
) -> Option<smelt_types::signatures::SmeltType> {
    use smelt_types::signatures::{source_ref_field, SmeltType};
    let is_source_ref = ctx
        .lookup_function_param_smelt_type(binding_name)
        .map(|ty| matches!(ty, SmeltType::SourceRef))
        .unwrap_or(false);
    if !is_source_ref {
        return None;
    }
    source_ref_field(field_name).cloned()
}

/// Infer the [`SmeltType`] of a ColumnRef field projection `<binding>.<field>`.
///
/// Returns `Some(field_type)` when:
///   - `binding_name` is registered in `ctx` as `SmeltType::ColumnRef` via
///     `add_function_param_smelt_type`, AND
///   - `field_name` is in the closed `COLUMN_REF_FIELDS` set.
///
/// Returns `None` otherwise (unknown binding or unknown field).
///
/// Pure — no Salsa dependency.
pub fn infer_field_on_column_ref(
    binding_name: &str,
    field_name: &str,
    ctx: &TypeContext,
) -> Option<smelt_types::signatures::SmeltType> {
    use smelt_types::signatures::{column_ref_field, SmeltType};

    // Check the binding is a ColumnRef.
    let is_column_ref = ctx
        .lookup_function_param_smelt_type(binding_name)
        .map(|ty| matches!(ty, SmeltType::ColumnRef))
        .unwrap_or(false);

    if !is_column_ref {
        return None;
    }

    // Look up the field in the closed field set and clone the type.
    column_ref_field(field_name).cloned()
}
