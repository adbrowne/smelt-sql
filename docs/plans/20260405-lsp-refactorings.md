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

## Phase 1: Symbol Resolution Extraction `[ ]`

**Priority**: High — shared by references, rename, and code actions.

**Goal**: Extract cursor-to-symbol resolution from `goto_definition` into a reusable `symbol_at_cursor` pure function. Add AST range helpers for rename edits.

**Red tests (write first)**:
- [ ] `test_symbol_at_cursor_ref_call` — cursor inside `ref('model')` returns `SymbolAtCursor::RefCall { name: "model" }`
- [ ] `test_symbol_at_cursor_source_call` — cursor inside `source('raw.users')` returns `SourceCall`
- [ ] `test_symbol_at_cursor_cte_reference` — cursor on CTE name in FROM returns `CteReference`
- [ ] `test_symbol_at_cursor_cte_definition` — cursor on CTE name in WITH clause returns `CteDefinition`
- [ ] `test_symbol_at_cursor_column_ref` — cursor on `t.user_id` returns `ColumnRef { qualifier: "t", name: "user_id" }`
- [ ] `test_cte_name_range` — `Cte::name_range()` returns correct TextRange for CTE identifier
- [ ] `test_ref_content_range` — `RefCall::content_range()` returns range inside quotes (excluding quotes)

**Green implementation**:
- [ ] Create `SymbolAtCursor` enum and `fn symbol_at_cursor(file, text, offset)` pure function in `crates/smelt-lsp/src/main.rs`
- [ ] Add `Cte::name_range() -> Option<TextRange>` to `crates/smelt-parser/src/ast.rs`
- [ ] Add `RefCall::content_range() -> Option<TextRange>` to ast.rs (string content inside quotes)
- [ ] Add `SourceCall::table_name_range() -> Option<TextRange>` to ast.rs
- [ ] Refactor `goto_definition` to use `symbol_at_cursor` (behavior must not change)

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — symbol_at_cursor, refactor goto_definition
- `crates/smelt-parser/src/ast.rs` — name_range, content_range, table_name_range
- `crates/smelt-lsp/tests/integration.rs` — red tests for symbol resolution

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
- [ ] `cargo test -p smelt-cli --test example_diagnostics`
- [ ] Manual: goto-definition still works in VSCode (no regression)

---

## Phase 2: Find References `[ ]`

**Priority**: High — needed by rename (Phase 4+), independently useful.

**Goal**: Implement `textDocument/references` for models, sources, and CTEs.

**Red tests (write first)**:
- [ ] `test_find_model_references_single_file` — model referenced by one file returns 1 location
- [ ] `test_find_model_references_multiple_files` — model referenced by 3 files returns 3 locations
- [ ] `test_find_model_references_unreferenced` — model with no refs returns empty
- [ ] `test_find_source_references` — source referenced by 2 files returns 2 locations
- [ ] `test_find_cte_references_in_from` — CTE used in FROM clause found
- [ ] `test_find_cte_references_in_join` — CTE used in JOIN found
- [ ] `test_find_cte_references_as_qualifier` — CTE used as column qualifier (`cte.col`) found
- [ ] `test_find_cte_references_includes_definition` — the CTE's own name token is included

**Green implementation**:
- [ ] Add pure function `find_model_references(model_name, all_refs) -> Vec<(PathBuf, Range)>` in `crates/smelt-db/src/lib.rs`
- [ ] Add Salsa query `model_references(model_name: String)` as thin wrapper
- [ ] Add pure function `find_source_references(qualified_name, all_sources) -> Vec<(PathBuf, Range)>`
- [ ] Add Salsa query `source_references(qualified_name: String)`
- [ ] Add pure function `find_cte_references(file: &AstFile, text: &str, cte_name: &str) -> Vec<TextRange>` in `crates/smelt-db/src/type_inference.rs`
- [ ] Implement `textDocument/references` handler in main.rs using `symbol_at_cursor` + reference queries
- [ ] Wire `TestWorkspace::references_for()` to call the db queries directly

**Files to modify**:
- `crates/smelt-db/src/lib.rs` — reference queries (pure functions + Salsa wrappers)
- `crates/smelt-db/src/type_inference.rs` — find_cte_references
- `crates/smelt-lsp/src/main.rs` — textDocument/references handler
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
- [ ] `cargo test -p smelt-cli --test example_diagnostics`

---

## Phase 3: Quick-Fix Code Actions — CAST Fixes `[ ]`

**Priority**: High — low-hanging fruit with immediate user value.

