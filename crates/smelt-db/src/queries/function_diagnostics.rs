//! Per-file function diagnostics: duplicate names, body checks, frontmatter,
//! backends-widening, call-cycle, and `smelt.fn.*` call-site validation.
//!
//! These wrappers gather signatures/bodies via Salsa, then delegate to pure
//! helpers in `function_body_check`, `backends`, and `provenance_validator`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use smelt_parser::{self, ast::SmeltPathRef, File as AstFile};
use smelt_types::signatures::FunctionSig;
use smelt_types::{DataType, TypedColumn};

use crate::queries::functions::file_signature_inputs;
use crate::queries::parse::parse_file;
use crate::queries::project::{project_paths, sources_config};
use crate::queries::schema::{
    resolved_model_schema, type_context, RefSchemaProvider, SalsaRefSchemaProvider,
};
use crate::{
    backends, find_project, function_body_check, function_call_cycle_fn_ids, resolve_function,
    resolve_ref_path, type_inference, Diagnostic, DiagnosticCode, DiagnosticSeverity, Position,
    Range, RefKind, SourceFile, TypeContext, Workspace,
};

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
            // Phase 10: Every `smelt.extern` whose name collides with a
            // built-in in the canonical registry is an error. Checked before
            // the duplicate-user-definition check so externs always surface
            // the registry-collision message (more actionable than "already
            // defined in <other extern>" when both are shadowing the same
            // built-in).
            if sig.origin == smelt_types::SigOrigin::Extern
                && smelt_types::BuiltinRegistry::resolve(&sig.name).is_some()
            {
                diagnostics.push((
                    path.clone(),
                    Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Function `{}` is a built-in and cannot be redeclared with `smelt.extern`",
                            sig.name
                        ),
                        range: sig.name_range,
                        code: Some(DiagnosticCode::ExternCollidesWithBuiltin),
                        data: None,
                    },
                ));
                // Still fall through to the seen-map tracking so a second
                // extern with the same name also flags DuplicateFunctionDefinition.
            }

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

