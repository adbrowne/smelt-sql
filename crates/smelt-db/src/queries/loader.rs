//! `smelt.config.load_yaml` / `load_json` / `load_toml` loader queries.
//!
//! Salsa-tracked wrappers that read `LoaderFileInput` (raw text + existence)
//! and per-target overlay inputs from `Workspace`, then delegate to the pure
//! `loader::parse_*` / validation helpers.

use std::sync::Arc;

use smelt_parser::{self, File as AstFile};

use crate::loader;
use crate::queries::parse::parse_file;
use crate::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, LoaderFileInput, Position, Range,
    SourceFile, Workspace,
};

// ============================================================================
// Loader queries (Phase E1 Phase 5)
// ============================================================================

/// A unique identifier for a loader call site.
///
/// Constructed from the source file path, the byte offset of the
/// `smelt.config.load_yaml` / `load_json` call expression in that file,
/// the resolved loader-target path, and the schema text from the second
/// argument.
///
/// Used as the Salsa key for `loader_resolved_value`.
///
/// The key is stable across Salsa re-evaluations as long as the call site
/// itself (including its schema argument) hasn't been edited. When the call
/// site moves or the schema changes, the old key is invalidated and a new one
/// is created; Salsa evicts the cached result as expected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoaderCallSiteId {
    /// Workspace-relative path of the smelt model file containing the call.
    pub file_path: Arc<str>,
    /// Byte offset of the SMELT_PATH_CALL node in the source text.
    pub byte_offset: u32,
    /// Resolved workspace-relative path of the loaded file.
    pub loader_path: Arc<str>,
    /// The raw schema argument text (e.g. `"Cohort"`, `"{name: Text}"`,
    /// `"List<Cohort>"`). Included in the key so that a schema change
    /// (without a path or offset change) still invalidates the cached result.
    pub schema_text: Arc<str>,
}

/// The resolved (parsed + validated) value of a loader call site.
#[derive(Debug, Clone, PartialEq)]
pub struct LoaderResolvedValue {
    /// The parsed config file (contains the root `ParsedNode` with spans).
    /// `None` when the path was invalid or the file did not exist.
    pub parsed: Option<Arc<crate::loader::ParsedConfigFile>>,
    /// Validation diagnostics from parsing and schema-checking the file.
    pub diagnostics: Vec<crate::loader::LoaderDiagnostic>,
    /// The merged `MetaValue` after applying an overlay (if any).
    ///
    /// For base-only resolution (`loader_resolved_value`) this is `None`.
    /// For overlay resolution (`loader_resolved_value_with_overlay`) this
    /// holds the result of merging the base and overlay validated values
    /// according to the per-target overlay rules.
    ///
    /// Only `Some` when both the base and overlay parsed successfully and
    /// both validated against the schema without errors.
    pub merged: Option<crate::loader::MetaValue>,
}

/// Parse a loader file's text into a [`ParsedConfigFile`].
///
/// This is a Salsa tracked function so that re-parsing is memoised across
/// multiple call sites that load the same file.
///
/// Pure computation: `text` is the raw file text; `path` is the
/// workspace-relative path (for error messages); `format` identifies the
/// parser to use.
#[salsa::tracked]
pub fn loader_file_parsed(
    _db: &dyn salsa::Database,
    input: LoaderFileInput,
) -> Arc<Result<crate::loader::ParsedConfigFile, crate::loader::ConfigParseError>> {
    let text = input.text(_db);
    let path = input.path(_db);
    let exists = input.exists(_db);
    if !exists {
        // File does not exist — return a sentinel error so callers can emit FileNotFound.
        return Arc::new(Err(crate::loader::ConfigParseError {
            message: format!("file `{}` does not exist", path),
            span: crate::loader::FileSpan::zero(),
        }));
    }
    // Determine format from the file extension.
    let format = if path.ends_with(".json") {
        crate::loader::ConfigFormat::Json
    } else {
        crate::loader::ConfigFormat::Yaml
    };
    let result = match format {
        crate::loader::ConfigFormat::Yaml => crate::loader::parse_yaml(text, path),
        crate::loader::ConfigFormat::Json => crate::loader::parse_json(text, path),
    };
    Arc::new(result)
}

