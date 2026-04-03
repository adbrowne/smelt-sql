# Plan: Goto-Definition Expansion for LSP

**Date**: 2026-04-03
**Research**: `docs/research/2026-04-03-goto-definition-expansion.md`
**Status**: Draft

## Overview

Expand the LSP's goto-definition to cover every navigable symbol: `smelt.source()` calls, CTE references, table aliases, and column references (traced through wildcard chains to explicit definitions). Currently only `smelt.ref()` is supported.

Column goto-def will work everywhere columns appear (SELECT, WHERE, GROUP BY, ORDER BY, JOIN ON, HAVING, QUALIFY).

## Current State

`goto_definition` in `crates/smelt-lsp/src/main.rs:949-1046` only handles `smelt.ref()`:
- Converts cursor position to byte offset
- Walks `file.refs()` to find RefCall at cursor
- Calls `db.resolve_ref(name)` → target file path

Existing infrastructure we'll reuse:
- `db.resolve_source()` (`lib.rs:369-382`) — source name → `SourceTableDef`
- `Cte` AST node (`ast.rs:1869-1920`) — `name()`, `query()` with position info
- `TypeContext::lookup_column()` (`type_inference.rs:145-199`) — resolves columns across CTEs, models, sources
- `ColumnSource::FromModel { model_name, column_name }` (`schema.rs:36-58`) — lineage tracking
- `model_schema()` (`lib.rs:723-848`) — returns `Column` with `range: TextRange`
- `Expr::as_column_ref()` → `ColumnRef { qualifier, name }` (`ast.rs:697-746`)

## Desired End State

From any position in a `.sql` model, the user can goto-definition on:
1. `smelt.ref('model')` → jumps to model file (existing)
2. `smelt.source('raw.users')` → jumps to the table entry in `sources.yml`
3. CTE name in FROM/JOIN → jumps to the CTE's definition in the WITH clause
4. Table alias (as qualifier in `t.column`) → jumps to the TableRef where alias is defined
5. Column reference → jumps to the column's explicit definition (in upstream model SELECT, CTE SELECT, or sources.yml), tracing through wildcard chains

Ambiguous columns (unqualified with multiple possible sources) return multiple locations.

## What We're NOT Doing

- Goto-definition for SQL keywords, function names, or type names
- Cross-file column lineage visualization (just single-hop navigation)
- Renaming/refactoring support
- Goto-definition inside `sources.yml` itself

## Implementation Phases

### Phase 1: Source Goto-Definition

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — extend `goto_definition` to handle SourceCall

**Changes**:
1. After the existing RefCall loop (L1004-1019), add a parallel loop over `file.sources()` checking if cursor is within a SourceCall's range
2. Extract `source_name` and `table_name` from the SourceCall
3. Call `db.resolve_source(project_root, source_name, table_name)` to verify it exists
4. Find the `sources.yml` file path for this project (the project root + `sources.yml`)
5. Read the `sources.yml` text and search for the table name to find its line number (simple text search for the table name pattern, similar to how yaml errors report positions)
6. Return `GotoDefinitionResponse::Scalar(Location { uri, range })` pointing to that line

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] Add integration test: model with `smelt.source('raw.users')` → goto-def resolves to sources.yml path
- [ ] Manual test: open `examples/timeseries/models/event_properties.sql`, goto-def on `smelt.source('raw.events')` → lands in `sources.yml` at the `events:` line

### Phase 2: CTE Goto-Definition

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — extend `goto_definition` to handle CTE references in FROM/JOIN

**Changes**:
1. After source resolution, collect CTE definitions: parse `select_stmt.with_clause()?.ctes()` into a map of `cte_name → TextRange` (the Cte node's range)
2. Walk FROM clause `table_refs()` and JOIN `table_ref()` nodes
3. For each TableRef that is a plain identifier (not a function call or subquery): check if cursor is within its range and its `identifier()` matches a CTE name
4. If match: convert the CTE's TextRange to an LSP Range using the file text, return `GotoDefinitionResponse::Scalar` pointing to same file at the CTE definition
5. Also handle CTE names used as qualifiers in column references (e.g., `cte_name.col`) — when cursor is on the qualifier part, jump to the CTE definition

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] Add integration test: model with `WITH totals AS (...) SELECT * FROM totals` → goto-def on `totals` in FROM jumps to CTE definition
- [ ] Manual test with `examples/retail_analytics/models/marts/mart_cohort_retention.sql` which has multi-CTE chains

### Phase 3: Column Goto-Definition Infrastructure (smelt-db)