/// Per-file diagnostics for `smelt.define` bodies (Phase 5).
///
/// For each function declared in `file`, invokes the pure
/// [`function_body_check::check_function_body`] against the extracted
/// [`FunctionSig`] and the body AST. Emitted diagnostic codes:
///   - [`DiagnosticCode::DuplicateParameterName`]
///   - [`DiagnosticCode::UnknownIdentifier`]
///   - [`DiagnosticCode::FunctionBodyTypeMismatch`]
///
/// Pure-function-rule note: this helper is the thin Salsa wrapper; all logic
/// lives in the pure `function_body_check::check_function_body`. It reads
/// `parse_file` (for the body AST) and `file_signature_inputs` (for the
/// signatures). Body-only edits re-run this query but do not invalidate the
/// signature query, preserving §20H.
pub fn function_body_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::{SmeltType, SmeltTypeParseError};
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw).to_string();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };
    let sigs = file_signature_inputs(db, file);
    // Phase 13 deferred sorts: `AggExpr`, `WindowExpr`, and `SelectItems`
    // still produce `UnsupportedSort` at signature-parse time (Phases
    // 17+). Skip their bodies here to keep `example_diagnostics` green.
    // Phase 15: bare `TableExpr` is a valid sort whose body check must
    // happen at call-site expansion (no caller schema is available
    // here), so we also skip signature-time body checking for defines
    // with any `TableExpr` parameter.
    let has_deferred_phase13_param = |sig: &FunctionSig| -> bool {
        sig.params.iter().any(|p| match &p.type_ref {
            Some(Err(SmeltTypeParseError::UnsupportedSort { sort, .. }))
                if matches!(
                    sort.as_str(),
                    "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems"
                ) =>
            {
                true
            }
            Some(Ok(SmeltType::TableExpr(_))) => true,
            _ => false,
        })
    };

    // Phase 26: Build closures for nested Tier 1 expansion, mirroring the
    // closure setup in `smelt_fn_call_diagnostics_for_file`. When the Tier 2
    // body contains a `smelt.functions.*` call to a Tier 1 callee, we expand
    // it using the Tier 2 context's concrete parameter types so errors cascade
    // to the Tier 2 body check site with full frame stacks.
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));

    let sig_lookup = |name: &str| -> Option<FunctionSig> {
        resolve_function(db, workspace, name.to_string()).map(|arc| (*arc).clone())
    };

    let builtin_lookup = |name: &str| -> Option<&'static smelt_types::signatures::Signature> {
        smelt_types::BuiltinRegistry::resolve(name)
    };

    let lub = |a: &DataType, b: &DataType| -> DataType {
        let lhs = TypedColumn {
            data_type: a.clone(),
            nullable: true,
        };
        let rhs = TypedColumn {
            data_type: b.clone(),
            nullable: true,
        };
        type_inference::promote_types(&lhs, &rhs).data_type
    };

    // Phase 41: short-circuit body cascade for cycle members so the
    // existing nested-call body re-walk does not infinite-recurse on a
    // function-call cycle.
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    let body_lookup = |sig: &FunctionSig| -> Option<(String, function_body_check::BodyShape)> {
        if sig.origin == smelt_types::SigOrigin::Extern {
            return None;
        }
        if cycle_set.contains(&sig.name) {
            return None;
        }
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                let f_text = f.text(db);
                let f_clean = smelt_parser::strip_frontmatter(f_text).to_string();
                let f_parse = parse_file(db, *f);
                let f_syntax = f_parse.syntax();
                if let Some(ast) = AstFile::cast(f_syntax) {
                    for define in ast.defines() {
                        if define.name().as_deref() == Some(&sig.name) {
                            if let Some(body) = define.body() {
                                if let Some(select_stmt) = body.select_stmt() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Select(select_stmt),
                                    ));
                                }
                                if let Some(body_expr) = body.expression() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Expression(body_expr),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    };

    let decl_lookup = |sig: &smelt_types::signatures::FunctionSig| -> Option<std::path::PathBuf> {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                return Some(f.path(db).clone());
            }
        }
        None
    };

    let tableexpr_schema_lookup = |_arg_expr: &smelt_parser::ast::Expr,
                                   _ctx: &TypeContext|
     -> Option<Vec<(String, TypedColumn)>> {
        // Tier 2 body checks have no call-site schema for TableExpr params.
        // Those are skipped by `has_deferred_phase13_param` above anyway.
        None
    };

    let table_ref_schema_lookup =
        |_table_ref: &smelt_parser::ast::TableRef| -> Option<Vec<(String, TypedColumn)>> {
            // Tier 2 body checks don't expand TableExpr-param bodies, so the
            // join-alias visitor never runs on this path.
            None
        };

    let default_type_lookup = |sig: &FunctionSig, param_name: &str| -> Option<DataType> {
        let decl_path = decl_lookup(sig)?;
        let f = files.iter().find(|f| f.path(db) == &decl_path)?;
        let f_parse = parse_file(db, *f);
        let ast = AstFile::cast(f_parse.syntax())?;
        for define in ast.defines() {
            if define.name().as_deref() != Some(&sig.name) {
                continue;
            }
            let Some(param_list) = define.param_list() else {
                continue;
            };
            for p in param_list.params() {
                if p.name().as_deref() != Some(param_name) {
                    continue;
                }
                let default_expr = p.default_value_expr()?;
                let empty_ctx = TypeContext::new();
                return type_inference::infer_expression_type(&default_expr, &empty_ctx)
                    .map(|t| t.data_type);
            }
        }
        None
    };

    // TB-5: see the parallel closure in `smelt_fn_call_diagnostics_for_file`.
    // This is the body-cascade variant — when a Tier 2 body contains a
    // `smelt.<dir>.<name>(...)` call, we need to enforce the same path-prefix
    // rule. Builds an identical validator over the workspace's files.
    //
    // Phase 2 (unified-paths): strip the matching scan-root prefix from the
    // file's directory so that a function in `models/fn.sql` is addressable
    // as `smelt.fn_name()` (empty dir_segments) rather than requiring
    // `smelt.models.fn_name()`.
    let scan_roots_for_body: Arc<Vec<String>> = {
        let proj_root = file.project_root(db).clone();
        match find_project(db, workspace, &proj_root) {
            Some(p) => project_paths(db, p),
            None => Arc::new(Vec::new()),
        }
    };
    let path_prefix_validator = |dir_segments: &[String], name: &str| -> bool {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if !sigs.iter().any(|s| s.name == name) {
                continue;
            }
            let abs_path = f.path(db);
            let proj_root = f.project_root(db);
            let Ok(rel) = abs_path.strip_prefix(proj_root) else {
                continue;
            };
            let effective_dir = rel.parent().unwrap_or(std::path::Path::new(""));
            // Strip the first matching scan-root prefix.
            let stripped_dir = scan_roots_for_body
                .iter()
                .find_map(|sr| effective_dir.strip_prefix(sr.as_str()).ok())
                .unwrap_or(effective_dir);
            let file_dir_segments: Vec<String> = stripped_dir
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
                .collect();
            if file_dir_segments == dir_segments {
                return true;
            }
        }
        false
    };

    let nested_handler = |call: &smelt_parser::ast::SmeltPathCall,
                          nested_ctx: &TypeContext,
                          nested_text: &str|
     -> Vec<Diagnostic> {
        function_body_check::check_smelt_path_call(
            call,
            nested_ctx,
            nested_text,
            &sig_lookup,
            &builtin_lookup,
            &lub,
            &body_lookup,
            &decl_lookup,
            &tableexpr_schema_lookup,
            &default_type_lookup,
            &table_ref_schema_lookup,
            &|_: &smelt_parser::ast::SmeltPathCall| None,
            &path_prefix_validator,
        )
    };

    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else {
            continue;
        };
        let Some(sig) = sigs.iter().find(|s| s.name == name) else {
            continue;
        };
        if has_deferred_phase13_param(sig) {
            continue;
        }
        // Phase 41: skip body checks for cycle members. The cycle pre-pass
        // emits `FunctionCallCycle` for every participant; running the body
        // checker would risk re-entering the same cycle through nested
        // expansions even with `body_lookup`'s guard, and we already know
        // the diagnostic is wrong-shaped (the body itself is fine, the call
        // graph is not).
        if cycle_set.contains(&name) {
            continue;
        }
        let Some(body) = define.body() else {
            continue;
        };
        if let Some(select_stmt) = body.select_stmt() {
            // Phase C: SELECT-body functions without TableExpr params can be
            // checked at definition time. `has_deferred_phase13_param` already
            // skipped TableExpr/SelectItems params above, so any SELECT-body
            // function that reaches here has only scalar/ColumnRef params.
            // Run `check_function_select_body` with a seeded param context to
            // surface `ColumnRefFieldUnknown` (and other SELECT-body codes)
            // anchored at the function file, not deferred to call-site.
            let body_ctx = function_body_check::seed_param_context(&sig.params);
            let no_op_handler = |_call: &smelt_parser::ast::SmeltPathCall,
                                 _ctx: &type_inference::TypeContext,
                                 _text: &str|
             -> Vec<Diagnostic> { Vec::new() };
            out.extend(function_body_check::check_function_select_body(
                sig,
                &select_stmt,
                &clean_text,
                &body_ctx,
                &no_op_handler,
                None,
            ));
            // Phase C: also run the HOF ColumnRef field dispatcher so that
            // `map(fn c => c.invalid, smelt.columns_of(t))` inside a function
            // SELECT body emits `ColumnRefFieldUnknown` at definition time.
            out.extend(function_body_check::check_hof_column_ref_field_diagnostics(
                &select_stmt,
                &clean_text,
            ));
            // Phase D: run the HOF ModelRef/SourceRef field dispatcher and
            // the wide-reflection accessor checker so that
            // `map(smelt.models.with_tag('x'), fn m => m.bogus)` emits
            // `ModelRefFieldUnknown` and `smelt.models.with_tag(42)` emits
            // `WithTagRequiresText` inside a function SELECT body.
            out.extend(
                function_body_check::check_hof_model_ref_source_ref_field_diagnostics(
                    &select_stmt,
                    &clean_text,
                ),
            );
            continue;
        }
        let Some(body_expr) = body.expression() else {
            continue;
        };
        // Phase 26: Use `check_function_body_with_expansion` for Tier 2/3
        // functions so that nested `smelt.fn.*` calls to Tier 1 callees are
        // expanded inline using the Tier 2 context's concrete parameter types.
        // For Tier 1 functions (unannotated), `check_function_body` already
        // runs expansion at call-site only; the nested handler still handles
        // Tier 1 → Tier 1 chains correctly via the same guard in
        // `check_smelt_path_call`.
        out.extend(function_body_check::check_function_body_with_expansion(
            sig,
            &body_expr,
            &clean_text,
            &nested_handler,
        ));
        // Phase 24: Tier 3 return type check.
        if sig.tier == smelt_types::signatures::Tier::Three {
            out.extend(function_body_check::check_tier3_return_type(
                sig,
                &body_expr,
                &clean_text,
            ));
        }
    }
    out
}

/// Final backend set for a function in `file`, applying the narrow-only
/// rule (§16 #23).
///
/// Steps:
///   1. Look up the function's [`FunctionSig`] in this file.
///   2. For defines, walk the body and intersect each nested
///      `smelt.fn.*` callee's declared set + each `<backend>.<foo>` SQL
///      call into a running inferred set.
///   3. Apply the narrow-only rule: declared ⊆ inferred.
///
/// Returns `None` when no signature with the given name exists in the
/// file. When the narrow rule is violated the query still returns a
/// best-effort set (the inferred one) so downstream call-site checks
/// don't cascade — the diagnostic is emitted separately by
/// `backends_widening_diagnostics_for_file`.
///
/// Pure-function-rule note: the heavy lifting lives in
/// [`backends::infer_body_backends`] / [`backends::apply_narrow_rule`].
/// This wrapper builds a signature-lookup closure over the workspace
/// and re-parses the body via `parse_file`.
pub fn function_backends(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    name: String,
) -> Option<smelt_types::BackendSet> {
    let sig = file_signature_inputs(db, file)
        .iter()
        .find(|s| s.name == name)
        .cloned()?;
    Some(compute_function_backends(db, workspace, file, &sig))
}