/// Resolve a loader call site: parse the loader file and validate it against the
/// call site's schema.
///
/// This is the Salsa-tracked query that materialises content-validation
/// diagnostics (`ConfigLoaderParseError`, `ConfigLoaderRequiredFieldMissing`,
/// `ConfigLoaderUnknownField`, `ConfigLoaderTypeMismatch`,
/// `ConfigLoaderRootShapeMismatch`, `ConfigLoaderDuplicateMapKey`,
/// `ConfigLoaderNullCoercion`).
///
/// **Design note**: Salsa 0.26 tracked functions receive `&dyn salsa::Database`
/// and cannot downcast to the concrete `Database` type. To track the dependency
/// on the loader file's text, the caller must pass the `LoaderFileInput` struct
/// directly as the second parameter — it is a Salsa input, so Salsa tracks the
/// dependency automatically. The `LoaderCallSiteId` carries the schema text (for
/// dependency tracking when the source file changes) and the loader path (for
/// diagnostic messages).
///
/// Salsa invalidates this result automatically when:
/// - `input`'s text changes (via `set_loader_file`).
/// - Any field of `call_site` changes (schema text, loader path, byte offset,
///   or file path — all embedded in the key).
///
/// The function performs **no I/O** — it reads from Salsa inputs only.
#[salsa::tracked]
pub fn loader_resolved_value(
    db: &dyn salsa::Database,
    input: LoaderFileInput,
    call_site: LoaderCallSiteId,
) -> Arc<LoaderResolvedValue> {
    // 1. Parse the file via the memoised `loader_file_parsed` query.
    let parsed_result = loader_file_parsed(db, input);
    let parsed = match parsed_result.as_ref().as_ref() {
        Err(e) => {
            // File parse error — emit a ConfigLoaderParseError diagnostic.
            let diag = crate::loader::LoaderDiagnostic {
                code: crate::DiagnosticCode::ConfigLoaderParseError,
                severity: crate::loader::LoaderDiagnosticSeverity::Error,
                message: crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderParseError,
                    None,
                    Some(&call_site.loader_path),
                    Some(&e.message),
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
                primary_span: e.span,
                secondary_call_span: None,
            };
            return Arc::new(LoaderResolvedValue {
                parsed: None,
                diagnostics: vec![diag],
                merged: None,
            });
        }
        Ok(parsed) => parsed.clone(),
    };

    // 2. Parse the schema from the call site's schema text.
    // Named records are not resolved at this layer (schema admissibility was
    // already checked by `check_loader_call_diagnostics`).
    let schema_text = &*call_site.schema_text;
    let schema_ty = parse_smelt_type_from_field_annotation(schema_text, "");

    // 3. Check schema admissibility.
    if !crate::loader::is_admissible_loader_schema(&schema_ty) {
        // Schema was forbidden — already diagnosed by `check_loader_call_diagnostics`.
        // Return empty result (no content-validation).
        return Arc::new(LoaderResolvedValue {
            parsed: Some(Arc::new(parsed)),
            diagnostics: Vec::new(),
            merged: None,
        });
    }

    // 4. Validate the parsed file against the schema.
    // The call-site range is not known here (we'd need to re-parse the source
    // file to compute it). Use a zero range as the secondary frame; the LSP
    // orchestrator fills in the actual call-site range before displaying to the
    // user.
    let call_range = crate::Range {
        start: smelt_parser::ast::Position { line: 0, column: 0 },
        end: smelt_parser::ast::Position { line: 0, column: 0 },
    };
    let result = crate::loader::validate_against_schema(&parsed, &schema_ty, call_range);

    Arc::new(LoaderResolvedValue {
        parsed: Some(Arc::new(parsed)),
        diagnostics: result.diagnostics,
        merged: None,
    })
}

