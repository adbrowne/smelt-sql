# Research: LSP Refactorings and Code Actions

**Date**: 2026-04-05
**Topic**: What LSP refactoring/code-action infrastructure exists today, and what refactorings are feasible
**Branch**: main
**Commit**: 662a5a2

## Summary

The smelt LSP currently has **zero code action or refactoring support**. No `textDocument/codeAction`, `textDocument/rename`, or `textDocument/references` handlers exist. However, the underlying infrastructure — position tracking, symbol resolution, type inference, cross-model schema queries — is rich enough to support a useful set of refactorings and quick-fixes. The main gap is **inverse reference lookup** (finding all usages of a symbol across files).

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-lsp/src/main.rs` | LSP server, all handlers | L1364-1382 (capabilities), L1605-1984 (goto-def) |
| `crates/smelt-db/src/lib.rs` | Salsa queries, diagnostics | L384-470 (file_diagnostics), L1484-1622 (type_diagnostics) |
| `crates/smelt-db/src/type_inference.rs` | Type inference, undeclared columns | L1674-1725 (check_undeclared_columns) |
| `crates/smelt-db/src/schema.rs` | ModelSchema, Column, ColumnSource | L14-123 |
| `crates/smelt-core/src/sources.rs` | SourcesConfig, SourceTableDef, SourceColumnDef | L19-160 |
| `crates/smelt-parser/src/ast.rs` | AST types, RefCall, SourceCall, CTE | L1309-1432 (ref/source calls), L1461-1473 (Position/Range) |

## Current Capabilities (What Exists)

**Advertised LSP capabilities** (main.rs:1364-1382):
- `textDocument/sync` (FULL)
- `textDocument/definition` (goto-def for refs, sources, CTEs, columns)
- `textDocument/hover` (type info, model schemas)
- `textDocument/completion` (model names, columns, CTE names)

**Not advertised**: codeAction, rename, references, documentSymbol, workspaceSymbol, formatting.

**Diagnostic categories** that could drive quick-fixes:

| Diagnostic | Message Pattern | Source |
|-----------|----------------|--------|
| Undefined ref | `"Undefined model reference: '{name}'"` | lib.rs:427 |
| Undefined source | `"Undefined source: '{qualified}'"` | lib.rs:447 |
| Unknown type | `"Could not infer type for column '{name}'. Consider adding an explicit CAST."` | lib.rs:1523 |
| Undeclared column | `"Column '{name}' not found in any source, model, or CTE"` | lib.rs:1554 |
| Type mismatch | `"Column '{col}' from '{ref}' has type {actual} but is used where {expected} is expected"` | lib.rs:1586 |
| Circular dep | `"Circular dependency involving model '{name}'"` | lib.rs:1612 |
| PIVOT/UNPIVOT | `"PIVOT/UNPIVOT is not yet supported"` | lib.rs:634-651 |

**Internal Diagnostic struct** (lib.rs:690-695): Has only `severity`, `message`, `range`. No `code`, `data`, or `tags` fields — these would need to be added to carry structured metadata for code actions.

## Symbol Resolution Infrastructure

**Forward resolution** (goto-definition direction) is complete:
- `db.resolve_ref(name) -> Option<PathBuf>` — model name to file
- `db.resolve_source(root, source, table) -> Option<SourceTableDef>` — source to definition
- `db.type_context(path)` — column lookup with alias resolution, CTE awareness
- `resolve_column_definitions()` in main.rs:183-230 — columns to definition sites (traces through CTEs, models, sources)

**Inverse resolution** (find-references direction) does **not** exist as a query. Building it requires:
- For **model rename**: iterate `db.all_files()`, check each file's `db.model_refs()` for matching name
- For **source rename**: iterate all files, check `db.model_sources()` for matching source/table
- For **column rename**: iterate all files, check `db.type_context()` or walk expressions for matching column refs — complicated by aliases and SELECT *
- For **CTE rename**: single-file only, walk the AST

**Existing queries that help**:
- `db.model_refs(path) -> Vec<RefLocation>` — all ref() calls with positions per file
- `db.model_sources(path) -> Vec<SourceLocation>` — all source() calls with positions per file
- `db.all_files() -> Vec<PathBuf>` — all known model files
- `db.all_models() -> HashMap<PathBuf, Model>` — all models with names

## Feasible Refactorings (Ranked by Infrastructure Readiness)

### Tier 1: Directly Buildable (infrastructure exists)

**1. Rename model (cross-file)**
- `prepare_rename`: check cursor is on a model file name or inside `ref('name')`
- Find usages: `db.all_files()` + `db.model_refs(path)` — filter for matching name
- Apply: rename the file on disk + update all ref() string literals
- Complexity: moderate — need to handle file rename + re-index

**2. Rename CTE (single-file)**
- Scope: within one file, CTEs are local
- Find usages: walk AST for CTE name in FROM/JOIN clauses and qualified column refs
- Apply: text edits within the file
- Complexity: low

**3. Quick-fix: Add explicit CAST (for "Could not infer type" warning)**
- Diagnostic already says "Consider adding an explicit CAST"
- Code action: wrap expression in `CAST(expr AS type)` — but we don't always know the target type
- When we do know (from InputConstraint.expected_type or from usage context): suggest specific cast
- Complexity: low for the action, medium for choosing the right type

**4. Quick-fix: Add column to sources.yml (for "undeclared column" on source tables)**
- When a column ref like `raw_events.new_column` triggers "Column not found" and the qualifier resolves to a source table
- Code action: append column entry to the matching table in sources.yml
- Need: resolve qualifier to source, locate the table's column list in YAML, append
- Complexity: medium (YAML editing with correct indentation)

**5. Quick-fix: Add CAST for type mismatch**
- Diagnostic: "Column 'x' from 'ref' has type VARCHAR but is used where INTEGER is expected"
- Code action: wrap the column reference in `CAST(col AS expected_type)`
- All needed info is in the diagnostic (column name, expected type, range)
- Complexity: low

### Tier 2: Needs New Queries

**6. Rename source table (cross-file + YAML)**
- Need: find all `source('source.table')` calls across files + update sources.yml
- Queries exist (`model_sources`), but also need to edit sources.yml
- Complexity: medium

**7. Rename column (cross-file)**
- Hardest rename — columns flow through CTEs, SELECT *, aliases, and across model boundaries
- Need column lineage tracing (partially exists in ColumnSource enum)
- SELECT * makes this especially tricky — renaming upstream breaks downstream without visible reference
- Complexity: high

**8. Extract CTE**
- Select a subquery or expression, extract into a new CTE in the WITH clause
- Need: determine referenced columns, generate CTE definition, replace original with CTE ref
- Complexity: medium

### Tier 3: Additional Code Actions

**9. Quick-fix: Create missing model (for "Undefined model reference")**
- Scaffold a new .sql file with the referenced name in the models/ directory
- Complexity: low

**10. Quick-fix: Add missing source to sources.yml (for "Undefined source")**
- When `source('new_source.table')` is undefined, offer to add the source+table to YAML
- Complexity: medium

**11. Organize imports / add missing ref**
- When a column is used but no FROM clause references the model — suggest adding `smelt.ref('model')` to FROM
- Requires inferring which model provides the column
- Complexity: high

## Infrastructure Changes Needed

### For any code action support:
1. **Add `code` field to smelt-db Diagnostic** — structured enum identifying the diagnostic kind (e.g., `DiagnosticCode::UndeclaredColumn { qualifier, column }`) so code actions can pattern-match
2. **Register `code_action_provider` in ServerCapabilities**
3. **Implement `textDocument/codeAction` handler** — match diagnostics at the requested range, generate `WorkspaceEdit` or `Command` responses

### For rename support:
1. **Implement `textDocument/references`** — needed for rename and independently useful
2. **Register `rename_provider` in ServerCapabilities** (with `prepareProvider: true`)
3. **Implement `textDocument/prepareRename`** — validate the cursor is on a renamable symbol
4. **Implement `textDocument/rename`** — compute cross-file `WorkspaceEdit`

### For sources.yml editing:
- Need to locate the correct YAML position for insertion — line-level text manipulation or use a YAML library that preserves formatting

## Test Coverage

- **Diagnostics**: Extensive tests in lib.rs (L4635+) covering type mismatches, undeclared columns, circular deps
- **Goto-definition**: No dedicated unit tests found (tested via integration/example_diagnostics)
- **Code actions**: No tests (feature doesn't exist)
- **Example diagnostics CI gate**: `cargo test -p smelt-cli --test example_diagnostics` verifies zero diagnostics across all example workspaces

## Open Questions

1. **Diagnostic metadata**: Should we add a structured `code` enum to `Diagnostic`, or use the LSP `data` field on the LSP side only? The pure-function rule says analysis logic shouldn't depend on LSP types, so a smelt-db-level code enum seems right.
2. **YAML editing strategy**: For sources.yml modifications, should we use string-level manipulation (fragile) or pull in a format-preserving YAML library?
3. **Rename scope for columns**: Should column rename attempt cross-model propagation (through SELECT * chains), or limit to single-file scope initially?
4. **Priority ordering**: Which of these provides the most day-to-day value to implement first?