/// Non-Salsa helper: compute the final backend set for `sig` in `file`
/// using the given workspace for cross-file `smelt.fn.*` lookup.
fn compute_function_backends(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    sig: &FunctionSig,
) -> smelt_types::BackendSet {
    use smelt_types::BackendSet;
    if sig.origin == smelt_types::SigOrigin::Extern {
        return backends::resolve_backends(sig, None).unwrap_or(BackendSet::All);
    }

    // Walk the body to infer.
    let inferred =
        body_inferred_backends(db, workspace, file, &sig.name).unwrap_or(BackendSet::All);
    backends::resolve_backends(sig, Some(inferred.clone())).unwrap_or(inferred)
}

/// Walk the body of `name` in `file` to compute its inferred backend
/// set. Returns `None` when the define has no resolvable body (e.g. a
/// parse-error recovery fragment).
fn body_inferred_backends(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    name: &str,
) -> Option<smelt_types::BackendSet> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    for define in ast.defines() {
        if define.name().as_deref() == Some(name) {
            let body = define.body()?.expression()?;
            let sig_lookup = |callee_name: &str| -> Option<FunctionSig> {
                resolve_function(db, workspace, callee_name.to_string()).map(|arc| (*arc).clone())
            };
            return Some(backends::infer_body_backends(&body, &sig_lookup));
        }
    }
    None
}

/// Per-file diagnostics for the Phase 11 narrow-only rule. For each
/// `smelt.define` in `file` whose frontmatter declares a `backends:`
/// set broader than the body's inferred set, emit
/// [`DiagnosticCode::BackendsWideningNotAllowed`] anchored at the
/// declaration's name range. Also surfaces malformed-frontmatter errors
/// under the same code.
pub fn backends_widening_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for sig in sigs.iter() {
        if let Some(msg) = sig.frontmatter_parse_error.as_ref() {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Invalid frontmatter for `{}`: {}", sig.name, msg),
                range: sig.name_range,
                code: Some(DiagnosticCode::BackendsWideningNotAllowed),
                data: None,
            });
            continue;
        }
        if sig.origin == smelt_types::SigOrigin::Extern {
            // Externs with both a frontmatter `backends:` and the
            // dotted-backend sugar could disagree — but we accept the
            // frontmatter as authoritative and skip narrow checks
            // (there is no body to infer from).
            continue;
        }
        let Some(declared) = &sig.declared_backends else {
            continue;
        };
        let Some(inferred) = body_inferred_backends(db, workspace, file, &sig.name) else {
            continue;
        };
        if let Err(msg) = backends::apply_narrow_rule(Some(declared), &inferred) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Function `{}`: {}", sig.name, msg),
                range: sig.name_range,
                code: Some(DiagnosticCode::BackendsWideningNotAllowed),
                data: None,
            });
        }
    }
    out
}

/// Per-file diagnostics for the Phase 38 / Phase 42 `smelt.as_struct()`
/// backend-capability gate.
///
/// For each `smelt.define` in `file`, walks the body for `smelt.as_struct()`
/// calls. The set of backends to check against is determined by the function's
/// `declared_backends`:
///
/// - `Some(BackendSet::Only(names))` (Phase 38): the explicit declared set.
/// - `None` or `Some(BackendSet::All)` (Phase 42): the workspace's *active*
///   backend set — the distinct `target_type` values in `smelt.yml`'s
///   `targets:` map. Pass `active_backends = None` to fall back to the
///   Phase 38 behaviour (skip functions without an explicit `backends:`
///   declaration), e.g. when no `smelt.yml` is present in a synthetic test
///   workspace.
///
/// If any backend in the resolved set does not support struct literal
/// syntax, emits [`DiagnosticCode::AsStructUnsupportedBackend`] anchored at
/// the call span.
pub fn as_struct_backend_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
    active_backends: Option<&[String]>,
) -> Vec<Diagnostic> {
    use smelt_parser::ast::SmeltAsStructCall;
    use smelt_types::signatures::BackendSet;

    let sigs = file_signature_inputs(db, file);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };
    let raw_text = file.text(db);
    let text = smelt_parser::strip_frontmatter(raw_text);

    let mut out = Vec::new();
    for sig in sigs.iter() {
        // Resolve which backends to check against. Functions with an explicit
        // `Only(names)` keep the Phase 38 behaviour; functions with `All`
        // (no `backends:` declaration) fall back to the workspace's active
        // backends — the diagnostic now fires for the implicit-default case
        // too. When `active_backends` is `None` and the function declares no
        // explicit set, we cannot compute a meaningful intersection and skip
        // the check (Phase 38 behaviour).
        let backends_to_check: Vec<String> = match &sig.declared_backends {
            Some(BackendSet::Only(names)) => names.clone(),
            Some(BackendSet::All) | None => match active_backends {
                Some(active) => active.to_vec(),
                None => continue,
            },
        };
        // Walk the define's body looking for SMELT_AS_STRUCT_CALL nodes.
        let define = ast
            .defines()
            .find(|d| d.name().as_deref() == Some(sig.name.as_str()));
        let Some(define) = define else { continue };

        // Collect all SMELT_AS_STRUCT_CALL descendants.
        let body_syntax = define.syntax().descendants();
        for node in body_syntax {
            if let Some(call) = SmeltAsStructCall::cast(node) {
                // Check each backend in the resolved set.
                for backend in &backends_to_check {
                    if !function_body_check::backend_supports_struct_literal(backend) {
                        let range =
                            smelt_parser::ast::text_range_to_range(&text, call.text_range());
                        out.push(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "smelt.as_struct() is not supported on backend `{backend}` \
                                 which does not have struct-literal capability"
                            ),
                            range,
                            code: Some(DiagnosticCode::AsStructUnsupportedBackend),
                            data: None,
                        });
                        break; // One diagnostic per call site is enough.
                    }
                }
            }
        }
    }
    out
}

/// Per-file diagnostics for the Phase 31 `provenance:` unstable-schema gate.
///
/// For each `smelt.define` or `smelt.extern` in `file` that declares
/// `provenance:` in its frontmatter, emits
/// [`DiagnosticCode::UnstableSchemaRequired`] anchored at the declaration's
/// name range when `unstable_schema` is `false`.
///
/// The `unstable_schema` flag should be read from the workspace's `smelt.yml`
/// by the caller (a Salsa tracked function) before invoking this pure helper.
///
/// Phase 43 note: frontmatter-parse diagnostics (malformed YAML, unknown
/// keys) are emitted by [`frontmatter_parse_diagnostics_for_file`] instead;
/// they fire unconditionally regardless of the `unstable_schema` flag, so
/// production workspaces that opt into `unstable_schema: true` still surface
/// parse errors.
pub fn provenance_unstable_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
    unstable_schema: bool,
) -> Vec<Diagnostic> {
    use smelt_planner::logical::Provenance;

    if unstable_schema {
        return Vec::new();
    }

    let raw_text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    // Re-use the cached signature list to get accurate name_range values.
    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();

    // Check smelt.define declarations.
    for define in ast.defines() {
        let Some(fm) = define.frontmatter(raw_text) else {
            continue;
        };
        let (props, _fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        let name = define.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        if matches!(props.provenance, Provenance::Declared(_)) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "function `{name}` declares `provenance:` in its frontmatter \
                     but the workspace does not have `unstable_schema: true` in smelt.yml; \
                     set `unstable_schema: true` to enable this unstable feature"
                ),
                range,
                code: Some(DiagnosticCode::UnstableSchemaRequired),
                data: None,
            });
        }
    }

    // Check smelt.extern declarations.
    for ext in ast.externs() {
        let Some(fm) = ext.frontmatter(raw_text) else {
            continue;
        };
        let (props, _fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        let name = ext.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        if matches!(props.provenance, Provenance::Declared(_)) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "function `{name}` declares `provenance:` in its frontmatter \
                     but the workspace does not have `unstable_schema: true` in smelt.yml; \
                     set `unstable_schema: true` to enable this unstable feature"
                ),
                range,
                code: Some(DiagnosticCode::UnstableSchemaRequired),
                data: None,
            });
        }
    }

    out
}

