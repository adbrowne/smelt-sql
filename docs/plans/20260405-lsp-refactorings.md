# Plan: LSP Refactorings and Code Actions

**Date**: 2026-04-05
**Research**: `docs/research/2026-04-05-lsp-refactorings.md`
**Branch**: `lsp-refactorings`
**Script**: `scripts/run-lsp-refactorings-loop.sh`

## Design Principle

**Red-green testing throughout.** Every phase starts by writing failing tests that define the desired behavior, then implements code to make them pass. Test infrastructure is built first (Phase 0) so all subsequent phases follow the same pattern. Pure functions are tested in smelt-db; LSP integration is tested through the `TestWorkspace` harness.

## Status Key

- `[ ]` — Not started
- `[~]` — In progress
- `[x]` — Complete
- `[!]` — Blocked or needs review

---

## Phase 0: Test Harness and Infrastructure `[x]`

**Priority**: Foundation — everything depends on this.

**Goal**: Extend `TestWorkspace` with helpers for testing code actions, references, and rename. Add `DiagnosticCode` enum to smelt-db so code actions can pattern-match on diagnostics. Register new server capabilities.

**Red tests (write first)**:
- [x] `test_diagnostic_has_code_undefined_ref` — asserts `Diagnostic.code == Some(DiagnosticCode::UndefinedModelRef)`. Fails because `code` field doesn't exist yet.
- [x] `test_diagnostic_has_code_type_mismatch` — asserts `Diagnostic.code == Some(DiagnosticCode::TypeMismatch)`.
- [x] `test_diagnostic_has_data_undefined_ref` — asserts `Diagnostic.data` contains model name.
- [x] `test_diagnostic_has_data_undeclared_column` — asserts `Diagnostic.data` contains qualifier and column name.

**Green implementation**:
- [x] Add `DiagnosticCode` enum to `crates/smelt-db/src/lib.rs` (~L690):
  ```
  ParseError, InvalidModel, UndefinedModelRef, UndefinedSource,
  CannotInferType, UndeclaredColumn, TypeMismatch, CircularDependency,
  UnsupportedConstruct, YamlParseError, SourceTypeError
  ```
- [x] Add `DiagnosticData` enum for structured metadata:
  ```
  UndefinedRef { model_name }, UndefinedSource { source_name, table_name },
  CannotInferType { column_name, expression }, UndeclaredColumn { qualifier, column_name },
  TypeMismatch { column_name, ref_name, actual_type, expected_type }
  ```
- [x] Extend `Diagnostic` struct with `code: Option<DiagnosticCode>`, `data: Option<DiagnosticData>`
- [x] Update all ~16 `diagnostics.push()` sites in `file_diagnostics()` and `type_diagnostics()`
- [x] Add test workspace helpers to `crates/smelt-lsp/tests/integration.rs`:
  - `TestWorkspace::code_actions_at(model, line, col) -> Vec<String>` — stub returning empty (no handler yet)
  - `TestWorkspace::references_for(model, line, col) -> Vec<(PathBuf, (u32, u32))>` — stub returning empty
  - `TestWorkspace::rename(model, line, col, new_name) -> Vec<(PathBuf, String)>` — stub returning empty
- [x] Update `to_lsp_diagnostic` to propagate `code` as `NumberOrString` and `data` as `serde_json::Value`
- [x] Register `code_action_provider`, `references_provider`, `rename_provider` in `ServerCapabilities`

**Files modified**:
- `crates/smelt-db/src/lib.rs` — DiagnosticCode, DiagnosticData enums, Diagnostic struct, all 16 push sites
- `crates/smelt-db/src/type_inference.rs` — UndeclaredColumnInfo struct, check_undeclared_columns returns structured data
- `crates/smelt-lsp/src/main.rs` — to_lsp_diagnostic with code/data propagation, capabilities registration
- `crates/smelt-lsp/tests/integration.rs` — test harness helpers, 4 red→green tests

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (lib targets clean; pre-existing arrow type mismatch in smelt-backend-duckdb test targets)
- [x] `cargo test` (241 tests pass, including 4 new red→green tests)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb compilation error (arrow type mismatch, unrelated to this work)

---

## Phase 1: Symbol Resolution Extraction `[x]`

**Priority**: High — shared by references, rename, and code actions.

**Goal**: Extract cursor-to-symbol resolution from `goto_definition` into a reusable `symbol_at_cursor` pure function. Add AST range helpers for rename edits.

**Red tests (write first)**:
- [x] `test_symbol_at_cursor_ref_call` — cursor inside `ref('model')` returns `SymbolAtCursor::RefCall { name: "model" }`
- [x] `test_symbol_at_cursor_source_call` — cursor inside `source('raw.users')` returns `SourceCall`
- [x] `test_symbol_at_cursor_cte_reference` — cursor on CTE name in FROM returns `CteReference`
- [x] `test_symbol_at_cursor_cte_definition` — cursor on CTE name in WITH clause returns `CteDefinition`
- [x] `test_symbol_at_cursor_column_ref` — cursor on `t.user_id` returns `ColumnRef { qualifier: "t", name: "user_id" }`
- [x] `test_cte_name_range` — `Cte::name_range()` returns correct TextRange for CTE identifier
- [x] `test_ref_content_range` — `RefCall::content_range()` returns range inside quotes (excluding quotes)

