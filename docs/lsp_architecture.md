# LSP Architecture with Salsa

## Design Goals

1. **Incremental Everything**: Only recompute what changed when files are edited
2. **Error Resilience**: Provide useful IDE features even with syntax/semantic errors
3. **Low Latency**: Sub-100ms response for common operations (completions, diagnostics)
4. **Scalability**: Handle projects with 1000s of models without degrading

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        Editor (VSCode)                       │
└─────────────────────┬───────────────────────────────────────┘
                      │ LSP Protocol (JSON-RPC)
                      │
┌─────────────────────▼───────────────────────────────────────┐
│                   smelt-lsp (LSP Server)                       │
│  ┌──────────────────────────────────────────────────────┐   │
│  │         tower-lsp (Protocol Implementation)          │   │
│  └────────────────────┬─────────────────────────────────┘   │
│                       │                                      │
│  ┌────────────────────▼─────────────────────────────────┐   │
│  │             Salsa Database (smelt-db)                  │   │
│  │  • Manages query cache                               │   │
│  │  • Tracks dependencies between queries               │   │
│  │  • Invalidates on input changes                      │   │
│  └────────────────────┬─────────────────────────────────┘   │
│                       │                                      │
│  ┌────────────────────▼─────────────────────────────────┐   │
│  │              Query Implementations                   │   │
│  │  • parse_file()     → CST + errors                   │   │
│  │  • file_ast()       → AST from CST                   │   │
│  │  • resolve_refs()   → Name resolution                │   │
│  │  • type_check()     → Type inference                 │   │
│  │  • diagnostics()    → Errors + warnings              │   │
│  │  • optimize()       → Optimized physical plan        │   │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

## Salsa Query Design (salsa 0.26)

All queries are free `#[salsa::tracked]` functions (no query groups or traits).
Inputs are `#[salsa::input]` structs; the `Database` provides a path-keyed
registry for looking up inputs.

### Input Structs (Managed by LSP)

```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)] pub path: PathBuf,
    #[returns(ref)] pub text: String,
    #[returns(ref)] pub project_root: PathBuf,
}

#[salsa::input]
pub struct ProjectInput {
    #[returns(ref)] pub root: PathBuf,
    #[returns(ref)] pub sources_yaml: String,
}

#[salsa::input(singleton)]
pub struct Workspace {
    #[returns(ref)] pub files: Vec<SourceFile>,
    #[returns(ref)] pub projects: Vec<ProjectInput>,
}
```

### Parsing Layer

```rust
#[salsa::tracked(returns(ref))]
pub fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> Parse;

#[salsa::tracked]
pub fn parse_model(db: &dyn salsa::Database, file: SourceFile) -> Option<Arc<Model>>;
```

**Key insight**: `parse_file` always succeeds, producing a CST with error nodes. This enables other queries to work with partial trees.

### Semantic Layer

```rust
#[salsa::tracked]
pub fn model_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<RefLocation>>;

#[salsa::tracked]
pub fn resolve_ref(db: &dyn salsa::Database, ws: Workspace, name: String) -> Option<SourceFile>;

pub fn file_diagnostics(db: &dyn salsa::Database, ws: Workspace, file: SourceFile) -> Vec<Diagnostic>;

#[salsa::tracked(cycle_initial = typed_model_schema_initial)]
pub fn typed_model_schema(db: &dyn salsa::Database, ws: Workspace, file: SourceFile) -> Arc<ModelSchema>;
```

**Key insight**: Cycle handling uses salsa 0.26's `cycle_initial` — circular model
references resolve to empty schemas without panicking (no `catch_unwind` needed).

### Diagnostics via Accumulator

```rust
#[salsa::accumulator]
pub struct DiagnosticAcc(pub Diagnostic);
```

Diagnostics are pushed via `DiagnosticAcc(d).accumulate(db)` inside tracked functions
and collected at the top level via `check_file_diagnostics::accumulated::<DiagnosticAcc>(db, ws, file)`.

## Error Recovery Parser with Rowan

### Tokenization

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    // Tokens
    SELECT_KW,
    FROM_KW,
    WHERE_KW,
    IDENT,
    NUMBER,
    STRING,
    LBRACE,      // {
    RBRACE,      // }
    LPAREN,      // (
    RPAREN,      // )

    // Template syntax
    TEMPLATE_START,  // {{
    TEMPLATE_END,    // }}
    REF_KW,          // ref
    CONFIG_KW,       // config

    // Composite nodes
    MODEL,
    REF_EXPR,
    CONFIG_EXPR,
    SQL_QUERY,

    // Error handling
    ERROR,
    WHITESPACE,
    COMMENT,
}
```

### Parser Structure

```rust
pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    builder: GreenNodeBuilder<'a>,
}

impl<'a> Parser<'a> {
    /// Parse entry point - always succeeds
    pub fn parse(input: &str) -> Parse {
        let tokens = tokenize(input);
        let mut parser = Parser::new(&tokens);
        parser.parse_file();

        Parse {
            green_node: parser.builder.finish(),
            errors: parser.errors,
        }
    }

    /// Parse model file - recovers from errors
    fn parse_file(&mut self) {
        self.builder.start_node(SyntaxKind::FILE);

        while !self.at_end() {
            match self.current() {
                TEMPLATE_START => self.parse_template(),
                _ => self.parse_sql_fragment(),
            }
        }

        self.builder.finish_node();
    }