/// Per-file diagnostics for malformed / unknown-key frontmatter on
/// `smelt.define` and `smelt.extern` declarations.
///
/// For each declaration's frontmatter (if any), runs the pure
/// [`smelt_planner::logical::parse_function_properties`] parser and converts
/// every [`smelt_planner::logical::FrontmatterDiagnostic`] it returns into a
/// full [`Diagnostic`] anchored at the declaration's name range.
///
/// Unlike [`provenance_unstable_diagnostics_for_file`], this helper does
/// **not** consult the `unstable_schema` flag — frontmatter parse errors and
/// unknown-key warnings fire unconditionally so they remain visible on
/// workspaces that opt into `unstable_schema: true`. (Phase 43.)
pub fn frontmatter_parse_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let raw_text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    // Re-use the cached signature list to get accurate name_range values.
    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();

    // Check smelt.define declarations.
    for define in ast.defines() {
        let Some(fm) = define.frontmatter(raw_text) else {
            continue;
        };
        let (_props, fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        if fm_diags.is_empty() {
            continue;
        }
        let name = define.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        for fm_diag in fm_diags {
            out.push(frontmatter_diag_to_diagnostic(fm_diag, range));
        }
    }

    // Check smelt.extern declarations.
    for ext in ast.externs() {
        let Some(fm) = ext.frontmatter(raw_text) else {
            continue;
        };
        let (_props, fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        if fm_diags.is_empty() {
            continue;
        }
        let name = ext.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        for fm_diag in fm_diags {
            out.push(frontmatter_diag_to_diagnostic(fm_diag, range));
        }
    }

    out
}

/// Translate a parser-side [`smelt_planner::logical::FrontmatterDiagnostic`]
/// into a full [`Diagnostic`] anchored at the declaration's name range.
/// Phase 43.
fn frontmatter_diag_to_diagnostic(
    fm: smelt_planner::logical::FrontmatterDiagnostic,
    range: Range,
) -> Diagnostic {
    use smelt_planner::logical::FrontmatterSeverity;
    let severity = match fm.severity {
        FrontmatterSeverity::Error => DiagnosticSeverity::Error,
        FrontmatterSeverity::Warning => DiagnosticSeverity::Warning,
    };
    Diagnostic {
        severity,
        message: fm.message,
        range,
        code: Some(DiagnosticCode::FrontmatterParseError),
        data: None,
    }
}

/// Phase 52 — per-file check: reject `smelt.extern` declarations with
/// fragment-sort parameters (`SelectItems`, `OrderSpec`).
///
/// Fragment sorts require PASSING clauses, which `smelt.extern` does not
/// support (§16 #18 deferral). This catches `SelectItems<…>` (a valid
/// `SmeltType` that passes type-ref parse) and `OrderSpec` (which is an
/// `UnsupportedSort` parse error, but with a specific sort name).
pub fn extern_fragment_param_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::SmeltType;
    use smelt_types::signatures::SmeltTypeParseError;

    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();

    for sig in sigs.iter() {
        if sig.origin != smelt_types::SigOrigin::Extern {
            continue;
        }
        for param in &sig.params {
            let is_fragment = match &param.type_ref {
                // `SelectItems<…>` parses as Ok(SmeltType::SelectItems)
                Some(Ok(SmeltType::SelectItems { .. })) => true,
                // `OrderSpec<…>` (with angle brackets) parses as UnsupportedSort;
                // check the sort name to distinguish fragment sorts from
                // genuinely unknown types (which emit InvalidFunctionTypeRef).
                Some(Err(SmeltTypeParseError::UnsupportedSort { sort, .. })) => {
                    sort.as_str() == "OrderSpec"
                }
                // Bare `OrderSpec` (no angle brackets) parses as Malformed because
                // `parse_smelt_type` requires `<…>` for non-TableExpr sorts. Detect
                // it by inspecting the span_text's leading identifier.
                Some(Err(SmeltTypeParseError::Malformed { span_text })) => {
                    let head = span_text
                        .trim()
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("");
                    head == "OrderSpec"
                }
                _ => false,
            };
            if is_fragment {
                if let Some(range) = param.type_ref_range {
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "parameter `{}` of `smelt.extern {}` uses a fragment-sort type; \
                             fragment sorts (`SelectItems`, `OrderSpec`) require PASSING clauses \
                             which `smelt.extern` does not support (§16 #18)",
                            param.name, sig.name
                        ),
                        range,
                        code: Some(DiagnosticCode::ExternFragmentParamUnsupported),
                        data: None,
                    });
                }
            }
        }
    }
    out
}

/// Phase 52 — per-file advisory: when a transparent function lacks declared
/// provenance and a WHERE clause sits above its call site, emit a Hint.
///
/// Runs on all files (model and function). Anchors the Hint at the WHERE
/// clause's text range. Only fires when `unstable_schema: true` (so the
/// provenance system is active).
pub fn missing_provenance_advisory_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_planner::logical::Provenance;

    let raw_text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    // Walk all SELECT statements in the file.
    for node in ast.syntax().descendants() {
        let Some(select) = smelt_parser::ast::SelectStmt::cast(node) else {
            continue;
        };
        // Only interested in SELECTs with a WHERE clause.
        let Some(where_clause) = select.where_clause() else {
            continue;
        };
        let where_range =
            smelt_parser::ast::text_range_to_range(raw_text, where_clause.text_range());

        // Find smelt.functions.* calls in the FROM clause.
        let Some(from_clause) = select.from_clause() else {
            continue;
        };
        for table_ref in from_clause.table_refs() {
            let Some(call) = table_ref.smelt_path_call() else {
                continue;
            };
            // Derive the function name from the call path segments (last segment = name).
            let segments = call.segments();
            let Some(fn_name) = segments.last().cloned() else {
                continue;
            };

            let Some(sig) = resolve_function(db, workspace, fn_name.clone()) else {
                continue;
            };
            // Only transparent (smelt.define) functions.
            if sig.origin != smelt_types::SigOrigin::Define {
                continue;
            }

            // Check if the function has declared provenance.
            // Sort by path to match resolve_function's deterministic "first wins" order.
            let mut sorted_files: Vec<SourceFile> = workspace.files(db).to_vec();
            sorted_files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
            let has_provenance = sorted_files
                .iter()
                .copied()
                .find(|f| {
                    file_signature_inputs(db, *f)
                        .iter()
                        .any(|s| s.name == fn_name)
                })
                .and_then(|decl_file| {
                    let decl_raw = decl_file.text(db).clone();
                    let decl_parse = parse_file(db, decl_file);
                    let decl_syntax = decl_parse.syntax();
                    let decl_ast = AstFile::cast(decl_syntax)?;
                    let fm = decl_ast
                        .defines()
                        .find(|d| d.name().as_deref() == Some(fn_name.as_str()))
                        .and_then(|d| d.frontmatter(&decl_raw))?;
                    let (props, _) = smelt_planner::logical::parse_function_properties(&fm);
                    Some(matches!(props.provenance, Provenance::Declared(_)))
                })
                .unwrap_or(false);

            if !has_provenance {
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Hint,
                    message: format!(
                        "function `{fn_name}` is transparent but has no declared `provenance:` \
                         — filter pushdown into the function body will be skipped; \
                         add `provenance:` frontmatter to enable this optimisation"
                    ),
                    range: where_range,
                    code: Some(DiagnosticCode::MissingProvenancePushdownAdvisory),
                    data: None,
                });
            }
        }
    }
    out
}