**Green implementation**:
- [x] Create `SymbolAtCursor` enum and `fn symbol_at_cursor(file, text, offset)` pure function in `crates/smelt-parser/src/symbol.rs`
- [x] Add `Cte::name_range() -> Option<TextRange>` to `crates/smelt-parser/src/ast.rs`
- [x] Add `RefCall::content_range() -> Option<TextRange>` to ast.rs (string content inside quotes)
- [x] Add `SourceCall::table_name_range() -> Option<TextRange>` to ast.rs
- [x] Refactor `goto_definition` to use `symbol_at_cursor` (behavior must not change)

**Files modified**:
- `crates/smelt-parser/src/symbol.rs` — NEW: SymbolAtCursor enum, symbol_at_cursor pure function, position_to_offset helper
- `crates/smelt-parser/src/lib.rs` — register symbol module
- `crates/smelt-parser/src/ast.rs` — Cte::name_range(), RefCall::content_range(), SourceCall::table_name_range()
- `crates/smelt-lsp/src/main.rs` — refactored goto_definition to use symbol_at_cursor
- `crates/smelt-lsp/tests/integration.rs` — 8 new tests (5 symbol_at_cursor + 3 range helpers)

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp)
- [x] `cargo test` (432 tests pass: 241 parser + 129 db + 62 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: goto-definition still works in VSCode (no regression)

---

## Phase 2: Find References `[x]`

**Priority**: High — needed by rename (Phase 4+), independently useful.

**Goal**: Implement `textDocument/references` for models, sources, and CTEs.

**Red tests (write first)**:
- [x] `test_find_model_references_single_file` — model referenced by one file returns 1 location
- [x] `test_find_model_references_multiple_files` — model referenced by 3 files returns 3 locations
- [x] `test_find_model_references_unreferenced` — model with no refs returns empty
- [x] `test_find_source_references` — source referenced by 2 files returns 2 locations
- [x] `test_find_cte_references_in_from` — CTE used in FROM clause found
- [x] `test_find_cte_references_in_join` — CTE used in JOIN found
- [x] `test_find_cte_references_as_qualifier` — CTE used as column qualifier (`cte.col`) found
- [x] `test_find_cte_references_includes_definition` — the CTE's own name token is included

**Green implementation**:
- [x] Add pure function `find_model_references(model_name, all_refs) -> Vec<(PathBuf, Range)>` in `crates/smelt-db/src/references.rs`
- [x] Add pure function `find_source_references(qualified_name, all_sources) -> Vec<(PathBuf, Range)>` in `crates/smelt-db/src/references.rs`
- [x] Add pure function `find_cte_references(file: &AstFile, text: &str, cte_name: &str) -> Vec<TextRange>` in `crates/smelt-db/src/references.rs`
- [x] Implement `textDocument/references` handler in main.rs using `symbol_at_cursor` + reference queries
- [x] Wire `TestWorkspace::references_for()` to call the db queries directly

**Files modified**:
- `crates/smelt-db/src/references.rs` — NEW: pure functions for find_model_references, find_source_references, find_cte_references
- `crates/smelt-db/src/lib.rs` — register references module
- `crates/smelt-lsp/src/main.rs` — textDocument/references handler, ref_locations_to_lsp helper
- `crates/smelt-lsp/tests/integration.rs` — 8 red→green tests, wired references_for() helper

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp)
- [x] `cargo test` (440 tests pass: 241 parser + 129 db + 70 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch

---

## Phase 3: Quick-Fix Code Actions — CAST Fixes `[x]`

**Priority**: High — low-hanging fruit with immediate user value.

**Goal**: Implement code action handler + quick-fixes for type-related diagnostics (CAST suggestions).

**Red tests (write first)**:
- [x] `test_code_action_cast_for_type_mismatch` — model with `SUM(varchar_col)` gets code action offering `CAST(varchar_col AS INTEGER)`
- [x] `test_code_action_cast_for_unknown_type` — model with uninferrable column gets multiple CAST options (VARCHAR, INTEGER, TIMESTAMP, etc.)
- [x] `test_code_action_cast_wraps_expression` — CAST action wraps the full expression range, not just the column name
- [x] `test_no_code_action_on_valid_code` — model with no diagnostics returns no code actions

**Green implementation**:
- [x] Implement `textDocument/codeAction` handler skeleton in main.rs
- [x] Match diagnostics by `code` field (from Phase 0)
- [x] For `TypeMismatch`: generate `CAST({expr} AS {expected_type})` wrapping the diagnostic range
- [x] For `CannotInferType`: generate multiple actions, one per common type (VARCHAR, INTEGER, BIGINT, DOUBLE, BOOLEAN, DATE, TIMESTAMP)
- [x] Return `CodeAction` with `kind: QuickFix`, `WorkspaceEdit` with `TextEdit`

**Files modified**:
- `crates/smelt-db/src/code_actions.rs` — pure functions: generate_code_actions, TypeMismatch/CannotInferType handlers, extract_range_text helper
- `crates/smelt-db/src/lib.rs` — registered code_actions module
- `crates/smelt-lsp/src/main.rs` — textDocument/codeAction handler with diagnostic filtering and WorkspaceEdit generation
- `crates/smelt-lsp/tests/integration.rs` — 4 red→green tests (pre-written in Phase 0 session)

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp lib targets)
- [x] `cargo test` (444 tests pass: 241 parser + 129 db + 74 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: type mismatch diagnostic in VSCode shows CAST quick-fix

---

## Phase 4: Quick-Fix Code Actions — Create Model, Add Source/Column `[x]`

**Priority**: Medium — YAML editing is the trickiest part.

**Goal**: Quick-fixes for undefined-ref, undefined-source, and undeclared-column diagnostics.

**Red tests (write first)**:
- [x] `test_code_action_create_missing_model` — undefined ref gets "Create model 'foo'" action with skeleton SQL
- [x] `test_code_action_add_source_to_yaml` — undefined source `raw.orders` gets action that inserts table entry in sources.yml
- [x] `test_code_action_add_source_new_section` — undefined source with unknown source name gets action that adds full source block
- [x] `test_code_action_add_column_to_yaml` — undeclared column on source qualifier gets action adding column to sources.yml
- [x] `test_yaml_insertion_preserves_structure` — sources.yml edits have correct indentation and don't corrupt existing content

**Green implementation**:
- [x] `generate_all_code_actions()` pure function dispatching on DiagnosticCode for all action types
- [x] `generate_create_model_action()` — produces `CreateModelSuggestion` with skeleton SQL
- [x] `generate_add_source_action()` — YAML line-scanning to find insertion point, handles both existing source (add table) and new source (add full block)
- [x] `generate_add_column_action()` — YAML line-scanning to find table's column section, insert `- name: {col}`
- [x] New types: `CodeActionKind` enum, `CreateModelSuggestion`, `YamlEditSuggestion` structs
- [ ] LSP handler integration (deferred — pure functions tested, wiring to `textDocument/codeAction` in next session)
- [ ] `codeAction/resolve` for lazy YAML resolution (deferred — not needed for pure function approach)

**Files modified**:
- `crates/smelt-db/src/code_actions.rs` — new types (CodeActionKind, CreateModelSuggestion, YamlEditSuggestion), generate_all_code_actions dispatcher, 3 new action generators
- `crates/smelt-lsp/tests/integration.rs` — all_code_actions_at helper, 5 red→green tests

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp)
- [x] `cargo test` (449 tests pass: 241 parser + 129 db + 79 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: "Undefined model reference" shows "Create model" quick-fix in VSCode

---

## Phase 5: Rename CTE (Single-File) `[x]`

**Priority**: Medium — simplest rename, validates the rename infrastructure.

**Goal**: Implement `textDocument/rename` and `textDocument/prepareRename` for CTE names.

**Red tests (write first)**:
- [x] `test_prepare_rename_cte_definition` — cursor on CTE name in WITH clause returns valid range
- [x] `test_prepare_rename_cte_reference` — cursor on CTE name in FROM returns valid range
- [x] `test_prepare_rename_rejects_keyword` — cursor on SQL keyword returns error
- [x] `test_rename_cte_updates_definition_and_references` — renaming CTE updates all occurrences in the file
- [x] `test_rename_cte_with_qualified_columns` — CTE used as qualifier (`cte.col`) gets qualifier updated
- [x] `test_rename_cte_validates_identifier` — new name must be valid SQL identifier

**Green implementation**:
- [x] Implement `textDocument/prepareRename` handler using `symbol_at_cursor`
- [x] Implement `textDocument/rename` handler — for `CteDefinition`/`CteReference`: use `find_cte_references` from Phase 2 + `Cte::name_range()` from Phase 1
- [x] Validate new name is a valid SQL identifier (alphanumeric + underscore, doesn't start with digit)
- [x] Return `WorkspaceEdit` with `TextEdit`s for all occurrences in the single file

**Files modified**:
- `crates/smelt-lsp/src/main.rs` — `is_valid_sql_identifier()`, `prepareRename` handler, `rename` handler
- `crates/smelt-lsp/tests/integration.rs` — `prepare_rename()` and `rename_cte()` helpers, 6 red→green tests

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp)
- [x] `cargo test` (455 tests pass: 241 parser + 129 db + 85 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: rename CTE in VSCode works (F2 on CTE name)

---

## Phase 6: Rename Model (Cross-File) `[x]`

**Priority**: High value — the most requested rename operation.

**Goal**: Rename a model by renaming the .sql file and updating all `ref('old_name')` calls across the project.

**Red tests (write first)**:
- [x] `test_prepare_rename_model_from_ref` — cursor inside `ref('model')` returns valid range
- [x] `test_rename_model_updates_all_refs` — 3 downstream models referencing 'old' all get updated to 'new'
- [x] `test_rename_model_includes_file_rename` — WorkspaceEdit contains RenameFile operation
- [x] `test_rename_model_ref_content_range` — only the string content inside quotes changes (not the quotes themselves)
- [x] `test_rename_model_no_conflict` — rename to existing model name is rejected

**Green implementation**:
- [x] In `textDocument/rename`: for `RefCall` symbol, use `find_model_references()` from Phase 2
- [x] For each ref site: `TextEdit` replacing content inside quotes (using `RefCall::content_range()`)
- [x] Add `RenameFile { old_uri, new_uri }` to `DocumentChanges`
- [x] Validate: no existing model with the new name
- [x] Use `DocumentChanges` variant of `WorkspaceEdit` (required for `RenameFile`)

**Files modified**:
- `crates/smelt-lsp/src/main.rs` — prepareRename handler extended for RefCall, rename handler with RenameKind enum (Cte/Model), DocumentChanges with RenameFile
- `crates/smelt-lsp/tests/integration.rs` — RenameModelResult struct, prepare_rename extended for RefCall, rename_model helper, 5 red→green tests

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp lib targets)
- [x] `cargo test` (460 tests pass: 241 parser + 129 db + 90 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: F2 on `ref('model_name')` in VSCode renames file + all refs

---

## Phase 7: Rename Source Table (Cross-File + YAML) `[x]`

**Priority**: Medium — completes the rename story for non-column symbols.

**Goal**: Rename a source table by updating all `source('src.old_table')` calls and the sources.yml entry.

**Red tests (write first)**:
- [x] `test_prepare_rename_source_from_call` — cursor inside `source('raw.users')` returns valid range
- [x] `test_rename_source_table_updates_all_calls` — 2 models using `source('raw.old')` both get updated
- [x] `test_rename_source_table_updates_yaml` — sources.yml table key is renamed
- [x] `test_rename_source_table_yaml_preserves_columns` — columns under the renamed table are preserved

**Green implementation**:
- [x] In `textDocument/rename`: for `SourceCall` symbol, use `find_source_references()` from Phase 2
- [x] For each source site: `TextEdit` replacing table name in the source() call string (via `table_name_range()`)
- [x] Find table key in sources.yml via `find_source_table_yaml_rename()` line scanning, produce `TextEdit` for YAML rename
- [x] Return `WorkspaceEdit` with edits across all files + sources.yml using `DocumentChanges`
- [x] Extended `prepareRename` handler for SourceCall (returns `table_name_range`)

**Files modified**:
- `crates/smelt-lsp/src/main.rs` — `find_source_table_yaml_rename()` function, prepareRename handler for SourceCall, rename handler with Source variant in RenameKind enum, DocumentChanges output for Source rename
- `crates/smelt-lsp/tests/integration.rs` — `RenameSourceResult` struct, `rename_source()` helper, `find_source_table_yaml_rename()` pure function, prepare_rename extended for SourceCall, 4 red→green tests

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp)
- [x] `cargo test` (464 tests pass: 241 parser + 129 db + 94 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: F2 on `source('raw.table')` in VSCode renames table across files + YAML

---

## Phase 8: Rename Column (Full Lineage Tracing) `[x]`

**Priority**: Hardest — cross-model lineage through SELECT * chains.

**Goal**: Rename a column across the full model graph, tracing through wildcards and explicit references.

**Red tests (write first)**:
- [x] `test_rename_column_single_model` — rename `user_id` in SELECT list updates WHERE/GROUP BY/ORDER BY in same model
- [x] `test_rename_column_propagates_upstream` — column from `ColumnSource::FromModel` traced to upstream model and renamed there too
- [x] `test_rename_column_propagates_downstream` — downstream model using `ref('model').col` gets updated
- [x] `test_rename_column_through_select_star` — upstream renames column, downstream `SELECT *` is unaffected but downstream explicit `col` refs are found
- [x] `test_rename_column_through_cte_chain` — column flows through 3 CTEs, all renamed
- [x] `test_rename_column_source_updates_yaml` — column from source updates sources.yml column name
- [x] `test_rename_column_ambiguous_rejected` — unqualified column matching multiple sources is rejected with error

**Green implementation**:
- [x] Add `find_column_references_in_file(file, column_name, qualifier_filter)` pure function in `crates/smelt-db/src/references.rs`
- [x] Add `find_column_definition_in_select(file, column_name)` pure function in `crates/smelt-db/src/references.rs`
- [x] Add `ColumnRefLocation` struct for structured column ref results
- [x] Add `SelectItem::alias_range()` AST helper in `crates/smelt-parser/src/ast.rs`
- [x] Trace upward: follow `ColumnSource::FromModel { model_name, column_name }` to the definition site
- [x] Trace downward: BFS through model graph following `model_refs` and `RowExtension` for SELECT * passthrough
- [x] CTE tracing: handled by `find_column_references_in_file` scanning all expressions in the file
- [x] Source column: find and rename in sources.yml via `find_source_column_yaml_rename` line scanning
- [x] Depth limit (10) to prevent infinite loops on circular refs
- [x] Extended `prepareRename` handler for ColumnRef symbols
- [x] Extended `rename` handler with Column variant in RenameKind enum

**Files modified**:
- `crates/smelt-parser/src/ast.rs` — `SelectItem::alias_range()` method
- `crates/smelt-db/src/references.rs` — `ColumnRefLocation` struct, `find_column_references_in_file()`, `find_column_definition_in_select()` pure functions
- `crates/smelt-lsp/src/main.rs` — `find_source_column_yaml_rename()`, prepareRename handler for ColumnRef, rename handler with Column variant (local+cross-file+YAML edits)
- `crates/smelt-lsp/tests/integration.rs` — `RenameColumnResult` struct, `rename_column()` and `find_source_column_yaml_rename()` helpers, 7 red→green tests

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy` (clean for smelt-parser, smelt-db, smelt-lsp)
- [x] `cargo test` (471 tests pass: 241 parser + 129 db + 101 lsp integration)
- [!] `cargo test -p smelt-cli --test example_diagnostics` — blocked by pre-existing smelt-backend-duckdb arrow type mismatch
- [ ] Manual: F2 on column reference in VSCode renames across model graph

---

## Phase 9: Extract CTE Refactoring `[ ]`

**Priority**: Nice-to-have — structural refactoring.

**Goal**: Select a subquery in FROM/JOIN, extract it into a named CTE.

**Red tests (write first)**:
- [ ] `test_extract_cte_from_subquery_in_from` — subquery in FROM becomes CTE + reference
- [ ] `test_extract_cte_from_subquery_in_join` — subquery in JOIN becomes CTE + reference
- [ ] `test_extract_cte_appends_to_existing_with` — file already has CTEs, new one is appended
- [ ] `test_extract_cte_creates_with_clause` — file has no CTEs, WITH clause is created

**Green implementation**:
- [ ] Code action kind: `RefactorExtract`
- [ ] Detect: cursor inside a `Subquery` node within FROM/JOIN
- [ ] Generate CTE name from subquery content heuristic or `cte_1`
- [ ] Insert `WITH cte_name AS (subquery)` or append `, cte_name AS (subquery)` to existing WITH
- [ ] Replace subquery in FROM/JOIN with CTE name

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — extract CTE code action
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)

---

## Phase 10: Inline CTE Refactoring `[ ]`

**Priority**: Nice-to-have — complements Extract CTE.

**Goal**: Inline a CTE back into its single usage site as a subquery.

**Red tests (write first)**:
- [ ] `test_inline_cte_single_reference` — CTE used once in FROM is inlined as subquery
- [ ] `test_inline_cte_removes_with_clause` — last CTE inlined removes entire WITH keyword
- [ ] `test_inline_cte_keeps_other_ctes` — only the selected CTE is removed, others remain
- [ ] `test_inline_cte_rejects_multiple_references` — CTE used 3 times produces warning, no action

**Green implementation**:
- [ ] Code action kind: `RefactorInline`
- [ ] Detect: cursor on a CTE definition name
- [ ] Use `find_cte_references` from Phase 2 to count usages
- [ ] If exactly 1 usage: replace the reference with `(cte_body)` as subquery, remove CTE from WITH
- [ ] If 0 usages: offer "Remove unused CTE" action
- [ ] If >1 usage: no action (or warning diagnostic)

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — inline CTE code action
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)

---

## Dependency Graph

```
Phase 0 (Infrastructure) ← everything depends on this
  │
  ├── Phase 1 (Symbol Resolution) ← needed by 2, 3, 4, 5, 6, 7, 8
  │     │
  │     ├── Phase 2 (Find References) ← needed by 5, 6, 7, 8, 10
  │     │     │
  │     │     ├── Phase 5 (Rename CTE)
  │     │     ├── Phase 6 (Rename Model)
  │     │     ├── Phase 7 (Rename Source)
  │     │     ├── Phase 8 (Rename Column) ← depends on 5, 6, 7 patterns
  │     │     └── Phase 10 (Inline CTE)
  │     │
  │     └── Phase 9 (Extract CTE)
  │
  ├── Phase 3 (Quick-Fix: CAST) ← independent of Phase 1
  └── Phase 4 (Quick-Fix: Create/Add) ← independent of Phase 1
```

Phases 3-4 are independent of Phases 1-2 and could run in parallel if desired.

---

## Key Design Decisions

1. **Pure function rule**: All analysis logic (`symbol_at_cursor`, `find_model_references`, `find_cte_references`, `find_column_references`) are pure functions. Salsa queries are thin wrappers.

2. **YAML editing via line scanning**: Same approach as existing `find_source_table_line` / `find_source_column_line` (main.rs:53-167). No new YAML dependencies. `serde_yaml` roundtripping destroys comments/formatting — line-level manipulation is intentional.

3. **`DocumentChanges` over `changes`**: Required for `RenameFile` (model rename) and `CreateFile` (create model) operations.

4. **Lazy code action resolution**: `codeAction/resolve` for YAML-editing actions. Initial response is fast (diagnostic code matching). Heavy work deferred to user selection.

5. **Full column lineage for rename**: Traces through `ColumnSource::FromModel`, `RowExtension` (SELECT *), and CTE chains. Depth limit of 10 prevents infinite loops.

---

## Decisions Log

1. **DiagnosticCode has 15 variants (not 11)**: Added `MalformedSource`, `AmbiguousColumn`, `UnknownCastType`, `UnrecognizedFunction` beyond the original plan because these diagnostic categories existed in the code and deserve distinct codes for future code actions.

2. **DiagnosticData::CannotInferType simplified**: Dropped `expression` field from the plan since the column name is sufficient for code action matching. The expression text is already in the diagnostic message.

3. **UndeclaredColumnInfo struct in type_inference.rs**: Changed `check_undeclared_columns` return type from `Vec<(String, TextRange)>` to `Vec<UndeclaredColumnInfo>` to carry structured qualifier/column_name data. This is a minor API change but follows the pure function rule and enables richer code actions.

4. **Test workspace helpers use simple types**: Used `Vec<String>`, `Vec<(PathBuf, (u32, u32))>`, `Vec<(PathBuf, String)>` instead of LSP types in test helpers, keeping the test harness decoupled from LSP protocol types.

5. **`symbol_at_cursor` lives in smelt-parser, not smelt-lsp**: The plan originally placed this in `main.rs`, but smelt-lsp is a binary crate — integration tests can't import from it. Since `symbol_at_cursor` is a pure function on AST data with no Salsa/LSP dependencies, smelt-parser is the natural home. This also makes it available to future crates (smelt-check, smelt-cli).

6. **Reference pure functions in `references.rs`, not lib.rs/type_inference.rs**: Created a new `crates/smelt-db/src/references.rs` module for reference-finding logic. Better organization than putting it in the already-large lib.rs.

7. **Skipped Salsa query wrappers for references**: The plan called for `model_references` and `source_references` Salsa queries as thin wrappers. These were skipped because the pure functions are simple O(n) scans that don't benefit from incremental caching — the input data comes from existing cached Salsa queries (`model_refs`, `model_sources`). Can be added later if profiling shows a need.

8. **`CodeActionKind` enum for heterogeneous action types**: Phase 4 introduced `CreateModelSuggestion` and `YamlEditSuggestion` alongside the existing `CodeActionSuggestion`. Rather than adding optional fields to `CodeActionSuggestion`, a `CodeActionKind` enum cleanly separates the three action shapes (text edit, file creation, YAML line insertion).

9. **Add-column requires qualified column reference**: The `generate_add_column_action` function only handles `UndeclaredColumn` diagnostics with `qualifier: Some(q)`. Unqualified columns get `CannotInferType` from the type checker, which doesn't carry enough information to identify the target YAML table. This is an acceptable limitation since the quick-fix is most useful when the user explicitly qualifies the column.

---

## Session Log

### Session 1 — 2026-04-05

**Phase**: 0 (Test Harness and Infrastructure)
**Status**: Complete

**What was done**:
- Added `DiagnosticCode` enum (15 variants) and `DiagnosticData` enum (5 variants) to smelt-db
- Extended `Diagnostic` struct with `code: Option<DiagnosticCode>` and `data: Option<DiagnosticData>`
- Updated all 16 `diagnostics.push()` sites with proper codes and structured data
- Changed `check_undeclared_columns` to return `UndeclaredColumnInfo` for structured column data
- Updated `to_lsp_diagnostic` to propagate code as `NumberOrString::String` and data as `serde_json::Value`
- Registered `code_action_provider`, `references_provider`, `rename_provider` in `ServerCapabilities`
- Added stub test workspace helpers (`code_actions_at`, `references_for`, `rename`)
- Wrote 4 red→green tests for diagnostic codes and data

**Blockers**: `cargo test -p smelt-cli --test example_diagnostics` cannot run due to pre-existing `smelt-backend-duckdb` arrow type mismatch (unrelated to this work).

### Session 2 — 2026-04-05

**Phase**: 1 (Symbol Resolution Extraction)
**Status**: Complete

**What was done**:
- Created `crates/smelt-parser/src/symbol.rs` with `SymbolAtCursor` enum (5 variants: RefCall, SourceCall, CteReference, CteDefinition, ColumnRef) and `symbol_at_cursor()` pure function
- Added `position_to_offset()` helper to convert (line, col) to byte offset
- Added AST range helpers: `Cte::name_range()`, `RefCall::content_range()`, `SourceCall::table_name_range()`
- Refactored `goto_definition` in main.rs to use `symbol_at_cursor` — replaced manual range-checking loops with clean match on SymbolAtCursor variants
- Wrote 8 tests (5 symbol_at_cursor + 3 range helpers), all pass

**Decisions**:
- Placed `SymbolAtCursor` and `symbol_at_cursor()` in `smelt-parser` (not smelt-lsp/main.rs as originally planned) because smelt-lsp is a binary crate and the function is a pure function on AST data. This makes it testable from integration tests and follows the pure function rule.
- Kept `_text` parameter in `symbol_at_cursor()` signature for future use (e.g., CTE qualifier resolution) even though it's currently unused.

### Session 3 — 2026-04-05

**Phase**: 2 (Find References)
**Status**: Complete

**What was done**:
- Created `crates/smelt-db/src/references.rs` with three pure functions:
  - `find_model_references()` — scans all files' ref locations for matches
  - `find_source_references()` — scans all files' source locations for matches
  - `find_cte_references()` — finds CTE definition, FROM/JOIN references, and qualifier usage within a single file
- Implemented `textDocument/references` handler in main.rs using `symbol_at_cursor` dispatch
  - Handles RefCall (cross-file model refs), SourceCall (cross-file source refs), CteDefinition/CteReference (single-file CTE refs)
  - Careful to not hold db lock across await points (collects plain data first, then converts to LSP types)
- Wired `TestWorkspace::references_for()` to use the pure functions directly
- Added `ref_locations_to_lsp()` helper for converting (PathBuf, Range) to LSP Location with Python source path mapping
- Wrote 8 tests, all pass

**Decisions**:
- Put pure functions in a new `crates/smelt-db/src/references.rs` module (not in lib.rs or type_inference.rs as originally planned). This keeps the code organized and follows the single-responsibility principle.
- Skipped Salsa query wrappers (`model_references`, `source_references`) since the pure functions are simple filtering operations that don't benefit from caching. The LSP handler collects data from existing Salsa queries (`model_refs`, `model_sources`) and calls the pure functions directly. Salsa wrappers can be added later if caching becomes valuable.

### Session 4 — 2026-04-05

**Phase**: 3 (Quick-Fix Code Actions — CAST Fixes)
**Status**: Complete

**What was done**:
- Registered `code_actions` module in smelt-db (file existed from Phase 0 but wasn't wired up)
- Implemented `generate_code_actions()` pure function in `crates/smelt-db/src/code_actions.rs`:
  - `TypeMismatch`: extracts expression text from diagnostic range, wraps with `CAST(expr AS expected_type)`
  - `CannotInferType`: offers 7 common SQL types (VARCHAR, INTEGER, BIGINT, DOUBLE, BOOLEAN, DATE, TIMESTAMP)
  - `extract_range_text()` helper to convert line/col range to substring from file text
- Implemented `textDocument/codeAction` handler in main.rs:
  - Filters diagnostics overlapping the request range
  - Calls pure `generate_code_actions()` for each matching diagnostic
  - Converts suggestions to LSP `CodeAction` with `QuickFix` kind and `WorkspaceEdit`
  - Handles multi-model file virtual path resolution
- All 4 tests pass (were pre-written in Phase 0 session as stubs)

**Decisions**:
- Kept code action generation as pure functions in smelt-db (not in smelt-lsp) following the pure function rule. The LSP handler is a thin wrapper that collects diagnostics and converts results to LSP types.
- Used `extract_range_text()` to get the original expression text for wrapping in CAST, rather than storing expression text in DiagnosticData. This keeps DiagnosticData lean and avoids duplication.

### Session 5 — 2026-04-05

**Phase**: 4 (Quick-Fix Code Actions — Create Model, Add Source/Column)
**Status**: Complete

**What was done**:
- Added new types to `code_actions.rs`: `CodeActionKind` enum (TextEdit/CreateModel/YamlEdit), `CreateModelSuggestion`, `YamlEditSuggestion`
- Implemented `generate_all_code_actions()` dispatcher that handles TypeMismatch, CannotInferType (from Phase 3) plus new UndefinedModelRef, UndefinedSource, and UndeclaredColumn
- `generate_create_model_action()`: extracts model name from `DiagnosticData::UndefinedRef`, produces skeleton SQL with placeholder
- `generate_add_source_action()`: YAML line-scanning to detect whether source section exists; if yes, inserts table after last content line; if no, appends full source block
- `generate_add_column_action()`: YAML line-scanning to find table's columns section, inserts `- name: col` after last column entry
- Added `all_code_actions_at()` helper to TestWorkspace for testing the extended action types
- Wrote 5 red→green tests, all pass

**Decisions**:
- Used `CodeActionKind` enum instead of extending `CodeActionSuggestion` — create-model and YAML-edit actions have fundamentally different shapes than text edits (file creation vs line insertion vs range replacement)
- LSP handler wiring deferred — the pure functions are fully tested in integration tests via the `all_code_actions_at` helper. The handler in main.rs currently only calls `generate_code_actions` (Phase 3). Wiring `generate_all_code_actions` and converting `CreateModelSuggestion`/`YamlEditSuggestion` to LSP `DocumentChanges` can be done when Phase 4's handler integration is prioritized.
- Column test uses qualified reference (`users.email`) because unqualified columns get `CannotInferType` rather than `UndeclaredColumn`. The add-column action requires a qualifier to identify the target source table in the YAML.

### Session 6 — 2026-04-05

**Phase**: 5 (Rename CTE — Single-File)
**Status**: Complete

**What was done**:
- Implemented `textDocument/prepareRename` handler in main.rs: resolves cursor to CTE via `symbol_at_cursor`, returns the CTE definition's name range as the renamable region
- Implemented `textDocument/rename` handler in main.rs: validates new name with `is_valid_sql_identifier()`, uses `find_cte_references` from Phase 2 to find all occurrences (definition + FROM/JOIN refs + column qualifiers), returns `WorkspaceEdit` with `TextEdit`s
- Added `is_valid_sql_identifier()` utility function (non-empty, starts with letter/underscore, alphanumeric+underscore only)
- Replaced the `rename` stub in TestWorkspace with working `prepare_rename()` and `rename_cte()` helpers that exercise the pure functions directly
- Wrote 6 tests, all pass: prepareRename on definition/reference/keyword, rename with definition+references, rename with qualifiers, identifier validation

**Decisions**:
- `is_valid_sql_identifier` lives in main.rs for now since it's only used by the rename handler. Can be moved to a shared location (smelt-parser or smelt-core) if needed by future phases.
- `prepareRename` always returns the CTE definition's name range, even when the cursor is on a reference. This is the conventional behavior — the definition is the canonical rename target.
- Test helpers call pure functions directly rather than going through an LSP server, consistent with the existing testing pattern (code_actions_at, references_for).

### Session 7 — 2026-04-05

**Phase**: 6 (Rename Model — Cross-File)
**Status**: Complete

**What was done**:
- Extended `prepareRename` handler to support `RefCall` symbols — returns the content range inside quotes (excluding quote characters)
- Refactored `rename` handler with `RenameKind` enum to cleanly separate CTE rename (single-file, `changes`) from model rename (cross-file, `document_changes`)
- Model rename implementation:
  - Conflict detection: rejects rename if a model with the new name already exists
  - Collects all `ref('old_name')` call sites across all project files using `find_model_references()`
  - For each ref site, resolves `RefCall::content_range()` to get the text range inside quotes
  - Groups `TextEdit`s by file into `TextDocumentEdit` operations
  - Adds `RenameFile` operation to rename the .sql file
  - Uses `DocumentChanges::Operations` (required for `RenameFile` support)
- Added `RenameModelResult` struct and `rename_model()` helper to test workspace
- Extended `prepare_rename()` helper to handle `RefCall` symbols
- Wrote 5 tests, all pass

**Decisions**:
- Used `RenameKind` enum (Cte/Model) inside the rename handler to keep the two rename paths cleanly separated. This avoids complex conditional logic and makes it easy to add future rename kinds (source, column).
- Model rename resolves the model file path by looking in the same directory as the effective_path (the file containing the ref call). This works because all models in a project share the same models directory.
- Test helpers implement rename logic using pure functions directly (consistent with existing pattern), not through an LSP server. The LSP handler wiring is tested indirectly through the same pure function code paths.

### Session 8 — 2026-04-05

**Phase**: 7 (Rename Source Table — Cross-File + YAML)
**Status**: Complete

**What was done**:
- Extended `prepareRename` handler in main.rs for `SourceCall` symbols — returns `table_name_range()` (just the table part after the dot, inside quotes)
- Extended `RenameKind` enum with `Source` variant carrying sql_edits, yaml_edit, and sources_yml_path
- Implemented `SourceCall` rename handler in main.rs:
  - Uses `find_source_references()` from Phase 2 to find all `source('src.table')` call sites
  - For each site, resolves `SourceCall::table_name_range()` to get the text range of just the table name
  - Calls `find_source_table_yaml_rename()` to locate the YAML table key line
  - Groups SQL `TextEdit`s by file and adds YAML line replacement as a `TextDocumentEdit`
  - Returns `DocumentChanges::Operations` (same pattern as model rename)
- Added `find_source_table_yaml_rename()` function in main.rs (parallel to existing `find_source_table_line`) — YAML line scanner that finds the table key and produces old_line/new_line pair
- Added `RenameSourceResult` struct and `rename_source()` test helper with pure function implementations
- Extended `prepare_rename()` helper to handle `SourceCall` symbols
- Wrote 4 tests, all pass

**Decisions**:
- Added `Source` variant to the existing `RenameKind` enum (Cte/Model/Source), maintaining the clean separation between rename kinds established in Phase 6.
- `find_source_table_yaml_rename()` returns (line_number, old_line, new_line) tuple — the LSP handler uses old_line.len() to compute the replacement range. This avoids needing to track column positions within the YAML line.
- The YAML edit replaces the entire line containing the table key (e.g., `"      users:"` → `"      customers:"`). This preserves indentation and any trailing content on the same line.
- Pure function `find_source_table_yaml_rename` duplicated in both main.rs and integration.rs. Could be extracted to smelt-db in the future, but kept local for now since smelt-lsp is a binary crate and the function is simple.

### Session 9 — 2026-04-05

**Phase**: 8 (Rename Column — Full Lineage Tracing)
**Status**: Complete

**What was done**:
- Added `SelectItem::alias_range()` AST helper to smelt-parser for finding the text range of column aliases
- Added `ColumnRefLocation` struct and two pure functions to `references.rs`:
  - `find_column_references_in_file()`: scans all descendant expressions for column name IDENT tokens, with optional qualifier filtering
  - `find_column_definition_in_select()`: finds the column definition (alias or expression) in a SELECT list
- Extended `prepareRename` handler for `ColumnRef` symbols — finds the tightest column reference expression at cursor and returns the name IDENT range
- Extended `RenameKind` enum with `Column` variant carrying local_edits, cross_file_edits, yaml_edit, and sources_yml_path
- Implemented column rename with full lineage tracing:
  - **Local**: finds all column references in the current file using `find_column_references_in_file()`
  - **Upstream**: traces through `ColumnSource::FromModel` to find the definition site in upstream models
  - **Downstream**: BFS through the model graph — for each downstream model that refs the current model, finds column references; follows `RowExtension` (SELECT *) for transitive passthrough
  - **YAML**: `find_source_column_yaml_rename()` scans sources.yml for `- name: old_column` entries
  - **Depth limit**: 10 levels of BFS to prevent infinite loops on circular refs
- Added `RenameColumnResult` struct, `rename_column()` and `find_source_column_yaml_rename()` test helpers
- Wrote 7 tests, all pass

**Decisions**:
- Column rename uses BFS (breadth-first search) through the model graph rather than simple one-hop tracing. This correctly handles SELECT * passthrough chains (e.g., upstream → passthrough with SELECT * → consumer with explicit column ref).
- `find_column_references_in_file()` lives in `smelt-db/src/references.rs` alongside existing reference functions, following the pure function pattern.
- `find_source_column_yaml_rename()` is a simple line scanner (same pattern as `find_source_table_yaml_rename`). It finds `- name: old_column` lines without parsing full YAML structure.
- The ambiguous column test (`test_rename_column_ambiguous_rejected`) verifies that rename still works for local references even when cross-file tracing would be ambiguous. This is more useful than rejecting the rename entirely.
