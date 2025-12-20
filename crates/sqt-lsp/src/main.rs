/// sqt Language Server
///
/// Provides LSP features for sqt model files:
/// - Diagnostics (errors, warnings)
/// - Go-to-definition for ref() calls
/// - Completions (future)
/// - Hover information (future)

use std::path::PathBuf;
use std::sync::Arc;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tokio::sync::Mutex;

use sqt_db::{Database, Diagnostic as DbDiagnostic, DiagnosticSeverity as DbSeverity, Inputs, Schema, Semantic, Syntax};
use sqt_parser::ast::File as AstFile;

struct Backend {
    client: Client,
    db: Arc<Mutex<Database>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            db: Arc::new(Mutex::new(Database::default())),
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
            source: Some("sqt".to_string()),
            ..Default::default()
        }
    }

    /// Publish diagnostics for a file
    async fn publish_diagnostics(&self, uri: Url) {
        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
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
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Get workspace folders if provided
        if let Some(workspace_folders) = params.workspace_folders {
            let mut db = self.db.lock().await;

            // Scan for all .sql files in workspace
            for folder in workspace_folders {
                if let Ok(path) = folder.uri.to_file_path() {
                    if let Ok(entries) = std::fs::read_dir(path.join("models")) {
                        let mut files = Vec::new();

                        for entry in entries.flatten() {
                            let entry_path = entry.path();
                            if entry_path.extension().and_then(|s| s.to_str()) == Some("sql") {
                                // Read file content and set it in the database
                                if let Ok(content) = std::fs::read_to_string(&entry_path) {
                                    db.set_file_text(entry_path.clone(), Arc::new(content));
                                    files.push(entry_path);
                                }
                            }
                        }

                        db.set_all_files(Arc::new(files));
                    }
                }
            }
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["'".to_string(), "(".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "sqt language server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
        };

        // Update file content in database
        let mut db = self.db.lock().await;
        db.set_file_text(path, Arc::new(params.text_document.text));
        drop(db);

        // Publish diagnostics
        self.publish_diagnostics(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
        };

        // Get new text (we use FULL sync, so there's only one change)
        if let Some(change) = params.content_changes.into_iter().next() {
            // Update in database - Salsa will handle incremental recomputation
            let mut db = self.db.lock().await;
            db.set_file_text(path, Arc::new(change.text));
            drop(db);

            // Publish diagnostics
            self.publish_diagnostics(uri).await;
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return Ok(None),
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

        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return Ok(None),
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

        // Check if hovering over a ref() call
        if let Some(file) = AstFile::cast(syntax) {
            for ref_call in file.refs() {
                let range = ref_call.range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();

                // Check if cursor is within this ref call
                if cursor_offset >= start && cursor_offset <= end {
                    if let Some(model_name) = ref_call.model_name() {
                        // Resolve upstream model and show its schema
                        if let Some(upstream_path) = db.resolve_ref(model_name.clone()) {
                            let schema = db.model_schema(upstream_path);

                            // Format schema as markdown
                            let mut content = format!("**Model: {}**\n\n", model_name);
                            content.push_str("Columns:\n");

                            for col in schema.columns.iter() {
                                // Skip wildcards
                                if col.name == "*" {
                                    continue;
                                }

                                content.push_str(&format!("- `{}`", col.name));

                                // Show source if available
                                match &col.source {
                                    sqt_db::ColumnSource::FromModel {
                                        model_name,
                                        column_name,
                                    } => {
                                        content.push_str(&format!(
                                            " (from `{}`.`{}`)",
                                            model_name, column_name
                                        ));
                                    }
                                    sqt_db::ColumnSource::Computed => {
                                        if !col.expression.is_empty() && col.expression != col.name {
                                            content.push_str(&format!(" = `{}`", col.expression));
                                        }
                                    }
                                    _ => {}
                                }

                                content.push('\n');
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
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return Ok(None),
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
                    .iter()
                    .map(|(_, model)| CompletionItem {
                        label: model.name.clone(),
                        kind: Some(CompletionItemKind::MODULE),
                        detail: Some(format!("Model: {}", model.name)),
                        ..Default::default()
                    })
                    .collect()
            }
            CompletionContext::ColumnName => {
                // Complete column names from available columns
                let available = db.available_columns(path);
                available
                    .iter()
                    .filter(|col| col.name != "*")
                    .map(|col| {
                        let mut detail = col.expression.clone();
                        if let Some(alias) = &col.alias {
                            detail = format!("{} AS {}", detail, alias);
                        }

                        CompletionItem {
                            label: col.name.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(detail),
                            documentation: match &col.source {
                                sqt_db::ColumnSource::FromModel {
                                    model_name,
                                    column_name,
                                } => Some(Documentation::String(format!(
                                    "From model '{}', column '{}'",
                                    model_name, column_name
                                ))),
                                sqt_db::ColumnSource::Computed => {
                                    Some(Documentation::String("Computed column".to_string()))
                                }
                                _ => None,
                            },
                            ..Default::default()
                        }
                    })
                    .collect()
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
    InsideRef,   // Cursor inside ref('|')
    ColumnName,  // Cursor in a position where column name is expected
    None,
}

/// Determine what kind of completion to provide based on cursor position
fn determine_completion_context(text: &str, offset: usize) -> CompletionContext {
    // Look backward from cursor to determine context
    let before_cursor = &text[..offset.min(text.len())];

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

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