/// Per-file diagnostics for `smelt.functions.<name>(...)` call sites.
///
/// For every `SMELT_PATH_CALL` AST node in `file`, runs the pure
/// [`function_body_check::check_smelt_path_call`] with closures over
/// [`resolve_function`] (for signature lookup) and [`parse_file`] (for
/// body lookup). Call-site diagnostics emitted:
///   - [`DiagnosticCode::UnknownSmeltFn`]
///   - [`DiagnosticCode::MissingArgument`]
///   - [`DiagnosticCode::ArgTypeMismatch`]
///   - Any body-cascading [`DiagnosticCode::FunctionBodyTypeMismatch`] /
///     [`DiagnosticCode::UnknownIdentifier`] with
///     [`DiagnosticData::ExpansionFrames`] attached.
///
/// Pure-function-rule note: the analysis lives in
/// `function_body_check::check_smelt_path_call`; this wrapper builds the
/// call-site [`TypeContext`] (seeded with the workspace's signatures) and
/// threads Salsa-backed closures through for signature / body lookup.
pub fn smelt_fn_call_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::syntax_kind::SyntaxKind;

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw).to_string();

    // Build the set of define names whose bodies are deferred (i.e. skipped
    // by `function_body_diagnostics_for_file` because they have TableExpr /
    // SelectItems params). Calls inside those bodies are NOT dispatched
    // through the body re-walk, so we must check them here.
    use smelt_types::signatures::SmeltType;
    let deferred_define_names: std::collections::HashSet<String> = {
        let ast_file = AstFile::cast(syntax.clone());
        ast_file
            .as_ref()
            .map(|af| {
                af.defines()
                    .filter(|def| {
                        let Some(sig) = def.name().and_then(|n| {
                            let sigs = file_signature_inputs(db, file);
                            sigs.iter().find(|s| s.name == n).cloned()
                        }) else {
                            return false;
                        };
                        sig.params.iter().any(|p| match &p.type_ref {
                            Some(Err(
                                smelt_types::signatures::SmeltTypeParseError::UnsupportedSort {
                                    sort,
                                    ..
                                },
                            )) if matches!(
                                sort.as_str(),
                                "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems"
                            ) =>
                            {
                                true
                            }
                            Some(Ok(SmeltType::TableExpr(_))) => true,
                            _ => false,
                        })
                    })
                    .filter_map(|def| def.name())
                    .collect()
            })
            .unwrap_or_default()
    };

    // Collect SMELT_PATH_CALL nodes (smelt.functions.* form).
    //
    // For nodes outside a DEFINE_BODY: always include (top-level call sites
    // in models / expression-only defines).
    //
    // For nodes inside a DEFINE_BODY: include only when the enclosing define
    // is deferred (has TableExpr/SelectItems params) — those bodies are
    // skipped by `function_body_diagnostics_for_file`, so any nested
    // `smelt.functions.*` calls would otherwise go unchecked. For
    // non-deferred defines, the body re-walk dispatches nested calls through
    // `nested_path_handler`, so we exclude them here to avoid duplication.
    let path_call_nodes: Vec<SmeltPathCall> = syntax
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::SMELT_PATH_CALL)
        .filter(|n| {
            // Skip `smelt.config.var(...)` — those are handled by
            // `check_config_var_call_diagnostics` in `check_file_diagnostics`,
            // not by the smelt-function call checker.
            //
            // Skip `smelt.columns_of(...)` — a Phase C meta-builtin handled
            // by `check_columns_of_diagnostics` and the
            // `ColumnsOfUnresolvableSchema` wiring block in
            // `check_file_diagnostics`.
            if let Some(call) = SmeltPathCall::cast(n.clone()) {
                let segs = call.segments();
                if segs.len() == 2
                    && segs[0].to_lowercase() == "config"
                    && segs[1].to_lowercase() == "var"
                {
                    return false;
                }
                // Skip `smelt.config.load_yaml`, `smelt.config.load_json`,
                // `smelt.config.load_toml` — Phase E1 loader meta-builtins
                // handled by `loader_call_diagnostics_for_file`, not by
                // the smelt-function call checker.
                if segs.len() == 2
                    && segs[0].to_lowercase() == "config"
                    && matches!(
                        segs[1].to_lowercase().as_str(),
                        "load_yaml" | "load_json" | "load_toml"
                    )
                {
                    return false;
                }
                if segs.len() == 1 && segs[0].to_lowercase() == "columns_of" {
                    return false;
                }
            }
            let inside_define_body = n.ancestors().any(|a| a.kind() == SyntaxKind::DEFINE_BODY);
            if !inside_define_body {
                return true; // Top-level call site — always check.
            }
            // Inside a define body: only check if the define is deferred.
            // Walk up to find the enclosing DEFINE node and check its name.
            use smelt_parser::syntax_kind::SyntaxKind as Sk;
            let enclosing_define_name = n
                .ancestors()
                .find(|a| a.kind() == Sk::SMELT_DEFINE)
                .and_then(|def| {
                    use smelt_parser::ast::SmeltDefine;
                    SmeltDefine::cast(def).and_then(|d| d.name())
                });
            enclosing_define_name
                .map(|nm| deferred_define_names.contains(&nm))
                .unwrap_or(false)
        })
        .filter_map(SmeltPathCall::cast)
        .collect();

    if path_call_nodes.is_empty() {
        return Vec::new();
    }

    // Build the call-site type context. For model-files we reuse the
    // model's TypeContext (source/CTE/model columns all in scope) and
    // layer the workspace's FunctionSig map on top. For pure function-only
    // files there is no SELECT, so `type_context` returns an empty TC —
    // that's fine, the body walk doesn't need SQL scope.
    let mut ctx: TypeContext = (*type_context(db, workspace, file)).clone();

    // Seed every workspace-visible `FunctionSig` so path-call type inference
    // can resolve nested function returns. Iterating all files is O(N) but
    // only runs when a call site is present — pure function files with no
    // calls skip this entirely.
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in &files {
        let sigs = file_signature_inputs(db, *f);
        for sig in sigs.iter() {
            ctx.add_function_signature(&sig.name, sig.clone());
        }
    }

    // Closures for pure checker. `sig_lookup` wraps `resolve_function`;
    // `body_lookup` re-parses the declaring file and locates the matching
    // `smelt.define` body. `builtin_lookup` dispatches to the built-in
    // registry so calls like `smelt.fn.COALESCE(...)` go through
    // `unify_call` when no user-declared function shadows the name.
    let sig_lookup = |name: &str| -> Option<FunctionSig> {
        resolve_function(db, workspace, name.to_string()).map(|arc| (*arc).clone())
    };

    let builtin_lookup = |name: &str| -> Option<&'static smelt_types::signatures::Signature> {
        smelt_types::BuiltinRegistry::resolve(name)
    };

    // LUB closure for `unify_call` — forwards to the canonical numeric
    // promotion routine in `type_inference`.
    let lub = |a: &DataType, b: &DataType| -> DataType {
        let lhs = TypedColumn {
            data_type: a.clone(),
            nullable: true,
        };
        let rhs = TypedColumn {
            data_type: b.clone(),
            nullable: true,
        };
        type_inference::promote_types(&lhs, &rhs).data_type
    };

    // Phase 41: capture the cycle set so the body cascade short-circuits
    // for cycle-participant functions. Without this guard, calling
    // `body_lookup(cycle_a)` triggers a re-walk of cycle_a's body which
    // calls cycle_b, which calls back into cycle_a — overflowing the stack.
    // The cycle pre-pass surfaces a `FunctionCallCycle` diagnostic instead.
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    let body_lookup = |sig: &FunctionSig| -> Option<(String, function_body_check::BodyShape)> {
        // Externs have no body — skip. Defines alone carry a re-walkable body.
        if sig.origin == smelt_types::SigOrigin::Extern {
            return None;
        }
        if cycle_set.contains(&sig.name) {
            return None;
        }
        // Find the file declaring this function.
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                let f_text = f.text(db);
                let f_clean = smelt_parser::strip_frontmatter(f_text).to_string();
                let f_parse = parse_file(db, *f);
                let f_syntax = f_parse.syntax();
                if let Some(ast) = AstFile::cast(f_syntax) {
                    for define in ast.defines() {
                        if define.name().as_deref() == Some(&sig.name) {
                            if let Some(body) = define.body() {
                                // Phase 15: prefer the SELECT shape when
                                // present (e.g. `TableExpr`-returning
                                // defines). Fall back to the scalar
                                // expression shape for Phase-5-shaped
                                // bodies.
                                if let Some(select_stmt) = body.select_stmt() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Select(select_stmt),
                                    ));
                                }
                                if let Some(body_expr) = body.expression() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Expression(body_expr),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    };

    // Phase 12: resolve the declaring file path for a `FunctionSig` so that
    // each expansion frame records `decl_path` + `decl_range`. The renderer
    // and the LSP use these to build outer-to-inner frame trailers and
    // `DiagnosticRelatedInformation` entries linking back to each declaration.
    let decl_lookup = |sig: &smelt_types::signatures::FunctionSig| -> Option<std::path::PathBuf> {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                return Some(f.path(db).clone());
            }
        }
        None
    };

    // Phase 15: resolve a `TableExpr` argument expression to its
    // caller-supplied column schema. Today we recognise two shapes:
    //   - `smelt.models.model_name` — look up the referenced model's
    //     typed schema through the Salsa provider.
    //   - `smelt.sources.src.table` — look up the source's declared
    //     columns (Phase 15 fixtures only use `smelt.ref`, but extending
    //     is cheap).
    // Any other shape (bare subquery, another function call, …)
    // resolves to `None`; the body checker then sees no FROM-scope
    // entries for that parameter and bare columns emit
    // `UnknownIdentifier` with a frame rooted at the call site.
    let tableexpr_schema_lookup = |arg_expr: &smelt_parser::ast::Expr,
                                   ctx: &TypeContext|
     -> Option<Vec<(String, TypedColumn)>> {
        // Try to extract a `smelt.models.X` from the argument's function
        // call node, if any. We accept the call nested inside an
        // EXPRESSION wrapper.
        use smelt_parser::ast::Subquery;
        use smelt_parser::syntax_kind::SyntaxKind as Sk;

        // Phase 4: unified `smelt.<path>` value form. Walks the
        // expression for any `SMELT_PATH_REF`; resolves through the
        // path-tuple resolver and dispatches on the resolved kind.
        for node in arg_expr.syntax().descendants() {
            if node.kind() == Sk::SMELT_PATH_REF {
                let path_ref = match SmeltPathRef::cast(node) {
                    Some(p) => p,
                    None => continue,
                };
                let path = path_ref.segments();
                if let Some(resolved) = resolve_ref_path(db, workspace, path.clone()) {
                    match resolved.kind {
                        RefKind::Model => {
                            if let Some(sf) = resolved.source_file {
                                let schema = resolved_model_schema(db, workspace, sf);
                                let cols: Vec<(String, TypedColumn)> = schema
                                    .columns
                                    .iter()
                                    .map(|c| {
                                        let tc = c.data_type.clone().unwrap_or(TypedColumn {
                                            data_type: DataType::Unknown,
                                            nullable: true,
                                        });
                                        (c.name.clone(), tc)
                                    })
                                    .collect();
                                if !cols.is_empty() {
                                    return Some(cols);
                                }
                            }
                        }
                        RefKind::Seed => {
                            // Seed columns come from `discover_seed_infos`;
                            // reuse the legacy provider helper.  The lookup
                            // key is `address_segments.join("_")` which equals
                            // `path.join("_")` since resolve_ref_path matches
                            // seeds by address_segments == path.
                            let key = path.join("_");
                            let provider = SalsaRefSchemaProvider::new(db, workspace);
                            if let Some(cols) = provider.seed_columns(&key) {
                                return Some(cols);
                            }
                        }
                        RefKind::Source => {
                            // Path tuple shape: ["sources", <src>, <tbl>].
                            if path.len() >= 3 {
                                let source_name = path[path.len() - 2].clone();
                                let table_name = path[path.len() - 1].clone();
                                let sources = sources_config(
                                    db,
                                    *workspace
                                        .projects(db)
                                        .first()
                                        .expect("at least one project"),
                                );
                                for s in &sources.sources {
                                    if s.name != source_name {
                                        continue;
                                    }
                                    for t in &s.tables {
                                        if t.name == table_name {
                                            let cols: Vec<(String, TypedColumn)> = t
                                                .columns
                                                .iter()
                                                .map(|c| {
                                                    (
                                                        c.name.clone(),
                                                        TypedColumn {
                                                            data_type: c
                                                                .data_type
                                                                .clone()
                                                                .unwrap_or(DataType::Unknown),
                                                            nullable: true,
                                                        },
                                                    )
                                                })
                                                .collect();
                                            return Some(cols);
                                        }
                                    }
                                }
                            }
                        }
                        RefKind::Function | RefKind::Test => {
                            // A function or test in `TableExpr` arg
                            // position is a kind mismatch; surface the
                            // failure to downstream UnknownIdentifier
                            // diagnostics — no schema is provided.
                            return None;
                        }
                    }
                }
            }
        }

        // Phase 46: derived-table / inline-subquery argument shape —
        // `(SELECT … [FROM y])`. The arg expression contains a
        // `SUBQUERY` node; infer its output schema from the inner
        // SELECT statement using the call-site context (so any CTE /
        // model / source qualifiers in the SELECT's FROM clause
        // resolve).
        for node in arg_expr.syntax().descendants() {
            if node.kind() == Sk::SUBQUERY {
                if let Some(sub) = Subquery::cast(node) {
                    if let Some(inner) = sub.select_stmt() {
                        let cols = type_inference::infer_select_output_schema(&inner, ctx);
                        if !cols.is_empty() {
                            return Some(cols);
                        }
                    }
                }
            }
        }

        // Phase 46: CTE reference — the arg is a bare identifier
        // matching a CTE in scope. Walking the AST of a bare
        // identifier descends through EXPRESSION → COLUMN_REF → IDENT,
        // so we just compare the trimmed text against the CTE name
        // table on the call-site context.
        let trimmed = arg_expr.text().trim().to_string();
        if !trimmed.is_empty() && ctx.is_cte(&trimmed) {
            let cols: Vec<(String, TypedColumn)> = ctx
                .cte_columns(&trimmed)
                .into_iter()
                .map(|(name, tc)| (name.to_string(), tc.clone()))
                .collect();
            if !cols.is_empty() {
                return Some(cols);
            }
        }

        None
    };

    // Phase 45: resolve a `TableRef` inside a function body's FROM /
    // JOIN clauses to the joined schema, so the body's bare-column
    // resolver can see e.g. `dim_customer.col` from
    // `JOIN smelt.models.dim_customer AS dim_customer`.
    //
    // Supported shapes:
    //   - `smelt.models.model` — same path as `tableexpr_schema_lookup`.
    //   - `smelt.sources.src.tbl` — same path as `tableexpr_schema_lookup`.
    // Unsupported shapes (subqueries, CTE refs, derived tables) return
    // `None`. Phase 46 widens to those.
    let table_ref_schema_lookup =
        |table_ref: &smelt_parser::ast::TableRef| -> Option<Vec<(String, TypedColumn)>> {
            // `smelt.<path>` value-form in JOIN position.
            if let Some(path_ref) = table_ref.smelt_path_ref() {
                let segs = path_ref.segments();
                let model_name = segs.last().cloned().unwrap_or_default();
                let seed_key = segs.join("_");
                let provider = SalsaRefSchemaProvider::new(db, workspace);
                return provider
                    .resolved_columns(&model_name)
                    .or_else(|| provider.seed_columns(&seed_key));
            }

            None
        };

    // Path-call CTE wildcard expansion deferred to a follow-on phase.
    // Return `None` to fall back to the opaque-CTE marker.
    let smelt_path_schema_lookup =
        |_call: &smelt_parser::ast::SmeltPathCall| -> Option<Vec<(String, TypedColumn)>> { None };

    // TB-5: validates that a `smelt.<dir>.<name>(...)` call path's directory
    // segments equal the workspace-relative directory of the file declaring
    // a function with `name`. Returns `true` iff some workspace file at
    // exactly that directory declares the function.
    //
    // Spec rule (`functions.md` §"Function call syntax"): the file stem is
    // *not* a path component. So a function declared in `functions/status.sql`
    // is callable as `smelt.functions.is_shipped(...)`, not
    // `smelt.functions.status.is_shipped(...)`.
    //
    // Phase 2 (unified-paths): strip the matching scan-root prefix from the
    // file's directory so that a function in `models/fn.sql` under
    // `paths: ["models"]` is addressable as `smelt.fn_name()` (empty
    // dir_segments) rather than requiring `smelt.models.fn_name()`.
    let scan_roots_for_call: Arc<Vec<String>> = {
        let proj_root = file.project_root(db).clone();
        match find_project(db, workspace, &proj_root) {
            Some(p) => project_paths(db, p),
            None => Arc::new(Vec::new()),
        }
    };
    let path_prefix_validator = |dir_segments: &[String], name: &str| -> bool {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if !sigs.iter().any(|s| s.name == name) {
                continue;
            }
            let abs_path = f.path(db);
            let proj_root = f.project_root(db);
            let Ok(rel) = abs_path.strip_prefix(proj_root) else {
                continue;
            };
            let effective_dir = rel.parent().unwrap_or(std::path::Path::new(""));
            // Strip the first matching scan-root prefix.
            let stripped_dir = scan_roots_for_call
                .iter()
                .find_map(|sr| effective_dir.strip_prefix(sr.as_str()).ok())
                .unwrap_or(effective_dir);
            let file_dir_segments: Vec<String> = stripped_dir
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
                .collect();
            if file_dir_segments == dir_segments {
                return true;
            }
        }
        false
    };

    // Phase 17: resolve a parameter's default-value expression by
    // re-parsing the declaring file and walking to the matching
    // PARAM node. Returns the inferred `DataType` of the default
    // expression (against an empty context — defaults are
    // self-contained per research §3).
    let default_type_lookup = |sig: &FunctionSig, param_name: &str| -> Option<DataType> {
        let decl_path = decl_lookup(sig)?;
        let f = files.iter().find(|f| f.path(db) == &decl_path)?;
        let f_parse = parse_file(db, *f);
        let ast = AstFile::cast(f_parse.syntax())?;
        for define in ast.defines() {
            if define.name().as_deref() != Some(&sig.name) {
                continue;
            }
            let Some(param_list) = define.param_list() else {
                continue;
            };
            for p in param_list.params() {
                if p.name().as_deref() != Some(param_name) {
                    continue;
                }
                let default_expr = p.default_value_expr()?;
                let empty_ctx = TypeContext::new();
                return type_inference::infer_expression_type(&default_expr, &empty_ctx)
                    .map(|t| t.data_type);
            }
        }
        None
    };

    let mut out = Vec::new();
    for call in &path_call_nodes {
        out.extend(function_body_check::check_smelt_path_call(
            call,
            &ctx,
            &clean_text,
            &sig_lookup,
            &builtin_lookup,
            &lub,
            &body_lookup,
            &decl_lookup,
            &tableexpr_schema_lookup,
            &default_type_lookup,
            &table_ref_schema_lookup,
            &smelt_path_schema_lookup,
            &path_prefix_validator,
        ));
    }
    out
}

