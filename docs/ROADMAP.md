# smelt Development Roadmap

This document tracks the implementation status of the smelt parser and LSP, aligned with the spec in [README.md](../README.md).

## Current Status

**Phases 1, 2, & 3 Complete**: Parser, LSP, and CLI with DuckDB execution fully implemented.

```sql
-- ✅ Supported syntax (parser & LSP)
SELECT * FROM smelt.ref('model_name')
SELECT * FROM smelt.ref('events', filter => date > '2024-01-01')
SELECT * FROM smelt.ref('orders', filter => status = 'active', limit => 100)
```

```bash
# ✅ Supported CLI commands
smelt run                           # Execute all models
smelt run --show-results            # Preview query results
smelt run --verbose                 # Show compiled SQL
smelt run --dry-run                 # Validate without executing
```

---

## ✅ Phase 1: Basic smelt.ref() Support (COMPLETED)

**Completed**: December 20, 2025

### What Was Implemented

- Parser recognizes `smelt.ref('model')` pattern in FROM and expressions
- AST extracts namespace and function name separately
- RefCall strictly validates namespace is "smelt"
- smelt-db and LSP work automatically through AST
- Test workspace updated to `smelt.ref()` syntax

### Key Changes

- **Parser** (`crates/smelt-parser/src/parser.rs`): Handle `IDENT.IDENT()` pattern
- **AST** (`crates/smelt-parser/src/ast.rs`): Add `FunctionCall::namespace()` method
- **RefCall**: Validate `sqt.` namespace prefix
- **Test workspace**: All models use new syntax

---

## ✅ Phase 2: Named Parameters Support (COMPLETED)

**Completed**: December 20, 2025

### What Was Implemented

- ARROW (`=>`) token in lexer
- NAMED_PARAM node type in parser
- `parse_argument()` handles `name => expr` pattern
- NamedParam AST type with `name()` and `value_text()` methods
- `FunctionCall::named_params()` and `RefCall::named_params()` iterators
- Test workspace demonstrates usage

### Key Changes

- **Lexer** (`crates/smelt-parser/src/lexer.rs`): Add `=>` token
- **SyntaxKind** (`crates/smelt-parser/src/syntax_kind.rs`): Add ARROW and NAMED_PARAM
- **Parser** (`crates/smelt-parser/src/parser.rs`): Parse named arguments
- **AST** (`crates/smelt-parser/src/ast.rs`): Add NamedParam type
- **Test workspace**: Example with `filter =>` parameter

---

## ✅ Phase 3: CLI and DuckDB Execution (COMPLETED)

**Completed**: December 26, 2025

### What Was Implemented

- New `smelt-cli` crate with `sqt run` command
- DuckDB-based model execution with file-based database persistence
- YAML configuration (`smelt.yml` and `sources.yml`)
- Model discovery from `models/` directory
- Dependency graph construction with topological sort
- SQL compilation (replacing `smelt.ref()` with table references)
- Table and view materialization strategies
- Source table validation
- Named parameter detection with clear error messages
- Test workspace configuration for end-to-end testing

### Key Features

- **CLI**: Full-featured command-line interface using `clap`
  - `sqt run` - Execute models and materialize in DuckDB
  - `--project-dir` - Specify project root
  - `--database` - Custom database file path
  - `--show-results` - Preview query results
  - `--verbose` - Show compiled SQL
  - `--dry-run` - Validate without executing

- **Configuration**: YAML-based project configuration
  - `smelt.yml` - Project settings, targets, model materialization
  - `sources.yml` - External source table definitions

- **Execution Engine**:
  - Dependency resolution with cycle detection
  - Topological sort for correct execution order
  - Both CREATE TABLE AS and CREATE VIEW support
  - Row counts and timing statistics
  - Pretty-printed result preview using Arrow

- **Error Handling**:
  - Clear error messages with file/line/column positions
  - Named parameter detection with helpful error messages
  - Undefined reference validation
  - Circular dependency detection
  - Source table validation

### Implementation Details

**New Files**:
- `crates/smelt-cli/Cargo.toml` - CLI crate definition
- `crates/smelt-cli/src/main.rs` - Entry point and orchestration
- `crates/smelt-cli/src/lib.rs` - Public API
- `crates/smelt-cli/src/config.rs` - YAML configuration parsing
- `crates/smelt-cli/src/discovery.rs` - Model file discovery
- `crates/smelt-cli/src/graph.rs` - Dependency graph and topological sort
- `crates/smelt-cli/src/compiler.rs` - SQL compilation (ref replacement)
- `crates/smelt-cli/src/executor.rs` - DuckDB execution engine
- `crates/smelt-cli/src/errors.rs` - Custom error types

**Test Configuration**:
- `test-workspace/smelt.yml` - Example project configuration
- `test-workspace/sources.yml` - Source table definitions
- `test-workspace/setup_sources.sql` - Sample data generation

### Limitations (By Design)

- Named parameters in `smelt.ref()` are detected but not yet compiled
  - Gives clear error: "named parameters which are not yet supported"
  - Can be implemented in future phase
- JOIN syntax not yet supported in parser
  - Parser currently only handles comma-separated table references
  - Use `FROM table1, table2 WHERE ...` instead of `FROM table1 JOIN table2 ON ...`
  - Can be implemented in future phase
- No incremental materialization (always full refresh)
- Single-threaded execution (no parallel model execution)
- DuckDB only (multi-backend support deferred)

### Test Results

Successfully executed test-workspace models:
- `raw_events` - 100 rows (table)
- `user_sessions` - 33 rows (view)
- `user_stats` - 10 rows (table)

All executed in ~8ms with correct dependency resolution.

---

## ⏸️ Phase 4: smelt.metric() Support (DEFERRED)

**Status**: Deferred until metrics DSL design is finalized