/// Resolve a loader call site with a per-target overlay:
/// parses and validates both the base file and the overlay file, then merges
/// the validated values according to the per-target overlay rules.
///
/// **Overlay rules (spec §Per-target overlay):**
/// - Record root: per-field replace (overlay fields replace base; absent fields taken from base).
/// - List root: overlay replaces base entirely.
/// - Map root: per-key replace (overlay keys replace base; absent keys taken from base).
///
/// **Diagnostics anchoring:**
/// - Base-file diagnostics are anchored at the base file's rows.
/// - Overlay-file diagnostics are anchored at the **overlay file's rows** — not the base or call
///   site. This is the key spec invariant tested by `overlay_validation_failure_anchors_at_overlay_row`.
///
/// If the overlay file does not exist (`exists == false`), this is a no-op and returns
/// the same result as `loader_resolved_value`.
///
/// The function performs **no I/O** — it reads from Salsa inputs only.
#[salsa::tracked]
pub fn loader_resolved_value_with_overlay(
    db: &dyn salsa::Database,
    base_input: LoaderFileInput,
    overlay_input: LoaderFileInput,
    call_site: LoaderCallSiteId,
) -> Arc<LoaderResolvedValue> {
    // Parse base.
    let base_parsed_result = loader_file_parsed(db, base_input);
    let base_parsed = match base_parsed_result.as_ref().as_ref() {
        Err(e) => {
            let diag = crate::loader::LoaderDiagnostic {
                code: crate::DiagnosticCode::ConfigLoaderParseError,
                severity: crate::loader::LoaderDiagnosticSeverity::Error,
                message: crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderParseError,
                    None,
                    Some(&call_site.loader_path),
                    Some(&e.message),
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
                primary_span: e.span,
                secondary_call_span: None,
            };
            return Arc::new(LoaderResolvedValue {
                parsed: None,
                diagnostics: vec![diag],
                merged: None,
            });
        }
        Ok(p) => p.clone(),
    };

    // Parse schema.
    let schema_text = &*call_site.schema_text;
    let schema_ty = parse_smelt_type_from_field_annotation(schema_text, "");

    if !crate::loader::is_admissible_loader_schema(&schema_ty) {
        return Arc::new(LoaderResolvedValue {
            parsed: Some(Arc::new(base_parsed)),
            diagnostics: Vec::new(),
            merged: None,
        });
    }

    let call_range = crate::Range {
        start: smelt_parser::ast::Position { line: 0, column: 0 },
        end: smelt_parser::ast::Position { line: 0, column: 0 },
    };

    // Validate base.
    let base_result = crate::loader::validate_against_schema(&base_parsed, &schema_ty, call_range);

    // Check whether the overlay file exists.
    let overlay_exists = overlay_input.exists(db);
    if !overlay_exists {
        // No overlay — return base result unchanged (with merged: None).
        return Arc::new(LoaderResolvedValue {
            parsed: Some(Arc::new(base_parsed)),
            diagnostics: base_result.diagnostics,
            merged: None,
        });
    }

    // Parse overlay.
    let overlay_parsed_result = loader_file_parsed(db, overlay_input);
    let overlay_parsed = match overlay_parsed_result.as_ref().as_ref() {
        Err(e) => {
            let overlay_path = overlay_input.path(db);
            let diag = crate::loader::LoaderDiagnostic {
                code: crate::DiagnosticCode::ConfigLoaderParseError,
                severity: crate::loader::LoaderDiagnosticSeverity::Error,
                message: crate::meta_loader_diagnostic_message(
                    crate::DiagnosticCode::ConfigLoaderParseError,
                    None,
                    Some(overlay_path.as_ref()),
                    Some(&e.message),
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
                primary_span: e.span,
                secondary_call_span: None,
            };
            // Return base diagnostics + overlay parse error.
            let mut diags = base_result.diagnostics;
            diags.push(diag);
            return Arc::new(LoaderResolvedValue {
                parsed: Some(Arc::new(base_parsed)),
                diagnostics: diags,
                merged: None,
            });
        }
        Ok(p) => p.clone(),
    };

    // Validate overlay against the same schema in **partial** mode.
    // Partial mode skips the missing-required-field check: the overlay provides
    // replacements for a subset of the base's fields; absent fields come from the base.
    let overlay_result =
        crate::loader::validate_against_schema_partial(&overlay_parsed, &schema_ty, call_range);

    // Combine diagnostics — base diags + overlay diags (each carries its own row span).
    let mut all_diags = base_result.diagnostics;
    all_diags.extend(overlay_result.diagnostics.iter().cloned());

    // Only merge values when both sides validated without errors.
    let merged = if all_diags
        .iter()
        .any(|d| d.severity == crate::loader::LoaderDiagnosticSeverity::Error)
    {
        None
    } else {
        // `ValidationResult.value` is always present (best-effort even on error paths).
        Some(crate::loader::merge_values(
            base_result.value,
            overlay_result.value,
            &schema_ty,
        ))
    };

    Arc::new(LoaderResolvedValue {
        parsed: Some(Arc::new(base_parsed)),
        diagnostics: all_diags,
        merged,
    })
}

/// Collect all top-level `smelt.record` declarations from all files in the workspace.
///
/// Returns a `Vec<SmeltRecordDeclaration>` (in file-order, declarations within
/// each file in textual order). The Salsa memoisation ensures this runs only
/// when the workspace's file set or file contents change.
///
/// Pure computation: reads the parse tree via `parse_file` and walks the AST.
#[salsa::tracked]
pub fn smelt_record_declarations(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<Vec<smelt_types::signatures::SmeltRecordDeclaration>> {
    use smelt_parser::ast::SmeltRecordDecl;
    use smelt_parser::SyntaxKind::SMELT_RECORD_DECL;
    use smelt_types::signatures::SmeltRecordDeclaration;

    let mut all_decls: Vec<SmeltRecordDeclaration> = Vec::new();

    for file in workspace.files(db).iter() {
        let path = file.path(db);
        let path_str: Arc<str> = Arc::from(path.to_str().unwrap_or(""));
        let text = file.text(db);
        let parse = parse_file(db, *file);
        let syntax = parse.syntax();

        // Walk all SMELT_RECORD_DECL nodes in the file.
        for node in syntax.descendants() {
            if node.kind() != SMELT_RECORD_DECL {
                continue;
            }
            let decl_node = match SmeltRecordDecl::cast(node.clone()) {
                Some(d) => d,
                None => continue,
            };
            let name = match decl_node.name() {
                Some(n) => n,
                None => continue,
            };
            // Find the name-token span.
            let name_span = {
                // The name is the third IDENT token in the SMELT_RECORD_DECL node.
                let idents: Vec<_> = node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == smelt_parser::SyntaxKind::IDENT)
                    .collect();
                idents
                    .get(2)
                    .map(|t| t.text_range())
                    .unwrap_or_else(|| rowan::TextRange::new(0.into(), 0.into()))
            };

            // Collect field definitions.
            let mut fields: Vec<(
                String,
                smelt_types::signatures::SmeltType,
                smelt_parser::TextRange,
            )> = Vec::new();

            for field in decl_node.fields() {
                let field_name = match field.name() {
                    Some(n) => n,
                    None => continue,
                };
                // The type reference text.
                let type_text = field
                    .type_ref()
                    .map(|tr| tr.syntax().text().to_string())
                    .or_else(|| {
                        field
                            .inline_record_type()
                            .map(|r| r.syntax().text().to_string())
                    })
                    .unwrap_or_default();
                let type_span = field
                    .type_ref()
                    .map(|tr| tr.syntax().text_range())
                    .or_else(|| field.inline_record_type().map(|r| r.syntax().text_range()))
                    .unwrap_or_else(|| rowan::TextRange::new(0.into(), 0.into()));

                // Parse the type text into a SmeltType.
                let smelt_ty = parse_smelt_type_from_field_annotation(&type_text, text);
                fields.push((field_name, smelt_ty, type_span));
            }

            all_decls.push(SmeltRecordDeclaration {
                name,
                fields,
                name_span,
                source_path: path_str.clone(),
            });
        }
    }

    Arc::new(all_decls)
}

/// Parse a field type annotation text into a [`SmeltType`].
///
/// This handles the common cases: scalar types (`Text`, `Integer`, etc.),
/// `List<T>`, `Map<K, V>`, named record references, and inline record types.
///
/// Pure — no Salsa dependency, no I/O.
fn parse_smelt_type_from_field_annotation(
    type_text: &str,
    _file_text: &str,
) -> smelt_types::signatures::SmeltType {
    use smelt_types::signatures::{SmeltType, TypeConstraint};
    use smelt_types::DataType;

    let trimmed = type_text.trim();

    // Scalar types
    match trimmed {
        "Text" | "Varchar" => return SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        "Integer" | "Int" | "BigInt" => {
            return SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))
        }
        "Float" | "Double" => return SmeltType::Expr(TypeConstraint::Concrete(DataType::Float)),
        "Boolean" | "Bool" => return SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
        "Date" => return SmeltType::Expr(TypeConstraint::Concrete(DataType::Date)),
        "Timestamp" => {
            return SmeltType::Expr(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: false,
            }))
        }
        _ => {}
    }

    // List<T>
    if let Some(inner) = trimmed
        .strip_prefix("List<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let inner_ty = parse_smelt_type_from_field_annotation(inner, _file_text);
        return SmeltType::List(Box::new(inner_ty));
    }

    // Map<K, V>
    if let Some(inner) = trimmed
        .strip_prefix("Map<")
        .and_then(|s| s.strip_suffix('>'))
    {
        // Split on the first comma not inside angle brackets.
        if let Some((k_str, v_str)) = split_type_args(inner) {
            let k_ty = parse_smelt_type_from_field_annotation(k_str.trim(), _file_text);
            let v_ty = parse_smelt_type_from_field_annotation(v_str.trim(), _file_text);
            return SmeltType::Map {
                key: Box::new(k_ty),
                value: Box::new(v_ty),
            };
        }
    }

    // Inline record `{ f: T, ... }` — parse field list.
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let body = &trimmed[1..trimmed.len() - 1];
        let mut fields = std::collections::BTreeMap::new();
        for part in body.split(',') {
            let part = part.trim();
            if let Some((name, ty_text)) = part.split_once(':') {
                let name = name.trim().to_string();
                let ty = parse_smelt_type_from_field_annotation(ty_text.trim(), _file_text);
                fields.insert(name, ty);
            }
        }
        return SmeltType::Record { fields, name: None };
    }

    // Named record reference: an identifier that's not a known scalar.
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return SmeltType::Record {
            fields: std::collections::BTreeMap::new(),
            name: Some(trimmed.to_string()),
        };
    }

    // Fallback: Unknown
    SmeltType::Unknown
}