/// Per-file diagnostics for malformed `smelt.define` parameter / return type
/// annotations (Phase 4). Iterates `functions_in_file(file)` and emits a
/// diagnostic for each [`ParamSpec::type_ref`] or
/// [`FunctionSig::return_type`] that carries a parse error.
///
/// Pure-function-rule note: the heavy lifting lives on
/// `smelt_types::signatures` — this helper is just a thin reader over the
/// signature query's cached output.
pub fn invalid_function_type_ref_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::SmeltTypeParseError;
    // Phase 13: the parser accepts `TableExpr`, `AggExpr`, `WindowExpr`, and
    // `SelectItems` sort heads in parameter / return positions so the Step 3
    // fixtures can land. `smelt-types::parse_smelt_type` still rejects those
    // sorts (full type-system wiring is Phases 14+), so we filter out known
    // Phase-13-deferred shapes here to keep `example_diagnostics` green:
    //   - `TableExpr`, `AggExpr`, `WindowExpr`, `SelectItems` in a tagged
    //     `UnsupportedSort` (forms with `<...>`),
    //   - the same heads with no angle brackets (surface as `Malformed`
    //     with leading identifier `TableExpr` / etc.). The bare form is
    //     legal per the plan (e.g. `-> TableExpr`), so a malformed whose
    //     source starts with one of these heads is deferred too.
    // Any *other* `UnsupportedSort` (e.g. `FooExpr<T>`) or any
    // `UnknownInner` / `NestedExpr` error still surfaces.
    let is_phase13_deferred_sort_name = |sort: &str| -> bool {
        matches!(sort, "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems")
    };
    let is_deferred_phase13_sort = |err: &SmeltTypeParseError| -> bool {
        match err {
            SmeltTypeParseError::UnsupportedSort { sort, .. } => {
                is_phase13_deferred_sort_name(sort)
            }
            SmeltTypeParseError::Malformed { span_text } => {
                // Leading identifier is the source head (bare `TableExpr`
                // with no `<...>` lands here). Extract the first word.
                let head = span_text
                    .trim()
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("");
                is_phase13_deferred_sort_name(head)
            }
            _ => false,
        }
    };
    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for sig in sigs.iter() {
        for param in &sig.params {
            if let (Some(Err(err)), Some(range)) = (&param.type_ref, param.type_ref_range) {
                if is_deferred_phase13_sort(err) {
                    continue;
                }
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!("Invalid type for parameter `{}`: {}", param.name, err),
                    range,
                    code: Some(DiagnosticCode::InvalidFunctionTypeRef),
                    data: None,
                });
            }
        }
        if let (Some(Err(err)), Some(range)) = (&sig.return_type, sig.return_type_range) {
            if is_deferred_phase13_sort(err) {
                continue;
            }
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Invalid return type for function `{}`: {}", sig.name, err),
                range,
                code: Some(DiagnosticCode::InvalidFunctionTypeRef),
                data: None,
            });
        }
    }
    out
}

