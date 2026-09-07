use crate::*;
use salsa::Accumulator;
use smelt_parser::File as AstFile;

/// Second half of `check_file_diagnostics`'s straight-line checks — split out
/// purely to keep file_check.rs under the large-file ratchet. Runs
/// unconditionally after `check_file_diagnostics`'s only two early-return
/// gates (the CSV-sidecar check and the parse_model gate) have both passed,
/// so it re-derives its own `path`/`text`/`project`/`parse` (cheap: these
/// are memoized Salsa queries, not fresh computation).
pub(crate) fn check_file_diagnostics_tail(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) {
    let _path = file.path(db);
    let text = file.text(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);
    let parse = parse_file(db, file);

    // Unified path-form refs. Resolve through the path-tuple
    // resolver and either (a) flag undefined paths or (b) flag a
    // kind-mismatch when a `smelt.tests.*` path appears in a FROM
    // position (architecture Surface §"Resolution").
    let path_refs = model_path_refs(db, file);
    for path_ref_loc in path_refs.iter() {
        match resolve_ref_path(db, workspace, path_ref_loc.path.clone()) {
            Some(resolved) => {
                if resolved.kind == RefKind::Test && path_ref_loc.in_table_expr_position {
                    let leaf = path_ref_loc.path.last().cloned().unwrap_or_default();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Cannot reference test '{leaf}' in a FROM position — \
                             smelt.tests.* paths are not valid as TableExpr values"
                        ),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::KindMismatch),
                        data: None,
                    })
                    .accumulate(db);
                }
                if resolved.kind == RefKind::Check && path_ref_loc.in_table_expr_position {
                    let leaf = path_ref_loc.path.last().cloned().unwrap_or_default();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Cannot reference check '{leaf}' in a FROM position — \
                             smelt.check files produce no DB object and cannot be used as TableExpr values"
                        ),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::KindMismatch),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
            None => {
                let path_str = format!("smelt.{}", path_ref_loc.path.join("."));
                // Emit the right diagnostic code based on the path namespace so
                // code-action providers can offer the correct quickfix:
                //   smelt.sources.* → UndefinedSource (offer "Add table to YAML")
                //   smelt.models.*  → UndefinedModelRef (offer "Create model")
                //   anything else   → UndefinedModelRef (generic fallback)
                let is_source_path =
                    path_ref_loc.path.first().map(|s| s.as_str()) == Some("sources");
                if is_source_path && path_ref_loc.path.len() >= 3 {
                    let source_name = path_ref_loc.path[path_ref_loc.path.len() - 2].clone();
                    let table_name = path_ref_loc.path[path_ref_loc.path.len() - 1].clone();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined source: '{}.{}'", source_name, table_name),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::UndefinedSource),
                        data: Some(DiagnosticData::UndefinedSource {
                            source_name,
                            table_name,
                        }),
                    })
                    .accumulate(db);
                } else {
                    // Compute a "did you mean" hint by scanning for models
                    // whose leaf segment matches the last segment of the
                    // unresolved path. This helps users find the full canonical
                    // address when they used only a leaf or partial path.
                    let leaf = path_ref_loc.path.last().map(|s| s.as_str()).unwrap_or("");
                    let hint = if !leaf.is_empty() {
                        let candidates = leaf_did_you_mean(db, workspace, project, leaf);
                        match candidates.as_slice() {
                            [] => String::new(),
                            [single] => format!(" did you mean '{single}'?"),
                            many => {
                                let list = many
                                    .iter()
                                    .map(|s| format!("'{s}'"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!(" did you mean one of {list}?")
                            }
                        }
                    } else {
                        String::new()
                    };
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined ref: {path_str}{hint}"),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::UndefinedModelRef),
                        data: Some(DiagnosticData::UndefinedRef {
                            model_name: path_ref_loc.path.last().cloned().unwrap_or_default(),
                        }),
                    })
                    .accumulate(db);
                }
            }
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

    // BUG-078: checked whenever the project carries aggregate `sources.yml`
    // text — NOT gated on `sources` (legacy `smelt.source()` call sites, which
    // are always empty since the per-entity migration made `smelt.source()` a
    // parse error). Gating here made a YAML-broken aggregate file silently
    // fall back to `SourcesConfig::default()` with no diagnostic.
    if let Some(p) = project {
        if let Some(yaml_error) = sources_yaml_error(db, p) {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("sources.yml parse error: {}", yaml_error.message),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::YamlParseError),
                data: None,
            })
            .accumulate(db);
        }
    }

    if !sources.is_empty() {
        if let Some(p) = project {
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
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::SourceTypeError),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Unsupported constructs + malformed sources + CAST / unknown fn / ambiguous column
    queries::check_types::check_unsupported_constructs(&parse.syntax(), db);

    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        // Phase 4: smelt.source() is a parse error so there are no SourceCall
        // nodes to validate. The malformed-source check is superseded by the
        // parser rejection.

        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        queries::check_types::check_expression_types(&expr, db);
                    }
                    // The `_smelt_` alias prefix is reserved for smelt's own
                    // generated identifiers (`multi_backend.md` §"Output-schema
                    // type conformance") — most visibly the synthesized
                    // `_smelt_col{n}` alias bound to a nameless projection
                    // item. Emitted here (the analyzer) rather than only at
                    // build time so the LSP and the CLI build path agree
                    // (`architecture.md` §"Diagnostic parity rule").
                    if let Some(alias) = item.alias() {
                        if alias.starts_with("_smelt_") {
                            let range = item.alias_range().unwrap_or_else(|| item.range());
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                message: format!(
                                    "column alias `{alias}` uses the reserved `_smelt_` prefix; \
                                     smelt uses this prefix for its own generated identifiers \
                                     (e.g. the synthesized name for an unaliased expression \
                                     column) — choose a different alias"
                                ),
                                range,
                                code: Some(DiagnosticCode::ReservedProjectionAliasPrefix),
                                data: None,
                            })
                            .accumulate(db);
                        }
                    }
                }
            }

            // Phase 14 (§16 #24): reject window-kind expressions in WHERE
            // and GROUP BY positions. Kind synthesis is independent of any
            // column-schema lookups (column refs are always Scalar), so
            // the check runs on a fresh empty `TypeContext`.
            let kind_ctx = type_inference::TypeContext::new();
            for info in type_inference::check_window_in_scalar_contexts(&select_stmt, &kind_ctx) {
                let range = info.range;
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Window function `{}` is not allowed in {} (only scalar / aggregate \
                         expressions are permitted here)",
                        info.expression_text, info.clause
                    ),
                    range,
                    code: Some(DiagnosticCode::WindowInScalarContext),
                    data: None,
                })
                .accumulate(db);
            }

            // Phase A (meta-language) Phase 3: list + spread diagnostics.
            //
            // 1. Walk LIST_SPREAD nodes in the SELECT list.
            //    Handles: MetaSpreadOnNonList, MetaListHeterogeneous (for inline
            //    spread-of-literal), MetaListEmptyTypeUnknown.
            //    GroupBy / OrderBy / function args / IN-list / VALUES remain
            //    deferred — the parser DOES emit LIST_SPREAD there, but the
            //    orchestrator does not yet walk those positions.
            //
            // 2. Walk SELECT_ITEM expressions for bare list literals
            //    (`SELECT [1, 'x'] FROM t`).
            //    Handles: MetaListHeterogeneous and MetaListEmptyTypeUnknown
            //    for non-spread list literals appearing directly in the
            //    SELECT list.
            //
            // 3. Detect spreads in forbidden positions (WHERE, etc.).
            //    Handles: MetaSpreadInForbiddenPosition.
            //
            // All three checks use an empty TypeContext (no column schema
            // available at this point) — consistent with the window-function
            // check above.
            // Ranges of meta diagnostics already emitted for this select
            // statement. A `List<T>`-in-scalar-position check (below) is
            // suppressed for any select item that already carries another meta
            // error (drop-on-error: a single malformed item does not avalanche).
            let mut flagged_meta_ranges: Vec<rowan::TextRange> = Vec::new();

            let spread_result = type_inference::check_select_list_spreads(&select_stmt, &kind_ctx);
            for diag in spread_result.diagnostics {
                flagged_meta_ranges.push(diag.range);
                DiagnosticAcc(diag).accumulate(db);
            }

            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        if let Some(arr) = expr.as_array_literal() {
                            let elements = arr.elements();
                            // Use the expression's span for the diagnostic anchor.
                            let span = expr.syntax().text_range();
                            for diag in type_inference::list_literal_sentinels_to_diagnostics(
                                &elements, &kind_ctx, span,
                            ) {
                                flagged_meta_ranges.push(diag.range);
                                DiagnosticAcc(diag).accumulate(db);
                            }
                        }
                    }
                }
            }

            let forbidden_diags =
                type_inference::check_forbidden_position_spreads(&select_stmt, &kind_ctx);
            for diag in forbidden_diags {
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase B (meta-language) Phase 3: HOF + lambda + pipe diagnostics.
            //
            // Walks every LAMBDA, FUNCTION_CALL (HOF), and PIPE_EXPR descendant.
            // Covers: LambdaInForbiddenPosition, LambdaArityMismatch, LambdaZeroParameters,
            //   LambdaDuplicateParameter, LambdaResultTypeMismatch, HofExpectsLambda,
            //   HofExpectsReducer, PipeRhsNotCall, PipeInDataPosition,
            //   ReducerInputTypeMismatch, ReducerEmptyNoIdentity.
            // Also covers Phase F REDUCER_CALL nodes (parameterised reducers):
            //   ReducerArityMismatch, ReducerArgTypeMismatch, ReducerArgNotCompileTime,
            //   ReducerNamedArgument.
            // Uses an empty TypeContext (consistent with spread/window checks above).
            let hof_diags =
                type_inference::check_hof_position_diagnostics(&select_stmt, &kind_ctx, text);
            for diag in hof_diags {
                flagged_meta_ranges.push(diag.range);
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase F (meta-language) — Ternary expression diagnostics.
            //
            // Walks every TERNARY_EXPR descendant and bare THEN_KW tokens.
            // Covers: TernaryConditionNotBoolean, TernaryBranchTypeMismatch,
            //   TernaryDanglingElse, TernaryDanglingThen.
            // Uses an empty TypeContext (consistent with HOF checks above).
            {
                let ternary_diags =
                    type_inference::check_ternary_expr_diagnostics(&select_stmt, &kind_ctx);
                for diag in ternary_diags {
                    flagged_meta_ranges.push(diag.range);
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // BUG-017: cross-family binary arithmetic → TypeMismatch.
            //
            // Walks every BINARY_EXPR and emits exactly one TypeMismatch Error
            // at the operator span when a numeric/string/boolean/temporal
            // cross-family pair is detected (spec §1 and §14).
            // Uses an empty TypeContext — literal operands (`42 + '3'`)
            // resolve without column context; column-typed operands resolve
            // if a full ctx is available later in check_type_diagnostics.
            {
                let xfamily_diags = type_inference::check_crossfamily_arithmetic_diagnostics(
                    &select_stmt,
                    &kind_ctx,
                );
                for diag in xfamily_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §15 — decimal precision overflow → `DecimalPrecisionOverflow`.
            //
            // Walks every `+`, `-`, `*`, `%` BINARY_EXPR and emits exactly one
            // `DecimalPrecisionOverflow` Error at the operator span when the
            // Spark-style growth formula yields `p' > 38`. Division is excluded
            // (handled below). The result type in such expressions is already
            // `DataType::Unknown` as computed by `promote_numeric_operands_for_op`.
            {
                let overflow_diags = type_inference::check_decimal_precision_overflow_diagnostics(
                    &select_stmt,
                    &kind_ctx,
                );
                for diag in overflow_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §15 — division rejection → `TypeMismatch`.
            //
            // `Decimal / T` for any numeric `T` is not in the portable surface.
            // Emits one `TypeMismatch` Error at the `/` operator span directing
            // the user to cast to Double. The inferred result type is already
            // `DataType::Unknown` (set by `promote_numeric_operands_for_op`).
            {
                let div_diags =
                    type_inference::check_decimal_division_diagnostics(&select_stmt, &kind_ctx);
                for diag in div_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §17 — non-portable collation → `NonPortableCollation`.
            //
            // Walks every COLLATE_EXPR in the SELECT statement. For any
            // non-binary collation name the diagnostic fires at the COLLATE
            // clause span and the expression type degrades to Unknown
            // (handled in `infer_expression_type` via
            // `infer_collate_expr_type`). Binary collations (COLLATE "C",
            // COLLATE BINARY, COLLATE UTF8_BINARY, COLLATE POSIX) are
            // silent no-ops.
            {
                let collation_diags =
                    type_inference::check_collation_diagnostics(&select_stmt, &kind_ctx);
                for diag in collation_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §16 — mixed naive/tz-aware Timestamp in set operations, CASE
            // branches, and arithmetic → TypeMismatch.
            //
            // These three checks need the full per-file TypeContext (column types
            // from upstream models) so that column references such as `ts_col` and
            // `tstz_col` resolve to their inferred DataType. They cannot run on the
            // empty `kind_ctx` used for shape checks above. `type_context` is a
            // Salsa query that builds the column-schema context for this file; it
            // is safe to call from within a Salsa tracked function.
            //
            // Only run for model files that have at least one data reference
            // (the model_path filter is already satisfied by the outer `if let
            // Some(select_stmt)` guard and the `models/` path check earlier).
            {
                let tz_ctx = type_context(db, workspace, file);

                // Set-operations (UNION/INTERSECT/EXCEPT)
                let setop_diags =
                    type_inference::check_mixed_tz_setop_diagnostics(&select_stmt, &tz_ctx);
                for diag in setop_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // CASE branches
                let case_diags =
                    type_inference::check_mixed_tz_case_diagnostics(&select_stmt, &tz_ctx);
                for diag in case_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // Arithmetic operators (-, +, *, /, %)
                let mixed_tz_arith_diags =
                    type_inference::check_mixed_tz_arithmetic_diagnostics(&select_stmt, &tz_ctx);
                for diag in mixed_tz_arith_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // VALUES-clause columns (§16 strict temporal mixing rule)
                let values_temporal_diags =
                    type_inference::check_mixed_temporal_values_diagnostics(&select_stmt, &tz_ctx);
                for diag in values_temporal_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Meta-language (P6) — `MetaListInScalarPosition`.
            //
            // A `List<T>`-typed expression that reaches a Data-World scalar /
            // SELECT-item position without being consumed (by a spread, a HOF,
            // a reducer, a record, a map, or a generator) cannot materialise as
            // a scalar value — there is no implicit auto-spread
            // (`meta_language.md` §Semantics "Lists and spread" rule 10). A bare
            // list literal (`SELECT [1, 2, 3]`), or a bare `map`/`filter` /
            // pipe-to-`map`/`filter` result (`SELECT xs |> map(fn c => …)`),
            // left in a select item is unconsumed. `reduce` collapses a list to
            // a scalar, so a `reduce(...)` select item is consumed and clean.
            //
            // This is a select-shape check that runs for every model, including
            // a model with no FROM clause — `check_type_diagnostics`
            // early-returns when a model has no data refs, so the check lives
            // here (the meta walk runs regardless of FROM). Suppressed for any
            // item already carrying another meta diagnostic (drop-on-error).
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    let Some(expr) = item.expression() else {
                        continue;
                    };
                    if !select_item_yields_bare_list(&expr) {
                        continue;
                    }
                    let span = expr.syntax().text_range();
                    if flagged_meta_ranges
                        .iter()
                        .any(|r| r.intersect(span).is_some())
                    {
                        continue;
                    }
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "a List<T> cannot be used as a scalar value here; consume it \
                                  with a spread (`...xs`), a reducer (`reduce(xs, …)`), or a HOF \
                                  before splicing"
                            .to_string(),
                        range: span,
                        code: Some(DiagnosticCode::MetaListInScalarPosition),
                        data: None,
                    })
                    .accumulate(db);
                }
            }

            // Phase C (meta-language) — smelt.columns_of diagnostic wiring.
            //
            // Walks every SMELT_PATH_CALL for `smelt.columns_of(...)` in the
            // select statement. Emits:
            //   - ColumnsOfNamedArgument: named argument passed to columns_of
            //   - ColumnsOfRequiresTableExpr: non-TableExpr positional arg
            // Uses the same empty TypeContext as HOF checks (no column schema
            // available at this stage in the orchestrator).
            {
                let cols_of_diags =
                    type_inference::check_columns_of_diagnostics(&select_stmt, &kind_ctx);
                for diag in cols_of_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase C (meta-language) — ColumnsOfUnresolvableSchema wiring.
            //
            // For each `smelt.columns_of(smelt.models.<name>)` (or
            // `smelt.columns_of(<name>)` where `<name>` is a bare identifier that
            // resolves via the workspace) call in the select statement, attempt to
            // resolve the model schema via `columns_of_for_table_expr`. When the
            // schema cannot be resolved (the model does not exist or has an unknown
            // schema), emit exactly one `ColumnsOfUnresolvableSchema` diagnostic
            // anchored at the full `smelt.columns_of(...)` call span.
            //
            // This implements the drop-on-error recovery policy (same as
            // `MetaSpreadInForbiddenPosition`): the call-site gets one diagnostic
            // and no cascading errors from the surrounding expression.
            {
                use smelt_parser::ast::SmeltPathCall;
                use smelt_parser::SyntaxKind::SMELT_PATH_CALL;
                for node in select_stmt.syntax().descendants() {
                    if node.kind() != SMELT_PATH_CALL {
                        continue;
                    }
                    let call = match SmeltPathCall::cast(node.clone()) {
                        Some(c) => c,
                        None => continue,
                    };
                    let segs = call.segments();
                    if segs.len() != 1 || segs[0].to_lowercase() != "columns_of" {
                        continue;
                    }
                    let arg_list = match call.arg_list() {
                        Some(al) => al,
                        None => continue,
                    };
                    // Only check positional args (named args are caught by
                    // ColumnsOfNamedArgument above).
                    for pos_arg in arg_list.positional_args() {
                        // Extract the model name from the positional argument:
                        // - smelt path ref: e.g. `smelt.models.orders` → last segment
                        // - bare identifier: e.g. `orders`
                        let model_name: Option<String> = {
                            // Try smelt path ref child.
                            let path_ref_name = pos_arg
                                .syntax()
                                .children()
                                .find_map(smelt_parser::ast::SmeltPathRef::cast)
                                .and_then(|r| r.segments().last().cloned());
                            if let Some(n) = path_ref_name {
                                Some(n)
                            } else {
                                // Try direct SmeltPathRef cast.
                                smelt_parser::ast::SmeltPathRef::cast(pos_arg.syntax().clone())
                                    .and_then(|r| r.segments().last().cloned())
                                    .or_else(|| {
                                        // Bare identifier: must start with a letter or
                                        // underscore (not a numeric literal like `42`).
                                        let arg_text = pos_arg.text().trim().to_string();
                                        let is_bare = !arg_text.is_empty()
                                            && arg_text
                                                .chars()
                                                .next()
                                                .is_some_and(|c| c.is_alphabetic() || c == '_')
                                            && arg_text
                                                .chars()
                                                .all(|c| c.is_alphanumeric() || c == '_');
                                        if is_bare {
                                            Some(arg_text)
                                        } else {
                                            None
                                        }
                                    })
                            }
                        };
                        let model_name = match model_name {
                            Some(n) => n,
                            None => continue,
                        };
                        let resolves = project
                            .map(|p| {
                                columns_of_for_table_expr(db, workspace, p, model_name.clone())
                                    .is_ok()
                            })
                            .unwrap_or(false);
                        if !resolves {
                            let call_range = node.text_range();
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                message: meta_reflection_diagnostic_message_with_table_expr(
                                    DiagnosticCode::ColumnsOfUnresolvableSchema,
                                    None,
                                    None,
                                    Some(&model_name),
                                ),
                                range: call_range,
                                code: Some(DiagnosticCode::ColumnsOfUnresolvableSchema),
                                data: None,
                            })
                            .accumulate(db);
                        }
                    }
                }
            }

            // Phase C (meta-language) — ColumnRefFieldUnknown HOF dispatcher.
            //
            // For each `map`/`filter` HOF call whose first argument is
            // `smelt.columns_of(…)`, walk the lambda body and emit
            // `ColumnRefFieldUnknown` for any `<param>.<field>` access where
            // `<field>` is not in the closed ColumnRef field set
            // `{name, type, is_numeric}`.
            //
            // This runs on MODEL select statements (the outer `select_stmt`).
            // Function-file SELECT bodies are handled separately in
            // `function_body_diagnostics_for_file`.
            {
                for diag in
                    function_body_check::check_hof_column_ref_field_diagnostics(&select_stmt)
                {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase D (meta-language) — wide-reflection accessor diagnostics.
            //
            // Walks every SMELT_PATH_CALL for `smelt.models.*` / `smelt.sources.*`
            // in the model SELECT statement.  Emits:
            //   - WideReflectionUnknownAccessor: unknown accessor name
            //   - WideReflectionUnexpectedArgument: argument to `all`
            //   - WithTagRequiresText: non-compile-time-Text argument to `with_tag`
            //   - WithTagNamedArgument: named argument to `with_tag`
            //
            // Uses an empty TypeContext (no ModelRef/SourceRef bindings exist at
            // the top-level model SELECT scope).
            {
                let phase_d_ctx = type_inference::TypeContext::new();
                for diag in type_inference::check_wide_reflection_diagnostics(
                    &select_stmt,
                    &phase_d_ctx,
                    text,
                ) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase D (meta-language) — ModelRef / SourceRef HOF field dispatcher.
            //
            // For each `map`/`filter` HOF call whose first argument is a
            // `smelt.models.*` / `smelt.sources.*` wide-reflection call, walk
            // the lambda body and emit `ModelRefFieldUnknown` /
            // `SourceRefFieldUnknown` for any `<param>.<field>` access where
            // `<field>` is not in the closed field set `{path, name, tags, columns}`.
            //
            // This runs on MODEL select statements (the outer `select_stmt`).
            // Function-file SELECT bodies are handled separately in
            // `function_body_diagnostics_for_file` via `check_function_select_body`.
            {
                for diag in function_body_check::check_hof_model_ref_source_ref_field_diagnostics(
                    &select_stmt,
                ) {
                    DiagnosticAcc(diag).accumulate(db);
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
                                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
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

        // Phase F (meta-language) — File-level dangling THEN_KW detection.
        //
        // The parser's error recovery may eject a bare `then` keyword to the
        // top-level FILE node when it appears in an unexpected expression
        // position (e.g. `SELECT then x FROM t`).  `check_ternary_expr_diagnostics`
        // walks only the SelectStmt subtree and cannot reach FILE-level tokens.
        // This block walks the FULL file syntax so dangling THEN_KW tokens are
        // always caught, regardless of where error recovery placed them.
        //
        // Emits: TernaryDanglingThen.
        {
            let file_syntax = ast.syntax().clone();
            for diag in type_inference::check_dangling_ternary_keywords(&file_syntax) {
                DiagnosticAcc(diag).accumulate(db);
            }
        }
    }
}

fn count_from_sources(select_stmt: &smelt_parser::ast::SelectStmt) -> usize {
    let mut count = 0;
    if let Some(from_clause) = select_stmt.from_clause() {
        count += from_clause.table_refs().count();
        count += from_clause.joins().count();
    }
    count
}
