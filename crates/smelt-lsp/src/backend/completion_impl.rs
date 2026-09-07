use super::*;

impl Backend {
    pub(crate) async fn completion_impl(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
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

        let db = self.snapshot().await;

        // Get file content
        let text = file_text(&db, &effective_path);

        // Convert cursor position to offset
        let cursor_offset = {
            let mut offset = 0usize;
            let mut line = 0u32;
            let mut col = 0u32;

            for ch in text.chars() {
                if line == effective_position.line && col == effective_position.character {
                    break;
                }
                if ch == '\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
                offset += ch.len_utf8();
            }
            offset
        };

        // Phase E2: generator-file frontmatter completion — `generates: <cursor>`.
        // Detection: cursor is in the frontmatter of a Generator file, on a line
        // that starts with `generates:` (with optional partial value typed).
        {
            let raw = text.as_str();
            if let Ok(smelt_core::metadata::FileMetadata::Generator { body_offset, .. }) =
                smelt_core::metadata::extract_file_metadata(raw)
            {
                if cursor_offset < body_offset {
                    let line_start = raw[..cursor_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
                    let line_end = raw[cursor_offset..]
                        .find('\n')
                        .map(|p| cursor_offset + p)
                        .unwrap_or(raw.len());
                    let line_text = &raw[line_start..line_end];
                    if line_text.trim_start().starts_with("generates:") {
                        let items = completion_for_generates_value();
                        if !items.is_empty() {
                            return Ok(Some(CompletionResponse::Array(items)));
                        }
                    }
                }
            }
        }

        // Phase E2: ModelDef field-key completion — cursor inside a `ModelDef { <cursor> … }`
        // record literal in a generator file body.  Detection mirrors the hover dispatch.
        {
            let raw = text.as_str();
            if let Ok(smelt_core::metadata::FileMetadata::Generator { body_offset, .. }) =
                smelt_core::metadata::extract_file_metadata(raw)
            {
                if cursor_offset >= body_offset {
                    use smelt_parser::SyntaxKind;
                    let file_input = lookup_file(&db, &effective_path);
                    let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
                    if let Some(syntax) = parse.as_ref().map(|p| p.syntax()) {
                        if let Some(file_ast) = AstFile::cast(syntax) {
                            // Find the tightest ModelDef RECORD_LITERAL containing cursor.
                            let model_def_node = file_ast
                                .syntax()
                                .descendants()
                                .filter(|n| n.kind() == SyntaxKind::RECORD_LITERAL)
                                .filter(|n| {
                                    let s: usize = n.text_range().start().into();
                                    let e: usize = n.text_range().end().into();
                                    cursor_offset >= s && cursor_offset <= e
                                })
                                .filter(|n| {
                                    n.children_with_tokens()
                                        .filter_map(|e| e.into_token())
                                        .find(|t| !t.kind().is_trivia())
                                        .map(|t| t.text() == "ModelDef")
                                        .unwrap_or(false)
                                })
                                .min_by_key(|n| {
                                    let s: usize = n.text_range().start().into();
                                    let e: usize = n.text_range().end().into();
                                    e - s
                                });

                            if let Some(rec_node) = model_def_node {
                                // Collect already-filled field names.
                                let already_filled: Vec<String> = rec_node
                                    .children()
                                    .filter(|n| n.kind() == SyntaxKind::RECORD_FIELD)
                                    .filter_map(|field| {
                                        field
                                            .children_with_tokens()
                                            .filter_map(|e| e.into_token())
                                            .find(|t| !t.kind().is_trivia())
                                            .map(|t| t.text().to_string())
                                    })
                                    .collect();
                                let items = completion_for_model_def_field_key(&already_filled);
                                if !items.is_empty() {
                                    return Ok(Some(CompletionResponse::Array(items)));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase B: check for reduce second-arg position BEFORE the standard
        // context dispatch — this is a meta-language-specific completion that
        // should be offered regardless of the SQL-level context.
        {
            use smelt_parser::syntax_kind::SyntaxKind;
            let file_input = lookup_file(&db, &effective_path);
            let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
            if let Some(syntax) = parse.as_ref().map(|p| p.syntax()) {
                if let Some(file) = AstFile::cast(syntax) {
                    // Find a `reduce(...)` call where the cursor is in the second-arg position.
                    let reduce_call = file
                        .syntax()
                        .descendants()
                        .filter_map(smelt_parser::ast::FunctionCall::cast)
                        .find(|c| {
                            c.name().as_deref() == Some("reduce") || {
                                // keyword-name fallback (same as infer_hof)
                                c.syntax()
                                    .children_with_tokens()
                                    .filter_map(|e| e.into_token())
                                    .find(|t| matches!(t.kind(), SyntaxKind::IDENT))
                                    .map(|t| t.text().to_lowercase() == "reduce")
                                    .unwrap_or(false)
                            }
                        });
                    if let Some(reduce) = reduce_call {
                        let args = reduce.arguments();
                        if !args.is_empty() {
                            // Check if cursor is after the first comma inside the call.
                            // We approximate: cursor > end of first argument.
                            let first_end: usize = args
                                .first()
                                .map(|a| a.text_range().end().into())
                                .unwrap_or(0);
                            let call_end: usize = reduce.syntax().text_range().end().into();
                            let call_start: usize = reduce.syntax().text_range().start().into();
                            if cursor_offset > first_end
                                && cursor_offset <= call_end
                                && cursor_offset >= call_start
                            {
                                // We're in the second-arg position. Infer first-arg list type.
                                let ctx = smelt_db::TypeContext::new();
                                use smelt_types::signatures::SmeltType;
                                let list_ty: Option<SmeltType> = args.first().and_then(|a| {
                                    if let Some(arr) = a.as_array_literal() {
                                        let elems = arr.elements();
                                        let r = smelt_db::type_inference::infer_list_literal(
                                            &elems, &ctx, None,
                                        );
                                        Some(r.inferred)
                                    } else {
                                        None
                                    }
                                });
                                let items = completion_items_for_reduce_second_arg_with_snippets(
                                    list_ty.as_ref(),
                                );
                                if !items.is_empty() {
                                    return Ok(Some(CompletionResponse::Array(items)));
                                }
                            }
                        }
                    }

                    // Phase D: smelt.models.<cursor> / smelt.sources.<cursor> accessor
                    // namespace completion — offer the closed accessor set {with_tag, all}.
                    //
                    // Detection: text before cursor ends with `smelt.models.` or
                    // `smelt.sources.` (possibly with a partial accessor name typed).
                    {
                        let before = &text[..cursor_offset.min(text.len())];
                        let is_models_ns = before.ends_with("smelt.models.")
                            || before
                                .rfind("smelt.models.")
                                .map(|p| {
                                    let after = &before[p + "smelt.models.".len()..];
                                    after.chars().all(|c| c.is_alphanumeric() || c == '_')
                                })
                                .unwrap_or(false);
                        let is_sources_ns = !is_models_ns
                            && (before.ends_with("smelt.sources.")
                                || before
                                    .rfind("smelt.sources.")
                                    .map(|p| {
                                        let after = &before[p + "smelt.sources.".len()..];
                                        after.chars().all(|c| c.is_alphanumeric() || c == '_')
                                    })
                                    .unwrap_or(false));
                        if is_models_ns || is_sources_ns {
                            let accessor_names = wide_reflection_accessor_completions();
                            let items: Vec<CompletionItem> = accessor_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FUNCTION),
                                    detail: Some(format!(
                                        "smelt.{}.{}",
                                        if is_models_ns { "models" } else { "sources" },
                                        name
                                    )),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase D: ModelRef field completion — at `m.<cursor>` inside a
                    // lambda body where `m` is a ModelRef-typed parameter,
                    // offer the closed field set {path, name, tags, columns}.
                    {
                        if is_model_ref_param_before_dot(&file, &text, cursor_offset).is_some() {
                            let field_names = model_ref_field_completions();
                            let items: Vec<CompletionItem> = field_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: hover_text_for_model_ref_field(&name),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase D: SourceRef field completion — at `s.<cursor>` inside a
                    // lambda body where `s` is a SourceRef-typed parameter,
                    // offer the closed field set {path, name, tags, columns}.
                    {
                        if is_source_ref_param_before_dot(&file, &text, cursor_offset).is_some() {
                            let field_names = source_ref_field_completions();
                            let items: Vec<CompletionItem> = field_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: hover_text_for_source_ref_field(&name),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase C: ColumnRef field completion — at `c.<cursor>` inside
                    // a lambda body where `c` is a ColumnRef-typed parameter,
                    // offer the closed field set.
                    //
                    // Detection: check if the text immediately before the cursor
                    // (within the lambda body) ends with `<ident>.` where `<ident>`
                    // is a ColumnRef-typed lambda parameter name.  We use
                    // `is_column_ref_param_before_dot` which checks that the
                    // receiver param is bound by a HOF whose first arg is
                    // `smelt.columns_of(...)`.  This prevents false-positive
                    // completions when an unrelated `smelt.columns_of` call
                    // appears elsewhere in the file.
                    {
                        if is_column_ref_param_before_dot(&file, &text, cursor_offset).is_some() {
                            let field_names = column_ref_field_completions();
                            let items: Vec<CompletionItem> = field_names
                                .into_iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: hover_text_for_column_ref_field(&name),
                                    sort_text: Some(format!("0_{name}")),
                                    ..Default::default()
                                })
                                .collect();
                            if !items.is_empty() {
                                return Ok(Some(CompletionResponse::Array(items)));
                            }
                        }
                    }

                    // Phase C: smelt.columns_of(<cursor>) argument completion —
                    // offer in-scope TableExpr-valued names.
                    {
                        let before = &text[..cursor_offset.min(text.len())];
                        // Detect cursor inside `smelt.columns_of(` argument position.
                        // Simple heuristic: text before cursor contains `columns_of(`
                        // without a matching `)`.
                        if let Some(call_start) = before.rfind("columns_of(") {
                            let after_paren = &before[call_start + "columns_of(".len()..];
                            let paren_depth: i32 = after_paren.chars().fold(0i32, |d, c| match c {
                                '(' => d + 1,
                                ')' => d - 1,
                                _ => d,
                            });
                            // paren_depth >= 0 means we are inside the argument list
                            // (not yet closed by a matching `)`).
                            if paren_depth >= 0 {
                                let names = columns_of_arg_completions_for_sql(&text);
                                // Also add Salsa-backed model names from the workspace.
                                let ws = Workspace::try_get(&db);
                                let mut all_names = names;
                                if let Some(w) = ws {
                                    let models = smelt_db::all_models(&db, w);
                                    for model in models.values() {
                                        if !all_names.contains(&model.name) {
                                            all_names.push(model.name.clone());
                                        }
                                    }
                                }
                                if !all_names.is_empty() {
                                    let items: Vec<CompletionItem> = all_names
                                        .into_iter()
                                        .map(|name| CompletionItem {
                                            label: name.clone(),
                                            kind: Some(CompletionItemKind::MODULE),
                                            detail: Some(format!("model: {name}")),
                                            ..Default::default()
                                        })
                                        .collect();
                                    return Ok(Some(CompletionResponse::Array(items)));
                                }
                            }
                        }
                    }

                    // Phase B: check if cursor is inside a lambda body — prepend
                    // the bound lambda parameter to the completion list.
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
                            // Only inject param completions when cursor is in the BODY,
                            // i.e. past the lambda arrow token.
                            let arrow_pos: Option<usize> =
                                lambda.syntax().children_with_tokens().find_map(|c| {
                                    c.as_token()
                                        .filter(|t| t.kind() == SyntaxKind::ARROW)
                                        .map(|t| t.text_range().end().into())
                                });
                            if arrow_pos.map(|p| cursor_offset >= p).unwrap_or(false) {
                                let params = lambda_params_for_completion(&lambda);
                                // Build param completions — they will be prepended
                                // to the standard column completions below.
                                // Phase F: also prepend the `if` snippet since a lambda
                                // body is a meta-expression context where ternary is valid.
                                let mut param_items: Vec<CompletionItem> =
                                    vec![completion_item_for_if_snippet()];
                                param_items.extend(params.iter().map(|p| CompletionItem {
                                    label: p.clone(),
                                    kind: Some(CompletionItemKind::VARIABLE),
                                    detail: Some("lambda parameter".to_string()),
                                    sort_text: Some(format!("0_{p}")), // sort first
                                    ..Default::default()
                                }));
                                if !param_items.is_empty() {
                                    // Return the param completions immediately so they
                                    // appear first in the list.
                                    return Ok(Some(CompletionResponse::Array(param_items)));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase F: `if` snippet fallback for generator-file body context.
        // If none of the earlier meta-language blocks claimed the cursor (reduce
        // second-arg, ModelDef field-key, lambda body, etc.), and the cursor is
        // in the body of a Generator file, offer `if … then … else …` as the
        // sole completion item.  Generator bodies are meta-expression contexts
        // where ternary is valid.
        {
            let raw = text.as_str();
            if let Ok(smelt_core::metadata::FileMetadata::Generator { body_offset, .. }) =
                smelt_core::metadata::extract_file_metadata(raw)
            {
                if cursor_offset >= body_offset {
                    return Ok(Some(CompletionResponse::Array(vec![
                        completion_item_for_if_snippet(),
                    ])));
                }
            }
        }

        // Determine completion context
        let context = determine_completion_context(&text, cursor_offset);

        let items = match context {
            CompletionContext::InsideRef => {
                // Complete model names
                let ws = Workspace::try_get(&db);
                let models = ws.map(|w| smelt_db::all_models(&db, w)).unwrap_or_default();
                models
                    .values()
                    .map(|model| CompletionItem {
                        label: model.name.clone(),
                        kind: Some(CompletionItemKind::MODULE),
                        detail: Some(format!("Model: {}", model.name)),
                        ..Default::default()
                    })
                    .collect()
            }
            CompletionContext::InsideSource => {
                // Complete source.table names
                let project_root = file_project_root(&db, &effective_path);
                let project = lookup_project(&db, &project_root);
                let config = project
                    .map(|p| smelt_db::sources_config(&db, p))
                    .unwrap_or_default();
                let mut items = Vec::new();

                for source in &config.sources {
                    for table in &source.tables {
                        let qualified_name = format!("{}.{}", source.name, table.name);
                        let detail = table
                            .description
                            .clone()
                            .unwrap_or_else(|| format!("Source table: {}", qualified_name));
                        items.push(CompletionItem {
                            label: qualified_name.clone(),
                            kind: Some(CompletionItemKind::FILE),
                            detail: Some(detail),
                            documentation: if !table.columns.is_empty() {
                                let cols: Vec<_> =
                                    table.columns.iter().map(|c| c.name.as_str()).collect();
                                Some(Documentation::String(format!(
                                    "Columns: {}",
                                    cols.join(", ")
                                )))
                            } else {
                                None
                            },
                            ..Default::default()
                        });
                    }
                }

                items
            }
            CompletionContext::ColumnName => {
                // Complete column names from available columns
                // Use typed schema for type information
                let ws = Workspace::try_get(&db);
                let fi = lookup_file(&db, &effective_path);
                let typed_schema = match (ws, fi) {
                    (Some(w), Some(f)) => smelt_db::typed_model_schema(&db, w, f),
                    _ => Arc::new(smelt_db::ModelSchema::empty()),
                };
                let available = match (ws, fi) {
                    (Some(w), Some(f)) => smelt_db::available_columns(&db, w, f),
                    _ => Arc::new(Vec::new()),
                };

                // Build a map of column names to types from the typed schema
                let type_map: std::collections::HashMap<&str, &TypedColumn> = typed_schema
                    .columns
                    .iter()
                    .filter_map(|col| col.data_type.as_ref().map(|t| (col.name.as_str(), t)))
                    .collect();

                available
                    .iter()
                    .filter(|col| col.name != "*")
                    .map(|col| {
                        // Build detail with type info
                        let type_str = col
                            .data_type
                            .as_ref()
                            .or_else(|| type_map.get(col.name.as_str()).copied())
                            .map(format_type)
                            .unwrap_or_else(|| "unknown".to_string());

                        let detail = format!("{}: {}", col.name, type_str);

                        // Build documentation with expression and source info
                        let mut doc_parts = Vec::new();
                        if !col.expression.is_empty() && col.expression != col.name {
                            doc_parts.push(format!("Expression: `{}`", col.expression));
                        }
                        match &col.source {
                            smelt_db::ColumnSource::FromModel {
                                model_name,
                                column_name,
                            } => {
                                doc_parts.push(format!(
                                    "From model '{}', column '{}'",
                                    model_name, column_name
                                ));
                            }
                            smelt_db::ColumnSource::Computed => {
                                doc_parts.push("Computed column".to_string());
                            }
                            _ => {}
                        }

                        CompletionItem {
                            label: col.name.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(detail),
                            documentation: if doc_parts.is_empty() {
                                None
                            } else {
                                Some(Documentation::String(doc_parts.join("\n")))
                            },
                            ..Default::default()
                        }
                    })
                    .collect()
            }
            CompletionContext::QualifiedColumn(alias) => {
                // Complete columns for the specified table alias
                // Parse the file to find what the alias refers to
                let fi = lookup_file(&db, &effective_path);
                let parse = fi.map(|f| smelt_db::parse_file(&db, f));
                let syntax = parse.as_ref().map(|p| p.syntax());

                if let Some(syntax) = syntax {
                    if let Some(file) = smelt_parser::ast::File::cast(syntax) {
                        if let Some(select_stmt) = file.select_stmt() {
                            // Extract alias mappings from FROM clause
                            let alias_map = extract_from_aliases(&select_stmt, &db);

                            // Look up what this alias refers to
                            if let Some(target) = alias_map.get(&alias) {
                                match target {
                                    AliasTarget::Source {
                                        source_name,
                                        table_name,
                                    } => {
                                        // Get columns from sources.yml
                                        let project_root = file_project_root(&db, &effective_path);
                                        let project = lookup_project(&db, &project_root);
                                        let config = project
                                            .map(|p| smelt_db::sources_config(&db, p))
                                            .unwrap_or_default();
                                        for source in &config.sources {
                                            if source.name == *source_name {
                                                for table in &source.tables {
                                                    if table.name == *table_name {
                                                        return Ok(Some(
                                                            CompletionResponse::Array(
                                                                table
                                                                    .columns
                                                                    .iter()
                                                                    .map(|col| {
                                                                        let type_str = col
                                                                            .data_type
                                                                            .as_ref()
                                                                            .map(|t| t.to_string())
                                                                            .unwrap_or_else(|| {
                                                                                "unknown"
                                                                                    .to_string()
                                                                            });
                                                                        CompletionItem {
                                                                    label: col.name.clone(),
                                                                    kind: Some(
                                                                        CompletionItemKind::FIELD,
                                                                    ),
                                                                    detail: Some(format!(
                                                                        "{}: {}",
                                                                        col.name, type_str
                                                                    )),
                                                                    documentation: col
                                                                        .description
                                                                        .as_ref()
                                                                        .map(|d| {
                                                                            Documentation::String(
                                                                                d.clone(),
                                                                            )
                                                                        }),
                                                                    ..Default::default()
                                                                }
                                                                    })
                                                                    .collect(),
                                                            ),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    AliasTarget::Model { model_name } => {
                                        // Get columns from the model schema
                                        let ws = Workspace::try_get(&db);
                                        let models = ws
                                            .map(|w| smelt_db::all_models(&db, w))
                                            .unwrap_or_default();
                                        if let Some(model) =
                                            models.values().find(|m| m.name == *model_name)
                                        {
                                            let model_file = lookup_file(&db, &model.path);
                                            let schema = model_file
                                                .map(|f| smelt_db::model_schema(&db, f))
                                                .unwrap_or_else(|| {
                                                    Arc::new(smelt_db::ModelSchema::empty())
                                                });
                                            return Ok(Some(CompletionResponse::Array(
                                                schema
                                                    .columns
                                                    .iter()
                                                    .filter(|col| col.name != "*")
                                                    .map(|col| CompletionItem {
                                                        label: col.name.clone(),
                                                        kind: Some(CompletionItemKind::FIELD),
                                                        detail: Some(format!(
                                                            "Column from {}",
                                                            model_name
                                                        )),
                                                        ..Default::default()
                                                    })
                                                    .collect(),
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Vec::new()
            }
            CompletionContext::FromClause => {
                // Offer CTE names defined in the current query's WITH clause
                let fi = lookup_file(&db, &effective_path);
                let parse = fi.map(|f| smelt_db::parse_file(&db, f));
                let syntax = parse.as_ref().map(|p| p.syntax());

                let mut items = Vec::new();

                if let Some(syntax) = syntax {
                    if let Some(file) = smelt_parser::ast::File::cast(syntax) {
                        if let Some(select_stmt) = file.select_stmt() {
                            if let Some(with_clause) = select_stmt.with_clause() {
                                let ws = Workspace::try_get(&db);
                                let type_ctx = match (ws, fi) {
                                    (Some(w), Some(f)) => smelt_db::type_context(&db, w, f),
                                    _ => Arc::new(smelt_db::TypeContext::new()),
                                };

                                for cte in with_clause.ctes() {
                                    if let Some(cte_name) = cte.name() {
                                        // Get column info for documentation
                                        let columns = type_ctx.cte_columns(&cte_name);
                                        let doc = if columns.is_empty() {
                                            None
                                        } else {
                                            let col_strs: Vec<String> = columns
                                                .iter()
                                                .map(|(name, typed_col)| {
                                                    format!("{}: {}", name, format_type(typed_col))
                                                })
                                                .collect();
                                            Some(Documentation::String(col_strs.join("\n")))
                                        };

                                        items.push(CompletionItem {
                                            label: cte_name.clone(),
                                            kind: Some(CompletionItemKind::STRUCT),
                                            detail: Some("CTE".to_string()),
                                            documentation: doc,
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                items
            }
            // Phase 48: PASSING-body completions — offer aggregate functions
            // and any columns from the parameter's declared context schema.
            CompletionContext::InPassingBody {
                callee,
                passing_name,
            } => {
                let ws = Workspace::try_get(&db);
                let mut items: Vec<CompletionItem> = Vec::new();

                // Resolve the callee's signature to find the parameter's
                // declared context (e.g. `SelectItems<Agg, sessionized>`).
                // Project isolation: resolve in the cursor file's project.
                let project_root = file_project_root(&db, &effective_path);
                let project = lookup_project(&db, &project_root);
                if let (Some(w), Some(p)) = (ws, project) {
                    if let Some(sig) = smelt_db::resolve_function(&db, w, p, callee.clone())
                        .map(|arc| (*arc).clone())
                    {
                        // Look up the parameter by name.
                        if let Some(param) = sig.params.iter().find(|p| p.name == passing_name) {
                            use smelt_types::signatures::SmeltType;
                            if let Some(Ok(SmeltType::SelectItems {
                                context: Some(smelt_types::signatures::ContextRef(ctx_name)),
                                ..
                            })) = &param.type_ref
                            {
                                // Surface columns from the context schema (e.g. the
                                // `sessionized` CTE) so the user can pick column refs.
                                let cols = passing_body_completion_columns(&db, w, &sig, ctx_name);
                                for (col_name, typed_col) in &cols {
                                    items.push(CompletionItem {
                                        label: col_name.clone(),
                                        kind: Some(CompletionItemKind::FIELD),
                                        detail: Some(format_type(typed_col)),
                                        ..Default::default()
                                    });
                                }
                            }

                            // Always offer aggregate function keywords for
                            // `SelectItems<Agg>`-kinded parameters.
                            use smelt_types::signatures::ExprKind;
                            let needs_agg = matches!(
                                &param.type_ref,
                                Some(Ok(SmeltType::SelectItems {
                                    kind: ExprKind::Agg | ExprKind::Window,
                                    ..
                                }))
                            );
                            if needs_agg {
                                for label in passing_body_aggregate_labels() {
                                    items.push(CompletionItem {
                                        label: label.to_string(),
                                        kind: Some(CompletionItemKind::FUNCTION),
                                        detail: Some("aggregate function".to_string()),
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                }

                items
            }
            CompletionContext::SmeltPath => {
                // Phase 2c: return all workspace entities as `smelt.<segments>` labels.
                let ws = Workspace::try_get(&db);
                let Some(w) = ws else { return Ok(None) };
                let all_files = w.files(&db).clone();
                // Determine the project root from the current file.
                let project_root = file_project_root(&db, &effective_path);
                let mut items: Vec<CompletionItem> = all_files
                    .iter()
                    .filter_map(|f| {
                        let file_path = f.path(&db);
                        // Only SQL files.
                        if file_path.extension().and_then(|e| e.to_str()) != Some("sql") {
                            return None;
                        }
                        let rel = file_path.strip_prefix(&project_root).ok()?;
                        let parent = rel.parent()?;
                        let mut segments: Vec<String> = parent
                            .components()
                            .filter_map(|c| match c {
                                std::path::Component::Normal(s) => {
                                    Some(s.to_string_lossy().into_owned())
                                }
                                _ => None,
                            })
                            .collect();
                        let stem = file_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())?;
                        segments.push(stem.clone());
                        let label = format!("smelt.{}", segments.join("."));
                        let insert = segments.join(".");
                        Some(CompletionItem {
                            label,
                            insert_text: Some(insert),
                            kind: Some(CompletionItemKind::MODULE),
                            ..Default::default()
                        })
                    })
                    .collect();
                items.sort_by(|a, b| a.label.cmp(&b.label));
                items
            }
            CompletionContext::None => Vec::new(),
        };

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }
}
