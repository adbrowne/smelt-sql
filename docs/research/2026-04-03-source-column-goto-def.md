# Research: Go-to-Definition for Source Columns in SELECT List

**Date**: 2026-04-03
**Topic**: Why goto-definition on bare column names (e.g., `event_timestamp`) in a SELECT list doesn't navigate to the column definition in `sources.yml`
**Branch**: main
**Commit**: 6cd5690

## Summary

Go-to-definition on bare column names in SELECT lists (like `event_timestamp` in `events.sql`) fails silently. The root cause is a bug in the goto-definition handler's expression-finding logic: when a bare identifier has the same text range as its parent `SELECT_ITEM` node, the `SELECT_ITEM` is incorrectly selected as the "tightest expression" because it's visited first in the pre-order traversal and the comparison uses strict `<` (not `<=`). The `SELECT_ITEM`-backed `Expr` then fails `as_column_ref()` because IDENT tokens aren't direct children of `SELECT_ITEM`. The downstream `find_column_in_sources` function is correct and would resolve the column if reached.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-lsp/src/main.rs` | Goto-definition handler + column resolution | L1537-1916, L183-532 |
| `crates/smelt-parser/src/ast.rs` | `Expr::cast` with fallback branch, `ColumnRef::from_expr` | L551-594, L728-761 |
| `crates/smelt-parser/src/parser.rs` | `parse_select_item` and `parse_primary_expr` | L412-433, L1246-1304 |
| `crates/smelt-lsp/tests/integration.rs` | Test coverage (no column goto-def tests) | L206-248, L898-1097 |

## Architecture & Data Flow

The goto-definition handler (`main.rs:1537`) follows this chain:

```
LSP request → cursor position → byte offset → find tightest Expr → check column ref → resolve_column_definitions
                                                     ↓                                        ↓
                                            Expr::cast on descendants             find_column_in_sources
                                                     ↓                                        ↓
                                            as_column_ref()                       resolve_source + find_source_column_line
                                                     ↓                                        ↓
                                            ColumnRef { qualifier, name }         ColumnDefLocation in sources.yml
```

## Current Behavior

### The Bug: `SELECT_ITEM` Selected Instead of `EXPRESSION`

For `events.sql`:
```sql
SELECT
    event_timestamp
FROM smelt.source('raw.events')
```

The AST structure is:
```
SELECT_ITEM          (range: "event_timestamp")
  EXPRESSION (outer) (range: "event_timestamp")  ← same range as parent
    EXPRESSION (inner)(range: "event_timestamp")  ← same range
      IDENT "event_timestamp" (token)
```

The handler at `main.rs:1720-1831` walks `file.syntax().descendants()` to find the tightest `Expr`:

```rust
for node in file.syntax().descendants() {
    if let Some(expr) = Expr::cast(node) {
        let len = end - start;
        if cursor_offset >= start && cursor_offset <= end && len < best_len {
            best_len = len;
            best_expr = Some(expr);
        }
    }
}
```

**Problem**: `descendants()` yields nodes in pre-order (parent before children). Since `SELECT_ITEM`, outer `EXPRESSION`, and inner `EXPRESSION` all have the **exact same text range** (no whitespace padding inside `SELECT_ITEM` for bare identifiers), the comparison `len < best_len` (strict less-than) means:

1. `SELECT_ITEM` is visited first → `Expr::cast` succeeds via the `_ =>` fallback branch (it has an EXPRESSION child) → becomes `best_expr` with len=15
2. Outer `EXPRESSION` → `Expr::cast` returns inner EXPRESSION with same len=15 → `15 < 15` is false → **does NOT replace**
3. Inner `EXPRESSION` → same len → **does NOT replace**

So `best_expr` = `Expr(SELECT_ITEM)`.

Then `as_column_ref()` → `ColumnRef::from_expr()` looks for IDENT/DOT tokens among **direct** `children_with_tokens()` of `SELECT_ITEM`. The IDENT token is nested inside EXPRESSION children (not a direct child of SELECT_ITEM) → `tokens` is empty → returns `None`.

Column resolution code at `main.rs:1819-1828` is **never reached**.

### The Correct Path (if bug were fixed)

If the inner `EXPRESSION` were selected as `best_expr`:
1. `as_column_ref()` → finds IDENT "event_timestamp" as direct child token → `ColumnRef { qualifier: None, name: "event_timestamp" }`
2. `resolve_column_definitions(db, path, None, "event_timestamp")` is called
3. `find_column_in_sources` at `main.rs:451` parses the FROM clause, finds `smelt.source('raw.events')`
4. `db.resolve_source(project_root, "raw", "events")` returns the `SourceTableDef`
5. Column matched: `table_def.columns.iter().any(|c| c.name == "event_timestamp")` → true
6. `find_source_column_line` at `main.rs:106` scans `sources.yml` and returns the line for `- name: event_timestamp`
7. `GotoTarget::ColumnDefs` with location in `sources.yml` is returned to the LSP client

### `Expr::cast` Fallback Branch (root cause)

At `ast.rs:573-593`, the `_ =>` fallback:
```rust
_ => {
    if node.children().any(|n| matches!(n.kind(), EXPRESSION | ...)) {
        Some(Self(node))  // wraps the non-expression node itself
    } else {
        None
    }
}
```

This allows `SELECT_ITEM`, `SELECT_LIST`, and other non-expression nodes to be cast as `Expr` if they contain expression children. The intent is to handle edge cases, but it creates this ambiguity.

## Related Patterns

- **`smelt.ref()` goto-definition** (`main.rs:1611-1625`): Works correctly because it checks RefCall ranges directly (not via Expr walking)
- **`smelt.source()` goto-definition** (`main.rs:1627-1660`): Works correctly for the same reason
- **CTE name goto-definition** (`main.rs:1662-1717`): Works correctly via TableRef iteration
- **Qualifier goto-definition** (`main.rs:1744-1818`): Would have the same bug if triggered, but it's only reached when `as_column_ref()` succeeds

## Test Coverage

- **No tests** for column goto-definition through the full handler flow
- `test_source_column_available_in_context` (`integration.rs:1045`): Tests `TypeContext::lookup_column` with a qualifier — confirms type system works
- `test_column_traced_through_single_ref` (`integration.rs:952`): Tests column tracing through model refs — doesn't test source columns
- `goto_definition` module (`integration.rs:206`): Only tests `resolve_ref` and `model_refs` — no column tests

## Open Questions

1. **Does this bug affect qualified columns too?** For `e.event_timestamp` (with a qualifier), the EXPRESSION node would span `e.event_timestamp` but SELECT_ITEM might also span the same range. Same bug would apply.

2. **Does this affect columns from `smelt.ref()` models?** Yes — the same `best_expr` selection logic applies regardless of the source type. Any bare column reference in a SELECT list would hit this bug.

3. **What about columns in WHERE/GROUP BY/HAVING clauses?** Those are inside different parent nodes (WHERE_CLAUSE, etc.), and the parent node likely has a larger range (includes the keyword), so the EXPRESSION would be strictly smaller and correctly selected.
