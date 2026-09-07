use super::*;

impl Backend {
    /// Convert (PathBuf, TextRange) reference locations to LSP Location objects.
    pub(crate) async fn ref_locations_to_lsp(
        &self,
        refs: &[(PathBuf, rowan::TextRange)],
    ) -> Vec<Location> {
        let py_sources = self.python_model_sources.lock().await;
        refs.iter()
            .filter_map(|(path, text_range)| {
                let (actual_path, line_offset) = py_sources
                    .get(path)
                    .map(|(p, line)| (p.clone(), *line))
                    .unwrap_or((path.clone(), 0));
                let uri = Url::from_file_path(&actual_path).ok()?;
                // Convert TextRange (byte offset) to line/col via file text.
                let file_text = std::fs::read_to_string(&actual_path).unwrap_or_default();
                let pr = crate::diagnostics_boundary::text_range_to_lsp_codepoint(
                    &file_text,
                    *text_range,
                );
                Some(Location {
                    uri,
                    range: Range {
                        start: Position::new(pr.start.line + line_offset, pr.start.character),
                        end: Position::new(pr.end.line + line_offset, pr.end.character),
                    },
                })
            })
            .collect()
    }

    /// Convert our database diagnostic to LSP diagnostic
    pub(crate) fn to_lsp_diagnostic(
        &self,
        diag: &DbDiagnostic,
        converter: &crate::diagnostics_boundary::BoundaryConverter,
    ) -> lsp_types::Diagnostic {
        let code = diag
            .code
            .map(|c| NumberOrString::String(diagnostic_code_str(c).to_string()));

        let data = diag.data.as_ref().map(|d| match d {
            DbData::UndefinedRef { model_name } => {
                serde_json::json!({ "kind": "undefined-ref", "modelName": model_name })
            }
            DbData::UndefinedSource {
                source_name,
                table_name,
            } => {
                serde_json::json!({ "kind": "undefined-source", "sourceName": source_name, "tableName": table_name })
            }
            DbData::CannotInferType { column_name } => {
                serde_json::json!({ "kind": "cannot-infer-type", "columnName": column_name })
            }
            DbData::UndeclaredColumn {
                qualifier,
                column_name,
            } => {
                serde_json::json!({ "kind": "undeclared-column", "qualifier": qualifier, "columnName": column_name })
            }
            DbData::TypeMismatch {
                column_name,
                ref_name,
                actual_type,
                expected_type,
            } => {
                serde_json::json!({
                    "kind": "type-mismatch",
                    "columnName": column_name,
                    "refName": ref_name,
                    "actualType": actual_type,
                    "expectedType": expected_type
                })
            }
            DbData::ExpansionFrames(frames) => {
                let frames_json: Vec<_> = frames
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "function": f.function,
                            "param": f.param,
                            "boundType": f.bound_type,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "kind": "expansion-frames",
                    "frames": frames_json,
                })
            }
            DbData::MissingSeedSidecar {
                csv_path,
                sidecar_path,
            } => {
                serde_json::json!({
                    "kind": "missing-seed-sidecar",
                    "csvPath": csv_path,
                    "sidecarPath": sidecar_path,
                })
            }
        });

        // Phase 12 (smelt-functions Step 1): expand the message body and
        // `DiagnosticRelatedInformation` list from the diagnostic's
        // `ExpansionFrames` payload. The pure helper below is unit-testable
        // directly (see `render_expansion_frames` tests).
        let (message, related_information) = render_expansion_frames(diag);

        lsp_types::Diagnostic {
            range: converter.convert(diag),
            severity: Some(match diag.severity {
                DbSeverity::Error => DiagnosticSeverity::ERROR,
                DbSeverity::Warning => DiagnosticSeverity::WARNING,
                DbSeverity::Info => DiagnosticSeverity::INFORMATION,
                DbSeverity::Hint => DiagnosticSeverity::HINT,
            }),
            message,
            source: Some("smelt".to_string()),
            code,
            data,
            related_information,
            ..Default::default()
        }
    }

    /// For a multi-model file, resolve a cursor position to the virtual path
    /// and adjusted line number within that section. Returns None for single-model files.
    pub(crate) async fn resolve_virtual_path(
        &self,
        real_path: &std::path::Path,
        line: u32,
    ) -> Option<(PathBuf, u32)> {
        let mm = self.multi_model_files.lock().await;
        let entries = mm.get(real_path)?;

        // Find the section that contains this line (last section whose sql_start_line <= line)
        let mut best: Option<&(PathBuf, u32, u32)> = None;
        for entry in entries {
            if entry.1 <= line {
                best = Some(entry);
            }
        }

        best.map(|(vp, sql_start_line, _)| (vp.clone(), line - sql_start_line))
    }

    /// Register a SQL file's content in the Salsa database, handling multi-model
    /// files by splitting them into virtual paths.
    ///
    /// Returns the list of paths that were registered (either `[real_path]` for
    /// single-model files, or `[real_path::name1, real_path::name2, ...]` for
    /// multi-model files).
    pub(crate) async fn register_sql_content(
        &self,
        db: &mut Database,
        real_path: &std::path::Path,
        content: &str,
        project_root: &std::path::Path,
    ) -> Vec<PathBuf> {
        let mut registered = Vec::new();

        // Try to detect multi-model file
        if let Ok(FileMetadata::Multi { models }) = extract_file_metadata(content) {
            let mut virtual_entries = Vec::new();

            for section in &models {
                let model_name = match &section.metadata.name {
                    Some(n) => n.clone(),
                    None => continue,
                };

                let virtual_path =
                    PathBuf::from(format!("{}::{}", real_path.display(), model_name));
                let sql_content = &content[section.sql_range.clone()];

                // Calculate the starting line of this section's SQL in the original file.
                let sql_start_line = content[..section.sql_range.start]
                    .chars()
                    .filter(|&c| c == '\n')
                    .count() as u32;

                // Find the `--- name: foo ---` delimiter line so the VSCode
                // TestController gutter icon lands on the declaration, not on
                // the closing `---` of the YAML block.
                let delimiter_line = content[..section.sql_range.start]
                    .lines()
                    .enumerate()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .find(|(_, l)| {
                        let t = l.trim();
                        if let Some(rest) = t.strip_prefix("--- name:") {
                            rest.trim_end().trim_end_matches("---").trim() == model_name.as_str()
                        } else {
                            false
                        }
                    })
                    .map(|(i, _)| i as u32)
                    .unwrap_or(0);

                // Upsert the SourceFile input. `set_source_file` only mutates the
                // underlying text/project_root when they differ, so spurious
                // revision bumps are avoided.
                let should_update = match db.source_file(&virtual_path) {
                    Some(f) => f.text(db) != sql_content,
                    None => true,
                };
                if should_update {
                    db.set_source_file(
                        virtual_path.clone(),
                        sql_content.to_string(),
                        project_root.to_path_buf(),
                    );
                }

                virtual_entries.push((virtual_path.clone(), sql_start_line, delimiter_line));
                registered.push(virtual_path);
            }

            // Store the mapping for diagnostics aggregation
            let mut mm = self.multi_model_files.lock().await;
            mm.insert(real_path.to_path_buf(), virtual_entries);
        } else {
            // Single-model or no frontmatter: register as-is
            let path_buf = real_path.to_path_buf();
            let should_update = match db.source_file(&path_buf) {
                Some(f) => f.text(db) != content,
                None => true,
            };
            if should_update {
                db.set_source_file(
                    path_buf.clone(),
                    content.to_string(),
                    project_root.to_path_buf(),
                );
            }
            registered.push(path_buf);

            // Clean up any old multi-model mapping
            let mut mm = self.multi_model_files.lock().await;
            mm.remove(real_path);
        }

        registered
    }

    /// Snapshot the DB under the write lock and return a cheap clone for
    /// lock-free reads. `Database: Clone` shares salsa storage internally via
    /// `Arc`, so this is a constant-time, memory-cheap operation.
    pub(crate) async fn snapshot(&self) -> Database {
        self.db.lock().await.clone()
    }

    /// Build a `BoundaryConverter` for `text` using the encoding negotiated
    /// with the client during `initialize`. Falls back to UTF-16 before
    /// negotiation completes (i.e. during the `initialize` request itself).
    pub(crate) async fn boundary_converter(
        &self,
        text: &str,
    ) -> crate::diagnostics_boundary::BoundaryConverter {
        let kind = self.negotiated_encoding.lock().await.clone();
        crate::diagnostics_boundary::BoundaryConverter::new_from_kind(text, &kind)
    }

    /// Rebuild the `Workspace` singleton from every currently-registered
    /// `SourceFile` + `ProjectInput`. Call after any input-set change so
    /// `all_models` / `resolve_ref` / diagnostics see the new set.
    pub(crate) fn sync_workspace(db: &mut Database, paths: &[PathBuf], project_roots: &[PathBuf]) {
        let files: Vec<SourceFile> = paths.iter().filter_map(|p| db.source_file(p)).collect();
        let projects: Vec<ProjectInput> = project_roots
            .iter()
            .filter_map(|r| db.project_input(r))
            .collect();
        db.set_workspace(files, projects);
    }

    /// Publish diagnostics for a file
    pub(crate) async fn publish_diagnostics(&self, uri: Url) {
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Check if this is a multi-model file
        let mm = self.multi_model_files.lock().await;
        let multi_entries = mm.get(&path).cloned();
        drop(mm);

        let db = self.snapshot().await;

        let mut lsp_diagnostics: Vec<lsp_types::Diagnostic> =
            if let Some(virtual_entries) = multi_entries {
                let mut lsp_diagnostics = Vec::new();
                for (virtual_path, sql_start_line, _) in virtual_entries {
                    let virtual_text = file_text(&db, &virtual_path);
                    let converter = self.boundary_converter(&virtual_text).await;
                    let diagnostics = diagnostics_for(&db, &virtual_path);
                    for d in &diagnostics {
                        let mut lsp_diag = self.to_lsp_diagnostic(d, &converter);
                        lsp_diag.range.start.line += sql_start_line;
                        lsp_diag.range.end.line += sql_start_line;
                        lsp_diagnostics.push(lsp_diag);
                    }
                }
                lsp_diagnostics
            } else {
                let text = file_text(&db, &path);
                let converter = self.boundary_converter(&text).await;
                diagnostics_for(&db, &path)
                    .iter()
                    .map(|d| self.to_lsp_diagnostic(d, &converter))
                    .collect()
            };

        // D6: append the cached `PropertyDowngrade` diagnostics for this
        // path INTO the same publish, rather than a second
        // `publish_diagnostics` call — `publish_diagnostics` replaces a
        // file's whole diagnostic set, so a separate call would clobber, or
        // be clobbered by, the Salsa set just published above.
        {
            let text = file_text(&db, &path);
            let converter = self.boundary_converter(&text).await;
            let property_diff = self.property_diff.lock().await;
            for state in property_diff.values() {
                if let Some(diags) = state.diagnostics.get(&path) {
                    lsp_diagnostics
                        .extend(diags.iter().map(|d| self.to_lsp_diagnostic(d, &converter)));
                }
            }
        }

        self.client
            .publish_diagnostics(uri, lsp_diagnostics, None)
            .await;
    }

    /// Publish diagnostics for all known model files
    pub(crate) async fn publish_all_diagnostics(&self) {
        let files = self.tracked_files.lock().await.clone();

        // Collect real file paths for multi-model files
        let mm = self.multi_model_files.lock().await;
        let multi_model_real_paths: Vec<PathBuf> = mm.keys().cloned().collect();
        // Collect all virtual paths so we can skip them in the main loop
        let virtual_paths: std::collections::HashSet<PathBuf> = mm
            .values()
            .flat_map(|entries| entries.iter().map(|(vp, _, _)| vp.clone()))
            .collect();
        drop(mm);

        for path in files.iter() {
            // Skip virtual paths — they'll be handled via their real file
            if virtual_paths.contains(path) {
                continue;
            }
            if let Ok(uri) = Url::from_file_path(path) {
                self.publish_diagnostics(uri).await;
            }
        }

        // Publish diagnostics for multi-model real files
        for path in &multi_model_real_paths {
            if let Ok(uri) = Url::from_file_path(path) {
                self.publish_diagnostics(uri).await;
            }
        }

        self.publish_source_diagnostics().await;
    }

    /// Refresh one project's property-diff state
    /// (`docs/specs/property_diff.md` §Surface "Editor";
    /// `docs/outcomes/20260905-property-diff/phases/07-plan.md` D7/R6): the
    /// pipeline (git resolution + working-tree/baseline derivation) runs in
    /// `spawn_blocking`, off the request path. Triggered on workspace load
    /// (`initialized`), a model save, an external `.sql`/`smelt.yml`
    /// change, and a `.git` HEAD/refs change (a promptness trigger, not the
    /// correctness mechanism — `crate::property_diff::refresh` always
    /// re-resolves and compares the commit). Concurrent triggers for the
    /// same project coalesce via `ProjectDiffState::running`.
    pub(crate) async fn refresh_property_diff(&self, project_root: PathBuf) {
        {
            let mut state = self.property_diff.lock().await;
            let entry = state.entry(project_root.clone()).or_default();
            if entry.running {
                // A refresh for this project is already in flight — record
                // that another trigger landed mid-flight rather than
                // running the ~2.4-3.0s pipeline again right now. The
                // in-flight run schedules exactly one trailing re-run when
                // it finishes (below), however many triggers set this flag
                // while it was running (risk R3: a burst collapses to one
                // extra run, not one per event).
                entry.pending = true;
                return;
            }
            entry.running = true;
        }

        // `running` is now true for this project and this call owns it
        // until it breaks out of the loop below (clearing `running` or
        // handing off to exactly one trailing iteration when `pending` was
        // set while a pass was in flight).
        loop {
            let cached_baseline = {
                let state = self.property_diff.lock().await;
                state
                    .get(&project_root)
                    .and_then(|e| e.cached_baseline.clone())
            };

            // Snapshot open-buffer overlays for this project's tracked
            // model files before handing off to the blocking task — Salsa
            // access must stay on the async side (`docs/outcomes/20260905-
            // property-diff/phases/07-plan.md` D4).
            let db = self.snapshot().await;
            let tracked = self.tracked_files.lock().await.clone();
            let mut overlays = std::collections::BTreeMap::new();
            for path in tracked.iter().filter(|p| p.starts_with(&project_root)) {
                overlays.insert(path.clone(), file_text(&db, path));
            }
            drop(db);

            self.property_diff_derivation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root_for_blocking = project_root.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                crate::property_diff::refresh(&root_for_blocking, &overlays, cached_baseline)
            })
            .await;

            let mut affected: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
            let mut log_info: Option<String> = None;
            let mut run_again = false;
            {
                let mut state = self.property_diff.lock().await;
                let entry = state.entry(project_root.clone()).or_default();
                match outcome {
                    Ok(crate::property_diff::RefreshOutcome::Report {
                        commit,
                        baseline,
                        lenses,
                        diagnostics,
                    }) => {
                        affected.extend(entry.lenses.keys().cloned());
                        affected.extend(entry.diagnostics.keys().cloned());
                        affected.extend(lenses.keys().cloned());
                        affected.extend(diagnostics.keys().cloned());
                        entry.baseline_commit = Some(commit);
                        entry.cached_baseline = Some(baseline);
                        entry.lenses = lenses;
                        entry.diagnostics = diagnostics;
                        entry.silent_reason = None;
                    }
                    Ok(crate::property_diff::RefreshOutcome::Silent(reason)) => {
                        // D8: not a git work tree, or the baseline cannot be
                        // resolved — no lens, no diagnostic, logged at info
                        // only (an un-versioned workspace is not an error).
                        affected.extend(entry.lenses.keys().cloned());
                        affected.extend(entry.diagnostics.keys().cloned());
                        entry.lenses.clear();
                        entry.diagnostics.clear();
                        entry.cached_baseline = None;
                        entry.baseline_commit = None;
                        entry.silent_reason = Some(reason.clone());
                        log_info = Some(reason);
                    }
                    Ok(crate::property_diff::RefreshOutcome::Failed(reason)) => {
                        // Δ3 ("in-flight behaviour"): a transient derivation
                        // failure keeps whatever diff was last computed rather
                        // than showing nothing.
                        log_info = Some(format!("property-diff refresh failed: {reason}"));
                    }
                    Err(join_err) => {
                        log_info = Some(format!("property-diff refresh task panicked: {join_err}"));
                    }
                }

                if entry.pending {
                    // Exactly one trailing re-run, however many triggers
                    // set `pending` while this pass was running.
                    entry.pending = false;
                    run_again = true;
                } else {
                    entry.running = false;
                }
            }

            if let Some(reason) = log_info {
                self.client.log_message(MessageType::INFO, reason).await;
            }
            for path in affected {
                if let Ok(uri) = Url::from_file_path(&path) {
                    self.publish_diagnostics(uri).await;
                }
            }
            // Best-effort: ask the client to re-pull code lenses. Ignored
            // if the client never advertised `codeLens.refreshSupport` —
            // the next `textDocument/codeLens` request (e.g. on
            // scroll/focus) picks up the fresh state regardless, since
            // `code_lens` only reads cached state.
            let _ = self
                .client
                .send_request::<request::CodeLensRefresh>(())
                .await;

            if !run_again {
                break;
            }
        }
    }

    /// Collect test entries from sql_files and merge into `known_tests`.
    ///
    /// Does NOT send the notification — call `publish_known_tests()` after.
    pub(crate) async fn collect_tests_into_cache(
        &self,
        sql_files: &[smelt_core::discovery::ModelFile],
    ) {
        use crate::notifications::TestInfo;

        let mm = self.multi_model_files.lock().await;
        let virtual_to_real_and_line: std::collections::HashMap<PathBuf, (PathBuf, u32)> = mm
            .iter()
            .flat_map(|(real_path, entries)| {
                entries
                    .iter()
                    .map(move |(virtual_path, _sql_start, delimiter_line)| {
                        (virtual_path.clone(), (real_path.clone(), *delimiter_line))
                    })
            })
            .collect();
        drop(mm);

        let mut tests = self.known_tests.lock().await;
        for model in sql_files {
            if !model.is_test() {
                continue;
            }
            let (real_path, line) =
                if let Some((rp, sl)) = virtual_to_real_and_line.get(&model.path) {
                    (rp.clone(), *sl)
                } else {
                    let source_path = model.model_id.source_path().to_path_buf();
                    (source_path, 0u32)
                };

            if let Ok(uri) = Url::from_file_path(&real_path) {
                // Avoid duplicates (multiple workspace folders may share paths)
                if !tests.iter().any(|t| t.name == model.name) {
                    tests.push(TestInfo {
                        name: model.name.clone(),
                        uri: uri.to_string(),
                        line,
                    });
                }
            }
        }
    }

    /// Send `smelt/publishTests` with the current `known_tests` cache.
    pub(crate) async fn publish_known_tests(&self) {
        use crate::notifications::{PublishTests, PublishTestsParams};

        let tests = self.known_tests.lock().await.clone();
        self.client
            .send_notification::<PublishTests>(PublishTestsParams { tests })
            .await;
    }

    /// Rebuild the test cache from all project roots and re-publish.
    ///
    /// Called after file-change events so the TestController stays up to date.
    pub(crate) async fn refresh_and_publish_tests(&self) {
        let project_roots = self.project_roots.lock().await.clone();
        let mut all_sql_files = Vec::new();
        for root in &project_roots {
            let loaded = smelt_core::load_workspace(root);
            all_sql_files.extend(loaded.sql_files);
        }

        *self.known_tests.lock().await = Vec::new();
        self.collect_tests_into_cache(&all_sql_files).await;
        self.publish_known_tests().await;
    }

    /// Publish per-entity source YAML diagnostics, project-scoped.
    ///
    /// These `.yml` files are not tracked `SourceFile` inputs (they have no SQL
    /// body the per-file diagnostic path checks), so a malformed source is
    /// published here to its own file URI — flagging it red in the editor exactly
    /// as the build gate refuses it (`architecture.md` §"Diagnostic parity
    /// rule"). Source discovery is keyed on `ProjectInput` and restart-scoped, so
    /// publishing once at `initialized` (and on `sources.yml` refresh) is the
    /// source surface's lifecycle.
    pub(crate) async fn publish_source_diagnostics(&self) {
        let db = self.snapshot().await;
        let Some(ws) = Workspace::try_get(&db) else {
            return;
        };
        // Group by file so each `.yml` gets a single publish (overwriting any
        // prior publish for that URI), even if it ever carries >1 diagnostic.
        let mut by_path: std::collections::HashMap<PathBuf, Vec<lsp_types::Diagnostic>> =
            std::collections::HashMap::new();
        for project in ws.projects(&db).iter().copied() {
            for sd in project_source_diagnostics(&db, project).iter() {
                let text = std::fs::read_to_string(&sd.path).unwrap_or_default();
                let converter = self.boundary_converter(&text).await;
                let lsp_diag = self.to_lsp_diagnostic(&sd.diagnostic, &converter);
                by_path.entry(sd.path.clone()).or_default().push(lsp_diag);
            }
        }
        for (path, diags) in by_path {
            if let Ok(uri) = Url::from_file_path(&path) {
                self.client.publish_diagnostics(uri, diags, None).await;
            }
        }
    }

    /// Publish `DuplicateAddress` diagnostics for every project in the workspace.
    ///
    /// Address collisions involve two files; the diagnostic is anchored at the
    /// second (later-discovered) file, mirroring the `project_source_diagnostics`
    /// pattern. Called once at `initialized` alongside `publish_source_diagnostics`.
    pub(crate) async fn publish_address_collision_diagnostics(&self) {
        let db = self.snapshot().await;
        let Some(ws) = Workspace::try_get(&db) else {
            return;
        };
        let mut by_path: std::collections::HashMap<PathBuf, Vec<lsp_types::Diagnostic>> =
            std::collections::HashMap::new();
        for project in ws.projects(&db).iter().copied() {
            for cd in project_address_collisions(&db, project).iter() {
                let text = std::fs::read_to_string(&cd.path).unwrap_or_default();
                let converter = self.boundary_converter(&text).await;
                let lsp_diag = self.to_lsp_diagnostic(&cd.diagnostic, &converter);
                by_path.entry(cd.path.clone()).or_default().push(lsp_diag);
            }
        }
        for (path, diags) in by_path {
            if let Ok(uri) = Url::from_file_path(&path) {
                self.client.publish_diagnostics(uri, diags, None).await;
            }
        }
    }

    /// Publish `DuplicateEmittedName` diagnostics for every project in the workspace.
    ///
    /// Like `publish_address_collision_diagnostics` but for the emitted-name
    /// collision check (the `_`-join non-injective clobber). Called once at
    /// `initialized` alongside the other project-scoped diagnostic publishers.
    pub(crate) async fn publish_emitted_name_collision_diagnostics(&self) {
        let db = self.snapshot().await;
        let Some(ws) = Workspace::try_get(&db) else {
            return;
        };
        let mut by_path: std::collections::HashMap<PathBuf, Vec<lsp_types::Diagnostic>> =
            std::collections::HashMap::new();
        for project in ws.projects(&db).iter().copied() {
            for ec in project_emitted_name_collisions(&db, project).iter() {
                let text = std::fs::read_to_string(&ec.path).unwrap_or_default();
                let converter = self.boundary_converter(&text).await;
                let lsp_diag = self.to_lsp_diagnostic(&ec.diagnostic, &converter);
                by_path.entry(ec.path.clone()).or_default().push(lsp_diag);
            }
        }
        for (path, diags) in by_path {
            if let Ok(uri) = Url::from_file_path(&path) {
                self.client.publish_diagnostics(uri, diags, None).await;
            }
        }
    }

    /// Handle a Python model file change: re-execute and update Salsa.
    /// Uses background execution with last-known-good fallback on failure.
    pub(crate) async fn handle_python_file_change(&self, py_path: &std::path::Path) {
        // Find the project root for this file
        let project_roots = self.project_roots.lock().await.clone();
        let project_root = match find_project_root_for_file(py_path, &project_roots) {
            Some(root) => root,
            None => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!(
                            "Cannot find project root for Python model: {}",
                            py_path.display()
                        ),
                    )
                    .await;
                return;
            }
        };

        let py_path = py_path.to_path_buf();
        let db = self.db.clone();
        let tracked_files = self.tracked_files.clone();
        let project_roots_handle = self.project_roots.clone();
        let py_sources = self.python_model_sources.clone();
        let py_diags = self.python_diagnostics.clone();
        let cache = self.python_cache.clone();
        let client = self.client.clone();
        let negotiated_encoding = self.negotiated_encoding.clone();

        // Build context from current model list
        let context_json = {
            let all_files = self.tracked_files.lock().await.clone();
            let config =
                smelt_core::Config::load(&project_root).unwrap_or_else(|_| smelt_core::Config {
                    name: String::new(),
                    version: 1,
                    paths: vec!["models".to_string()],
                    targets: std::collections::HashMap::new(),
                    default_materialization: smelt_core::Materialization::View,
                    models: std::collections::HashMap::new(),
                    python: None,
                    target: None,
                    state: Default::default(),
                    maintenance: None,
                    probes: Default::default(),
                });
            build_python_context(&all_files, &config, &project_root)
        };

        // Spawn background task for subprocess execution
        tokio::task::spawn(async move {
            let py_path_for_blocking = py_path.clone();
            let project_root_for_blocking = project_root.clone();
            let cache_for_blocking = cache.clone();

            let scan_result = tokio::task::spawn_blocking(move || {
                let mut cache_guard = cache_for_blocking.blocking_lock();
                crate::python_scan::execute_single_python_file(
                    &py_path_for_blocking,
                    &project_root_for_blocking,
                    &mut cache_guard,
                    &context_json,
                )
            })
            .await;

            let scan_result = match scan_result {
                Ok(r) => r,
                Err(e) => {
                    client
                        .log_message(
                            MessageType::ERROR,
                            format!("Python model re-execution panicked: {}", e),
                        )
                        .await;
                    return;
                }
            };

            // Update Python diagnostics for this file
            {
                let mut diags = py_diags.lock().await;
                if scan_result.errors.is_empty() {
                    // Clear previous errors
                    diags.remove(&py_path);
                    if let Ok(uri) = Url::from_file_path(&py_path) {
                        client.publish_diagnostics(uri, Vec::new(), None).await;
                    }
                } else {
                    let file_diags: Vec<lsp_types::Diagnostic> = scan_result
                        .errors
                        .iter()
                        .map(|error| {
                            let line = error.line.unwrap_or(1).saturating_sub(1);
                            lsp_types::Diagnostic {
                                range: Range {
                                    start: Position::new(line, 0),
                                    end: Position::new(line, 0),
                                },
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: error.message.clone(),
                                source: Some("smelt-python".to_string()),
                                ..Default::default()
                            }
                        })
                        .collect();
                    diags.insert(py_path.clone(), file_diags.clone());
                    if let Ok(uri) = Url::from_file_path(&py_path) {
                        client.publish_diagnostics(uri, file_diags, None).await;
                    }
                }
            }

            // On failure, keep last-known-good SQL in Salsa (don't update)
            if scan_result.models.is_empty() && !scan_result.errors.is_empty() {
                client
                    .log_message(
                        MessageType::WARNING,
                        format!(
                            "Python model {} failed, keeping last-known-good SQL",
                            py_path.display()
                        ),
                    )
                    .await;
                return;
            }

            // Update Salsa with new SQL
            {
                let mut db_guard = db.lock().await;
                let mut sources = py_sources.lock().await;
                let mut files = tracked_files.lock().await;

                // Remove old virtual paths from this .py file
                let old_virtual_paths: Vec<PathBuf> = sources
                    .iter()
                    .filter(|(_, (src, _))| *src == py_path)
                    .map(|(vp, _)| vp.clone())
                    .collect();

                for old_vp in &old_virtual_paths {
                    sources.remove(old_vp);
                    files.retain(|f| f != old_vp);
                }

                // Register new models (skip mutations when values unchanged)
                for py_model in &scan_result.models {
                    let virtual_sql_path = py_model
                        .source_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(format!("{}.sql", py_model.name));

                    let should_update = match db_guard.source_file(&virtual_sql_path) {
                        Some(f) => f.text(&*db_guard) != &py_model.sql,
                        None => true,
                    };
                    if should_update {
                        db_guard.set_source_file(
                            virtual_sql_path.clone(),
                            py_model.sql.clone(),
                            project_root.clone(),
                        );
                    }
                    sources.insert(
                        virtual_sql_path.clone(),
                        (py_model.source_path.clone(), py_model.decorator_line),
                    );
                    if !files.contains(&virtual_sql_path) {
                        files.push(virtual_sql_path);
                    }
                }

                let project_roots = project_roots_handle.lock().await.clone();
                Backend::sync_workspace(&mut db_guard, &files, &project_roots);
            }

            // Republish all diagnostics since ref resolution may have changed
            let files = tracked_files.lock().await.clone();
            let db_snapshot = db.lock().await.clone();
            let enc_kind = negotiated_encoding.lock().await.clone();

            for path in files.iter() {
                if let Ok(uri) = Url::from_file_path(path) {
                    let diagnostics = diagnostics_for(&db_snapshot, path);
                    let file_text = crate::db_helpers::file_text(&db_snapshot, path);
                    let converter = crate::diagnostics_boundary::BoundaryConverter::new_from_kind(
                        &file_text, &enc_kind,
                    );
                    let lsp_diagnostics: Vec<lsp_types::Diagnostic> = diagnostics
                        .iter()
                        .map(|d| lsp_types::Diagnostic {
                            range: converter.convert(d),
                            severity: Some(match d.severity {
                                DbSeverity::Error => DiagnosticSeverity::ERROR,
                                DbSeverity::Warning => DiagnosticSeverity::WARNING,
                                DbSeverity::Info => DiagnosticSeverity::INFORMATION,
                                DbSeverity::Hint => DiagnosticSeverity::HINT,
                            }),
                            message: d.message.clone(),
                            source: Some("smelt".to_string()),
                            ..Default::default()
                        })
                        .collect();
                    client.publish_diagnostics(uri, lsp_diagnostics, None).await;
                }
            }

            client
                .log_message(
                    MessageType::INFO,
                    format!(
                        "Python model {} re-executed successfully ({} model(s))",
                        py_path.display(),
                        scan_result.models.len()
                    ),
                )
                .await;
        });
    }
}