**Goal**: Implement code action handler + quick-fixes for type-related diagnostics (CAST suggestions).

**Red tests (write first)**:
- [ ] `test_code_action_cast_for_type_mismatch` — model with `SUM(varchar_col)` gets code action offering `CAST(varchar_col AS INTEGER)`
- [ ] `test_code_action_cast_for_unknown_type` — model with uninferrable column gets multiple CAST options (VARCHAR, INTEGER, TIMESTAMP, etc.)
- [ ] `test_code_action_cast_wraps_expression` — CAST action wraps the full expression range, not just the column name
- [ ] `test_no_code_action_on_valid_code` — model with no diagnostics returns no code actions

**Green implementation**:
- [ ] Implement `textDocument/codeAction` handler skeleton in main.rs
- [ ] Match diagnostics by `code` field (from Phase 0)
- [ ] For `TypeMismatch`: generate `CAST({expr} AS {expected_type})` wrapping the diagnostic range
- [ ] For `CannotInferType`: generate multiple actions, one per common type (VARCHAR, INTEGER, BIGINT, DOUBLE, BOOLEAN, DATE, TIMESTAMP)
- [ ] Return `CodeAction` with `kind: QuickFix`, `WorkspaceEdit` with `TextEdit`

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — codeAction handler, CAST quick-fixes
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
- [ ] Manual: type mismatch diagnostic in VSCode shows CAST quick-fix

---

## Phase 4: Quick-Fix Code Actions — Create Model, Add Source/Column `[ ]`

**Priority**: Medium — YAML editing is the trickiest part.

**Goal**: Quick-fixes for undefined-ref, undefined-source, and undeclared-column diagnostics.

**Red tests (write first)**:
- [ ] `test_code_action_create_missing_model` — undefined ref gets "Create model 'foo'" action that produces CreateFile + skeleton SQL
- [ ] `test_code_action_add_source_to_yaml` — undefined source `raw.newtable` gets action that inserts table entry in sources.yml
- [ ] `test_code_action_add_source_new_section` — undefined source with unknown source name gets action that adds full source block
- [ ] `test_code_action_add_column_to_yaml` — undeclared column on source qualifier gets action adding column to sources.yml
- [ ] `test_yaml_insertion_preserves_structure` — sources.yml edits have correct indentation and don't corrupt existing content

**Green implementation**:
- [ ] Create model: `WorkspaceEdit` with `CreateFile` + `TextEdit` inserting skeleton SQL
- [ ] Add source/table to YAML: line-scanning to find insertion point (extends `find_source_table_line` pattern, main.rs:53-102)
- [ ] Add column to YAML: line-scanning to find table's column section, insert `- name: {col}` with correct indentation
- [ ] Use `DocumentChanges` (not `changes` map) to support `CreateFile` operations
- [ ] Implement `codeAction/resolve` for YAML-editing actions (lazy resolution)

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — code actions for create/add, YAML insertion logic, codeAction/resolve
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
- [ ] Manual: "Undefined model reference" shows "Create model" quick-fix in VSCode

---

## Phase 5: Rename CTE (Single-File) `[ ]`

**Priority**: Medium — simplest rename, validates the rename infrastructure.

**Goal**: Implement `textDocument/rename` and `textDocument/prepareRename` for CTE names.

**Red tests (write first)**:
- [ ] `test_prepare_rename_cte_definition` — cursor on CTE name in WITH clause returns valid range
- [ ] `test_prepare_rename_cte_reference` — cursor on CTE name in FROM returns valid range
- [ ] `test_prepare_rename_rejects_keyword` — cursor on SQL keyword returns error
- [ ] `test_rename_cte_updates_definition_and_references` — renaming CTE updates all occurrences in the file
- [ ] `test_rename_cte_with_qualified_columns` — CTE used as qualifier (`cte.col`) gets qualifier updated
- [ ] `test_rename_cte_validates_identifier` — new name must be valid SQL identifier

