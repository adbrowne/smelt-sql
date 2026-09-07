use super::*;

impl Backend {
    pub(crate) async fn goto_definition_impl(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path and adjust position
        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        // Resolve goto-definition target while holding the db snapshot and AST.
        // We collect the result as plain data (no Rowan nodes) so we can drop
        // the non-Send AST before any await points.
        enum GotoTarget {
            RefModel(PathBuf),
            /// CTE definition in the same file — target is an LSP Range
            SameFile(Range),
            /// Column definitions (potentially multiple for ambiguous refs)
            ColumnDefs(Vec<ColumnDefLocation>),
            /// Lambda parameter binder in the same file (Phase B).
            LambdaParam {
                binder_start: u32,
                binder_col: u32,
                /// End column of the binder token (exclusive), so the
                /// full param name is highlighted, not just the first char.
                binder_end_col: u32,
            },
            /// smelt.config.var('x') — resolves to a line in smelt.yml (Phase B).
            ConfigVarYml {
                yml_path: PathBuf,
                line: u32,
            },
            /// Phase E2: goto-def from a generator-emitted model reference to the
            /// emitting `ModelDef.name` field's value-token in the generator file.
            EmittedModelRef {
                gen_file: PathBuf,
                name_range: Range,
            },
            /// Goto-def from a `smelt.functions.<name>(...)` call to the
            /// `smelt.define <name>(...)` declaration. Lands the cursor on
            /// the name token (precise position derived from the file's
            /// current text via `name_range`).
            FunctionDef {
                target_file: PathBuf,
                name_start: u32,
                name_end: u32,
            },
        }

        let target = {
            let db = self.snapshot().await;
            let text = file_text(&db, &effective_path);
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            let syntax = parse.as_ref().map(|p| p.syntax());
            let cursor_offset =
                position_to_offset(&text, effective_position.line, effective_position.character);

            if let Some(syntax) = syntax {
                if let Some(file) = AstFile::cast(syntax) {
                    match symbol_at_cursor(&file, &text, cursor_offset) {
                        Some(SymbolAtCursor::CteReference { name }) => {
                            // Jump to CTE definition
                            let mut result = None;
                            if let Some(select_stmt) = file.select_stmt() {
                                if let Some(with_clause) = select_stmt.with_clause() {
                                    for cte in with_clause.ctes() {
                                        if cte.name().as_deref() == Some(name.as_str()) {
                                            let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                &text,
                                                cte.syntax().text_range(),
                                            );
                                            result = Some(GotoTarget::SameFile(Range {
                                                start: Position::new(
                                                    pr.start.line,
                                                    pr.start.character,
                                                ),
                                                end: Position::new(pr.end.line, pr.end.character),
                                            }));
                                            break;
                                        }
                                    }
                                }
                            }
                            result
                        }
                        Some(SymbolAtCursor::CteDefinition { .. }) => {
                            // Already at definition site — no-op
                            None
                        }
                        Some(SymbolAtCursor::ColumnRef { qualifier, name }) => {
                            // Check if cursor is on the qualifier token — if so, jump to
                            // the CTE or table alias definition rather than doing column resolution
                            let cursor_on_qualifier = qualifier.is_some() && {
                                // Find the tightest Expr at cursor and check if cursor is on first IDENT
                                let mut best_expr: Option<smelt_parser::ast::Expr> = None;
                                let mut best_len = usize::MAX;
                                for node in file.syntax().descendants() {
                                    if let Some(expr) = smelt_parser::ast::Expr::cast(node) {
                                        let range = expr.text_range();
                                        let start: usize = range.start().into();
                                        let end: usize = range.end().into();
                                        let len = end - start;
                                        if cursor_offset >= start
                                            && cursor_offset <= end
                                            && len <= best_len
                                        {
                                            best_len = len;
                                            best_expr = Some(expr);
                                        }
                                    }
                                }
                                best_expr
                                    .map(|expr| {
                                        use smelt_parser::SyntaxKind::{DOT, IDENT};
                                        expr.syntax()
                                            .children_with_tokens()
                                            .filter_map(|e| e.into_token())
                                            .find(|t| t.kind() == IDENT || t.kind() == DOT)
                                            .map(|first_ident| {
                                                let start: usize =
                                                    first_ident.text_range().start().into();
                                                let end: usize =
                                                    first_ident.text_range().end().into();
                                                first_ident.kind() == IDENT
                                                    && cursor_offset >= start
                                                    && cursor_offset <= end
                                            })
                                            .unwrap_or(false)
                                    })
                                    .unwrap_or(false)
                            };

                            if cursor_on_qualifier {
                                let qualifier_str = qualifier.as_deref().unwrap();
                                let mut result = None;

                                // Check if qualifier is a CTE name
                                if let Some(select_stmt) = file.select_stmt() {
                                    if let Some(with_clause) = select_stmt.with_clause() {
                                        for cte in with_clause.ctes() {
                                            if cte.name().as_deref() == Some(qualifier_str) {
                                                let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                    &text,
                                                    cte.syntax().text_range(),
                                                );
                                                result = Some(GotoTarget::SameFile(Range {
                                                    start: Position::new(
                                                        pr.start.line,
                                                        pr.start.character,
                                                    ),
                                                    end: Position::new(pr.end.line, pr.end.character),
                                                }));
                                                break;
                                            }
                                        }
                                    }
                                }

                                // Check if qualifier is a table alias in FROM/JOIN
                                if result.is_none() {
                                    if let Some(select_stmt) = file.select_stmt() {
                                        if let Some(from_clause) = select_stmt.from_clause() {
                                            let table_refs: Vec<_> = from_clause
                                                .table_refs()
                                                .chain(
                                                    from_clause
                                                        .joins()
                                                        .filter_map(|j| j.table_ref()),
                                                )
                                                .collect();

                                            for table_ref in table_refs {
                                                let matches = table_ref.alias().as_deref()
                                                    == Some(qualifier_str)
                                                    || table_ref.identifier().as_deref()
                                                        == Some(qualifier_str);
                                                if matches {
                                                    let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                        &text,
                                                        table_ref.syntax().text_range(),
                                                    );
                                                    result = Some(GotoTarget::SameFile(Range {
                                                        start: Position::new(
                                                            pr.start.line,
                                                            pr.start.character,
                                                        ),
                                                        end: Position::new(
                                                            pr.end.line,
                                                            pr.end.character,
                                                        ),
                                                    }));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                result
                            } else {
                                let defs = resolve_column_definitions(
                                    &db,
                                    &effective_path,
                                    qualifier.as_deref(),
                                    &name,
                                );
                                if !defs.is_empty() {
                                    Some(GotoTarget::ColumnDefs(defs))
                                } else {
                                    None
                                }
                            }
                        }
                        Some(SymbolAtCursor::PathRef { segments }) => {
                            // Resolve via the unified path data plane (Phase 2a).
                            // SQL files come back via `source_file`; seeds and
                            // sources (which aren't Salsa SourceFiles) fall
                            // through to `resolve_seed_or_source_path` which
                            // returns the on-disk `.csv` / `.yml` path.
                            let ws = Workspace::try_get(&db);
                            ws.and_then(|w| {
                                if let Some(sf) =
                                    smelt_db::resolve_ref_path(&db, w, segments.clone())
                                        .and_then(|r| r.source_file)
                                {
                                    Some(GotoTarget::RefModel(sf.path(&db).clone()))
                                } else {
                                    smelt_db::resolve_seed_or_source_path(&db, w, segments)
                                        .map(GotoTarget::RefModel)
                                }
                            })
                        }
                        Some(SymbolAtCursor::FunctionCall { segments }) => {
                            // Route `smelt.functions.<name>(...)` calls to the
                            // `smelt.define <name>(...)` declaration. Other call
                            // shapes (e.g. `smelt.metrics.foo`, when that namespace
                            // ships) fall through to None.
                            //
                            // Project isolation: resolve against functions
                            // declared in the same project as the call site.
                            // See docs/specs/architecture.md → "Project
                            // isolation rule".
                            if segments.len() == 2 && segments[0] == "functions" {
                                let name = segments[1].clone();
                                let ws = Workspace::try_get(&db);
                                ws.and_then(|w| {
                                    let project = file_input.and_then(|sf| {
                                        smelt_db::find_project(
                                            &db,
                                            w,
                                            &sf.project_root(&db).clone(),
                                        )
                                    })?;
                                    smelt_db::resolve_function_path(&db, w, project, name).map(
                                        |(f, name_range)| GotoTarget::FunctionDef {
                                            target_file: f.path(&db).clone(),
                                            name_start: name_range.start,
                                            name_end: name_range.end,
                                        },
                                    )
                                })
                            } else {
                                None
                            }
                        }
                        Some(SymbolAtCursor::FunctionDefinition { .. }) => None,
                        None => None,
                    }
                    // Fall through to Phase B checks when symbol_at_cursor returned None
                    // (lambda params and config.var args are not yet handled by the symbol scanner).
                    .or_else(|| {
                        use smelt_parser::syntax_kind::SyntaxKind;

                        // Phase B: goto-def on a lambda parameter IDENT in the body —
                        // jump to the binding occurrence in LAMBDA_PARAM_LIST.
                        let lambda_node = file
                            .syntax()
                            .descendants()
                            .filter(|n| n.kind() == SyntaxKind::LAMBDA)
                            .filter(|n| {
                                let s: usize = n.text_range().start().into();
                                let e: usize = n.text_range().end().into();
                                cursor_offset >= s && cursor_offset <= e
                            })
                            .min_by_key(|n| {
                                let s: usize = n.text_range().start().into();
                                let e: usize = n.text_range().end().into();
                                e - s
                            });
                        if let Some(ln) = lambda_node {
                            if let Some(lambda) = smelt_parser::ast::Lambda::cast(ln) {
                                for param_name in lambda.params() {
                                    if let Some(binder_range) =
                                        lambda_param_binder_range(&lambda, &param_name)
                                    {
                                        // Only navigate when the cursor is on the binder
                                        // itself or on a body-use IDENT with the same
                                        // name.  Without this guard, any cursor position
                                        // inside the lambda (e.g. on `=>`, whitespace,
                                        // or an unrelated sub-expression) would jump.
                                        let binder_s: usize = binder_range.start().into();
                                        let binder_e: usize = binder_range.end().into();
                                        let on_binder =
                                            cursor_offset >= binder_s && cursor_offset <= binder_e;
                                        let on_body_use = lambda.body().is_some_and(|body| {
                                            body.syntax()
                                                .descendants_with_tokens()
                                                .filter_map(|e| e.into_token())
                                                .filter(|t| {
                                                    t.kind() == SyntaxKind::IDENT
                                                        && t.text() == param_name.as_str()
                                                })
                                                .any(|t| {
                                                    let s: usize = t.text_range().start().into();
                                                    let e: usize = t.text_range().end().into();
                                                    cursor_offset >= s && cursor_offset <= e
                                                })
                                        });
                                        if !on_binder && !on_body_use {
                                            continue;
                                        }
                                        // Convert the binder range to an LSP Range.
                                        let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                            &text,
                                            binder_range,
                                        );
                                        return Some(GotoTarget::LambdaParam {
                                            binder_start: pr.start.line,
                                            binder_col: pr.start.character,
                                            binder_end_col: pr.end.character,
                                        });
                                    }
                                }
                            }
                        }

                        // Phase B: goto-def on `smelt.config.var('x')` argument —
                        // jump to `vars.x:` line in smelt.yml.
                        let var_call = file
                            .syntax()
                            .descendants()
                            .filter_map(smelt_parser::ast::FunctionCall::cast)
                            .find(|c| {
                                c.name().as_deref() == Some("var") && {
                                    let s: usize = c.syntax().text_range().start().into();
                                    let e: usize = c.syntax().text_range().end().into();
                                    cursor_offset >= s && cursor_offset <= e
                                }
                            });
                        if let Some(vc) = var_call {
                            let args = vc.arguments();
                            if let Some(arg) = args.first() {
                                if smelt_db::config_vars::is_string_literal_expr(arg) {
                                    if let Some(var_name) =
                                        smelt_db::config_vars::extract_string_literal_value(arg)
                                    {
                                        let project_root = file_project_root(&db, &effective_path);
                                        let project = lookup_project(&db, &project_root);
                                        let smelt_yml_text = project
                                            .map(|p| p.smelt_yml_text(&db).clone())
                                            .unwrap_or_default();
                                        // Only navigate when the variable is actually declared
                                        // in smelt.yml; return None for undeclared vars so we
                                        // don't silently land at the top of the file.
                                        if let Some(line) =
                                            find_var_line_in_smelt_yml(&smelt_yml_text, &var_name)
                                        {
                                            let yml_path = project_root.join("smelt.yml");
                                            return Some(GotoTarget::ConfigVarYml {
                                                yml_path,
                                                line,
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        // Phase E2: goto-def on a `smelt.<path>` ref that resolves to a
                        // generator-emitted model — jump to the `ModelDef.name` field's
                        // value-token in the generator file.
                        //
                        // We look at `smelt_db::emitted_models` to find a survivor whose
                        // computed smelt path matches the dotted path under the cursor.
                        // The path must NOT be a `smelt.models.*` or `smelt.sources.*`
                        // accessor call — those are already handled above.
                        {
                            // Find the SmeltPathRef under the cursor (excluding models/sources).
                            let path_ref_under_cursor = file
                                .syntax()
                                .descendants()
                                .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                                .filter(|pr| {
                                    let segs = pr.segments();
                                    let first = segs.first().map(|s| s.as_str());
                                    first != Some("models") && first != Some("sources")
                                })
                                .find(|pr| {
                                    let r = pr.text_range();
                                    let s: usize = r.start().into();
                                    let e: usize = r.end().into();
                                    cursor_offset >= s && cursor_offset <= e
                                });

                            if let Some(pr) = path_ref_under_cursor {
                                let segments = pr.segments();
                                let cursor_path = segments.join(".");
                                let ws = Workspace::try_get(&db);
                                if let Some(w) = ws {
                                    let survivors = smelt_db::emitted_models(&db, w);
                                    let project_root = file_project_root(&db, &effective_path);
                                    let project = lookup_project(&db, &project_root);
                                    let scan_roots = project
                                        .map(|p| smelt_db::project_paths(&db, p).as_ref().clone())
                                        .unwrap_or_else(|| vec!["models".to_string()]);
                                    if let Some(em) = survivors.survivors.iter().find(|em| {
                                        let sp = smelt_db::emitted_model_smelt_path(
                                            &em.generator_file,
                                            &project_root,
                                            &scan_roots,
                                            &em.name,
                                        );
                                        sp == cursor_path
                                    }) {
                                        // Convert the name_span (TextRange) to an LSP Range.
                                        let gen_text = std::fs::read_to_string(&em.generator_file)
                                            .unwrap_or_default();
                                        let pr_range = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                            &gen_text,
                                            em.name_span,
                                        );
                                        let name_range = Range {
                                            start: Position::new(
                                                pr_range.start.line,
                                                pr_range.start.character,
                                            ),
                                            end: Position::new(
                                                pr_range.end.line,
                                                pr_range.end.character,
                                            ),
                                        };
                                        return Some(GotoTarget::EmittedModelRef {
                                            gen_file: em.generator_file.clone(),
                                            name_range,
                                        });
                                    }
                                }
                            }
                        }

                        None
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }; // end of block — parse/syntax dropped here, before any awaits

        // Convert target to LSP response
        match target {
            Some(GotoTarget::RefModel(target_path)) => {
                // Map virtual .sql paths back to .py sources
                let py_sources = self.python_model_sources.lock().await;
                let (actual_path, target_line) = py_sources
                    .get(&target_path)
                    .map(|(p, line)| (p.clone(), *line))
                    .unwrap_or((target_path, 0));
                drop(py_sources);

                if let Ok(target_uri) = Url::from_file_path(&actual_path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: Range {
                            start: Position::new(target_line, 0),
                            end: Position::new(target_line, 0),
                        },
                    })))
                } else {
                    Ok(None)
                }
            }
            Some(GotoTarget::SameFile(target_range)) => {
                if let Ok(target_uri) = Url::from_file_path(&path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: target_range,
                    })))
                } else {
                    Ok(None)
                }
            }
            Some(GotoTarget::ColumnDefs(defs)) => {
                let locations: Vec<Location> = defs
                    .iter()
                    .filter_map(|def| {
                        Url::from_file_path(&def.path).ok().map(|uri| Location {
                            uri,
                            range: Range {
                                start: Position::new(def.line, def.col),
                                end: Position::new(def.end_line, def.end_col),
                            },
                        })
                    })
                    .collect();

                match locations.len() {
                    0 => Ok(None),
                    1 => Ok(Some(GotoDefinitionResponse::Scalar(
                        locations.into_iter().next().unwrap(),
                    ))),
                    _ => Ok(Some(GotoDefinitionResponse::Array(locations))),
                }
            }
            // Phase B: lambda param binder — jump to binder in same file.
            Some(GotoTarget::LambdaParam {
                binder_start,
                binder_col,
                binder_end_col,
            }) => {
                if let Ok(target_uri) = Url::from_file_path(&path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: Range {
                            start: Position::new(binder_start, binder_col),
                            end: Position::new(binder_start, binder_end_col),
                        },
                    })))
                } else {
                    Ok(None)
                }
            }
            // Phase B: config.var goto — jump to vars.<name>: in smelt.yml.
            Some(GotoTarget::ConfigVarYml { yml_path, line }) => {
                if let Ok(target_uri) = Url::from_file_path(&yml_path) {
                    Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: target_uri,
                        range: Range {
                            start: Position::new(line, 0),
                            end: Position::new(line, 0),
                        },
                    })))
                } else {
                    Ok(None)
                }
            }
            // Phase E2: emitted-model ref — jump to the ModelDef.name value-token.
            Some(GotoTarget::EmittedModelRef {
                gen_file,
                name_range,
            }) => {
                if let Ok(target_uri) = Url::from_file_path(&gen_file) {
                    let loc = goto_def_for_emitted_model_reference(&gen_file, name_range);
                    if let Some(location) = loc {
                        Ok(Some(GotoDefinitionResponse::Scalar(Location {
                            uri: target_uri,
                            range: location.range,
                        })))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }
            // smelt.functions.<name>(...) → smelt.define <name>(...).
            // Convert the stored byte range to LSP line/col using the target
            // file's current text. Done outside the AST-holding block so
            // there's no Salsa snapshot lifetime issue.
            Some(GotoTarget::FunctionDef {
                target_file,
                name_start,
                name_end,
            }) => {
                let target_uri = match Url::from_file_path(&target_file) {
                    Ok(u) => u,
                    // intentionally ignored: non-absolute or non-file path → no
                    // goto-def location can be produced; return None to the editor.
                    Err(_) => return Ok(None),
                };
                let target_text = std::fs::read_to_string(&target_file).unwrap_or_default();
                // `define.name_range()` returns offsets into the
                // frontmatter-stripped source (parse_file strips before
                // parsing). Strip here too so byte→line/col mapping aligns.
                let stripped = smelt_parser::strip_frontmatter(&target_text);
                let start = crate::diagnostics_boundary::offset_to_codepoint_position(
                    &stripped,
                    name_start as usize,
                );
                let end = crate::diagnostics_boundary::offset_to_codepoint_position(
                    &stripped,
                    name_end as usize,
                );
                Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: target_uri,
                    range: Range {
                        start: Position::new(start.line, start.column),
                        end: Position::new(end.line, end.column),
                    },
                })))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn references_impl(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path and adjust position
        let (effective_path, effective_position) = if let Some((vp, adjusted_line)) =
            self.resolve_virtual_path(&path, position.line).await
        {
            (
                vp,
                Position {
                    line: adjusted_line,
                    character: position.character,
                },
            )
        } else {
            (path.clone(), position)
        };

        // Collect reference data as plain types.
        // We use an enum to avoid holding AST nodes across await points.
        enum RefResult {
            PathRanges(Vec<(PathBuf, rowan::TextRange)>),
            CteRanges(PathBuf, Vec<(u32, u32, u32, u32)>),
            Empty,
        }

        let ref_result = {
            let db = self.snapshot().await;
            let text = file_text(&db, &effective_path);
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            let syntax = parse.as_ref().map(|p| p.syntax());
            let cursor_offset =
                position_to_offset(&text, effective_position.line, effective_position.character);

            if let Some(syntax) = syntax {
                if let Some(file) = AstFile::cast(syntax) {
                    // Project-scope the search per architecture.md → "Project
                    // isolation rule": a workspace folder may contain multiple
                    // smelt projects, and references do not cross project
                    // boundaries. Derive the project from the cursor file.
                    let project_files: Vec<smelt_db::SourceFile> = {
                        let ws = Workspace::try_get(&db);
                        match (ws, file_input) {
                            (Some(w), Some(sf)) => {
                                let project_root = sf.project_root(&db).clone();
                                w.files(&db)
                                    .iter()
                                    .copied()
                                    .filter(|f| f.project_root(&db) == &project_root)
                                    .collect()
                            }
                            _ => Vec::new(),
                        }
                    };

                    match symbol_at_cursor(&file, &text, cursor_offset) {
                        Some(SymbolAtCursor::PathRef { segments }) => {
                            let mut all_refs: Vec<(PathBuf, rowan::TextRange)> = Vec::new();
                            for f in &project_files {
                                let path_refs = smelt_db::model_path_refs(&db, *f);
                                for loc in path_refs.iter() {
                                    if loc.path == segments {
                                        all_refs.push((f.path(&db).clone(), loc.range));
                                    }
                                }
                            }
                            RefResult::PathRanges(all_refs)
                        }
                        Some(SymbolAtCursor::FunctionCall { segments }) => {
                            // Only `smelt.functions.<name>` calls are findable
                            // today. Other call shapes have no def to anchor on.
                            if segments.len() == 2 && segments[0] == "functions" {
                                RefResult::PathRanges(collect_function_call_sites(
                                    &db,
                                    &project_files,
                                    &segments[1],
                                ))
                            } else {
                                RefResult::Empty
                            }
                        }
                        Some(SymbolAtCursor::FunctionDefinition { name }) => {
                            // Cursor on the `<name>` token of a
                            // `smelt.define <name>(...)` declaration — return
                            // every `smelt.functions.<name>(...)` call site
                            // in the same project.
                            RefResult::PathRanges(collect_function_call_sites(
                                &db,
                                &project_files,
                                &name,
                            ))
                        }
                        Some(SymbolAtCursor::CteDefinition { name })
                        | Some(SymbolAtCursor::CteReference { name }) => {
                            let cte_refs =
                                smelt_db::references::find_cte_references(&file, &text, &name);
                            let ranges: Vec<_> = cte_refs
                                .iter()
                                .map(|text_range| {
                                    let r =
                                        crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                            &text,
                                            *text_range,
                                        );
                                    (r.start.line, r.start.character, r.end.line, r.end.character)
                                })
                                .collect();
                            RefResult::CteRanges(effective_path.clone(), ranges)
                        }
                        _ => RefResult::Empty,
                    }
                } else {
                    RefResult::Empty
                }
            } else {
                RefResult::Empty
            }
        }; // end of block — parse/syntax dropped before awaits

        let locations = match ref_result {
            RefResult::PathRanges(refs) => self.ref_locations_to_lsp(&refs).await,
            RefResult::CteRanges(path, ranges) => ranges
                .into_iter()
                .filter_map(|(sl, sc, el, ec)| {
                    let uri = Url::from_file_path(&path).ok()?;
                    Some(Location {
                        uri,
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                    })
                })
                .collect(),
            RefResult::Empty => vec![],
        };

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }
}
