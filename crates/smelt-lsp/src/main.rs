use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use smelt_db::{
    Database, Diagnostic as DbDiagnostic, DiagnosticSeverity as DbSeverity, Inputs, Schema,
    Semantic, Syntax, TypeChecking,
};

mod python_scan;
use smelt_parser::ast::File as AstFile;
use smelt_types::TypedColumn;

/// Find a config file supporting both .yml and .yaml extensions.
/// Returns the path if exactly one exists. Errors if both exist.
fn find_config_file(
    dir: &std::path::Path,
    base_name: &str,
) -> std::result::Result<Option<PathBuf>, String> {
    let yml = dir.join(format!("{}.yml", base_name));
    let yaml = dir.join(format!("{}.yaml", base_name));
    match (yml.exists(), yaml.exists()) {
        (true, true) => Err(format!(
            "Both {}.yml and {}.yaml exist in {}",
            base_name,
            base_name,
            dir.display()
        )),
        (true, false) => Ok(Some(yml)),
        (false, true) => Ok(Some(yaml)),
        (false, false) => Ok(None),
    }
}

/// Recursively find directories containing smelt.yml/smelt.yaml
fn find_smelt_projects(root: &std::path::Path) -> Vec<PathBuf> {
    let mut projects = Vec::new();
    find_smelt_projects_recursive(root, &mut projects);
    projects
}

/// Check if a file is a sources config file (sources.yml or sources.yaml)
fn is_sources_file(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "sources.yml" || n == "sources.yaml")
}

/// Find the project root for a file by longest prefix match against known roots
fn find_project_root_for_file(file: &std::path::Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| file.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

/// Walk up from a file path looking for smelt.yml/smelt.yaml to find project root
fn find_project_root_by_walking_up(file: &std::path::Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join("smelt.yml").exists() || dir.join("smelt.yaml").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

fn find_smelt_projects_recursive(dir: &std::path::Path, projects: &mut Vec<PathBuf>) {
    // Check if this directory has smelt.yml or smelt.yaml
    let has_yml = dir.join("smelt.yml").exists();
    let has_yaml = dir.join("smelt.yaml").exists();
    if has_yml || has_yaml {
        projects.push(dir.to_path_buf());
    }

    // Recurse into subdirectories, skipping common non-project dirs
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, ".git" | "target" | "node_modules") {
                        continue;
                    }
                }
                find_smelt_projects_recursive(&path, projects);
            }
        }
    }
}

/// Tracks errors that occurred during workspace initialization
#[derive(Default)]
struct InitErrors {
    workspace_errors: Vec<String>,
    source_errors: Vec<String>,
    model_errors: Vec<String>,
}

impl InitErrors {
    fn has_errors(&self) -> bool {
        !self.workspace_errors.is_empty()
            || !self.source_errors.is_empty()
            || !self.model_errors.is_empty()
    }

    fn total_count(&self) -> usize {
        self.workspace_errors.len() + self.source_errors.len() + self.model_errors.len()
    }
}

/// Format a TypedColumn for display in hover/completion
fn format_type(typed_col: &TypedColumn) -> String {
    let nullable_suffix = if typed_col.nullable { "?" } else { "" };
    format!("{}{}", typed_col.data_type, nullable_suffix)
}

