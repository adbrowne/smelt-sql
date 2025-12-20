# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sqt** - Semantic Query Tool

A next-generation data pipeline tool designed to improve upon dbt by:
- Separating logical specification (what to compute) from physical execution (how to execute)
- Enabling automatic optimization across models
- Supporting multi-backend execution (DuckDB, Databricks, etc.)
- Using a proper language instead of Jinja templates

**Current Phase**: Example-driven API design - building concrete optimization examples first to inform the optimizer API.

## Commands

### Build and Test
```bash
# Build the entire workspace
cargo build

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

## Architecture

### High-Level Design

sqt is a **compiler and orchestrator**, not a query engine:
```
User DSL → Parser → Logical IR → Optimizer → Physical IR → SQL for Target Engines
```

- **Logical IR**: Represents WHAT to compute (correctness specification)
- **Physical IR**: Represents HOW to execute (engine selection, materialization decisions)
- **Optimizer**: Transforms logical IR into optimized physical IR while preserving correctness

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

- `sqt-examples`: Concrete optimization examples used to drive API design
  - `src/utils.rs`: Shared utilities for DuckDB execution and pretty printing
  - `examples/`: Runnable examples comparing naive vs optimized approaches
- `sqt-db`: Salsa database with incremental queries
  - Input queries: `file_text()`, `all_files()`
  - Syntax queries: `parse_model()`, `model_refs()`
  - Semantic queries: `resolve_ref()`, `file_diagnostics()`
- `sqt-lsp`: Language Server Protocol implementation
  - Diagnostics for undefined refs
  - Go-to-definition for `{{ ref() }}`
  - Full Salsa integration for incremental updates

## Key Differentiators from dbt

1. **Logical/Physical Separation**: Users specify logic, optimizer decides execution strategy
2. **Cross-Model Optimization**: Can fuse or split queries across model boundaries
3. **Multi-Backend**: Automatically distribute work across engines (e.g., DuckDB for small data, Databricks for large)
4. **Proper Language**: No Jinja templates, proper compilation with type checking
5. **First-Class Editor Support**: LSP with incremental compilation via Salsa
   - Real-time diagnostics and completions
   - Error-recovery parser handles partial/invalid code
   - Incremental recompilation for fast feedback

## Development Workflow

1. Start with concrete examples showing optimization opportunities
2. Manually write both naive and optimized versions
3. Analyze what the optimizer needs to detect and transform
4. Extract API patterns from successful optimizations
5. Generalize into optimizer framework

## License

MIT License - Copyright (c) 2025 Andrew Browne
