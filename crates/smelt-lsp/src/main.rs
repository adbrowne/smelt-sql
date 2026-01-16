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
use smelt_parser::ast::File as AstFile;
use smelt_types::TypedColumn;

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

        // Initialize all_files and sources_yaml to empty first - ensures Salsa queries are always set
        // even if workspace folders aren't provided or models/ doesn't exist
        {
            let mut db = self.db.lock().await;
            db.set_all_files(Arc::new(Vec::new()));
            db.set_sources_yaml(Arc::new(String::new()));
        }

        // Get workspace folders if provided
        if let Some(workspace_folders) = params.workspace_folders {
            let mut db = self.db.lock().await;

            // Scan for .sql files in models/ directory at workspace root
            for folder in workspace_folders {
                let path = match folder.uri.to_file_path() {
                    Ok(p) => p,
                    Err(_) => {
                        init_errors.workspace_errors.push(format!(
                            "Cannot process workspace folder URI: {}",
                            folder.uri
                        ));
                        continue;
                    }
                };

                // Load sources.yml from workspace root (same location as smelt.yml)
                let sources_path = path.join("sources.yml");
                match std::fs::read_to_string(&sources_path) {
                    Ok(sources_content) => {
                        db.set_sources_yaml(Arc::new(sources_content));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Not an error - sources.yml is optional
                    }
                    Err(e) => {
                        init_errors
                            .source_errors
                            .push(format!("Failed to read sources.yml: {}", e));
                    }
                }

                // Scan models/ directory
                let models_path = path.join("models");
                match std::fs::read_dir(&models_path) {
                    Ok(entries) => {
                        let mut files = Vec::new();

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
                                                files.push(entry_path);
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
                                        "Failed to read directory entry in models/: {}",
                                        e
                                    ));
                                }
                            }
                        }

                        db.set_all_files(Arc::new(files));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Not an error - models/ directory is optional
                    }
                    Err(e) => {
                        init_errors
                            .workspace_errors
                            .push(format!("Failed to read models/ directory: {}", e));
                    }
                }
            }
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

        // Check if this is sources.yml - update sources config and refresh all diagnostics
        if path.file_name().is_some_and(|n| n == "sources.yml") {
            let mut db = self.db.lock().await;
            db.set_sources_yaml(Arc::new(params.text_document.text));
            drop(db);
            // Refresh diagnostics on all model files since source resolution may have changed
            self.publish_all_diagnostics().await;
        } else {
            // Update file content in database
            let mut db = self.db.lock().await;
            db.set_file_text(path, Arc::new(params.text_document.text));
            drop(db);
            // Publish diagnostics
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
            // Check if this is sources.yml - update sources config and refresh all diagnostics
            if path.file_name().is_some_and(|n| n == "sources.yml") {
                let mut db = self.db.lock().await;
                db.set_sources_yaml(Arc::new(change.text));
                drop(db);
                // Refresh diagnostics on all model files since source resolution may have changed
                self.publish_all_diagnostics().await;
            } else {
                // Update in database - Salsa will handle incremental recomputation
                let mut db = self.db.lock().await;
                db.set_file_text(path, Arc::new(change.text));
                drop(db);
                // Publish diagnostics
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
                        // Resolve upstream model and show its typed schema
                        if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
                            // Use typed_model_schema to get type information
                            let schema = db.typed_model_schema(upstream_path);

                            // Format schema as markdown
                            let mut content = format!("**Model: {}**\n\n", model_name);
                            content.push_str("| Column | Type | Source |\n");
                            content.push_str("|--------|------|--------|\n");

                            for col in schema.columns.iter() {
                                // Skip wildcards
                                if col.name == "*" {
                                    continue;
                                }

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
                        if let Some(table_def) =
                            db.resolve_source(source_name.clone(), table_name.clone())
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
                let config = db.sources_config();
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
                                    let config = db.sources_config();
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

    CompletionContext::None
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