struct Backend {
    client: Client,
    db: Arc<Mutex<Database>>,
    /// Errors collected during initialization, reported after `initialized` notification
    init_errors: Arc<Mutex<Option<InitErrors>>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            db: Arc::new(Mutex::new(Database::default())),
            init_errors: Arc::new(Mutex::new(None)),
        }
    }

    /// Convert URI to file path, logging a warning if conversion fails.
    /// Returns None for non-file URIs (e.g., untitled:, git:).
    async fn uri_to_path(&self, uri: &Url) -> Option<PathBuf> {
        match uri.to_file_path() {
            Ok(p) => Some(p),
            Err(_) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Cannot process non-file URI: {}", uri),
                    )
                    .await;
                None
            }
        }
    }

    /// Convert our database diagnostic to LSP diagnostic
    fn to_lsp_diagnostic(&self, diag: &DbDiagnostic) -> lsp_types::Diagnostic {
        lsp_types::Diagnostic {
            range: Range {
                start: Position {
                    line: diag.range.start.line,
                    character: diag.range.start.column,
                },
                end: Position {
                    line: diag.range.end.line,
                    character: diag.range.end.column,
                },
            },
            severity: Some(match diag.severity {
                DbSeverity::Error => DiagnosticSeverity::ERROR,
                DbSeverity::Warning => DiagnosticSeverity::WARNING,
                DbSeverity::Info => DiagnosticSeverity::INFORMATION,
            }),
            message: diag.message.clone(),
            source: Some("smelt".to_string()),
            ..Default::default()
        }
    }

    /// Publish diagnostics for a file
    async fn publish_diagnostics(&self, uri: Url) {
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        let db = self.db.lock().await;
        let diagnostics = db.file_diagnostics(path);

        let lsp_diagnostics: Vec<lsp_types::Diagnostic> = diagnostics
            .iter()
            .map(|d| self.to_lsp_diagnostic(d))
            .collect();

        self.client
            .publish_diagnostics(uri, lsp_diagnostics, None)
            .await;
    }

    /// Publish diagnostics for all known model files
    async fn publish_all_diagnostics(&self) {
        let db = self.db.lock().await;
        let files = db.all_files();
        let files = files.clone();
        drop(db);

        for path in files.iter() {
            if let Ok(uri) = Url::from_file_path(path) {
                self.publish_diagnostics(uri).await;
            }
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut init_errors = InitErrors::default();

        // Initialize inputs to empty first - ensures Salsa queries are always set
        // even if workspace folders aren't provided or models/ doesn't exist
        {
            let mut db = self.db.lock().await;
            db.set_all_files(Arc::new(Vec::new()));
            db.set_all_project_roots(Arc::new(Vec::new()));
        }

        // Get workspace folders if provided
        if let Some(workspace_folders) = params.workspace_folders {
            let mut db = self.db.lock().await;
            let mut all_files = Vec::new();
            let mut all_project_roots = Vec::new();

            for folder in &workspace_folders {
                let workspace_path = match folder.uri.to_file_path() {
                    Ok(p) => p,
                    Err(_) => {
                        init_errors.workspace_errors.push(format!(
                            "Cannot process workspace folder URI: {}",
                            folder.uri
                        ));
                        continue;
                    }
                };

                // Recursively discover smelt projects
                let project_roots = find_smelt_projects(&workspace_path);

                for project_root in project_roots {
                    // Check for ambiguous smelt config
                    if project_root.join("smelt.yml").exists()
                        && project_root.join("smelt.yaml").exists()
                    {
                        init_errors.workspace_errors.push(format!(
                            "Both smelt.yml and smelt.yaml exist in {}",
                            project_root.display()
                        ));
                        continue;
                    }

                    all_project_roots.push(project_root.clone());

                    // Load sources config for this project
                    match find_config_file(&project_root, "sources") {
                        Ok(Some(sources_path)) => match std::fs::read_to_string(&sources_path) {
                            Ok(content) => {
                                db.set_project_sources_yaml(
                                    project_root.clone(),
                                    Arc::new(content),
                                );
                            }
                            Err(e) => {
                                init_errors.source_errors.push(format!(
                                    "Failed to read {}: {}",
                                    sources_path.display(),
                                    e
                                ));
                                db.set_project_sources_yaml(
                                    project_root.clone(),
                                    Arc::new(String::new()),
                                );
                            }
                        },
                        Ok(None) => {
                            // No sources file - that's fine
                            db.set_project_sources_yaml(
                                project_root.clone(),
                                Arc::new(String::new()),
                            );
                        }
                        Err(msg) => {
                            init_errors.source_errors.push(msg);
                            db.set_project_sources_yaml(
                                project_root.clone(),
                                Arc::new(String::new()),
                            );
                        }
                    }

                    // Scan models/ directory for this project
                    let models_path = project_root.join("models");
                    match std::fs::read_dir(&models_path) {
                        Ok(entries) => {
                            for entry_result in entries {
                                match entry_result {
                                    Ok(entry) => {
                                        let entry_path = entry.path();
                                        if entry_path.extension().and_then(|s| s.to_str())
                                            == Some("sql")
                                        {
                                            match std::fs::read_to_string(&entry_path) {
                                                Ok(content) => {
                                                    db.set_file_text(
                                                        entry_path.clone(),
                                                        Arc::new(content),
                                                    );
                                                    db.set_file_project_root(
                                                        entry_path.clone(),
                                                        project_root.clone(),
                                                    );
                                                    all_files.push(entry_path);
                                                }
                                                Err(e) => {
                                                    init_errors.model_errors.push(format!(
                                                        "Failed to read {}: {}",
                                                        entry_path.display(),
                                                        e
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        init_errors.model_errors.push(format!(
                                            "Failed to read directory entry in {}: {}",
                                            models_path.display(),
                                            e
                                        ));
                                    }
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            // Not an error - models/ directory is optional
                        }
                        Err(e) => {
                            init_errors.workspace_errors.push(format!(
                                "Failed to read {}: {}",
                                models_path.display(),
                                e
                            ));
                        }
                    }

                    // Discover Python models and register their generated SQL
                    let python_models =
                        python_scan::discover_python_models(&models_path, &project_root);
                    for py_model in &python_models {
                        // Store generated SQL under a synthetic .sql path so it is not
                        // overwritten by did_open/did_change updates for the .py source file.
                        let mut virtual_sql_path = py_model.source_path.clone();
                        virtual_sql_path.set_extension("sql");

                        db.set_file_text(virtual_sql_path.clone(), Arc::new(py_model.sql.clone()));
                        db.set_file_project_root(virtual_sql_path.clone(), project_root.clone());
                        all_files.push(virtual_sql_path);
                    }
                }
            }

            db.set_all_files(Arc::new(all_files));
            db.set_all_project_roots(Arc::new(all_project_roots));
        }

        // Store errors for reporting after initialized notification
        *self.init_errors.lock().await = Some(init_errors);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "'".to_string(),
                        "(".to_string(),
                        ".".to_string(),
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "smelt language server initialized")
            .await;

        // Report any initialization errors
        if let Some(errors) = self.init_errors.lock().await.take() {
            if errors.has_errors() {
                // Log each error
                for err in &errors.workspace_errors {
                    self.client.log_message(MessageType::ERROR, err).await;
                }
                for err in &errors.source_errors {
                    self.client.log_message(MessageType::WARNING, err).await;
                }
                for err in &errors.model_errors {
                    self.client.log_message(MessageType::WARNING, err).await;
                }

                // Show summary notification to user
                self.client
                    .show_message(
                        MessageType::WARNING,
                        format!(
                            "smelt: {} file(s) failed to load. Check Output for details.",
                            errors.total_count()
                        ),
                    )
                    .await;
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Check if this is sources.yml/yaml - update sources config and refresh all diagnostics
        if is_sources_file(&path) {
            if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                let mut db = self.db.lock().await;
                db.set_project_sources_yaml(project_root, Arc::new(params.text_document.text));
                drop(db);
                self.publish_all_diagnostics().await;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let mut db = self.db.lock().await;
            // If this file wasn't seen during init, find its project root
            let project_roots = db.all_project_roots();
            let has_project_root = project_roots.iter().any(|root| path.starts_with(root));
            if has_project_root {
                // Find matching project root (longest prefix match)
                if let Some(project_root) = find_project_root_for_file(&path, &project_roots) {
                    db.set_file_project_root(path.clone(), project_root);
                }
            } else {
                // Try to discover project root by walking up
                if let Some(project_root) = find_project_root_by_walking_up(&path) {
                    // Register this new project
                    let mut roots = (*project_roots).clone();
                    if !roots.contains(&project_root) {
                        roots.push(project_root.clone());
                        db.set_all_project_roots(Arc::new(roots));
                        // Load sources for this project
                        let sources_content = find_config_file(&project_root, "sources")
                            .ok()
                            .flatten()
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .unwrap_or_default();
                        db.set_project_sources_yaml(
                            project_root.clone(),
                            Arc::new(sources_content),
                        );
                    }
                    db.set_file_project_root(path.clone(), project_root);
                }
            }
            // Register this file if not already known
            let mut files = (*db.all_files()).clone();
            if !files.contains(&path) {
                files.push(path.clone());
                db.set_all_files(Arc::new(files));
            }
            db.set_file_text(path, Arc::new(params.text_document.text));
            drop(db);
            self.publish_diagnostics(uri).await;
        } else {
            // Non-SQL, non-sources file
            let mut db = self.db.lock().await;
            db.set_file_text(path, Arc::new(params.text_document.text));
            drop(db);
            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Get new text (we use FULL sync, so there's only one change)
        if let Some(change) = params.content_changes.into_iter().next() {
            if is_sources_file(&path) {
                if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                    let mut db = self.db.lock().await;
                    db.set_project_sources_yaml(project_root, Arc::new(change.text));
                    drop(db);
                    self.publish_all_diagnostics().await;
                }
            } else {
                let mut db = self.db.lock().await;
                db.set_file_text(path, Arc::new(change.text));
                drop(db);
                self.publish_diagnostics(uri).await;
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let db = self.db.lock().await;

        // Get file content and parse tree
        let text = db.file_text(path.clone());
        let parse = db.parse_file(path.clone());
        let syntax = parse.syntax();

        // Convert cursor position to offset
        let cursor_offset = {
            let mut offset = 0usize;
            let mut line = 0u32;
            let mut col = 0u32;

            for ch in text.chars() {
                if line == position.line && col == position.character {
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

        // Find RefCall at cursor position using AST
        if let Some(file) = AstFile::cast(syntax) {
            for ref_call in file.refs() {
                let range = ref_call.range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();

                // Check if cursor is within this ref call
                if cursor_offset >= start && cursor_offset <= end {
                    if let Some(ref_name) = ref_call.model_name() {
                        // Resolve the ref
                        if let Some(target_path) = db.resolve_ref(ref_name) {
                            if let Ok(target_uri) = Url::from_file_path(&target_path) {
                                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                                    uri: target_uri,
                                    range: Range {
                                        start: Position::new(0, 0),
                                        end: Position::new(0, 0),
                                    },
                                })));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let db = self.db.lock().await;

        // Get file content and parse tree
        let text = db.file_text(path.clone());
        let parse = db.parse_file(path.clone());
        let syntax = parse.syntax();

        // Convert cursor position to offset
        let cursor_offset = {
            let mut offset = 0usize;
            let mut line = 0u32;
            let mut col = 0u32;

            for ch in text.chars() {
                if line == position.line && col == position.character {
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

        // Check if hovering over a ref() or source() call
        if let Some(file) = AstFile::cast(syntax) {
            // Check ref() calls
            for ref_call in file.refs() {
                let range = ref_call.range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();

                // Check if cursor is within this ref call
                if cursor_offset >= start && cursor_offset <= end {
                    if let Some(model_name) = ref_call.model_name() {
                        // Resolve upstream model and show its resolved schema
                        if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
                            // Use resolved_model_schema to get type information through wildcards
                            let resolved = db.resolved_model_schema(upstream_path.clone());

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
                                        if !col.expression.is_empty() && col.expression != col.name
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
                            let constraints = db.model_input_constraints(upstream_path);
                            if !constraints.is_empty() {
                                content.push_str("\n**Requires:**\n");
                                for constraint in constraints.iter() {
                                    for (col_name, col_constraint) in &constraint.required_columns {
                                        if let Some(ref typed_col) = col_constraint.expected_type {
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

            // Check source() calls
            for source_call in file.sources() {
                let range = source_call.range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();

                // Check if cursor is within this source call
                if cursor_offset >= start && cursor_offset <= end {
                    if let (Some(source_name), Some(table_name)) =
                        (source_call.source_name(), source_call.table_name())
                    {
                        let qualified_name = source_call.qualified_name().unwrap_or_default();

                        // Try to resolve the source
                        let project_root = db.file_project_root(path.clone());
                        if let Some(table_def) =
                            db.resolve_source(project_root, source_name.clone(), table_name.clone())
                        {
                            // Format source info as markdown
                            let mut content = format!("**Source: {}**\n\n", qualified_name);

                            // Show table description if available
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
                        } else {
                            // Source not found - show error hover
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
                }
            }
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let db = self.db.lock().await;

        // Get file content
        let text = db.file_text(path.clone());

        // Convert cursor position to offset
        let cursor_offset = {
            let mut offset = 0usize;
            let mut line = 0u32;
            let mut col = 0u32;

            for ch in text.chars() {
                if line == position.line && col == position.character {
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

        // Determine completion context
        let context = determine_completion_context(&text, cursor_offset);

        let items = match context {
            CompletionContext::InsideRef => {
                // Complete model names
                let models = db.all_models();
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
                let project_root = db.file_project_root(path.clone());
                let config = db.sources_config(project_root);
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
                let typed_schema = db.typed_model_schema(path.clone());
                let available = db.available_columns(path);

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
                let parse = db.parse_file(path.clone());
                let syntax = parse.syntax();

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
                                    let project_root = db.file_project_root(path.clone());
                                    let config = db.sources_config(project_root);
                                    for source in &config.sources {
                                        if source.name == *source_name {
                                            for table in &source.tables {
                                                if table.name == *table_name {
                                                    return Ok(Some(CompletionResponse::Array(
                                                        table
                                                            .columns
                                                            .iter()
                                                            .map(|col| {
                                                                let type_str = col
                                                                    .data_type
                                                                    .as_ref()
                                                                    .map(|t| t.to_string())
                                                                    .unwrap_or_else(|| {
                                                                        "unknown".to_string()
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
                                                    )));
                                                }
                                            }
                                        }
                                    }
                                }
                                AliasTarget::Model { model_name } => {
                                    // Get columns from the model schema
                                    let models = db.all_models();
                                    if let Some(model) =
                                        models.values().find(|m| m.name == *model_name)
                                    {
                                        let schema = db.model_schema(model.path.clone());
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
                Vec::new()
            }
            CompletionContext::FromClause => {
                // Offer CTE names defined in the current query's WITH clause
                let parse = db.parse_file(path.clone());
                let syntax = parse.syntax();

                let mut items = Vec::new();

                if let Some(file) = smelt_parser::ast::File::cast(syntax) {
                    if let Some(select_stmt) = file.select_stmt() {
                        if let Some(with_clause) = select_stmt.with_clause() {
                            let type_ctx = db.type_context(path.clone());

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

/// Completion context types
#[derive(Debug)]
enum CompletionContext {
    InsideRef,               // Cursor inside ref('|')
    InsideSource,            // Cursor inside source('|')
    ColumnName,              // Cursor in a position where column name is expected
    QualifiedColumn(String), // Cursor after alias. (e.g., "t." for table alias t)
    FromClause,              // Cursor in FROM/JOIN position (offer CTE names)
    None,
}

/// Determine what kind of completion to provide based on cursor position
fn determine_completion_context(text: &str, offset: usize) -> CompletionContext {
    // Look backward from cursor to determine context
    let before_cursor = &text[..offset.min(text.len())];

    // Check if we're inside source('')
    // Simple heuristic: look for source(' before cursor and no closing )
    if let Some(source_start) = before_cursor.rfind("source(") {
        let after_source = &before_cursor[source_start..];
        // Check if we're inside the quotes
        let quote_count = after_source
            .chars()
            .filter(|&c| c == '\'' || c == '"')
            .count();
        if quote_count == 1 && !after_source.contains(')') {
            // Odd number of quotes means we're inside a string, and no closing paren yet
            return CompletionContext::InsideSource;
        }
    }

    // Check if we're inside ref('')
    // Simple heuristic: look for ref(' before cursor and no closing )
    if let Some(ref_start) = before_cursor.rfind("ref(") {
        let after_ref = &before_cursor[ref_start..];
        // Check if we're inside the quotes
        let quote_count = after_ref.chars().filter(|&c| c == '\'' || c == '"').count();
        if quote_count == 1 && !after_ref.contains(')') {
            // Odd number of quotes means we're inside a string, and no closing paren yet
            return CompletionContext::InsideRef;
        }
    }

    // Check if we're after alias. (e.g., "t." for qualified column completion)
    // Look for pattern: identifier followed by dot at or just before cursor
    if let Some(alias) = extract_alias_before_dot(before_cursor) {
        return CompletionContext::QualifiedColumn(alias);
    }

    // Check if we're in a column context (after SELECT, comma in SELECT list)
    let before_trimmed = before_cursor.trim_end();

    // Look for SELECT keyword
    if let Some(select_pos) = before_trimmed.rfind("SELECT") {
        let after_select = &before_trimmed[select_pos..];
        // Make sure we haven't hit FROM yet
        if !after_select.contains("FROM") {
            // We're in the SELECT list
            return CompletionContext::ColumnName;
        }
    }

    // Check if we're in a FROM/JOIN position (after FROM or JOIN keyword)
    // Look for the last FROM or JOIN keyword and check we're in table-ref position
    let upper = before_trimmed.to_uppercase();
    if is_in_from_position(&upper) {
        return CompletionContext::FromClause;
    }

    CompletionContext::None
}

/// Check if cursor is in a FROM/JOIN table reference position
fn is_in_from_position(upper_text: &str) -> bool {
    // Find the last occurrence of FROM or JOIN keywords
    let from_pos = upper_text.rfind("FROM");
    let join_pos = upper_text.rfind("JOIN");

    let keyword_end = match (from_pos, join_pos) {
        (Some(f), Some(j)) => {
            if f > j {
                Some(f + 4) // "FROM" is 4 chars
            } else {
                Some(j + 4) // "JOIN" is 4 chars
            }
        }
        (Some(f), None) => Some(f + 4),
        (None, Some(j)) => Some(j + 4),
        (None, None) => None,
    };

    let keyword_end = match keyword_end {
        Some(e) => e,
        None => return false,
    };

    // Text after the keyword
    let after_keyword = &upper_text[keyword_end..];

    // We're in FROM position if:
    // 1. Nothing after keyword (just whitespace) - typing the first table ref
    // 2. Or after a comma (additional table ref in comma-separated list)
    // But NOT if we've already entered a complete expression (have ON, WHERE, etc.)
    let trimmed = after_keyword.trim();
    if trimmed.is_empty() {
        return true;
    }

    // If we see clause keywords after the FROM/JOIN, we've moved past table position
    let terminating_keywords = [
        "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "UNION", "ON", "USING", "INNER", "LEFT",
        "RIGHT", "FULL", "CROSS", "SELECT",
    ];
    for kw in &terminating_keywords {
        if trimmed.contains(kw) {
            return false;
        }
    }

    // If the text after keyword is just whitespace or a partial identifier, we're in position
    // Check: no complete table expression yet (no whitespace-separated tokens beyond one)
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    // If 0 tokens (just spaces) or 1 partial token being typed - we're in FROM position
    tokens.len() <= 1
}

/// Extract the alias/identifier before a dot at the end of the text
/// Returns Some(alias) if text ends with "identifier." or "identifier.partial"
fn extract_alias_before_dot(text: &str) -> Option<String> {
    // Find the last dot
    let dot_pos = text.rfind('.')?;

    // Check what's after the dot - should be empty or partial identifier
    let after_dot = &text[dot_pos + 1..];
    if !after_dot.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    // Find the identifier before the dot
    let before_dot = &text[..dot_pos];
    let before_dot_trimmed = before_dot.trim_end();

    // Walk backward to find the start of the identifier
    let mut ident_start = before_dot_trimmed.len();
    for (i, c) in before_dot_trimmed.char_indices().rev() {
        if c.is_alphanumeric() || c == '_' {
            ident_start = i;
        } else {
            break;
        }
    }

    let alias = &before_dot_trimmed[ident_start..];

    // Must be a valid identifier (not empty, starts with letter or underscore)
    if alias.is_empty() {
        return None;
    }
    let first_char = alias.chars().next()?;
    if !first_char.is_alphabetic() && first_char != '_' {
        return None;
    }

    // Avoid triggering on smelt.source() or smelt.ref() - these have dot but aren't aliases
    // Check if the identifier is "smelt" and followed by source or ref
    if alias.eq_ignore_ascii_case("smelt") {
        let after_dot_lower = after_dot.to_lowercase();
        if after_dot_lower.starts_with("source") || after_dot_lower.starts_with("ref") {
            return None;
        }
    }

    Some(alias.to_string())
}

/// Target of a table alias in FROM clause
#[derive(Debug, Clone)]
enum AliasTarget {
    Source {
        source_name: String,
        table_name: String,
    },
    Model {
        model_name: String,
    },
}

/// Extract alias mappings from a SELECT statement's FROM clause
fn extract_from_aliases(
    select_stmt: &smelt_parser::ast::SelectStmt,
    db: &smelt_db::Database,
) -> std::collections::HashMap<String, AliasTarget> {
    use smelt_parser::ast::{RefCall, SourceCall};

    let mut aliases = std::collections::HashMap::new();

    if let Some(from_clause) = select_stmt.from_clause() {
        // Process main table refs in FROM clause
        for table_ref in from_clause.table_refs() {
            if let Some(func) = table_ref.function_call() {
                // Check for smelt.source()
                if let Some(source_call) = SourceCall::from_function_call(func.clone()) {
                    if let (Some(source_name), Some(table_name)) =
                        (source_call.source_name(), source_call.table_name())
                    {
                        // Use explicit alias if present, otherwise use table name
                        let alias_name = table_ref.alias().unwrap_or_else(|| table_name.clone());
                        aliases.insert(
                            alias_name,
                            AliasTarget::Source {
                                source_name,
                                table_name,
                            },
                        );
                    }
                }
                // Check for smelt.ref()
                else if let Some(ref_call) = RefCall::from_function_call(func) {
                    if let Some(model_name) = ref_call.model_name() {
                        // Use explicit alias if present, otherwise use model name
                        let alias_name = table_ref.alias().unwrap_or_else(|| model_name.clone());
                        aliases.insert(alias_name, AliasTarget::Model { model_name });
                    }
                }
            }
        }

        // Process JOINed table refs
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if let Some(func) = table_ref.function_call() {
                    // Check for smelt.source()
                    if let Some(source_call) = SourceCall::from_function_call(func.clone()) {
                        if let (Some(source_name), Some(table_name)) =
                            (source_call.source_name(), source_call.table_name())
                        {
                            let alias_name =
                                table_ref.alias().unwrap_or_else(|| table_name.clone());
                            aliases.insert(
                                alias_name,
                                AliasTarget::Source {
                                    source_name,
                                    table_name,
                                },
                            );
                        }
                    }
                    // Check for smelt.ref()
                    else if let Some(ref_call) = RefCall::from_function_call(func) {
                        if let Some(model_name) = ref_call.model_name() {
                            let alias_name =
                                table_ref.alias().unwrap_or_else(|| model_name.clone());
                            aliases.insert(alias_name, AliasTarget::Model { model_name });
                        }
                    }
                }
            }
        }
    }

    // Note: db parameter reserved for future use (e.g., resolving model schemas)
    let _ = db;

    aliases
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_clause_context_after_from_keyword() {
        let text = "WITH cte AS (SELECT 1) SELECT * FROM ";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_from_clause_context_partial_identifier() {
        let text = "WITH cte AS (SELECT 1) SELECT * FROM ct";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_from_clause_context_after_join() {
        let text = "SELECT * FROM a JOIN ";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_not_from_context_inside_ref() {
        let text = "SELECT * FROM smelt.ref('";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::InsideRef));
    }

    #[test]
    fn test_not_from_context_inside_source() {
        let text = "SELECT * FROM smelt.source('";
        let ctx = determine_completion_context(text, text.len());
        assert!(matches!(ctx, CompletionContext::InsideSource));
    }

    #[test]
    fn test_not_from_context_after_where() {
        // After WHERE, we're past the FROM clause table position
        let text = "SELECT * FROM t WHERE ";
        let ctx = determine_completion_context(text, text.len());
        assert!(!matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_not_from_context_after_on() {
        let text = "SELECT * FROM a JOIN b ON ";
        let ctx = determine_completion_context(text, text.len());
        assert!(!matches!(ctx, CompletionContext::FromClause));
    }

    #[test]
    fn test_from_position_empty_after_from() {
        assert!(is_in_from_position("SELECT * FROM "));
    }

    #[test]
    fn test_from_position_partial_identifier() {
        assert!(is_in_from_position("SELECT * FROM CT"));
    }

    #[test]
    fn test_from_position_after_join() {
        assert!(is_in_from_position("SELECT * FROM A JOIN "));
    }

    #[test]
    fn test_not_from_position_complete_table_ref() {
        // After a complete table ref with alias, we're past the position
        assert!(!is_in_from_position("SELECT * FROM TABLE_A T"));
    }
}
