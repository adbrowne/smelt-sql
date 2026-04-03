# Plan: Diagnostics for Undeclared Column References

**Date**: 2026-04-03
**Research**: `docs/research/2026-04-03-session-rollup-goto-def.md`
**Status**: Validated

## Overview

Add LSP diagnostics (Error severity) when SQL references columns not declared in `sources.yml` or upstream model schemas. This catches typos and schema drift at edit time — a core value proposition of smelt over dbt. The infrastructure already exists (`TypeContext` column lookup, `walk_select_columns_with_visitor`); the main work is wiring them into `type_diagnostics()` and fixing untyped source column registration.

## Current State

- `type_diagnostics()` (`lib.rs:1474`) runs on the `TypeChecking` trait and already has access to `TypeContext` — this is where the new diagnostic belongs
- `walk_select_columns_with_visitor()` (`type_inference.rs:1215-1264`) walks all column refs in SELECT/WHERE/GROUP BY/HAVING/QUALIFY/JOIN ON/ORDER BY with `TextRange` for each
- `TypeContext::lookup_column()` (`type_inference.rs:135-143`) returns `None` for unresolvable columns
- Source columns are only registered in `TypeContext` when they have a `data_type` (`lib.rs:926`). Columns declared in `sources.yml` without a `type:` field are invisible to lookup
- `session_rollup.sql` references 8 columns (`visitor_id`, `product_views`, `widget_views`, `product_revenue`, `visit_source`, `platform`, `visit_campaign`, `product_category`) not declared in `sources.yml`

## Desired End State

- Any column reference that can't be resolved against declared sources/models/CTEs produces an Error diagnostic with the column name, qualifier, and source/model name
- Source columns declared without a `type:` in `sources.yml` are still visible for resolution (registered with `DataType::Unknown`)
- All example workspaces pass `cargo test -p smelt-cli --test example_diagnostics` (timeseries `sources.yml` updated with missing columns)
- The diagnostic logic is a pure function (per the pure function rule in CLAUDE.md)

## What We're NOT Doing

- LSP quick-fix to add missing columns to `sources.yml` (natural follow-up, deferred)
- Diagnostics for unknown table/qualifier references (different error category)
- Changes to `file_diagnostics()` — this goes in `type_diagnostics()` which already has `TypeChecking` access
- Validation of column types (already handled by cross-model type mismatch checks)

## Implementation Phases

### Phase 1: Register Untyped Source Columns

**Files to modify**:
- `crates/smelt-db/src/lib.rs` — `type_context()` function (~line 922-939)

**Changes**:
1. In the source column registration loop, register columns that have no `data_type` with `DataType::Unknown`. Currently the `if let Some(data_type) = &col.data_type` guard skips them entirely. Add an `else` branch:
   ```rust
   if let Some(data_type) = &col.data_type {
       ctx.add_source_column(&source.name, &table.name, &col.name, TypedColumn {
           data_type: data_type.clone(), nullable: true,
       });
   } else {
       ctx.add_source_column(&source.name, &table.name, &col.name, TypedColumn {
           data_type: DataType::Unknown, nullable: true,
       });
   }
   ```
   This mirrors how model columns already use `Unknown` for untyped columns (`lib.rs:1026`).

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass)
- [x] Columns declared without `type:` in `sources.yml` now resolve in `TypeContext`

### Phase 2: Add Undeclared Column Diagnostic

**Files to modify**:
- `crates/smelt-db/src/type_inference.rs` — add a new pure function `check_undeclared_columns`
- `crates/smelt-db/src/lib.rs` — call it from `type_diagnostics()`

**Changes**:

1. Add a pure function in `type_inference.rs`:
   ```rust
   /// Check for column references that don't resolve against declared schemas.
   /// Returns diagnostics with accurate source positions.
   pub fn check_undeclared_columns(
       select_stmt: &SelectStmt,
       ctx: &TypeContext,
   ) -> Vec<(String, TextRange)> { ... }
   ```
   This function uses `walk_select_columns_with_visitor` to visit every column reference. For each one, it calls `ctx.lookup_column()`. If the lookup returns `None`, it records `(message, range)`. The message should include the qualifier and source/model name when available, e.g., "Column 'visitor_id' not found in source 'raw.sessions'".

