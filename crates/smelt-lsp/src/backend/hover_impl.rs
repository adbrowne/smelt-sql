use super::*;

impl Backend {
    pub(crate) async fn hover_impl(&self, params: HoverParams) -> Result<Option<Hover>> {
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

        let db = self.snapshot().await;

        // Get file content and parse tree
        let text = file_text(&db, &effective_path);
        let file_input = lookup_file(&db, &effective_path);
        let parse = file_input.map(|f| smelt_db::parse_file(&db, f));
        let syntax = parse.as_ref().map(|p| p.syntax());

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

        // Check if hovering over a smelt.<path> ref
        if let Some(syntax) = syntax {
            if let Some(file) = AstFile::cast(syntax) {
                // Check smelt.models.<name> path refs
                for path_ref in file
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                {
                    let segments = path_ref.segments();
                    if segments.first().map(|s| s.as_str()) != Some("models") {
                        continue;
                    }
                    let range = path_ref.text_range();
                    let start: usize = range.start().into();
                    let end: usize = range.end().into();

                    // Check if cursor is within this path ref
                    if cursor_offset >= start && cursor_offset <= end {
                        if let Some(model_name) = segments.get(1).cloned() {
                            // Resolve upstream model and show its resolved schema
                            // Resolve using the full path tuple so multi-layer
                            // models (e.g. smelt.silver.events) resolve correctly.
                            let ws = Workspace::try_get(&db);
                            let upstream_file = ws.and_then(|w| {
                                smelt_db::resolve_ref_path(&db, w, segments.clone())
                                    .and_then(|r| r.source_file)
                            });
                            if let (Some(upstream), Some(w)) = (upstream_file, ws) {
                                // Use resolved_model_schema to get type information through wildcards
                                let resolved = smelt_db::resolved_model_schema(&db, w, upstream);

                                // Format schema as markdown
                                let mut content = format!("**Model: {}**\n\n", model_name);
                                content.push_str("| Column | Type | Source |\n");
                                content.push_str("|--------|------|--------|\n");

                                for col in resolved.columns.iter() {
                                    // Column name
                                    content.push_str(&format!("| `{}` | ", col.name));

                                    // Type (if known)
                                    if let Some(ref typed_col) = col.data_type {
                                        content.push_str(&format!("`{}`", format_type(typed_col)));
                                    } else {
                                        content.push_str("*unknown*");
                                    }
                                    content.push_str(" | ");

                                    // Source info
                                    match &col.source {
                                        smelt_db::ColumnSource::FromModel {
                                            model_name,
                                            column_name,
                                        } => {
                                            content.push_str(&format!(
                                                "from `{}.{}`",
                                                model_name, column_name
                                            ));
                                        }
                                        smelt_db::ColumnSource::Computed => {
                                            if !col.expression.is_empty()
                                                && col.expression != col.name
                                            {
                                                content.push_str(&format!("`{}`", col.expression));
                                            } else {
                                                content.push_str("computed");
                                            }
                                        }
                                        smelt_db::ColumnSource::Wildcard { model_name } => {
                                            content.push_str(&format!("* from `{}`", model_name));
                                        }
                                        smelt_db::ColumnSource::ExternalTable { table_name } => {
                                            content.push_str(&format!("from `{}`", table_name));
                                        }
                                        smelt_db::ColumnSource::Unknown => {
                                            content.push('-');
                                        }
                                    }

                                    content.push_str(" |\n");
                                }

                                // Show unresolved row extensions
                                if !resolved.unresolved_extensions.is_empty() {
                                    content.push_str("\n*...plus columns from:*\n");
                                    for ext in &resolved.unresolved_extensions {
                                        content.push_str(&format!("- `{}`\n", ext.ref_name));
                                    }
                                }

                                // Show input constraints
                                let constraints =
                                    smelt_db::model_input_constraints(&db, w, upstream);
                                if !constraints.is_empty() {
                                    content.push_str("\n**Requires:**\n");
                                    for constraint in constraints.iter() {
                                        for (col_name, col_constraint) in
                                            &constraint.required_columns
                                        {
                                            if let Some(ref typed_col) =
                                                col_constraint.expected_type
                                            {
                                                content.push_str(&format!(
                                                    "- `{}` (`{}`) from `{}`\n",
                                                    col_name,
                                                    format_type(typed_col),
                                                    constraint.ref_name,
                                                ));
                                            } else {
                                                content.push_str(&format!(
                                                    "- `{}` from `{}`\n",
                                                    col_name, constraint.ref_name,
                                                ));
                                            }
                                        }
                                    }
                                }

                                // Per-source clamp observability
                                // (`docs/specs/incremental_shapes.md`
                                // §"Observing the per-source clamp"): if the
                                // CURRENT file's own model is partition-grain
                                // and derives a bound for this hovered
                                // source, render it beside the schema table.
                                if let Some(fi) = file_input {
                                    let clamps = smelt_db::model_source_clamps(&db, w, fi);
                                    let key = segments.join(".");
                                    if let Some(clamp_line) =
                                        hover_text_for_source_clamp(&key, clamps.get(&key))
                                    {
                                        content.push('\n');
                                        content.push_str(&clamp_line);
                                        content.push('\n');
                                    }
                                }

                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: content,
                                    }),
                                    range: None,
                                }));
                            }
                        }
                    }
                }

                // Check smelt.sources.<source>.<table> path refs
                for path_ref in file
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                {
                    let segments = path_ref.segments();
                    if segments.first().map(|s| s.as_str()) != Some("sources") {
                        continue;
                    }
                    let range = path_ref.text_range();
                    let start: usize = range.start().into();
                    let end: usize = range.end().into();

                    // Check if cursor is within this path ref
                    if cursor_offset >= start && cursor_offset <= end {
                        // Need at least `sources.<schema>.<table>` (three segments).
                        if segments.len() < 3 {
                            continue;
                        }
                        let source_name = segments[segments.len() - 2].clone();
                        let table_name = segments[segments.len() - 1].clone();
                        let qualified_name = format!("{}.{}", source_name, table_name);

                        let project_root = file_project_root(&db, &effective_path);
                        let project = lookup_project(&db, &project_root);

                        // Try the per-entity source registry first (the canonical
                        // shape since the per-entity migration). Address segments
                        // include the leading `sources` segment, so the full
                        // `path_ref.segments()` is the lookup key.
                        let per_entity = project.and_then(|p| {
                            let sources = smelt_db::project_sources(&db, p);
                            sources
                                .iter()
                                .find(|s| s.address_segments == segments)
                                .cloned()
                        });

                        if let Some(info) = per_entity {
                            let mut content = format!("**Source: {}**\n\n", qualified_name);
                            if let Some(ref desc) = info.description {
                                content.push_str(&format!("{}\n\n", desc));
                            }
                            if !info.columns.is_empty() {
                                content.push_str("Columns:\n");
                                for col in &info.columns {
                                    content
                                        .push_str(&format!("- `{}` ({})", col.name, col.data_type));
                                    if let Some(ref d) = col.description {
                                        content.push_str(&format!(" - {}", d));
                                    }
                                    content.push('\n');
                                }
                            } else {
                                content.push_str("*(No column definitions)*\n");
                            }
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: content,
                                }),
                                range: None,
                            }));
                        }

                        // Fall back to the legacy aggregate sources.yml resolver
                        // for projects that haven't migrated to per-entity yet.
                        if let Some(table_def) = project.and_then(|p| {
                            smelt_db::resolve_source(
                                &db,
                                p,
                                source_name.clone(),
                                table_name.clone(),
                            )
                        }) {
                            let mut content = format!("**Source: {}**\n\n", qualified_name);
                            if let Some(ref desc) = table_def.description {
                                content.push_str(&format!("{}\n\n", desc));
                            }
                            if !table_def.columns.is_empty() {
                                content.push_str("Columns:\n");
                                for col in &table_def.columns {
                                    content.push_str(&format!("- `{}`", col.name));
                                    if let Some(ref dtype) = col.data_type {
                                        content.push_str(&format!(" ({})", dtype));
                                    }
                                    if let Some(ref desc) = col.description {
                                        content.push_str(&format!(" - {}", desc));
                                    }
                                    content.push('\n');
                                }
                            } else {
                                content.push_str("*(No column definitions)*\n");
                            }
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: content,
                                }),
                                range: None,
                            }));
                        }

                        // Source not found in either registry.
                        let content =
                            format!("**Source: {}**\n\n⚠️ *Undefined source*", qualified_name);
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: content,
                            }),
                            range: None,
                        }));
                    }
                }

                // Check smelt.<path> seed refs — path segments match address_segments
                for path_ref in file
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::SmeltPathRef::cast)
                {
                    let segments = path_ref.segments();
                    // Skip refs already handled as models or sources
                    let first = segments.first().map(|s| s.as_str());
                    if first == Some("models") || first == Some("sources") {
                        continue;
                    }
                    let range = path_ref.text_range();
                    let start: usize = range.start().into();
                    let end: usize = range.end().into();

                    if cursor_offset >= start && cursor_offset <= end {
                        let project_root = file_project_root(&db, &effective_path);
                        let project = lookup_project(&db, &project_root);
                        if let Some(proj) = project {
                            let seeds = smelt_db::project_seeds(&db, proj);
                            if let Some(seed) = seeds
                                .iter()
                                .find(|s| s.address_segments == segments.as_slice())
                            {
                                let qualified_name = segments.join(".");
                                let mut content = format!("**Seed: {}**\n\n", qualified_name);

                                if seed.columns.is_empty() {
                                    content.push_str("*(No column definitions)*\n");
                                } else {
                                    content.push_str("Columns:\n");
                                    for (col_name, dtype) in &seed.columns {
                                        content.push_str(&format!("- `{}` ({})", col_name, dtype));
                                        // Include description from sidecar if present
                                        if let Some(ref sidecar) = seed.sidecar {
                                            if let Some(ref cols) = sidecar.columns {
                                                if let Some(sc) =
                                                    cols.iter().find(|c| &c.name == col_name)
                                                {
                                                    if let Some(ref desc) = sc.description {
                                                        content.push_str(&format!(" - {}", desc));
                                                    }
                                                }
                                            }
                                        }
                                        content.push('\n');
                                    }
                                }

                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value: content,
                                    }),
                                    range: None,
                                }));
                            }
                        }
                    }
                }

                // Check smelt.define parameters — Phase 18 hover
                if let Some(file_input) = lookup_file(&db, &effective_path) {
                    let fn_sigs = functions_in_file(&db, file_input);
                    for define in file.defines() {
                        let fn_name = define.name().unwrap_or_default();
                        if let Some(param_list) = define.param_list() {
                            for param in param_list.params() {
                                let param_range = param.syntax().text_range();
                                let start: usize = param_range.start().into();
                                let end: usize = param_range.end().into();
                                if cursor_offset >= start && cursor_offset <= end {
                                    let param_name = param.name().unwrap_or_default();
                                    let type_display = fn_sigs
                                        .iter()
                                        .find(|s| s.name == fn_name)
                                        .and_then(|s| {
                                            s.params.iter().find(|p| p.name == param_name)
                                        })
                                        .and_then(|p| {
                                            p.type_ref
                                                .as_ref()?
                                                .as_ref()
                                                .ok()
                                                .map(format_smelt_type_hover)
                                        })
                                        .unwrap_or_else(|| "unknown".to_string());
                                    let content = format!(
                                        "**`{param_name}`** (parameter of `{fn_name}`)\n\n\
                                         `{param_name}: {type_display}`"
                                    );
                                    return Ok(Some(Hover {
                                        contents: HoverContents::Markup(MarkupContent {
                                            kind: MarkupKind::Markdown,
                                            value: content,
                                        }),
                                        range: None,
                                    }));
                                }
                            }
                        }
                    }
                }

                // Phase 4 (meta-language): hover on ARRAY_LITERAL — show the
                // inferred List<T> type (or dual meta + data-world reading).
                //
                // Guard: if the matched ARRAY_LITERAL is the operand child of a
                // LIST_SPREAD node, skip here and let the LIST_SPREAD dispatch
                // below handle it. This ensures "hover on `[…]` inside `...[…]`
                // shows the source list type" is honoured by design rather than
                // by accident (spec rule: hover on spread shows source list type).
                {
                    use smelt_parser::syntax_kind::SyntaxKind;

                    // Walk descendants to find an ARRAY_LITERAL node that
                    // contains the cursor offset and is NOT the operand of a
                    // LIST_SPREAD parent.
                    let array_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::ARRAY_LITERAL)
                        .find(|n| {
                            let start: usize = n.text_range().start().into();
                            let end: usize = n.text_range().end().into();
                            if !(cursor_offset >= start && cursor_offset <= end) {
                                return false;
                            }
                            // Skip if this ARRAY_LITERAL is the direct child of a
                            // LIST_SPREAD — the spread dispatch handles that case.
                            let parent_is_spread = n
                                .parent()
                                .map(|p| p.kind() == SyntaxKind::LIST_SPREAD)
                                .unwrap_or(false);
                            !parent_is_spread
                        });

                    if let Some(arr_node) = array_node {
                        if let Some(arr) = smelt_parser::ast::ArrayLiteral::cast(arr_node) {
                            let elems: Vec<smelt_parser::ast::Expr> = arr.elements();
                            let ctx = smelt_db::TypeContext::new();

                            let value = hover_text_for_list_literal_dual(&elems, &ctx);

                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }

                    // Phase 4 (meta-language): hover on LIST_SPREAD — show
                    // the source list's type.
                    let spread_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::LIST_SPREAD)
                        .find(|n| {
                            let start: usize = n.text_range().start().into();
                            let end: usize = n.text_range().end().into();
                            cursor_offset >= start && cursor_offset <= end
                        });

                    if let Some(sp_node) = spread_node {
                        if let Some(spread) = smelt_parser::ast::ListSpread::cast(sp_node) {
                            let ctx = smelt_db::TypeContext::new();
                            let value = hover_text_for_list_spread(&spread, &ctx);
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }
                }

                // Phase 48: hover on a `smelt.fn.<name>(...)` call site —
                // surface the declared return type or the parameter binding
                // for a `PASSING <name> AS (...)` clause.
                if let Some(call) = find_smelt_fn_call_at_cursor(file.syntax(), cursor_offset) {
                    let segments = call.segments();
                    let fn_name = segments.last().cloned().unwrap_or_default();
                    let ws = Workspace::try_get(&db);
                    // Project isolation: hover resolves the same way the
                    // diagnostic and goto-def code paths do — only against
                    // functions declared in the cursor file's project.
                    let project_root = file_project_root(&db, &effective_path);
                    let project = lookup_project(&db, &project_root);
                    let sig = ws
                        .zip(project)
                        .and_then(|(w, p)| smelt_db::resolve_function(&db, w, p, fn_name.clone()));

                    if let Some(sig) = sig {
                        // Phase 48 test 2: cursor on a PASSING clause name.
                        for passing in call.passing_clauses() {
                            if let Some(name_range) = passing.name_range() {
                                let start: usize = name_range.start().into();
                                let end: usize = name_range.end().into();
                                if cursor_offset >= start && cursor_offset <= end {
                                    if let Some(name) = passing.name() {
                                        let type_text = sig
                                            .params
                                            .iter()
                                            .find(|p| p.name == name)
                                            .and_then(|p| p.type_ref_text.clone())
                                            .unwrap_or_else(|| "unknown".to_string());
                                        let content = format!(
                                            "**`{name}`** (parameter of `{}`)\n\n`{name}: {type_text}`",
                                            sig.name
                                        );
                                        return Ok(Some(Hover {
                                            contents: HoverContents::Markup(MarkupContent {
                                                kind: MarkupKind::Markdown,
                                                value: content,
                                            }),
                                            range: None,
                                        }));
                                    }
                                }
                            }
                        }

                        // Phase 48 test 1: cursor on the call path —
                        // surface the declared return type.
                        if let Some(call_path_range) = call.call_path_range() {
                            let start: usize = call_path_range.start().into();
                            let end: usize = call_path_range.end().into();
                            if cursor_offset >= start && cursor_offset <= end {
                                if let Some(text) = smelt_db::declared_return_hover_text(&sig) {
                                    let content = format!("`{}` `{text}`", sig.name);
                                    return Ok(Some(Hover {
                                        contents: HoverContents::Markup(MarkupContent {
                                            kind: MarkupKind::Markdown,
                                            value: content,
                                        }),
                                        range: None,
                                    }));
                                }
                            }
                        }
                    }
                }

                // Phase D: wide-reflection accessor hover with Salsa-backed resolution.
                //
                // Must run BEFORE `hover_text_for_hof_meta_language` so the richer
                // Salsa-resolved version (with counts + names) wins over the pure
                // fallback (which shows None for workspace state).
                {
                    let wide_call = file
                        .syntax()
                        .descendants()
                        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
                        .find(|c| {
                            let segs = c.segments();
                            let first = segs.first().map(|s| s.as_str());
                            let second = segs.get(1).map(|s| s.as_str());
                            let is_wide = (first == Some("models") || first == Some("sources"))
                                && (second == Some("with_tag") || second == Some("all"));
                            if !is_wide {
                                return false;
                            }
                            let r = c.text_range();
                            let s: usize = r.start().into();
                            let e: usize = r.end().into();
                            cursor_offset >= s && cursor_offset <= e
                        });

                    if let Some(call) = wide_call {
                        let segs = call.segments();
                        let namespace = segs.first().map(|s| s.as_str()).unwrap_or("models");
                        let accessor = segs.get(1).map(|s| s.as_str()).unwrap_or("all");
                        let ws = Workspace::try_get(&db);

                        let value = if namespace == "models" {
                            if accessor == "with_tag" {
                                let tag = call
                                    .arg_list()
                                    .and_then(|al| al.positional_args().into_iter().next())
                                    .map(|a| {
                                        let t = a.text();
                                        t.trim_matches('\'').trim_matches('"').to_string()
                                    })
                                    .unwrap_or_default();
                                let resolved =
                                    ws.map(|w| smelt_db::models_with_tag(&db, w, tag.clone()));
                                hover_text_for_models_with_tag_call(
                                    &tag,
                                    resolved.as_ref().map(|v| v.as_slice()),
                                )
                            } else {
                                let resolved = ws.map(|w| smelt_db::models_all(&db, w));
                                hover_text_for_models_all(resolved.as_ref().map(|v| v.len()))
                            }
                        } else {
                            // sources
                            let project_root = file_project_root(&db, &effective_path);
                            let project = lookup_project(&db, &project_root);
                            if accessor == "with_tag" {
                                let tag = call
                                    .arg_list()
                                    .and_then(|al| al.positional_args().into_iter().next())
                                    .map(|a| {
                                        let t = a.text();
                                        t.trim_matches('\'').trim_matches('"').to_string()
                                    })
                                    .unwrap_or_default();
                                let resolved = project
                                    .map(|p| smelt_db::sources_with_tag(&db, p, tag.clone()));
                                hover_text_for_sources_with_tag_call(
                                    &tag,
                                    resolved.as_ref().map(|v| v.as_slice()),
                                )
                            } else {
                                let resolved = project.map(|p| smelt_db::sources_all(&db, p));
                                hover_text_for_sources_all(resolved.as_ref().map(|v| v.len()))
                            }
                        };
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Phase C: smelt.columns_of hover with Salsa-backed column resolution.
                //
                // Must run BEFORE `hover_text_for_hof_meta_language` so the richer
                // Salsa-resolved version (with column count + names) wins over the
                // pure fallback (which returns only `List<ColumnRef>` with no columns).
                {
                    let columns_of_call = file
                        .syntax()
                        .descendants()
                        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
                        .filter(|c| c.segments() == vec!["columns_of".to_string()])
                        .find(|c| {
                            let r = c.text_range();
                            let s: usize = r.start().into();
                            let e: usize = r.end().into();
                            cursor_offset >= s && cursor_offset <= e
                        });

                    if let Some(call) = columns_of_call {
                        let table_name = call
                            .arg_list()
                            .and_then(|al| al.positional_args().into_iter().next())
                            .map(|a| a.text())
                            .unwrap_or_else(|| "?".to_string());

                        // Try Salsa resolution (project-scoped — only consider
                        // models in the same project as the hover site).
                        let ws = Workspace::try_get(&db);
                        let cols_project_root = file_project_root(&db, &effective_path);
                        let cols_project = lookup_project(&db, &cols_project_root);
                        let resolved_cols = ws.zip(cols_project).and_then(|(w, p)| {
                            smelt_db::columns_of_for_table_expr(&db, w, p, table_name.clone()).ok()
                        });

                        let value = hover_text_for_columns_of_call(
                            &table_name,
                            resolved_cols.as_ref().map(|v| v.as_slice()),
                        );
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Phase B: meta-language hover (reducer name, lambda param binder/body
                // use, HOF result type, smelt.config.var, smelt.columns_of fallback,
                // ColumnRef field projection).  All sub-cases are handled by the
                // `hover_text_for_hof_meta_language` pure helper so they can be tested
                // without a live Backend.
                //
                // NOTE: this block MUST run before the PIPE_EXPR check below.
                // A pipe expression like `[1,2,3] |> filter(fn c => c > 0)` has a
                // PIPE_EXPR ancestor that spans `c`.  If pipe hover ran first it
                // would intercept the lambda-param hover for `c`.
                {
                    let project_root = file_project_root(&db, &effective_path);
                    let project = lookup_project(&db, &project_root);
                    let smelt_yml = project
                        .map(|p| p.smelt_yml_text(&db).clone())
                        .unwrap_or_default();
                    if let Some(value) =
                        hover_text_for_hof_meta_language(&file, cursor_offset, &smelt_yml)
                    {
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Phase B: hover on a PIPE_EXPR node — show result type of the
                // desugared call.
                {
                    use smelt_parser::syntax_kind::SyntaxKind;
                    let pipe_node = file
                        .syntax()
                        .descendants()
                        .filter(|n| n.kind() == SyntaxKind::PIPE_EXPR)
                        .find(|n| {
                            let start: usize = n.text_range().start().into();
                            let end: usize = n.text_range().end().into();
                            cursor_offset >= start && cursor_offset <= end
                        });
                    if let Some(pn) = pipe_node {
                        if let Some(pipe) = smelt_parser::ast::PipeExpr::cast(pn) {
                            let ctx = smelt_db::TypeContext::new();
                            let value = hover_text_for_pipe_expr(&pipe, &ctx);
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }
                }

                // Phase E2: generator-file hover —
                // (a) cursor in YAML frontmatter on the `generates: models` value,
                // (b) cursor on a `ModelDef { … }` opening brace,
                // (c) cursor on the `name:` field value in a `ModelDef` literal,
                // (d) cursor on the `body:` field value in a `ModelDef` literal.
                {
                    let raw = text.as_str();
                    // Detect generator files by checking frontmatter variant.
                    if let Ok(smelt_core::metadata::FileMetadata::Generator {
                        body_offset, ..
                    }) = smelt_core::metadata::extract_file_metadata(raw)
                    {
                        // (a) Is the cursor in the frontmatter (before body_offset)?
                        if cursor_offset < body_offset {
                            // Check if the line under the cursor contains `generates:`
                            let line_start =
                                raw[..cursor_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
                            let line_end = raw[cursor_offset..]
                                .find('\n')
                                .map(|p| cursor_offset + p)
                                .unwrap_or(raw.len());
                            let line_text = &raw[line_start..line_end];
                            if line_text.trim_start().starts_with("generates:") {
                                // Resolve emission count from Salsa.
                                let ws = Workspace::try_get(&db);
                                let emission_count = ws.and_then(|w| {
                                    let gen_files = smelt_db::generator_files(&db, w);
                                    let file_input = lookup_file(&db, &effective_path);
                                    file_input.and_then(|fi| {
                                        gen_files.iter().find(|&&gf| gf == fi).map(|&gf| {
                                            smelt_db::evaluate_generator(&db, w, gf).emissions.len()
                                        })
                                    })
                                });
                                let value = hover_text_for_generates_frontmatter(emission_count);
                                return Ok(Some(Hover {
                                    contents: HoverContents::Markup(MarkupContent {
                                        kind: MarkupKind::Markdown,
                                        value,
                                    }),
                                    range: None,
                                }));
                            }
                        } else {
                            // Cursor is in the generator body — check for ModelDef positions.
                            // We look at the CST for RECORD_LITERAL nodes whose first
                            // keyword is `ModelDef`.

                            // Walk record literals to find a ModelDef that contains cursor.
                            use smelt_parser::SyntaxKind;
                            for node in file.syntax().descendants() {
                                if node.kind() != SyntaxKind::RECORD_LITERAL {
                                    continue;
                                }
                                let rec_start: usize = node.text_range().start().into();
                                let rec_end: usize = node.text_range().end().into();
                                if !(cursor_offset >= rec_start && cursor_offset <= rec_end) {
                                    continue;
                                }
                                // Check that this record starts with `ModelDef`.
                                let first_tok = node
                                    .children_with_tokens()
                                    .filter_map(|e| e.into_token())
                                    .find(|t| !t.kind().is_trivia());
                                let is_model_def = first_tok
                                    .as_ref()
                                    .map(|t| t.text() == "ModelDef")
                                    .unwrap_or(false);
                                if !is_model_def {
                                    continue;
                                }

                                // (b) Is the cursor on the `ModelDef` IDENT keyword
                                // or the opening brace?  Both positions serve the
                                // same hover content per the spec.
                                let open_brace_tok = node
                                    .children_with_tokens()
                                    .filter_map(|e| e.into_token())
                                    .find(|t| t.kind() == SyntaxKind::LBRACE);
                                let on_keyword = first_tok
                                    .as_ref()
                                    .map(|t| {
                                        let s: usize = t.text_range().start().into();
                                        let e: usize = t.text_range().end().into();
                                        cursor_offset >= s && cursor_offset <= e
                                    })
                                    .unwrap_or(false);
                                let on_brace = open_brace_tok
                                    .as_ref()
                                    .map(|t| {
                                        let s: usize = t.text_range().start().into();
                                        let e: usize = t.text_range().end().into();
                                        cursor_offset >= s && cursor_offset <= e
                                    })
                                    .unwrap_or(false);
                                if on_keyword || on_brace {
                                    // Resolve emitted smelt path from Salsa survivors.
                                    let ws = Workspace::try_get(&db);
                                    let smelt_path: Option<String> = ws.and_then(|w| {
                                        let survivors = smelt_db::emitted_models(&db, w);
                                        let project_root = file_project_root(&db, &effective_path);
                                        let project = lookup_project(&db, &project_root);
                                        let scan_roots = project
                                            .map(|p| {
                                                smelt_db::project_paths(&db, p).as_ref().clone()
                                            })
                                            .unwrap_or_else(|| vec!["models".to_string()]);
                                        // Find the survivor whose generator_file
                                        // matches this file AND whose name_span
                                        // falls within the RECORD_LITERAL node
                                        // that contains the cursor's open brace.
                                        // This disambiguates multiple ModelDef
                                        // literals in the same generator file.
                                        let rec_start_u: u32 = node.text_range().start().into();
                                        let rec_end_u: u32 = node.text_range().end().into();
                                        survivors
                                            .survivors
                                            .iter()
                                            .find(|em| {
                                                if em.generator_file != effective_path {
                                                    return false;
                                                }
                                                // name_span must be contained within
                                                // this record literal's range.
                                                let ns: u32 = em.name_span.start().into();
                                                let ne: u32 = em.name_span.end().into();
                                                ns >= rec_start_u && ne <= rec_end_u
                                            })
                                            .map(|em| {
                                                smelt_db::emitted_model_smelt_path(
                                                    &em.generator_file,
                                                    &project_root,
                                                    &scan_roots,
                                                    &em.name,
                                                )
                                            })
                                    });
                                    let value = hover_text_for_model_def_literal_open_brace(
                                        smelt_path.as_deref(),
                                    );
                                    return Ok(Some(Hover {
                                        contents: HoverContents::Markup(MarkupContent {
                                            kind: MarkupKind::Markdown,
                                            value,
                                        }),
                                        range: None,
                                    }));
                                }

                                // Walk field entries of the RecordLiteral.
                                for field in node.children() {
                                    if field.kind() != SyntaxKind::RECORD_FIELD {
                                        continue;
                                    }
                                    let field_start: usize = field.text_range().start().into();
                                    let field_end: usize = field.text_range().end().into();
                                    if !(cursor_offset >= field_start && cursor_offset <= field_end)
                                    {
                                        continue;
                                    }
                                    // Extract field key and value tokens.
                                    let mut tokens = field
                                        .children_with_tokens()
                                        .filter_map(|e| e.into_token())
                                        .filter(|t| !t.kind().is_trivia());
                                    let key_tok = tokens.next();
                                    let key_text =
                                        key_tok.as_ref().map(|t| t.text()).unwrap_or_default();
                                    // Skip the colon token.
                                    let _colon = tokens.next();
                                    let val_tok = tokens.next();

                                    if let Some(val) = val_tok {
                                        let vs: usize = val.text_range().start().into();
                                        let ve: usize = val.text_range().end().into();
                                        if cursor_offset >= vs && cursor_offset <= ve {
                                            // (c) cursor on `name:` value.
                                            if key_text == "name" {
                                                let raw_name = val.text();
                                                let model_name =
                                                    raw_name.trim_matches('\'').trim_matches('"');
                                                let ws = Workspace::try_get(&db);
                                                let project_root =
                                                    file_project_root(&db, &effective_path);
                                                let project = lookup_project(&db, &project_root);
                                                let scan_roots = project
                                                    .map(|p| {
                                                        smelt_db::project_paths(&db, p)
                                                            .as_ref()
                                                            .clone()
                                                    })
                                                    .unwrap_or_else(|| vec!["models".to_string()]);
                                                let smelt_path = ws.map(|_w| {
                                                    smelt_db::emitted_model_smelt_path(
                                                        &effective_path,
                                                        &project_root,
                                                        &scan_roots,
                                                        model_name,
                                                    )
                                                });
                                                let value = match &smelt_path {
                                                    Some(p) => {
                                                        hover_text_for_model_def_name_field_value(p)
                                                    }
                                                    None => {
                                                        format!("Emitted as `smelt.{model_name}`")
                                                    }
                                                };
                                                return Ok(Some(Hover {
                                                    contents: HoverContents::Markup(
                                                        MarkupContent {
                                                            kind: MarkupKind::Markdown,
                                                            value,
                                                        },
                                                    ),
                                                    range: None,
                                                }));
                                            }
                                            // (d) cursor on `body:` value.
                                            if key_text == "body" {
                                                let value =
                                                    hover_text_for_model_def_body_field_value(None);
                                                return Ok(Some(Hover {
                                                    contents: HoverContents::Markup(
                                                        MarkupContent {
                                                            kind: MarkupKind::Markdown,
                                                            value,
                                                        },
                                                    ),
                                                    range: None,
                                                }));
                                            }
                                            // (e) cursor on optional field value:
                                            // `materialization`, `tags`, or `description`.
                                            if let Some(value) =
                                                hover_text_for_model_def_optional_field_value(
                                                    key_text,
                                                )
                                            {
                                                return Ok(Some(Hover {
                                                    contents: HoverContents::Markup(
                                                        MarkupContent {
                                                            kind: MarkupKind::Markdown,
                                                            value,
                                                        },
                                                    ),
                                                    range: None,
                                                }));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ── SQL column reference hover (fallback) ───────────────────
                // Runs LAST so the more-specific handlers above (smelt.<path>
                // refs, smelt.define parameters, meta-language constructs,
                // generator-file ModelDefs) win on overlapping positions.
                //
                // `symbol_at_cursor` returns `ColumnRef { qualifier, name }`
                // when the cursor is on a bare SQL identifier in a column-
                // reference position. We resolve the type via the file's
                // `TypeContext` and derive a source description from the
                // qualifier when present.
                if let Some(SymbolAtCursor::ColumnRef { qualifier, name }) =
                    symbol_at_cursor(&file, &text, cursor_offset)
                {
                    if let (Some(ws), Some(file_input)) =
                        (Workspace::try_get(&db), lookup_file(&db, &effective_path))
                    {
                        let ctx = smelt_db::type_context(&db, ws, file_input);
                        let typed_col = ctx.lookup_column(qualifier.as_deref(), &name).cloned();

                        // Build the display string the user typed.
                        let display = match qualifier.as_deref() {
                            Some(q) => format!("{q}.{name}"),
                            None => name.clone(),
                        };

                        // Convert `describe_qualifier`'s single-quoted output
                        // ("CTE 'sessions'") into markdown with the name in
                        // backticks ("CTE `sessions`") for visual consistency
                        // with the rest of the hover surface.
                        let source_desc = qualifier
                            .as_deref()
                            .and_then(|q| ctx.describe_qualifier(q))
                            .map(|d| d.replace('\'', "`"));

                        let value = hover_text_for_column_reference(
                            &display,
                            typed_col.as_ref(),
                            source_desc.as_deref(),
                        );
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }
            }
        }

        Ok(None)
    }
}