### Why Deferred

Requires architectural decisions about:
- Metric definition format (YAML? SQL? Other?)
- Metric storage and resolution strategy
- Parameter validation semantics
- Temporal computation model (trailing windows, decomposability)

### When Ready

Can follow similar pattern to RefCall implementation:
1. Add `MetricCall` AST type
2. Add `File::metrics()` iterator
3. Add `metric_refs()` query to smelt-db
4. Add LSP diagnostics for undefined metrics

---

## Next Steps (Choose One)

### Option A: Named Parameter Compilation ⭐ Recommended

**Value**: Make named parameters functional in CLI execution

**Work**:
1. Implement `filter =>` parameter compilation in `smelt-cli/src/compiler.rs`:
   - Parse filter expression from named parameter
   - Inject as WHERE clause in compiled SQL
   - Combine with existing WHERE clauses using AND
2. Add tests for filter parameter compilation
3. Update test-workspace to use parameterized refs
4. Add LSP validation for parameter types

**Effort**: Medium (requires SQL AST manipulation)

**Files**: `crates/smelt-cli/src/compiler.rs`, `crates/smelt-lsp/src/main.rs`

**Value**: Unlocks the full power of `smelt.ref()` parameters for filtering, partitioning, etc.

---

### Option B: Incremental Materialization

**Value**: Faster execution by only recomputing changed data

**Work**:
1. Track model state (last run timestamp, row counts)
2. Detect incremental-safe models (append-only, time-based filters)
3. Generate incremental SQL (INSERT INTO ... WHERE timestamp > last_run)
4. Handle failures and full refresh scenarios

**Effort**: Medium-High (requires state management)

**Files**: `crates/smelt-cli/src/executor.rs`, new state tracking module

---

### Option C: Column Schema Tracking

**Value**: Enable smarter LSP features (autocomplete, validation)

**Work**:
1. Track column schemas in smelt-db
2. Infer output columns from SELECT statements
3. Validate column references in expressions
4. Add LSP autocomplete for column names

**Effort**: Medium (requires SQL analysis logic)

**Files**: `crates/smelt-db/src/lib.rs`, `crates/smelt-lsp/src/main.rs`

---

### Option C: JOIN Syntax Support

**Value**: Enable standard SQL JOIN syntax for improved readability

**Work**:
1. Add JOIN keywords to lexer (JOIN, LEFT, RIGHT, INNER, OUTER, CROSS, ON)
2. Update `parse_from_clause()` to handle JOIN clauses
3. Add JOIN_CLAUSE node type and AST support
4. Parse ON conditions (join predicates)
5. Update tests to use JOIN syntax where appropriate

**Effort**: Medium (requires parser changes and new node types)

**Files**: `crates/smelt-parser/src/lexer.rs`, `crates/smelt-parser/src/parser.rs`, `crates/smelt-parser/src/syntax_kind.rs`, `crates/smelt-parser/src/ast.rs`

**Note**: Currently the parser only supports comma-separated table references (e.g., `FROM table1, table2 WHERE ...`). Adding JOIN support would allow standard syntax like `FROM table1 JOIN table2 ON ...`.

---

### Option D: Testing Framework

**Value**: Confidence in parser/AST correctness

**Work**:
1. Add parser tests for various SQL patterns
2. Add AST tests for ref/parameter extraction
3. Add integration tests for LSP features
4. Test error recovery scenarios

**Effort**: Low-Medium (infrastructure setup)

**Files**: New test files in `crates/smelt-parser/tests/`, `crates/smelt-lsp/tests/`

---

### Option E: VSCode Extension Improvements

**Value**: Better developer experience

**Work**:
1. Add syntax highlighting for `smelt.ref()` and parameters
2. Add snippets for common patterns
3. Improve error message formatting
4. Add "Find All References" support

**Effort**: Low (VSCode extension work)

**Files**: `editors/vscode/syntaxes/`, `editors/vscode/package.json`

---

### Option F: Documentation & Examples

**Value**: Help future users/contributors

**Work**:
1. Document parser architecture
2. Document AST traversal patterns
3. Add more test workspace examples
4. Write getting-started guide

**Effort**: Low (documentation)

**Files**: New docs in `docs/`, expanded examples in `test-workspace/`

---

## Future Work (Not Prioritized)

These features require significant architectural work and are deferred:

### Metrics DSL (Spec lines 132-153)
- YAML/declarative metric definitions
- Metric registry and resolution
- Temporal semantics (trailing windows, decomposability)
- Parameter validation

### Type System (Spec lines 183-230)
- Strict type checking
- NULL tracking in types
- LSP quick-fixes for type errors
- Inference within models, explicit at boundaries

### Configuration Annotations (Spec lines 437-464)
- Parse `@materialize`, `@partition_by` annotations
- Store config metadata in AST/database
- Validate configuration options

### Multi-Backend Support (Spec lines 256-272)
- Backend capability declarations
- Computation requirement tracking
- Backend-specific rewrites

### Rewrite Rules (Spec lines 284-346)
- Rule framework (possibly using Egg or similar)
- Engine-specific translations
- Cost-based optimization

### Incrementalization (Spec Phase 5)
- Batch safety proofs
- Incremental materialization
- State management

### Learning/Optimization (Spec Phase 6)
- Historical run data
- Optimization suggestions
- Cost modeling

---

## Contributing

When working on the next phase:

1. **Before starting**: Review the spec in [README.md](../README.md) for requirements
2. **During development**: Update this roadmap with progress
3. **After completion**: Mark phase as complete with commit hash
4. **Add tests**: Ensure new features have test coverage
5. **Update docs**: Keep CLAUDE.md and comments up to date

See [CLAUDE.md](../CLAUDE.md) for development workflow and architecture notes.