2. To produce good messages, `TypeContext` needs a method to check if a qualifier resolves to a known source/model/CTE without looking up a specific column. Add:
   ```rust
   /// Check if a qualifier (table name/alias) resolves to a known source, model, or CTE.
   /// Returns a human-readable description like "source 'raw.sessions'" or "model 'upstream'".
   pub fn describe_qualifier(&self, qualifier: &str) -> Option<String> { ... }
   ```
   This checks aliases, CTE names, and iterates source/model column keys to find a match.

3. In `type_diagnostics()` (`lib.rs:1474`), after the existing Unknown-type checks, add:
   ```rust
   // Check for undeclared column references
   let parse = db.parse_file(path.clone());
   let syntax = parse.syntax();
   if let Some(file) = AstFile::cast(syntax) {
       if let Some(select_stmt) = file.select_stmt() {
           let ctx = db.type_context(path.clone());
           let undeclared = check_undeclared_columns(&select_stmt, &ctx);
           for (message, text_range) in undeclared {
               let range = smelt_parser::ast::text_range_to_range(&text, text_range);
               diagnostics.push(Diagnostic {
                   severity: DiagnosticSeverity::Error,
                   message,
                   range,
               });
           }
       }
   }
   ```

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass)
- [x] A model referencing an undeclared column produces an Error diagnostic with correct range and message

### Phase 3: Update Example Sources and Add Tests

**Files to modify**:
- `examples/timeseries/sources.yml` — declare missing columns for `raw.sessions`
- `crates/smelt-db/src/lib.rs` — add unit tests

**Changes**:

1. Update `examples/timeseries/sources.yml` to declare all columns used by `session_rollup.sql`:
   ```yaml
   sessions:
     columns:
       - name: session_id
         type: INTEGER
       - name: user_id
         type: INTEGER
       - name: session_start
         type: TIMESTAMP
       - name: session_end
         type: TIMESTAMP
       - name: visitor_id
         type: INTEGER
       - name: product_views
         type: INTEGER
       - name: widget_views
         type: INTEGER
       - name: product_revenue
         type: DECIMAL(10,2)
       - name: visit_source
         type: VARCHAR
       - name: platform
         type: VARCHAR
       - name: visit_campaign
         type: VARCHAR
       - name: product_category
         type: VARCHAR
   ```

2. Add unit tests in `lib.rs` (in the existing `#[cfg(test)]` module):
   - `test_undeclared_column_from_source` — model references a column not in `sources.yml`, expect Error diagnostic naming the source
   - `test_declared_column_no_diagnostic` — model references a column that IS in `sources.yml`, expect no diagnostic
   - `test_undeclared_column_from_ref` — model references a column not in upstream model's SELECT, expect Error
   - `test_untyped_source_column_no_diagnostic` — column declared without `type:` in sources, should resolve (no false positive)
   - `test_cte_column_no_false_positive` — column defined in a CTE should not produce a diagnostic
   - `test_select_star_no_diagnostic` — `SELECT *` should not trigger the check

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass)
- [x] `cargo test -p smelt-cli --test example_diagnostics` (all examples clean)
- [x] All new tests pass

## Testing Strategy

- **Unit tests** verify the pure `check_undeclared_columns` function with constructed `TypeContext` + parsed SQL
- **Example diagnostics test** (`cargo test -p smelt-cli --test example_diagnostics`) ensures all example workspaces remain diagnostic-free after updating `sources.yml`
- **Manual LSP test**: Open `session_rollup.sql` before updating `sources.yml` — should see 8 Error diagnostics. After updating, all should clear.

## Risks & Mitigations

- **Risk**: False positives for columns from subqueries, CTEs, or `SELECT *` expansions.
  **Mitigation**: The visitor already skips subqueries (`walk_expression_columns_with_visitor` line 1120). CTE columns are registered in `TypeContext`. `SELECT *` has no column references to walk. Tests cover these cases.

- **Risk**: `type_context()` already calls `type_diagnostics()` could create a Salsa cycle.
  **Mitigation**: `type_diagnostics()` calls `type_context()`, not the other way around. No cycle — `type_context` is an input to `type_diagnostics`.

- **Risk**: Performance impact from walking all column references.
  **Mitigation**: The visitor is already used by `typed_model_schema` for type inference. The additional walk in `type_diagnostics` is O(n) in expression count — negligible for real models.
