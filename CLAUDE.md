# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sqt** - Semantic Query Tool

A next-generation data pipeline tool designed to improve upon dbt by:
- Separating logical specification (what to compute) from physical execution (how to execute)
- Enabling automatic optimization across models
- Supporting multi-backend execution (DuckDB, Databricks, etc.)
- Using a proper language instead of Jinja templates

**Current Phase**: Parser and LSP implementation - Phases 1 & 2 complete (sqt.ref() with named parameters).

**Project Status**: Early-stage development - no backward compatibility constraints. The codebase is evolving rapidly and breaking changes are expected.

## Key Documentation

- **README.md**: Full language specification and design decisions
  - Two-layer DSL architecture (Metrics DSL + SQL models)
  - Type system design
  - Extension syntax (`sqt.ref()`, `sqt.metric()` with `=>` parameters)
  - Computation requirements (stateless, windowed, sessionized, etc.)
  - Backend capabilities and rewrite rules
  - Incrementalization and optimization strategy

- **ROADMAP.md**: Implementation status and next steps
  - Track completed phases with completion dates
  - Document deferred work with rationale
  - Propose concrete next-step options
  - **Update after completing phases or making architectural decisions**

- **docs/**: Architecture and design documentation
  - `architecture_overview.md`: System design and component interactions
  - `lsp_architecture.md`: LSP implementation details
  - `lsp_quickstart.md`: Getting started with the LSP
  - `example1_insights.md`, `example2_insights.md`: Optimization pattern analysis
  - `optimization_rule_api_design.md`: Future optimizer API design

## Commands

### Build and Test
```bash
# Build the entire workspace
cargo build

# Run clippy (linter) - must pass with no warnings
cargo clippy --all-targets

# Run tests
cargo test

# Run examples
cargo run --example example1_naive       # Common intermediate aggregation (naive)
cargo run --example example1_optimized   # Common intermediate aggregation (optimized)
cargo run --example example2_naive       # Split large GROUP BY (naive)
cargo run --example example2_optimized   # Split large GROUP BY (optimized)

# Build with bundled DuckDB (no system dependency required)
cargo build  # bundled is default

# Run the LSP server
cargo run -p sqt-lsp

# Test with sample workspace
# (Configure your editor to use the LSP server, then open test-workspace/)
```

### VSCode Extension
```bash
# Install and build the extension
cd editors/vscode
npm install
npm run compile

# Test in development mode
# Open editors/vscode in VSCode and press F5 to launch Extension Host

# Package as VSIX (requires Node 18+)
npm run package

# Watch mode (auto-recompile on changes)
npm run watch
```

## Architecture

### High-Level Design

sqt is a **compiler and orchestrator**, not a query engine:
```
User DSL → Parser → Logical IR → Optimizer → Physical IR → SQL for Target Engines
```

- **Logical IR**: Represents WHAT to compute (correctness specification)
- **Physical IR**: Represents HOW to execute (engine selection, materialization decisions)
- **Optimizer**: Transforms logical IR into optimized physical IR while preserving correctness

### Parser Architecture

The parser is separated into reusable layers:
```
sqt-parser (pure parser)  →  sqt-db (Salsa queries)  →  sqt-lsp (LSP server)
                          ↘  sqt-optimizer (future)
                          ↘  sqt-cli (future)
```

- **sqt-parser**: Standalone Rowan-based parser (no Salsa dependency)
  - Pure function: text → CST with error recovery
  - Reusable in any context (LSP, optimizer, CLI)
  - Fast one-shot parsing for non-incremental use cases

- **sqt-db**: Salsa wrapper around sqt-parser
  - Incremental compilation for LSP responsiveness
  - Caches parse results and derived queries
  - Automatic invalidation when files change

This separation allows the LSP to get incremental parsing via Salsa, while the optimizer and CLI can use fast one-shot parsing directly from sqt-parser.

### Key Dependencies

- **Salsa**: Incremental computation framework (enables fast recompilation and LSP)
- **Rowan**: Lossless CST library (error-recovery parser foundation)
- **tower-lsp**: Language Server Protocol implementation
- **DataFusion**: SQL parsing, logical plan representation, optimizer framework
- **DuckDB**: Local execution engine for testing (bundled, no system install needed)
- **Arrow**: Data interchange format between components

### Example-Driven Development

The project uses concrete examples to discover the right optimizer API:

1. **Example 1** (`crates/sqt-examples/examples/`):
   - **Naive version** (`example1_naive.rs`): Three models computing sessions independently
   - **Optimized version** (`example1_optimized.rs`): Shared session computation
   - **Goal**: Extract patterns for detecting common intermediate aggregations
   - **Type**: Transparent optimization (preserves exact results)

2. **Example 2** (`crates/sqt-examples/examples/`):
   - **Naive version** (`example2_naive.rs`): Large multi-dimensional GROUP BY with massive shuffle
   - **Optimized version** (`example2_optimized.rs`): Split into independent dimensional queries
   - **Goal**: Demonstrate when optimizations require user consent (lossy transformation)
   - **Type**: Semantic optimization (changes result structure, requires consent)

### Crate Structure

- `sqt-parser`: Rowan-based error-recovery parser (standalone, reusable)
  - Lexer: Tokenizes SQL + sqt extensions (`sqt.ref()`, `sqt.metric()`, `=>` operator)
  - Parser: Recursive descent parser with error recovery at sync points
  - AST: Typed wrappers over Rowan CST for convenient traversal
  - Parses SQL structure: SELECT, FROM, WHERE, GROUP BY, expressions, functions
  - Named parameters: Handles `param => value` syntax in function calls
  - Position tracking: Accurate line/column information for diagnostics and goto-definition
- `sqt-examples`: Concrete optimization examples used to drive API design
  - `src/utils.rs`: Shared utilities for DuckDB execution and pretty printing
  - `examples/`: Runnable examples comparing naive vs optimized approaches
- `sqt-db`: Salsa database with incremental queries (wraps sqt-parser for incremental compilation)
  - Input queries: `file_text()`, `all_files()`
  - Syntax queries: `parse_file()`, `parse_model()`, `model_refs()` (with positions)
  - Semantic queries: `resolve_ref()`, `file_diagnostics()` (with accurate positions)
- `sqt-lsp`: Language Server Protocol implementation
  - Diagnostics for undefined refs and parse errors (with accurate positions)
  - Go-to-definition for `sqt.ref()` using CST position tracking
  - Extracts named parameters from ref calls for future validation
  - Full Salsa integration for incremental updates
- `editors/vscode`: VSCode extension
  - Language client that connects to sqt-lsp
  - Syntax highlighting for SQL + templates
  - Auto-activation when models/ directory detected
  - See editors/vscode/README.md for installation

## Key Differentiators from dbt

1. **Logical/Physical Separation**: Users specify logic, optimizer decides execution strategy
2. **Engineer controls optimizations**: Optimizer is not a black box - the API will allow data engineers to refactor specific logical plans to optimize - the framework should make it easy to do these and know that correctness is preserved - or where not - what has been lost.
3. **Cross-Model Optimization**: Can fuse or split queries across model boundaries
4. **Multi-Backend**: Automatically distribute work across engines (e.g., DuckDB for small data, Databricks for large)
5. **Proper Language**: No Jinja templates, proper compilation with type checking
6. **First-Class Editor Support**: LSP with incremental compilation via Salsa
   - Real-time diagnostics and completions
   - Error-recovery parser handles partial/invalid code
   - Incremental recompilation for fast feedback

## Development Workflow

### For Parser/LSP Features

1. Review the spec in README.md for requirements
2. Implement parser changes (lexer → syntax → parser → AST)
3. Update sqt-db queries if needed (usually automatic via AST)
4. Update LSP features if needed (diagnostics, goto-definition, etc.)
5. Test with test-workspace models
6. **Run `cargo clippy --all-targets` and fix all warnings**
7. Run `cargo build` and `cargo test` to ensure everything compiles and passes
8. Update ROADMAP.md with completion status and date
9. Commit with descriptive message (includes ROADMAP.md update)

### For Optimizer Features (Future)

1. Start with concrete examples showing optimization opportunities
2. Manually write both naive and optimized versions
3. Analyze what the optimizer needs to detect and transform
4. Extract API patterns from successful optimizations
5. Generalize into optimizer framework

## Maintaining ROADMAP.md

**When to update:**
- After completing a phase (mark as ✅ with completion date)
- When deferring work (mark as ⏸️ with rationale)
- When proposing new next steps (add as Option)
- When making architectural decisions (document reasoning)

**Format:**
- Use ✅ for completed phases
- Use ⏸️ for deferred work
- Use 🔄 for in-progress work
- Use 🔮 for future/speculative work
- Always include completion dates for finished work (e.g., "December 26, 2024")
- Always explain why work is deferred

**Note:** Use dates instead of commit hashes to avoid requiring a follow-up commit just to document the hash.

## License

MIT License - Copyright (c) 2025 Andrew Browne