/// Split `K, V` from a `Map<K, V>` inner string at the first top-level comma.
fn split_type_args(s: &str) -> Option<(&str, &str)> {
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

/// Wire loader call diagnostics into `check_file_diagnostics`.
///
/// This function emits two classes of diagnostics:
///
/// **Path/schema admissibility** (from the pure `check_loader_call_diagnostics`):
/// `ConfigLoaderPathNotLiteral`, `ConfigLoaderPathBackslash`,
/// `ConfigLoaderPathEscapesWorkspace`, `ConfigLoaderFileNotFound`,
/// `ConfigLoaderSchemaForbidden`, `ConfigLoaderTomlNotYetSupported`.
///
/// **Content-validation** (from `loader::validate_against_schema` for valid call
/// sites): `ConfigLoaderParseError`, `ConfigLoaderRequiredFieldMissing`,
/// `ConfigLoaderUnknownField`, `ConfigLoaderTypeMismatch`,
/// `ConfigLoaderRootShapeMismatch`, `ConfigLoaderDuplicateMapKey`,
/// `ConfigLoaderNullCoercion`.
///
/// The `get_loader_text` callback returns the raw text (and existence flag) of a
/// workspace-relative loader file path. In production the LSP seeds
/// `LoaderFileInput`s before running diagnostics; callers should look up inputs
/// and pass their text here. In test contexts the callback can return a static
/// fixture. Passing `None` for a path causes content-validation to be skipped
/// for that call site (path/schema admissibility is still checked).
///
/// The production path uses the Salsa-memoized `loader_resolved_value` query via
/// `workspace.loader_files(db)` — registered inputs are consulted automatically.
/// The `_with_content` variant is kept as a test convenience for callers that
/// supply file text out-of-band (e.g. unit tests that don't register Salsa inputs).
pub fn loader_call_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    // Copy the loader-file input list out of Workspace before walking the AST.
    // The copy avoids holding a reference to a Salsa field across other db calls,
    // which Salsa's borrow checker does not permit.
    let loader_inputs: Vec<LoaderFileInput> = workspace.loader_files(db).to_vec();

    if loader_inputs.is_empty() {
        // No loader files registered — fall back to the no-op path (path/schema
        // admissibility checks only, filesystem fallback for existence).
        return loader_call_diagnostics_for_file_with_content(db, workspace, file, &|_path| None);
    }

    // Build a path→input map for O(1) lookup per call site.
    let mut input_by_path: std::collections::HashMap<Arc<str>, LoaderFileInput> =
        std::collections::HashMap::new();
    for input in &loader_inputs {
        input_by_path.insert(input.path(db).clone(), *input);
    }

    // Use the _with_content variant, providing a closure that looks up text
    // from the registered Salsa inputs.
    loader_call_diagnostics_for_file_with_content(db, workspace, file, &|path| {
        input_by_path
            .get(path)
            .map(|inp| (inp.text(db).clone(), inp.exists(db)))
    })
}