    /// Parse template expression with error recovery
    fn parse_template(&mut self) {
        self.builder.start_node(SyntaxKind::TEMPLATE_EXPR);

        if !self.expect(TEMPLATE_START) {
            // Error: missing {{, but continue
            self.builder.token(ERROR, "");
        }

        match self.current() {
            REF_KW => self.parse_ref(),
            CONFIG_KW => self.parse_config(),
            _ => {
                // Error: unknown template expr
                self.builder.token(ERROR, self.current_text());
                self.advance();
            }
        }

        if !self.expect(TEMPLATE_END) {
            // Error: missing }}, but we can recover
            self.builder.token(ERROR, "");
        }

        self.builder.finish_node();
    }

    /// Parse ref() with recovery
    fn parse_ref(&mut self) {
        self.builder.start_node(SyntaxKind::REF_EXPR);
        self.expect(REF_KW);
        self.expect(LPAREN);

        if self.at(STRING) {
            self.bump(); // model name
        } else {
            // Error: expected string, insert placeholder
            self.builder.token(ERROR, "");
        }

        if !self.expect(RPAREN) {
            // Missing ), try to recover by finding }}
            while !self.at(TEMPLATE_END) && !self.at_end() {
                self.advance();
            }
        }

        self.builder.finish_node();
    }
}
```

### Error Recovery Strategies

1. **Synchronization Points**: When parser encounters error, skip to known "safe" point
   - After `}}` template end
   - At next `SELECT` keyword
   - At next model boundary

2. **Error Nodes**: Invalid syntax becomes `ERROR` node in tree
   - Preserves all text (for formatting tools)
   - Allows partial semantic analysis
   - Provides good error messages

3. **Best-Effort Parsing**: Try to extract maximum useful information
   - Even with errors, can often get model names, refs, etc.
   - Enables completions and other IDE features

## LSP Feature Implementation

### Diagnostics (Errors + Warnings)

```rust
impl LanguageServer for SqtLsp {
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let path = params.text_document.uri.to_file_path().unwrap();
        let new_text = /* extract from params */;

        // Update Salsa input
        self.db.set_file_text(path.clone(), Arc::new(new_text));

        // Salsa automatically invalidates dependent queries
        let diagnostics = self.db.file_diagnostics(path.clone());

        // Send to client
        self.client.publish_diagnostics(
            params.text_document.uri,
            diagnostics,
            None
        ).await;
    }
}
```

**Key insight**: Salsa handles incremental recomputation. Only changed files and their dependents are re-analyzed.

### Completion

```rust
async fn completion(&self, params: CompletionParams) -> Result<CompletionList> {
    let path = params.text_document_position.text_document.uri.to_file_path()?;
    let pos = params.text_document_position.position;

    // Get AST (cached by Salsa)
    let ast = self.db.file_ast(path.clone());

    // Find node at cursor position
    let node = ast.node_at_position(pos);

    match node.kind() {
        REF_EXPR => {
            // Complete model names
            let all_models = self.db.all_models();
            Ok(CompletionList {
                items: all_models.iter()
                    .map(|name| CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::CLASS),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
        }
        _ => Ok(CompletionList::default())
    }
}
```

### Go To Definition

```rust
async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<GotoDefinitionResponse> {
    let path = params.text_document_position_params.text_document.uri.to_file_path()?;
    let pos = params.text_document_position_params.position;

    let ast = self.db.file_ast(path.clone());
    let node = ast.node_at_position(pos);

    if let Some(ref_expr) = node.as_ref_expr() {
        let model_name = ref_expr.model_name();

        // Salsa query: where is this model defined?
        let ref_map = self.db.resolve_refs(path);

        if let Some(def_location) = ref_map.get(&model_name) {
            return Ok(GotoDefinitionResponse::Scalar(Location {
                uri: Url::from_file_path(def_location.path).unwrap(),
                range: def_location.range,
            }));
        }
    }

    Ok(GotoDefinitionResponse::Array(vec![]))
}
```

## Performance Characteristics

### Cold Start (First Open)
- Parse all files: ~1ms per file (1000 files = ~1s)
- Build dependency graph: ~10ms
- Initial diagnostics: ~50ms per file

### Hot Path (After Edit)
- Update Salsa input: ~1µs
- Reparse changed file: ~1ms
- Recompute diagnostics: ~5-50ms (depends on file size)
- Invalidate dependencies: automatic, ~100µs per dependent

### Memory Usage
- CST per file: ~5-10KB
- AST per file: ~2-5KB
- Salsa cache overhead: ~20% of data size
- Expected: ~50-100MB for 1000 models

## Testing Strategy

1. **Unit Tests**: Each Salsa query tested in isolation
2. **Error Recovery Tests**: Parse invalid code, verify useful tree
3. **Incremental Tests**: Edit file, verify only expected queries rerun
4. **LSP Integration Tests**: Simulate VSCode requests, verify responses
5. **Performance Tests**: Ensure sub-100ms latency for common operations

## Implementation Phases

### Phase 1: Basic LSP
- [ ] Rowan-based lexer/parser for template syntax
- [ ] Salsa database with basic queries
- [ ] LSP server with diagnostics
- [ ] VSCode extension

### Phase 2: Semantic Features
- [ ] Name resolution for `ref()`
- [ ] Go to definition
- [ ] Find references
- [ ] Completions

### Phase 3: Advanced Features
- [ ] Type inference from SQL
- [ ] Schema hovers
- [ ] Rename refactoring
- [ ] Code actions (suggest optimizations)

### Phase 4: Optimization Integration
- [ ] Show optimization decisions in IDE
- [ ] Visualize dependency graph
- [ ] Estimate execution costs
- [ ] Suggest manual optimizations