/// Phase 22 helper: collect the CTE names declared in the WITH clause of
/// `fn_name`'s body SELECT inside `ast`.
///
/// Returns an empty vec when the function has no SELECT body, no WITH
/// clause, or when `fn_name` isn't found.  Pure — only walks the CST.
fn cte_names_from_define(ast: &AstFile, fn_name: &str) -> Vec<String> {
    for define in ast.defines() {
        if define.name().as_deref() != Some(fn_name) {
            continue;
        }
        let Some(body) = define.body() else {
            return vec![];
        };
        let Some(select) = body.select_stmt() else {
            return vec![];
        };
        let Some(with_clause) = select.with_clause() else {
            return vec![];
        };
        return with_clause.ctes().filter_map(|c| c.name()).collect();
    }
    vec![]
}

/// Phase 19: emit `UnknownContext` when an `Expr<T, ctx>` or
/// `SelectItems<Kind, ctx>` parameter's context name doesn't match any other
/// parameter **or CTE name** in the same `smelt.define`.
///
/// Phase 22 extends this function to:
/// 1. Also validate the context embedded in `SelectItems<Kind, ctx>` type
///    annotations (previously only `param.context` / `Expr<T, ctx>` was
///    checked).
/// 2. Accept CTE names from the function body's WITH clause as valid
///    context names — so `metrics: SelectItems<Agg, sessionized>` does not
///    emit `UnknownContext` when `sessionized` is defined as a CTE in the
///    same function body.
///
/// Pure: re-uses the cached `file_signature_inputs` output.
pub fn unknown_context_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::{ContextRef, SmeltType};

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return vec![];
    };

    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for sig in sigs.iter() {
        let param_names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();
        let cte_names = cte_names_from_define(&ast, &sig.name);

        let is_valid_ctx = |ctx_name: &str| -> bool {
            param_names.contains(&ctx_name) || cte_names.iter().any(|n| n == ctx_name)
        };

        for param in &sig.params {
            // Case 1: `Expr<T, ctx>` / `AggExpr<T, ctx>` / `WindowExpr<T, ctx>` —
            // context is stored in `param.context`.
            if let Some(ctx_ref) = &param.context {
                let ctx_name = ctx_ref.name();
                if !is_valid_ctx(ctx_name) {
                    let Some(range) = param.type_ref_range else {
                        continue;
                    };
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Context `{ctx_name}` in parameter `{}` does not name a parameter \
                             or CTE in `{}`",
                            param.name, sig.name
                        ),
                        range,
                        code: Some(DiagnosticCode::UnknownContext),
                        data: None,
                    });
                }
            }

            // Case 2: `SelectItems<Kind, ctx>` — context is stored inside
            // `param.type_ref` as `SmeltType::SelectItems { context: Some(...) }`.
            if let Some(Ok(SmeltType::SelectItems {
                context: Some(ContextRef(ctx_name)),
                ..
            })) = &param.type_ref
            {
                if !is_valid_ctx(ctx_name) {
                    let Some(range) = param.type_ref_range else {
                        continue;
                    };
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Context `{ctx_name}` in parameter `{}` does not name a parameter \
                             or CTE in `{}`",
                            param.name, sig.name
                        ),
                        range,
                        code: Some(DiagnosticCode::UnknownContext),
                        data: None,
                    });
                }
            }
        }
    }
    out
}

