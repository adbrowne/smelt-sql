# Research: Diagnostics for Undeclared Column References

**Date**: 2026-04-03
**Topic**: Adding LSP diagnostics when SQL references columns not declared in sources.yml or upstream models
**Branch**: main
**Commit**: 5f5b347

## Summary

The LSP currently has no diagnostic for column references that can't be resolved against declared schemas. In `session_rollup.sql`, 8 of 12 referenced columns aren't in `sources.yml`, but the LSP shows no warnings. The infrastructure to detect this already exists: `TypeContext` has a `missed_lookups` mechanism, and a visitor pattern (`walk_select_columns_with_visitor`) walks all column references with text ranges. The main work is wiring these together into `file_diagnostics()`.

## Motivating Example

`examples/timeseries/models/session_rollup.sql` references columns like `visitor_id`, `product_views`, `visit_source`, `platform`, etc. from `smelt.source('raw.sessions')`. But `sources.yml:49-60` only declares 4 columns for `raw.sessions`:

```yaml
sessions:
  columns:
    - name: session_id
    - name: user_id
    - name: session_start
    - name: session_end
```

No diagnostic flags this. Goto-definition also silently fails for these columns.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-db/src/lib.rs` | `file_diagnostics()` — diagnostic entry point | L384-650 |
| `crates/smelt-db/src/lib.rs` | `type_context()` — builds column/alias context | L917-991 |
| `crates/smelt-db/src/lib.rs` | `check_expression_types()` — existing expr checks | L579-626 |
| `crates/smelt-db/src/lib.rs` | Ambiguous column diagnostic (existing pattern) | L524-551 |
| `crates/smelt-db/src/type_inference.rs` | `TypeContext` struct with `missed_lookups` | L15-29 |
| `crates/smelt-db/src/type_inference.rs` | `lookup_column()` records missed lookups | L135-143 |
| `crates/smelt-db/src/type_inference.rs` | `take_missed_lookups()` retrieves failures | L203-208 |
| `crates/smelt-db/src/type_inference.rs` | `walk_select_columns_with_visitor()` | L1215-1264 |
| `crates/smelt-db/src/type_inference.rs` | `walk_expression_columns_with_visitor()` | L1095-1204 |
| `crates/smelt-lsp/src/main.rs` | `find_column_in_sources()` — goto-def column lookup | L451-531 |

## Existing Infrastructure

### 1. TypeContext::missed_lookups

`TypeContext` already tracks failed column lookups (`type_inference.rs:28`):

```rust
missed_lookups: Mutex<Vec<(Option<String>, String)>>,
```

Every call to `lookup_column()` that returns `None` appends `(qualifier, column_name)` to this list (`type_inference.rs:138-140`). The `take_missed_lookups()` method (`type_inference.rs:203-208`) drains and returns the list. This is currently used only by property-based tests.

### 2. Column Visitor Pattern

`walk_select_columns_with_visitor()` (`type_inference.rs:1215-1264`) walks all column references in a SELECT statement — covering SELECT list, WHERE, GROUP BY, HAVING, QUALIFY, JOIN ON, and ORDER BY. For each column reference, it invokes a callback with:

```rust
(qualifier: Option<&str>, col_name: &str, type_hint: Option<&TypedColumn>, range: TextRange)
```

The `range: TextRange` gives exact source positions for diagnostic placement.

Internally, each column reference is resolved via `ctx.lookup_column()` (`type_inference.rs:1107-1115`), which records missed lookups.

### 3. Existing Column Diagnostic Pattern

There's already a column-level diagnostic for ambiguous unqualified columns (`lib.rs:524-551`):

```rust
if from_sources > 1 {
    for item in select_list.items() {
        if let Some(col_ref) = expr.as_column_ref() {
            if col_ref.qualifier().is_none() {
                diagnostics.push(Diagnostic { ... });
            }
        }
    }
}
```

This shows the pattern: walk expressions, extract column refs, emit diagnostics with ranges.

### 4. Current Diagnostic Scope

`file_diagnostics()` (`lib.rs:384-650`) currently validates:
- Parse errors (L389-399)
- Undefined model references — `smelt.ref('X')` where X doesn't exist (L422-431)
- Undefined source references — `smelt.source('X.Y')` where X.Y doesn't exist (L434-450)
- Source YAML parse errors (L453-483)
- Malformed source calls — missing dot separator (L487-510)
- Invalid CAST types (L579-605 via `check_expression_types()`)
- Unknown SQL functions (L608-625)
- Ambiguous unqualified columns with multiple FROM sources (L524-551)

**Not validated:** Individual column names against declared schemas.

## How Column Resolution Works

When `type_context()` is built (`lib.rs:917-991`):

1. **Source columns** registered from `sources.yml` (`lib.rs:922-939`): Keys like `raw.sessions.session_id` and `sessions.session_id`
2. **Model columns** registered from upstream `smelt.ref()` schemas (`lib.rs:1019-1031`): Keys like `model_name.col_name`
3. **CTE columns** inferred from CTE queries (`lib.rs:974-978`): Keys like `cte_name.col_name`
4. **Aliases** registered for each table ref (`lib.rs:1034-1036, 1050-1055`): e.g., `sessions` → `raw.sessions`

`lookup_column_inner()` (`type_inference.rs:145-199`) resolves by checking:
- CTE columns first (CTEs shadow outer scope)
- Then model columns
- Then source columns
- For unqualified names, searches all scopes

## Design Approach

### Philosophy: Strictness by default

smelt is an ETL tool. The whole point is to catch errors before they hit production. Column references against sources are a contract boundary — if a column isn't declared in `sources.yml`, the pipeline can't guarantee correctness. This should be an **Error**, not a Warning.

This is analogous to how a typed language treats undeclared variables: you must declare what you depend on. `sources.yml` is the schema contract for external data, and smelt should enforce it strictly.

### Severity: Error

Undeclared column references against sources should be **Error** severity. Rationale:
- Sources are the boundary between smelt and the outside world — the schema must be explicit
- Silent acceptance of undeclared columns defeats the purpose of having `sources.yml`
- Catching typos and schema drift early is a core value proposition of smelt over dbt
- Strictness here prevents broken pipelines from being deployed

### What constitutes an "undeclared column"

A column reference should be flagged when:
1. The column name can't be resolved in any in-scope source, model, or CTE
2. AND the qualifier (if present) resolves to a known source/model/CTE (i.e., the table exists but the column doesn't)

If the qualifier itself is unknown, that's a different error (unknown table reference).

### Sources with no column declarations

If a source table in `sources.yml` has an empty or missing `columns:` list, every column reference to that source would be flagged. This is the correct behavior — if you're using a source, you should declare its columns. This enforces the schema contract.

### Edge cases

1. **`SELECT *`**: Wildcard selects don't reference specific columns — skip these
2. **Aggregate functions**: `COUNT(*)` has no column reference — already handled by parser
3. **Expressions with multiple columns**: `a + b` — each column checked independently
4. **Subquery columns**: Columns from inline subqueries are inferred, not declared — no validation needed
5. **Model refs**: Model schemas are inferred from SELECT lists, so missing columns there also indicate real bugs (typo or upstream schema change) — same Error severity

### Future: LSP quick fix

A natural follow-up is an LSP code action (quick fix) that offers to add the missing column to `sources.yml`. This would make the strict error easy to resolve — click the lightbulb, pick a type, and the column is added. This is deferred for now but should be straightforward given that the diagnostic already identifies the source name, table name, and column name.

## Test Coverage

Existing tests in `crates/smelt-lsp/tests/integration.rs`:
- `column_goto_definition` module (L1153) — tests column ref resolution but only for columns that exist
- No tests for the diagnostic when columns are missing
- No tests for the `missed_lookups` mechanism in a diagnostic context
- `crates/smelt-db/tests/type_property_tests.rs` — uses `take_missed_lookups()` for test column discovery

## Decisions

1. **Diagnostic message should name the source.** E.g., "Column 'visit_source' not found in source 'raw.sessions'". More actionable than a generic "Undefined column" message.

2. **Fix existing examples.** The timeseries `sources.yml` (and any other examples) must be updated to declare all columns used by their models. This demonstrates the strictness catching a real problem.