use super::*;

impl Backend {
    pub(crate) async fn code_action_impl(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let request_range = params.range;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        // For multi-model files, resolve to the virtual path
        let (effective_path, line_offset) = if let Some((vp, adjusted_line)) = self
            .resolve_virtual_path(&path, request_range.start.line)
            .await
        {
            let offset = request_range.start.line - adjusted_line;
            (vp, offset)
        } else {
            (path.clone(), 0)
        };

        let db = self.snapshot().await;
        let text = file_text(&db, &effective_path);
        let converter = self.boundary_converter(&text).await;

        // Collect diagnostics overlapping the request range
        let all_diags = diagnostics_for(&db, &effective_path);

        // Adjust request range for virtual path offset
        let adj_start_line = request_range.start.line.saturating_sub(line_offset);
        let adj_end_line = request_range.end.line.saturating_sub(line_offset);

        let matching: Vec<_> = all_diags
            .into_iter()
            .filter(|d| {
                let r = converter.convert(d);
                // Diagnostic overlaps the request range
                !(r.end.line < adj_start_line
                    || (r.end.line == adj_start_line
                        && r.end.character < request_range.start.character)
                    || r.start.line > adj_end_line
                    || (r.start.line == adj_end_line
                        && r.start.character > request_range.end.character))
            })
            .collect();

        // Read sources.yml for YAML-editing code actions
        let project_root = file_project_root(&db, &effective_path);
        let sources_yml_content = project_sources_yaml(&db, &project_root);
        let sources_yml_path = project_root.join("sources.yml");

        let mut actions = Vec::new();

        // Diagnostic-based code actions
        for diag in &matching {
            use smelt_db::code_actions::CodeActionKind as CAK;

            let action_kinds = smelt_db::code_actions::generate_all_code_actions(
                diag,
                &text,
                &sources_yml_content,
            );
            for kind in action_kinds {
                match kind {
                    CAK::TextEdit(suggestion) => {
                        let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                            &text,
                            suggestion.range,
                        );
                        let range = Range {
                            start: Position::new(pr.start.line + line_offset, pr.start.character),
                            end: Position::new(pr.end.line + line_offset, pr.end.character),
                        };
                        let edit = TextEdit {
                            range,
                            new_text: suggestion.new_text,
                        };
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(uri.clone(), vec![edit]);
                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                changes: Some(changes),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                    CAK::CreateModel(suggestion) => {
                        // Build the new model file path in the same directory as the current file
                        let model_dir = effective_path.parent().unwrap_or(project_root.as_ref());
                        let new_file_path =
                            model_dir.join(format!("{}.sql", suggestion.model_name));
                        let new_file_uri =
                            Url::from_file_path(&new_file_path).unwrap_or_else(|_| uri.clone());

                        let document_changes = vec![
                            DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                                uri: new_file_uri.clone(),
                                options: None,
                                annotation_id: None,
                            })),
                            DocumentChangeOperation::Edit(TextDocumentEdit {
                                text_document: OptionalVersionedTextDocumentIdentifier {
                                    uri: new_file_uri,
                                    version: None,
                                },
                                edits: vec![OneOf::Left(TextEdit {
                                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                                    new_text: suggestion.skeleton_sql,
                                })],
                            }),
                        ];
                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                document_changes: Some(DocumentChanges::Operations(
                                    document_changes,
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                    CAK::YamlEdit(suggestion) => {
                        let yaml_uri =
                            Url::from_file_path(&sources_yml_path).unwrap_or_else(|_| uri.clone());
                        // Insert new lines after the specified line
                        let insert_line = (suggestion.insert_after_line + 1) as u32;
                        let new_text = suggestion.new_lines.join("\n") + "\n";
                        let edit = TextEdit {
                            range: Range::new(
                                Position::new(insert_line, 0),
                                Position::new(insert_line, 0),
                            ),
                            new_text,
                        };
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(yaml_uri, vec![edit]);
                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                changes: Some(changes),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                    CAK::PinSeedSchema(suggestion) => {
                        // Build the sidecar YAML content from inferred columns.
                        let mut yaml_content = String::from("columns:\n");
                        for (name, dtype) in &suggestion.inferred_columns {
                            yaml_content
                                .push_str(&format!("  - name: {}\n    type: {}\n", name, dtype));
                        }

                        let sidecar_uri = Url::from_file_path(&suggestion.sidecar_path)
                            .unwrap_or_else(|_| uri.clone());

                        let document_changes = vec![
                            DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                                uri: sidecar_uri.clone(),
                                options: None,
                                annotation_id: None,
                            })),
                            DocumentChangeOperation::Edit(TextDocumentEdit {
                                text_document: OptionalVersionedTextDocumentIdentifier {
                                    uri: sidecar_uri,
                                    version: None,
                                },
                                edits: vec![OneOf::Left(TextEdit {
                                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                                    new_text: yaml_content,
                                })],
                            }),
                        ];

                        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                            title: suggestion.title,
                            kind: Some(CodeActionKind::QUICKFIX),
                            edit: Some(WorkspaceEdit {
                                document_changes: Some(DocumentChanges::Operations(
                                    document_changes,
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                }
            }
        }

        // Cursor-based CTE refactorings
        if let Some(result) = smelt_db::code_actions::find_extract_cte_suggestion(
            &text,
            adj_start_line,
            request_range.start.character,
        ) {
            let edits: Vec<TextEdit> = result
                .edits
                .iter()
                .map(|e| {
                    let pr =
                        crate::diagnostics_boundary::text_range_to_lsp_codepoint(&text, e.range);
                    TextEdit {
                        range: Range {
                            start: Position::new(pr.start.line + line_offset, pr.start.character),
                            end: Position::new(pr.end.line + line_offset, pr.end.character),
                        },
                        new_text: e.new_text.clone(),
                    }
                })
                .collect();
            let mut changes = std::collections::HashMap::new();
            changes.insert(uri.clone(), edits);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: result.title,
                kind: Some(CodeActionKind::REFACTOR_EXTRACT),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        if let Some(result) = smelt_db::code_actions::find_inline_cte_suggestion(
            &text,
            adj_start_line,
            request_range.start.character,
        ) {
            let edits: Vec<TextEdit> = result
                .edits
                .iter()
                .map(|e| {
                    let pr =
                        crate::diagnostics_boundary::text_range_to_lsp_codepoint(&text, e.range);
                    TextEdit {
                        range: Range {
                            start: Position::new(pr.start.line + line_offset, pr.start.character),
                            end: Position::new(pr.end.line + line_offset, pr.end.character),
                        },
                        new_text: e.new_text.clone(),
                    }
                })
                .collect();
            let mut changes = std::collections::HashMap::new();
            changes.insert(uri.clone(), edits);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: result.title,
                kind: Some(CodeActionKind::REFACTOR_INLINE),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    pub(crate) async fn prepare_rename_impl(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

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

        let db = self.snapshot().await;
        let text = file_text(&db, &effective_path);
        let file_input = lookup_file(&db, &effective_path);
        let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
        let syntax = parse.as_ref().map(|p| p.syntax());

        let result = if let Some(syntax) = syntax {
            if let Some(file) = AstFile::cast(syntax) {
                let offset = position_to_offset(
                    &text,
                    effective_position.line,
                    effective_position.character,
                );
                match symbol_at_cursor(&file, &text, offset) {
                    Some(SymbolAtCursor::CteDefinition { name })
                    | Some(SymbolAtCursor::CteReference { name }) => {
                        // Find the CTE definition's name range for prepareRename
                        let mut found_range = None;
                        if let Some(select_stmt) = file.select_stmt() {
                            if let Some(with_clause) = select_stmt.with_clause() {
                                for cte in with_clause.ctes() {
                                    if cte.name().as_deref() == Some(&name) {
                                        if let Some(name_range) = cte.name_range() {
                                            let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                &text, name_range,
                                            );
                                            found_range = Some((
                                                r.start.line,
                                                r.start.character,
                                                r.end.line,
                                                r.end.character,
                                            ));
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        found_range.map(|(sl, sc, el, ec)| (sl, sc, el, ec, name))
                    }
                    Some(SymbolAtCursor::ColumnRef { qualifier: _, name }) => {
                        // For column references, find the IDENT token at the cursor
                        // and return its range
                        let mut best_range = None;
                        let mut best_len = usize::MAX;
                        for node in file.syntax().descendants() {
                            if let Some(expr) = smelt_parser::ast::Expr::cast(node) {
                                let range = expr.text_range();
                                let start: usize = range.start().into();
                                let end: usize = range.end().into();
                                let len = end - start;
                                if offset >= start && offset <= end && len <= best_len {
                                    if let Some(col_ref) = expr.as_column_ref() {
                                        if col_ref.name() == name {
                                            // Get the name IDENT token range
                                            let tokens: Vec<_> = expr
                                                .syntax()
                                                .children_with_tokens()
                                                .filter_map(|e| e.into_token())
                                                .filter(|t| {
                                                    t.kind() == smelt_parser::SyntaxKind::IDENT
                                                        || t.kind() == smelt_parser::SyntaxKind::DOT
                                                })
                                                .collect();
                                            let name_token = if tokens.len() >= 3 {
                                                Some(&tokens[2]) // qualified: table.column
                                            } else if tokens.len() == 1 {
                                                Some(&tokens[0]) // unqualified
                                            } else {
                                                None
                                            };
                                            if let Some(tok) = name_token {
                                                let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                    &text,
                                                    tok.text_range(),
                                                );
                                                best_range = Some((
                                                    r.start.line,
                                                    r.start.character,
                                                    r.end.line,
                                                    r.end.character,
                                                ));
                                                best_len = len;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Refuse rename if the column comes from an externally-managed
                        // source table (smelt.sources.*). Source columns are declared in
                        // YAML and must be renamed at the data source, not via LSP.
                        if best_range.is_some() {
                            if let (Some(fi), Some(ws)) = (file_input, Workspace::try_get(&db)) {
                                let path_refs = smelt_db::model_path_refs(&db, fi);
                                let project_root = file_project_root(&db, &effective_path);
                                let maybe_project =
                                    crate::db_helpers::lookup_project(&db, &project_root);
                                if let Some(project) = maybe_project {
                                    for pr in path_refs.iter() {
                                        if !pr.in_table_expr_position {
                                            continue;
                                        }
                                        if let Some(resolved) =
                                            smelt_db::resolve_ref_path(&db, ws, pr.path.clone())
                                        {
                                            if resolved.kind == smelt_db::RefKind::Source {
                                                // Legacy sources.yml: path is ["sources", src, tbl]
                                                let is_source_col = if pr.path.len() >= 3
                                                    && pr.path[0] == "sources"
                                                {
                                                    let src = pr.path[pr.path.len() - 2].clone();
                                                    let tbl = pr.path[pr.path.len() - 1].clone();
                                                    smelt_db::resolve_source(&db, project, src, tbl)
                                                        .map(|table_def| {
                                                            table_def
                                                                .columns
                                                                .iter()
                                                                .any(|c| c.name == name)
                                                        })
                                                        .unwrap_or(false)
                                                } else {
                                                    // Per-entity source — any column from it is a source column
                                                    true
                                                };
                                                if is_source_col {
                                                    return Err(tower_lsp::jsonrpc::Error {
                                                        code: tower_lsp::jsonrpc::ErrorCode::ServerError(-32001),
                                                        message: std::borrow::Cow::Owned(format!(
                                                            "Cannot rename '{}': it is declared by an \
                                                             externally-managed source table. Rename the \
                                                             column at the data source instead.",
                                                            name
                                                        )),
                                                        data: None,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        best_range.map(|(sl, sc, el, ec)| (sl, sc, el, ec, name))
                    }
                    Some(SymbolAtCursor::PathRef { segments }) => {
                        // For path refs, return the range of the entire smelt.<path> node
                        for path_ref in file
                            .syntax()
                            .descendants()
                            .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                        {
                            if path_ref.segments() == segments {
                                let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                    &text,
                                    path_ref.text_range(),
                                );
                                let placeholder = segments.last().cloned().unwrap_or_default();
                                return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                                    range: Range {
                                        start: Position::new(r.start.line, r.start.character),
                                        end: Position::new(r.end.line, r.end.character),
                                    },
                                    placeholder,
                                }));
                            }
                        }
                        return Ok(None);
                    }
                    _ => {
                        // Try lambda-parameter prepare-rename as a fallback.
                        if let Some((start_byte, end_byte, placeholder)) =
                            crate::rename_lambda::prepare_rename_lambda_param(&file, &text, offset)
                        {
                            use smelt_parser::TextRange;
                            let range = TextRange::new(
                                (start_byte as u32).into(),
                                (end_byte as u32).into(),
                            );
                            let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                &text, range,
                            );
                            return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                                range: Range {
                                    start: Position::new(r.start.line, r.start.character),
                                    end: Position::new(r.end.line, r.end.character),
                                },
                                placeholder,
                            }));
                        }
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        match result {
            Some((sl, sc, el, ec, placeholder)) => {
                Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: Range {
                        start: Position::new(sl, sc),
                        end: Position::new(el, ec),
                    },
                    placeholder,
                }))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn rename_impl(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        // Validate that new_name is a valid SQL identifier
        if !is_valid_sql_identifier(&new_name) {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(format!(
                "'{}' is not a valid SQL identifier",
                new_name
            )));
        }

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

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

        enum RenameKind {
            Cte {
                edits: Vec<(u32, u32, u32, u32)>,
            },
            Model {
                #[allow(dead_code)]
                model_name: String,
                /// (file_path, start_line, start_col, end_line, end_col)
                edits: Vec<(PathBuf, u32, u32, u32, u32)>,
                /// old .sql file path (if it exists in the project)
                old_model_path: Option<PathBuf>,
            },
            // RenameKind::Source removed in Phase 4: smelt.source() is a parse error;
            // source renames are handled through path-form refs (smelt.sources.*).
            Column {
                /// Local edits in the current file: (start_line, start_col, end_line, end_col)
                local_edits: Vec<(u32, u32, u32, u32)>,
                /// Cross-file edits: (file_path, start_line, start_col, end_line, end_col)
                cross_file_edits: Vec<(PathBuf, u32, u32, u32, u32)>,
            },
            /// Lambda parameter — binder + every use in the lambda body.
            LambdaParam {
                /// (start_line, start_col, end_line, end_col) for each renamed span.
                edits: Vec<(u32, u32, u32, u32)>,
            },
        }

        let rename_kind = {
            let db = self.snapshot().await;
            let text = file_text(&db, &effective_path);
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            let syntax = parse.as_ref().map(|p| p.syntax());

            if let Some(syntax) = syntax {
                if let Some(file) = AstFile::cast(syntax) {
                    let offset = position_to_offset(
                        &text,
                        effective_position.line,
                        effective_position.character,
                    );
                    match symbol_at_cursor(&file, &text, offset) {
                        Some(SymbolAtCursor::CteDefinition { name })
                        | Some(SymbolAtCursor::CteReference { name }) => {
                            let cte_refs =
                                smelt_db::references::find_cte_references(&file, &text, &name);
                            let edits = cte_refs
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
                            Some(RenameKind::Cte { edits })
                        }
                        Some(SymbolAtCursor::ColumnRef {
                            qualifier,
                            name: column_name,
                        }) => {
                            // Find all column references in the current file
                            let local_refs = smelt_db::references::find_column_references_in_file(
                                &file,
                                &column_name,
                                qualifier.as_deref(),
                            );
                            let mut local_edits: Vec<(u32, u32, u32, u32)> = local_refs
                                .iter()
                                .map(|r| {
                                    let range =
                                        crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                            &text,
                                            r.name_range,
                                        );
                                    (
                                        range.start.line,
                                        range.start.character,
                                        range.end.line,
                                        range.end.character,
                                    )
                                })
                                .collect();

                            // Include column definition in SELECT list
                            if let Some(def_range) =
                                smelt_db::references::find_column_definition_in_select(
                                    &file,
                                    &column_name,
                                )
                            {
                                let range =
                                    crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                        &text, def_range,
                                    );
                                let edit = (
                                    range.start.line,
                                    range.start.character,
                                    range.end.line,
                                    range.end.character,
                                );
                                if !local_edits.contains(&edit) {
                                    local_edits.push(edit);
                                }
                            }
                            local_edits.sort();
                            local_edits.dedup();

                            // Cross-file tracing
                            let mut cross_file_edits = Vec::new();
                            let all_files = all_file_paths(&db);
                            let trace_project_root = file_project_root(&db, &effective_path);
                            let schema = file_input
                                .map(|f| smelt_db::model_schema(&db, f))
                                .unwrap_or_else(|| Arc::new(smelt_db::ModelSchema::empty()));
                            let ws = Workspace::try_get(&db);
                            let ctx = file_input
                                .and_then(|f| ws.map(|w| smelt_db::type_context(&db, w, f)))
                                .unwrap_or_else(|| Arc::new(smelt_db::TypeContext::new()));

                            // Upstream tracing: resolve which models to check
                            // First try ColumnSource::FromModel (column is in SELECT list)
                            let mut upstream_traced = false;
                            if let Some(col) = schema.columns.iter().find(|c| c.name == column_name)
                            {
                                if let smelt_db::schema::ColumnSource::FromModel {
                                    model_name,
                                    column_name: ref upstream_col,
                                } = &col.source
                                {
                                    if upstream_col == &column_name {
                                        trace_upstream_column(
                                            &db,
                                            &all_files,
                                            &trace_project_root,
                                            model_name,
                                            &column_name,
                                            &mut cross_file_edits,
                                        );
                                        upstream_traced = true;
                                    }
                                }
                            }

                            // If column not in schema (used in expressions like e.col ->> 'key'),
                            // resolve the qualifier alias to find the upstream model
                            if !upstream_traced {
                                let model_names: Vec<String> = if let Some(ref q) = qualifier {
                                    // Resolve alias (e.g., "e" -> "events")
                                    let resolved =
                                        ctx.resolve_alias(q).unwrap_or_else(|| q.to_string());
                                    if !ctx.is_cte(&resolved) {
                                        vec![resolved]
                                    } else {
                                        vec![]
                                    }
                                } else {
                                    collect_from_model_names(&db, &effective_path)
                                };

                                for model_name in &model_names {
                                    trace_upstream_column(
                                        &db,
                                        &all_files,
                                        &trace_project_root,
                                        model_name,
                                        &column_name,
                                        &mut cross_file_edits,
                                    );
                                }
                            }

                            // Downstream tracing via BFS through model graph.
                            //
                            // Fix 1: Root the BFS at the definition site, not at the
                            // cursor's file. The definition site is the model that
                            // actually defines `column_name` in its SELECT list. Without
                            // this, sibling consumers of the definition model are missed.
                            let current_model_name = effective_path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_string();

                            // Find the initial upstream model to start the definition search
                            let init_upstream_name = if upstream_traced {
                                schema
                                    .columns
                                    .iter()
                                    .find(|c| c.name == column_name)
                                    .and_then(|col| {
                                        if let smelt_db::ColumnSource::FromModel {
                                            model_name: ref mn,
                                            ..
                                        } = col.source
                                        {
                                            Some(mn.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_else(|| current_model_name.clone())
                            } else {
                                current_model_name.clone()
                            };

                            let definition_model_name = find_definition_model_name(
                                &db,
                                &all_files,
                                &trace_project_root,
                                &init_upstream_name,
                                &column_name,
                            );

                            let mut models_exposing: Vec<String> =
                                vec![definition_model_name.clone()];
                            let mut visited = std::collections::HashSet::new();
                            visited.insert(definition_model_name);
                            // Also mark current model as visited to skip its local edits
                            // which are already in local_edits.
                            visited.insert(current_model_name);
                            let mut depth = 0;

                            // Workspace handle for Fix 2: path-ref resolution
                            let bfs_ws = Workspace::try_get(&db);

                            while depth < 10 {
                                let mut next_batch = Vec::new();
                                for exposing in &models_exposing {
                                    for down_path in all_files.iter() {
                                        if *down_path == effective_path {
                                            continue;
                                        }
                                        let down_name = down_path
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if visited.contains(&down_name) {
                                            continue;
                                        }
                                        let down_file_input = match lookup_file(&db, down_path) {
                                            Some(f) => f,
                                            None => continue,
                                        };

                                        // Fix 2: check if this downstream model references
                                        // the exposing model using resolve_ref_path (W1
                                        // universal addressing: `smelt.X` not `smelt.models.X`).
                                        let down_model_path_refs =
                                            smelt_db::model_path_refs(&db, down_file_input);
                                        let references_exposing =
                                            down_model_path_refs.iter().any(|r| {
                                                // Legacy: smelt.models.X
                                                let legacy_match =
                                                    r.path.first().map(|s| s.as_str())
                                                        == Some("models")
                                                        && r.path.get(1).map(|s| s.as_str())
                                                            == Some(exposing.as_str());
                                                if legacy_match {
                                                    return true;
                                                }
                                                // W1 universal addressing: resolve the path
                                                // and compare file stems.
                                                if let Some(ws) = bfs_ws {
                                                    if let Some(resolved) =
                                                        smelt_db::resolve_ref_path(
                                                            &db,
                                                            ws,
                                                            r.path.clone(),
                                                        )
                                                    {
                                                        if resolved.kind == smelt_db::RefKind::Model
                                                        {
                                                            if let Some(sf) = resolved.source_file {
                                                                let sf_stem = sf
                                                                    .path(&db)
                                                                    .file_stem()
                                                                    .and_then(|s| s.to_str())
                                                                    .unwrap_or("");
                                                                return sf_stem
                                                                    == exposing.as_str();
                                                            }
                                                        }
                                                    }
                                                }
                                                false
                                            });
                                        if !references_exposing {
                                            continue;
                                        }

                                        let down_text = down_file_input.text(&db).clone();
                                        let down_parse = smelt_db::parse_file(&db, down_file_input);
                                        let down_syntax = down_parse.syntax();
                                        if let Some(down_file) = AstFile::cast(down_syntax) {
                                            let col_refs =
                                                smelt_db::references::find_column_references_in_file(
                                                    &down_file,
                                                    &column_name,
                                                    None,
                                                );
                                            for col_ref in &col_refs {
                                                let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                    &down_text,
                                                    col_ref.name_range,
                                                );
                                                cross_file_edits.push((
                                                    down_path.clone(),
                                                    r.start.line,
                                                    r.start.character,
                                                    r.end.line,
                                                    r.end.character,
                                                ));
                                            }
                                            // Fix 3: propagate BFS if the downstream model
                                            // exposes the column — either via SELECT *
                                            // (row_extensions) or explicit passthrough
                                            // (column in output schema under same name).
                                            let down_schema =
                                                smelt_db::model_schema(&db, down_file_input);
                                            let propagates = down_schema
                                                .row_extensions
                                                .iter()
                                                .any(|ext| ext.ref_name == *exposing)
                                                || down_schema
                                                    .columns
                                                    .iter()
                                                    .any(|c| c.name == column_name);
                                            if propagates {
                                                next_batch.push(down_name.clone());
                                            }
                                            visited.insert(down_name);
                                        }
                                    }
                                }
                                if next_batch.is_empty() {
                                    break;
                                }
                                models_exposing = next_batch;
                                depth += 1;
                            }

                            // Deduplicate cross-file edits in case both upstream trace
                            // and downstream BFS added an edit for the same range.
                            cross_file_edits.sort();
                            cross_file_edits.dedup();

                            Some(RenameKind::Column {
                                local_edits,
                                cross_file_edits,
                            })
                        }
                        Some(SymbolAtCursor::PathRef { segments })
                            if segments.first().map(|s| s.as_str()) == Some("models") =>
                        {
                            if let Some(model_name) = segments.get(1).cloned() {
                                // Collect all smelt.models.<model_name> path-ref ranges across workspace
                                let ws = Workspace::try_get(&db);
                                let ws_files = ws.map(|w| w.files(&db).clone()).unwrap_or_default();
                                let mut edits: Vec<(PathBuf, u32, u32, u32, u32)> = Vec::new();
                                for f in &ws_files {
                                    let path = f.path(&db).clone();
                                    let f_parse = smelt_db::parse_file(&db, *f);
                                    let f_text = f.text(&db).clone();
                                    let f_syntax = f_parse.syntax();
                                    if let Some(f_file) = AstFile::cast(f_syntax) {
                                        for path_ref in f_file
                                            .syntax()
                                            .descendants()
                                            .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                                        {
                                            let segs = path_ref.segments();
                                            if segs.first().map(|s| s.as_str()) == Some("models")
                                                && segs.get(1).map(|s| s.as_str())
                                                    == Some(model_name.as_str())
                                            {
                                                let r = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                                                    &f_text,
                                                    path_ref.text_range(),
                                                );
                                                edits.push((
                                                    path.clone(),
                                                    r.start.line,
                                                    r.start.character,
                                                    r.end.line,
                                                    r.end.character,
                                                ));
                                            }
                                        }
                                    }
                                }

                                // Compute old model file path using the full path
                                // tuple from the parsed ref (project-isolation-safe).
                                let old_model_path = ws.and_then(|w| {
                                    smelt_db::resolve_ref_path(&db, w, segments.clone())
                                        .and_then(|r| r.source_file)
                                        .map(|sf| sf.path(&db).clone())
                                });

                                Some(RenameKind::Model {
                                    model_name,
                                    edits,
                                    old_model_path,
                                })
                            } else {
                                None
                            }
                        }
                        _ => {
                            // Try lambda-parameter rename as a fallback.
                            match crate::rename_lambda::rename_lambda_param(
                                &file, &text, offset, &new_name,
                            ) {
                                Ok(crate::rename_lambda::RenameLambdaResult::Edits(byte_edits)) => {
                                    let lsp_edits = crate::rename_lambda::byte_edits_to_lsp_ranges(
                                        &text, byte_edits,
                                    );
                                    Some(RenameKind::LambdaParam { edits: lsp_edits })
                                }
                                Ok(crate::rename_lambda::RenameLambdaResult::NotALambdaParam) => {
                                    None
                                }
                                Err(e) => {
                                    return Err(tower_lsp::jsonrpc::Error::invalid_params(
                                        e.to_string(),
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }; // end of block — parse/syntax dropped before awaits

        match rename_kind {
            Some(RenameKind::Cte { edits }) => {
                if edits.is_empty() {
                    return Ok(None);
                }
                let text_edits: Vec<TextEdit> = edits
                    .into_iter()
                    .map(|(sl, sc, el, ec)| TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    })
                    .collect();
                let mut changes = HashMap::new();
                changes.insert(uri, text_edits);
                Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }))
            }
            Some(RenameKind::Model {
                model_name: _,
                edits,
                old_model_path,
            }) => {
                if edits.is_empty() && old_model_path.is_none() {
                    return Ok(None);
                }

                // Build DocumentChanges with text edits per file + optional RenameFile
                let mut document_changes: Vec<DocumentChangeOperation> = Vec::new();

                // Group text edits by file path
                let mut edits_by_file: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
                for (file_path, sl, sc, el, ec) in edits {
                    edits_by_file.entry(file_path).or_default().push(TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    });
                }

                // Add text edit operations
                for (file_path, file_edits) in edits_by_file {
                    let file_uri = Url::from_file_path(&file_path).unwrap_or_else(|_| uri.clone());
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: file_uri,
                            version: None,
                        },
                        edits: file_edits.into_iter().map(OneOf::Left).collect(),
                    }));
                }

                // Add file rename operation and update Salsa DB
                if let Some(old_path) = old_model_path {
                    let new_path = old_path
                        .parent()
                        .unwrap_or(old_path.as_ref())
                        .join(format!("{}.sql", new_name));
                    let old_uri = Url::from_file_path(&old_path).unwrap_or_else(|_| uri.clone());
                    let new_uri = Url::from_file_path(&new_path).unwrap_or_else(|_| uri.clone());
                    document_changes.push(DocumentChangeOperation::Op(ResourceOp::Rename(
                        RenameFile {
                            old_uri,
                            new_uri,
                            options: None,
                            annotation_id: None,
                        },
                    )));

                    // Pre-update the Salsa DB so diagnostics see the new filename
                    // before VSCode sends didOpen/didChange notifications.
                    let mut db = self.db.lock().await;
                    let old_text = file_text(&db, &old_path);
                    let old_project_root = file_project_root(&db, &old_path);
                    db.set_source_file(new_path.clone(), old_text, old_project_root);
                    let mut tracked = self.tracked_files.lock().await;
                    tracked.retain(|p| *p != old_path);
                    if !tracked.contains(&new_path) {
                        tracked.push(new_path);
                    }
                    let project_roots = self.project_roots.lock().await.clone();
                    Backend::sync_workspace(&mut db, &tracked, &project_roots);
                    drop(tracked);
                    drop(db);
                }

                Ok(Some(WorkspaceEdit {
                    document_changes: Some(DocumentChanges::Operations(document_changes)),
                    ..Default::default()
                }))
            }
            Some(RenameKind::Column {
                local_edits,
                cross_file_edits,
            }) => {
                if local_edits.is_empty() && cross_file_edits.is_empty() {
                    return Ok(None);
                }

                let mut document_changes: Vec<DocumentChangeOperation> = Vec::new();

                // Local edits in the current file
                if !local_edits.is_empty() {
                    let local_text_edits: Vec<OneOf<TextEdit, AnnotatedTextEdit>> = local_edits
                        .into_iter()
                        .map(|(sl, sc, el, ec)| {
                            OneOf::Left(TextEdit {
                                range: Range {
                                    start: Position::new(sl, sc),
                                    end: Position::new(el, ec),
                                },
                                new_text: new_name.clone(),
                            })
                        })
                        .collect();
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: uri.clone(),
                            version: None,
                        },
                        edits: local_text_edits,
                    }));
                }

                // Cross-file edits
                let mut edits_by_file: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
                for (file_path, sl, sc, el, ec) in cross_file_edits {
                    edits_by_file.entry(file_path).or_default().push(TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    });
                }
                for (file_path, file_edits) in edits_by_file {
                    let file_uri = Url::from_file_path(&file_path).unwrap_or_else(|_| uri.clone());
                    document_changes.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: file_uri,
                            version: None,
                        },
                        edits: file_edits.into_iter().map(OneOf::Left).collect(),
                    }));
                }

                Ok(Some(WorkspaceEdit {
                    document_changes: Some(DocumentChanges::Operations(document_changes)),
                    ..Default::default()
                }))
            }
            Some(RenameKind::LambdaParam { edits }) => {
                if edits.is_empty() {
                    return Ok(None);
                }
                let text_edits: Vec<TextEdit> = edits
                    .into_iter()
                    .map(|(sl, sc, el, ec)| TextEdit {
                        range: Range {
                            start: Position::new(sl, sc),
                            end: Position::new(el, ec),
                        },
                        new_text: new_name.clone(),
                    })
                    .collect();
                let mut changes = HashMap::new();
                changes.insert(uri, text_edits);
                Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }))
            }
            None => Ok(None),
        }
    }
}