/// Phase 20: emit [`DiagnosticCode::CteCycle`] for every `smelt.define` in
/// `file` whose SELECT body contains a cyclic CTE reference.
///
/// Uses [`function_body_check::extract_function_body_cte_schemas`] with an
/// empty seed context (cycle detection is purely structural).
///
/// Pure: reads `parse_file`.
pub fn cte_cycle_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw);
    let Some(ast) = AstFile::cast(syntax) else {
        return vec![];
    };
    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(body) = define.body() else { continue };
        let Some(select) = body.select_stmt() else {
            continue;
        };
        let empty_ctx = type_inference::TypeContext::new();
        let (_ctx, cycle_diags) = function_body_check::extract_function_body_cte_schemas(
            &select,
            &empty_ctx,
            &clean_text,
        );
        out.extend(cycle_diags);
    }
    out
}

/// Phase 20: emit [`DiagnosticCode::ContextMismatch`] for every `smelt.define`
/// in `file` whose explicit `Expr<T, ctx>` annotation disagrees with the
/// context inferred from the parameter's splice point.
///
/// Uses [`function_body_check::context_mismatch_diagnostics_for_fn`].
///
/// Pure: reads `parse_file` and `file_signature_inputs`.
pub fn context_mismatch_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return vec![];
    };
    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else { continue };
        let Some(sig) = sigs.iter().find(|s| s.name == name) else {
            continue;
        };
        let Some(body) = define.body() else { continue };
        let Some(select) = body.select_stmt() else {
            continue;
        };
        out.extend(function_body_check::context_mismatch_diagnostics_for_fn(
            sig, &select,
        ));
    }
    out
}
