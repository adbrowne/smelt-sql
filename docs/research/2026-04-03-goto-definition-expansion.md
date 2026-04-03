# Research: Goto-Definition Expansion for LSP

**Date**: 2026-04-03
**Topic**: Understanding current goto-definition, and what's needed to support columns, CTEs, sources, and table aliases
**Branch**: main
**Commit**: 1fbce86

## Summary

The LSP currently supports goto-definition only for `smelt.ref()` calls (jumping to the referenced model file). The codebase already has rich infrastructure for column lineage (`ColumnSource`), CTE tracking (`WithClause`/`Cte` AST nodes), and source resolution (`resolve_source`). Extending goto-definition to columns, CTEs, sources, and table aliases is feasible by reusing existing AST traversal patterns from hover/completion.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-lsp/src/main.rs` | LSP server, current goto_definition | L949-1046 |
| `crates/smelt-lsp/src/main.rs` | Hover (reusable patterns) | L1048-1277 |
| `crates/smelt-parser/src/ast.rs` | AST node types (RefCall, SourceCall, Cte, ColumnRef, TableRef) | Full file |
| `crates/smelt-db/src/lib.rs` | Salsa queries (resolve_ref, resolve_source, model_schema, type_context) | L359-1119 |
| `crates/smelt-db/src/schema.rs` | Column, ColumnSource, ModelSchema, ResolvedSchema types | L1-273 |
| `crates/smelt-db/src/type_inference.rs` | TypeContext, column walkers | L14-199, L1139-1334 |
| `crates/smelt-core/src/sources.rs` | SourceTableDef, SourceColumnDef types | L151-196 |
| `crates/smelt-lsp/tests/integration.rs` | Existing goto_definition tests | L206-248 |

## Current Goto-Definition Behavior

**What works** (`main.rs:949-1046`):
- `smelt.ref('model_name')` → jumps to the model's `.sql` file (or `.py` source for multi-model files)
- Uses cursor-offset to find which `RefCall` the cursor is on
- Calls `db.resolve_ref(name)` → `Option<PathBuf>`

**What doesn't work**:
- `smelt.source('schema.table')` — no goto-definition despite `resolve_source` existing
- Column references (e.g., `user_id`, `t.user_id`) — no navigation
- CTE names in FROM clause — no jump to CTE definition
- Table aliases — no jump to alias definition

## AST Nodes Relevant to Goto-Definition

### Already Used
- **RefCall** (`ast.rs:1123-1182`): `model_name()`, `range()`, `name_range()` — fully utilized

### Available but Unused for Goto-Def
- **SourceCall** (`ast.rs:1186-1244`): `source_name()`, `table_name()`, `range()`, `name_range()` — has everything needed
- **Cte** (`ast.rs:1869-1920`): `name()`, `query()`, `column_names()` — CTE definition nodes
- **WithClause** (`ast.rs:1842-1865`): `ctes()` iterator — container for all CTEs
- **ColumnRef** (`ast.rs:697-746`): `qualifier()`, `name()`, `from_expr()` — column reference with optional table qualifier
- **TableRef** (`ast.rs:385-507`): `alias()`, `identifier()`, `function_call()` — table references in FROM/JOIN
- **Expr** (`ast.rs:541-695`): `as_column_ref()`, `text_range()` — expression wrapper with column extraction

## Goto-Definition Targets Analysis

### 1. Sources (`smelt.source()`)
**Feasibility**: Straightforward — mirrors ref() pattern exactly.

- AST: `file.sources()` iterates all SourceCall nodes
- Resolution: `db.resolve_source(project_root, source_name, table_name)` already exists (`lib.rs:369-382`)
- Target: Jump to `sources.yml` in the project root
- Challenge: Need to find the YAML line for the specific source/table. Currently `sources.yml` is parsed as a blob without position tracking. Would need either YAML position tracking or a text search for the table name.

### 2. CTE References (table name in FROM referencing a CTE)
**Feasibility**: Medium — need to match table identifiers against CTE names.

- AST: `select_stmt.with_clause()?.ctes()` gives CTE definitions with `name()` and position
- Detection: When cursor is on a `TableRef` identifier in FROM/JOIN, check if it matches a CTE name
- Target: Jump to the CTE definition (`Cte` node's text range)
- Pattern: Similar to how `type_context` (`lib.rs:950-984`) already resolves CTE names

### 3. Column References → Upstream Model Columns
**Feasibility**: Complex — requires column lineage resolution.

- AST: `Expr::as_column_ref()` gives `ColumnRef { qualifier, name }`
- Resolution path:
  1. Determine which table the column comes from (via qualifier or single-source inference)
  2. If from a `smelt.ref()`: resolve ref → get upstream model path → find column in upstream SELECT list
  3. If from a `smelt.source()`: resolve source → find column in `sources.yml`
  4. If from a CTE: find CTE definition → find column in CTE's SELECT list
- Existing infrastructure:
  - `TypeContext::lookup_column(qualifier, name)` (`type_inference.rs:145-199`) resolves columns across sources
  - `model_schema()` (`lib.rs:723-848`) tracks `ColumnSource::FromModel { model_name, column_name }`
  - `resolved_model_schema()` expands wildcards
- Challenge: Need to map column name back to a specific TextRange in the target file. Current `Column.range` tracks where the column is *output* in the SELECT list, but for goto-def we need the range in the *upstream* model.

### 4. Column References → CTE Columns
**Feasibility**: Medium — CTE columns are already tracked.

- `type_context` (`lib.rs:950-984`) uses `infer_cte_columns()` to get CTE column names/types
- Target: Jump to the corresponding SELECT item in the CTE's query
- Challenge: Same as above — need TextRange of the column definition in the CTE

### 5. Table Aliases → Table Definition
**Feasibility**: Easy for FROM clause aliases.

- When cursor is on `t` in `SELECT t.id FROM smelt.ref('users') AS t`, jump to the `smelt.ref('users')` call
- AST: Walk FROM/JOIN `TableRef` nodes, match cursor against alias position
- Target: The `TableRef` node itself (or the ref/source within it)

## Existing Patterns to Reuse

### Cursor → AST Node Resolution
The goto_definition method (`main.rs:984-1002`) already converts LSP position to byte offset and checks if it falls within AST node ranges. This pattern is reused in hover (`main.rs:1048+`).

### Hover's Position Detection
Hover (`main.rs:1100-1273`) already detects:
- Cursor on ref() calls → shows upstream schema
- Cursor on source() calls → shows source table definition
These same checks can be extended to return `GotoDefinitionResponse` instead of `Hover`.

### TypeContext Column Lookup
`TypeContext::lookup_column()` (`type_inference.rs:145-199`) resolves a column name to its source, searching CTE columns, model columns, and source columns with fallback. This is the core resolution engine for column goto-def.

## Schema & Column Range Tracking

The `Column` struct (`schema.rs:14-32`) has a `range: TextRange` field tracking position in the source file. This is set from `SelectItem::range()` (`lib.rs:830`), which covers the entire select item expression.

For goto-definition on columns, we need:
1. **Source column range**: Where the column is defined in the upstream model's SELECT list
2. **Current**: `Column.range` gives this for the model's own columns
3. **Gap**: No query returns "given column name X in model Y, what's the TextRange of its definition?"

Potential approach: `model_schema()` already returns columns with ranges. To jump to an upstream column, resolve the upstream model path, get its schema, and find the column by name — its `range` field gives the position.

## Test Coverage

- `integration.rs:206-248`: Three tests for ref() goto-definition (resolve, nonexistent, extraction)
- Hover tests (`integration.rs:254-342`): Schema extraction, type inference, available columns
- Completion tests (`integration.rs:537-891`): CTE names, aliases, column availability
- No tests for source(), CTE, or column goto-definition (these features don't exist yet)

## Resolved Questions

1. **Source goto-def target**: Yes — `smelt.source('raw.users')` should jump to `sources.yml`. Need YAML line-number tracking or text search fallback.

2. **Column goto-def across wildcards**: Trace through wildcard (`SELECT *`) chains until reaching a non-wildcard case (explicit column definition). E.g., if A does `SELECT * FROM ref('B')` and B does `SELECT id FROM ref('C')`, jumping to `id` should land on B's explicit `id` column, not continue to C.

3. **Ambiguous columns**: Show multiple locations (return `GotoDefinitionResponse::Array`). Ambiguous columns without qualifiers are errors in smelt, so showing all candidates is appropriate.

4. **CTE column definition position**: `infer_cte_columns()` returns column names but currently no TextRanges. Need to fix so it return ranges as well.

5. **Alias definition range**: `TableRef::alias()` returns the alias string but not its TextRange. Need to find the alias token's position for "jump to alias definition" to work as a goto-def target.
