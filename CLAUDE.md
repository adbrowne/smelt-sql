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

# Run a specific example
cargo run --example example1_naive
cargo run --example example1_optimized

# Build with bundled DuckDB (no system dependency required)
cargo build  # bundled is default
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

- **DataFusion**: SQL parsing, logical plan representation, optimizer framework
- **DuckDB**: Local execution engine for testing (bundled, no system install needed)
- **Arrow**: Data interchange format between components

### Example-Driven Development

The project uses concrete examples to discover the right optimizer API:

1. **Example 1** (`crates/sqt-examples/examples/`):
   - **Naive version** (`example1_naive.rs`): Three models computing sessions independently
   - **Optimized version** (`example1_optimized.rs`): Shared session computation
   - **Goal**: Extract patterns for detecting common intermediate aggregations

2. **Example 2** (planned): Split large GROUP BY to reduce shuffle overhead

### Crate Structure

- `sqt-examples`: Concrete optimization examples used to drive API design
  - `src/utils.rs`: Shared utilities for DuckDB execution and pretty printing
  - `examples/`: Runnable examples comparing naive vs optimized approaches

## Key Differentiators from dbt

1. **Logical/Physical Separation**: Users specify logic, optimizer decides execution strategy
2. **Cross-Model Optimization**: Can fuse or split queries across model boundaries
3. **Multi-Backend**: Automatically distribute work across engines (e.g., DuckDB for small data, Databricks for large)
4. **Proper Language**: No Jinja templates, proper compilation with type checking

## Development Workflow

1. Start with concrete examples showing optimization opportunities
2. Manually write both naive and optimized versions
3. Analyze what the optimizer needs to detect and transform
4. Extract API patterns from successful optimizations
5. Generalize into optimizer framework

## License

MIT License - Copyright (c) 2025 Andrew Browne