**Green implementation**:
- [ ] Implement `textDocument/prepareRename` handler using `symbol_at_cursor`
- [ ] Implement `textDocument/rename` handler — for `CteDefinition`/`CteReference`: use `find_cte_references` from Phase 2 + `Cte::name_range()` from Phase 1
- [ ] Validate new name is a valid SQL identifier (alphanumeric + underscore, doesn't start with digit)
- [ ] Return `WorkspaceEdit` with `TextEdit`s for all occurrences in the single file

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — prepareRename, rename handlers
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
- [ ] Manual: rename CTE in VSCode works (F2 on CTE name)

---

## Phase 6: Rename Model (Cross-File) `[ ]`

**Priority**: High value — the most requested rename operation.

**Goal**: Rename a model by renaming the .sql file and updating all `ref('old_name')` calls across the project.

**Red tests (write first)**:
- [ ] `test_prepare_rename_model_from_ref` — cursor inside `ref('model')` returns valid range
- [ ] `test_rename_model_updates_all_refs` — 3 downstream models referencing 'old' all get updated to 'new'
- [ ] `test_rename_model_includes_file_rename` — WorkspaceEdit contains RenameFile operation
- [ ] `test_rename_model_ref_content_range` — only the string content inside quotes changes (not the quotes themselves)
- [ ] `test_rename_model_no_conflict` — rename to existing model name is rejected

**Green implementation**:
- [ ] In `textDocument/rename`: for `RefCall` symbol, use `db.model_references(name)` from Phase 2
- [ ] For each ref site: `TextEdit` replacing content inside quotes (using `RefCall::content_range()`)
- [ ] Add `RenameFile { old_uri, new_uri }` to `DocumentChanges`
- [ ] Validate: no existing model with the new name
- [ ] Use `DocumentChanges` variant of `WorkspaceEdit` (required for `RenameFile`)

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — rename handler for models
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
- [ ] Manual: F2 on `ref('model_name')` in VSCode renames file + all refs

---

## Phase 7: Rename Source Table (Cross-File + YAML) `[ ]`

**Priority**: Medium — completes the rename story for non-column symbols.

**Goal**: Rename a source table by updating all `source('src.old_table')` calls and the sources.yml entry.

**Red tests (write first)**:
- [ ] `test_prepare_rename_source_from_call` — cursor inside `source('raw.users')` returns valid range
- [ ] `test_rename_source_table_updates_all_calls` — 2 models using `source('raw.old')` both get updated
- [ ] `test_rename_source_table_updates_yaml` — sources.yml table key is renamed
- [ ] `test_rename_source_table_yaml_preserves_columns` — columns under the renamed table are preserved

**Green implementation**:
- [ ] In `textDocument/rename`: for `SourceCall` symbol, use `db.source_references(qualified_name)` from Phase 2
- [ ] For each source site: `TextEdit` replacing table name in the source() call string
- [ ] Find table key in sources.yml via line scanning, produce `TextEdit` for YAML rename
- [ ] Return `WorkspaceEdit` with edits across all files + sources.yml

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — rename handler for source tables
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)

---

## Phase 8: Rename Column (Full Lineage Tracing) `[ ]`

**Priority**: Hardest — cross-model lineage through SELECT * chains.

**Goal**: Rename a column across the full model graph, tracing through wildcards and explicit references.

**Red tests (write first)**:
- [ ] `test_rename_column_single_model` — rename `user_id` in SELECT list updates WHERE/GROUP BY/ORDER BY in same model
- [ ] `test_rename_column_propagates_upstream` — column from `ColumnSource::FromModel` traced to upstream model and renamed there too
- [ ] `test_rename_column_propagates_downstream` — downstream model using `ref('model').col` gets updated
- [ ] `test_rename_column_through_select_star` — upstream renames column, downstream `SELECT *` is unaffected but downstream explicit `col` refs are found
- [ ] `test_rename_column_through_cte_chain` — column flows through 3 CTEs, all renamed
- [ ] `test_rename_column_source_updates_yaml` — column from source updates sources.yml column name
- [ ] `test_rename_column_ambiguous_rejected` — unqualified column matching multiple sources is rejected with error

**Green implementation**:
- [ ] Add `find_column_references(db, path, qualifier, column_name) -> Vec<(PathBuf, TextRange)>` pure function
- [ ] Trace upward: follow `ColumnSource::FromModel { model_name, column_name }` recursively to the definition site
- [ ] Trace downward: for each file in `db.all_files()`, check if it refs this model and uses this column (via `model_input_constraints`)
- [ ] Trace through wildcards: `RowExtension` in `ModelSchema` means downstream `SELECT *` passes the column through — follow the chain
- [ ] CTE tracing: walk CTE chain within the file
- [ ] Source column: find and rename in sources.yml via line scanning
- [ ] Depth limit (10) to prevent infinite loops on circular refs

**Files to modify**:
- `crates/smelt-db/src/lib.rs` — find_column_references pure function + Salsa query
- `crates/smelt-lsp/src/main.rs` — rename handler for columns
- `crates/smelt-lsp/tests/integration.rs` — red tests

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets`
- [ ] `cargo test` (all pass)
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
