# Plan: Fix Goto-Definition for Column References in SELECT Lists

**Date**: 2026-04-03
**Research**: `docs/research/2026-04-03-source-column-goto-def.md`
**Status**: Validated

## Overview

Fix the goto-definition bug where bare and qualified column names in SELECT lists don't navigate to their definitions. The root cause is a strict `<` comparison in the expression-finding loop that prevents deeper AST nodes from replacing same-sized parent nodes. Also add comprehensive tests covering all three open questions from research.

## Current State

The expression-finding loop at `crates/smelt-lsp/src/main.rs:~1720-1738` uses `len < best_len` (strict less-than). When `SELECT_ITEM` and its nested `EXPRESSION` nodes share the exact same text range (common for bare identifiers with no alias), `SELECT_ITEM` is visited first in pre-order and never replaced. `ColumnRef::from_expr` then fails because IDENT tokens aren't direct children of `SELECT_ITEM`.

## Desired End State

- Goto-definition works for bare column names (e.g., `event_timestamp`) in SELECT lists
- Goto-definition works for qualified column names (e.g., `e.event_timestamp`) in SELECT lists
- Goto-definition works for columns from `smelt.ref()` models
- Columns in WHERE/GROUP BY/HAVING clauses continue to work (confirmed by tests)
- Comprehensive integration tests cover all these scenarios

## What We're NOT Doing

- Tightening `Expr::cast` fallback branch (defense-in-depth, but separate concern)
- Cross-file column lineage
- Goto-definition inside `sources.yml` itself

## Implementation Phases

### Phase 1: Fix the Expression-Finding Loop

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — change `<` to `<=` in the best-expression comparison

**Changes**:
1. In the expression-finding loop (~line 1735), change `len < best_len` to `len <= best_len`. This ensures that deeper nodes (visited later in pre-order) with the same range replace shallower parent nodes like `SELECT_ITEM`.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (no warnings)
- [x] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (all pass)

### Phase 2: Add Integration Tests for Column Goto-Definition

**Files to modify**:
- `crates/smelt-lsp/tests/integration.rs` — add tests in the `goto_definition_extended` module

**Changes**:
1. Add `test_goto_def_bare_column_from_source` — bare column `event_timestamp` in SELECT from `smelt.source()`, verify goto-def resolves to `sources.yml`
2. Add `test_goto_def_qualified_column_from_source` — qualified `e.event_timestamp` in SELECT, verify goto-def resolves to `sources.yml`
3. Add `test_goto_def_column_from_ref_model` — column in SELECT from `smelt.ref()`, verify goto-def resolves to upstream model
4. Add `test_goto_def_column_in_where_clause` — column in WHERE clause, verify goto-def works (confirms open question 3)

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (no warnings)
- [x] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (all pass)
- [x] All new tests pass and exercise the fixed code path

## Testing Strategy

The new integration tests exercise the full goto-definition handler flow (LSP request → expression finding → column resolution → location result), not just the database layer. Each test constructs a workspace, positions a cursor on a column name, and verifies the returned location.

## Risks & Mitigations

- **Risk**: Changing `<` to `<=` could affect other goto-definition targets that rely on the current behavior.
  **Mitigation**: All existing tests must continue passing. The change only affects same-range nodes, which are specifically the problematic case.
- **Risk**: `SELECT_ITEM` with alias has different range than inner EXPRESSION, so the fix wouldn't affect aliased columns.
  **Mitigation**: Aliased items already have different ranges, so the current `<` works fine for them. The `<=` change is strictly beneficial.