**Files to modify**:
- `crates/smelt-db/src/lib.rs` — add new query for column definition lookup
- `crates/smelt-db/src/schema.rs` — possibly extend Column with more precise range info

**Changes**:
1. Add a new pure function `resolve_column_definition()` that takes a column name, optional qualifier, and the current model's context, and returns one or more `ColumnDefinitionLocation`:
   ```rust
   pub struct ColumnDefinitionLocation {
       pub path: PathBuf,           // file containing the definition
       pub range: TextRange,        // position in that file
       pub column_name: String,     // the column name at the definition site
       pub source_kind: String,     // "model", "source", "cte" for display
   }
   ```
2. Resolution logic:
   - **With qualifier**: resolve alias → determine if it's a CTE, model ref, or source
   - **Without qualifier**: search all sources; if multiple matches, return all (ambiguous = multiple locations)
3. For **model columns**: call `db.resolve_ref(model_name)` → `db.model_schema(upstream_path)` → find column by name → return its `range` in the upstream file
   - If the upstream column is a wildcard (`ColumnSource::Wildcard`): follow the chain by getting the upstream's upstream schema, recursively, until finding an explicit column definition
4. For **CTE columns**: find the CTE's SelectStmt, walk its SelectList to find the item producing the column, return its range in the current file
5. For **source columns**: return position in `sources.yml` (reuse the text-search approach from Phase 1)

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] Unit test: single-ref model, column `user_id` resolves to upstream model's SELECT item range
- [ ] Unit test: wildcard chain (A `SELECT *` from B, B `SELECT user_id` from C) → resolves to B's explicit column
- [ ] Unit test: ambiguous column returns multiple locations

### Phase 4: Column Goto-Definition in LSP

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — wire column resolution into `goto_definition`

**Changes**:
1. After CTE resolution (from Phase 2), if nothing matched yet:
2. Find the AST node at cursor position — walk all Expr nodes in the file, check if cursor offset falls within any `Expr` that is a column reference (`as_column_ref()` returns Some)
3. Extract qualifier and column name from the ColumnRef
4. Call the resolution function from Phase 3
5. If single result: return `GotoDefinitionResponse::Scalar`
6. If multiple results: return `GotoDefinitionResponse::Array`
7. Handle the special case where cursor is on a column qualifier (e.g., on `t` in `t.user_id`): this is table-alias goto-def — resolve the alias to its TableRef definition (the ref/source/CTE it refers to)

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] Integration test: goto-def on `user_id` in SELECT resolves to upstream model
- [ ] Integration test: goto-def on `e.event_id` in WHERE resolves to source/upstream
- [ ] Integration test: goto-def on CTE column in ORDER BY resolves to CTE's SELECT item
- [ ] `cargo test -p smelt-cli --no-default-features --features duckdb --test example_diagnostics`

### Phase 5: Integration Tests & Edge Cases

**Files to modify**:
- `crates/smelt-lsp/tests/integration.rs` — comprehensive test suite

**Changes**:
1. Add test cases for each goto-def target:
   - Source call → sources.yml
   - CTE name in FROM → CTE definition
   - Column through single ref → upstream SELECT item
   - Column through wildcard chain → first explicit definition
   - Ambiguous column → multiple locations
   - Qualified column with alias → upstream column
   - Column in WHERE, GROUP BY, ORDER BY, HAVING, JOIN ON
   - Column from source → sources.yml column entry
   - CTE column → CTE SELECT item
2. Edge cases:
   - Nonexistent column → returns None (no crash)
   - Column from subquery alias → subquery SELECT item
   - Self-referencing CTE (recursive) — should not infinite-loop

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`
- [ ] `cargo test -p smelt-cli --no-default-features --features duckdb --test example_diagnostics`
- [ ] All example workspaces still report zero diagnostics

## Testing Strategy

- **Unit tests** in smelt-db for column resolution logic (Phase 3)
- **Integration tests** in smelt-lsp for end-to-end goto-definition (Phase 5)
- **Manual testing** with example workspaces in editor (timeseries has CTEs + sources, retail_analytics has complex CTE chains and joins)
- **Regression**: example_diagnostics test ensures no new diagnostics introduced

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Wildcard chain infinite loop (circular refs) | `resolve_ref` uses Salsa cycle detection; add a depth limit (e.g., 10) as defense-in-depth |
| sources.yml line search finds wrong match | Search for `name:` pattern indented under the correct source, not just bare table name |
| Performance with deep wildcard chains | Salsa caches `model_schema` results, so repeated lookups are O(1) after first computation |
| Column at cursor ambiguity (cursor between two exprs) | Use tightest-enclosing Expr node — same approach as hover |