/// Callback type for `loader_call_diagnostics_for_file_with_content`.
///
/// Returns `Some((text, exists))` for a registered loader file, or `None` if
/// the file is not available in the caller's context.
pub type LoaderTextFn<'a> = &'a dyn Fn(&str) -> Option<(Arc<str>, bool)>;

/// Extended form of `loader_call_diagnostics_for_file` that also performs
/// content-validation when `get_loader_text(path)` returns `Some((text, exists))`.
///
/// Callers that have access to the concrete `Database` (e.g. the LSP layer) can
/// provide a lookup callback; others fall back to `loader_call_diagnostics_for_file`.
pub fn loader_call_diagnostics_for_file_with_content(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    get_loader_text: LoaderTextFn<'_>,
) -> Vec<Diagnostic> {
    let text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    // Collect the record registry from the workspace.
    let decls = smelt_record_declarations(db, workspace);
    let (registry, _sentinels) = type_inference::record_registry_for_workspace(&decls);

    // Build the file_exists callback from the get_loader_text closure.
    // If the caller didn't provide loader text, fall back to a filesystem check.
    let file_exists = |path: &str| -> bool {
        if let Some((_, exists)) = get_loader_text(path) {
            return exists;
        }
        // Fallback: filesystem check (best-effort when no Salsa input registered).
        let project_root = file.project_root(db);
        let full_path = project_root.join(path);
        full_path.exists()
    };

    // Step 1: path/schema admissibility diagnostics (pure).
    let mut diags =
        type_inference::check_loader_call_diagnostics(&syntax, &file_exists, &registry, text);

    // Step 2: content-validation for each valid call site.
    // Walk the syntax tree again to find `load_yaml`/`load_json` calls that
    // passed path + schema checks, then parse and validate the loaded file.
    {
        use smelt_parser::ast::SmeltPathCall;
        use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

        let to_range = |range: rowan::TextRange| -> crate::Range {
            smelt_parser::ast::text_range_to_range(text, range)
        };

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
            let loader_name = segs[1].to_lowercase();
            if loader_name != "load_yaml" && loader_name != "load_json" {
                continue;
            }

            // Get positional args.
            let positional_args: Vec<_> = call
                .arg_list()
                .map(|args| args.positional_args().into_iter().collect())
                .unwrap_or_default();

            // Need both path and schema arguments.
            let path_arg = match positional_args.first() {
                Some(a) => a,
                None => continue,
            };
            let schema_arg = match positional_args.get(1) {
                Some(a) => a,
                None => continue,
            };

            // Path must be a string literal.
            if !crate::config_vars::is_string_literal_expr(path_arg) {
                continue;
            }
            let path_value = match crate::config_vars::extract_string_literal_value(path_arg) {
                Some(v) => v,
                None => continue,
            };

            // Schema must be admissible.
            let schema_text = schema_arg.syntax().text().to_string();
            if !type_inference::schema_arg_text_is_admissible(&schema_text, &registry) {
                continue;
            }

            // Look up the loader file text.
            let (loader_text, loader_exists) = match get_loader_text(&path_value) {
                Some(t) => t,
                None => continue, // No text provided — skip content-validation.
            };
            if !loader_exists {
                continue; // File doesn't exist — path check already emitted FileNotFound.
            }

            // Parse the file.
            let format = if path_value.ends_with(".json") {
                crate::loader::ConfigFormat::Json
            } else {
                crate::loader::ConfigFormat::Yaml
            };
            let parsed_result = match format {
                crate::loader::ConfigFormat::Yaml => {
                    crate::loader::parse_yaml(&loader_text, &path_value)
                }
                crate::loader::ConfigFormat::Json => {
                    crate::loader::parse_json(&loader_text, &path_value)
                }
            };
            let parsed = match parsed_result {
                Err(e) => {
                    // Parse error — emit ConfigLoaderParseError.
                    diags.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: crate::meta_loader_diagnostic_message(
                            DiagnosticCode::ConfigLoaderParseError,
                            None,
                            Some(&path_value),
                            Some(&e.message),
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
                        range: to_range(path_arg.syntax().text_range()),
                        code: Some(DiagnosticCode::ConfigLoaderParseError),
                        data: None,
                    });
                    continue;
                }
                Ok(p) => p,
            };

            // Parse the schema type.
            let schema_ty = parse_smelt_type_from_field_annotation(&schema_text, "");
            if !crate::loader::is_admissible_loader_schema(&schema_ty) {
                continue; // Already diagnosed above.
            }

            // Validate the file against the schema.
            let call_range = to_range(node.text_range());
            let result = crate::loader::validate_against_schema(&parsed, &schema_ty, call_range);
            for loader_diag in result.diagnostics {
                // Convert to a `Diagnostic`.
                // Primary span is the loader file row; we anchor it at the call site
                // range (secondary frame) since we don't have a source range for the
                // loader file's rows in the LSP protocol.
                let severity = match loader_diag.severity {
                    crate::loader::LoaderDiagnosticSeverity::Error => DiagnosticSeverity::Error,
                    crate::loader::LoaderDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
                };
                diags.push(Diagnostic {
                    severity,
                    message: loader_diag.message,
                    range: call_range,
                    code: Some(loader_diag.code),
                    data: None,
                });
            }
        }
    }

    diags
}
