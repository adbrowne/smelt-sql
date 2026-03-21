# Plan: PostgreSQL Regex Operators

**Date**: 2026-03-22
**Status**: Proposed
**TODO item**: Add `~`, `~*`, `!~`, `!~*` PostgreSQL regex operators (none covered, type inference also missing)

## Context

The lexer and parser already tokenize and parse the four PostgreSQL regex match operators (`~`, `~*`, `!~`, `!~*`) as `BINARY_EXPR` nodes. However, two downstream layers are incomplete:

1. **AST operator extraction** -- `BinaryExpr::operator()` in `crates/smelt-parser/src/ast.rs` does not map `TILDE`, `TILDE_STAR`, `NOT_TILDE`, or `NOT_TILDE_STAR` tokens to operator strings, so it returns `None` for regex expressions.
2. **Type inference** -- `infer_binary_expr_type()` in `crates/smelt-db/src/type_inference.rs` has no match arms for regex operators. All four always return `Boolean`.
3. **Property test generators** -- `crates/smelt-db/tests/prop_helpers/generators.rs` has no `ExprKind` variant for regex operators, so they are never exercised in property tests.

The parser tests (`test_regex_match`, `test_regex_match_case_insensitive`, `test_regex_not_match`, `test_regex_not_match_case_insensitive`) already confirm the parser produces valid CST nodes. The work is purely in the AST, type inference, and test generator layers.

## Key Files

| File | Change |
|------|--------|
| `crates/smelt-parser/src/ast.rs` | Add four arms to `BinaryExpr::operator()` |
| `crates/smelt-db/src/type_inference.rs` | Add match arm for regex operators returning Boolean |
| `crates/smelt-db/tests/prop_helpers/generators.rs` | Add `RegexOp` variant to `ExprKind`, generate `col ~ 'pattern'` etc. |
| `crates/smelt-parser/src/printer.rs` | (Optional) Add printing support for regex operator tokens if round-trip printing is needed |

## Implementation Steps

### Step 1: AST operator() mapping

In `crates/smelt-parser/src/ast.rs`, method `BinaryExpr::operator()` (line ~767), add four arms after the existing `NOT_KW` arm:

```rust
TILDE => return Some("~".to_string()),
TILDE_STAR => return Some("~*".to_string()),
NOT_TILDE => return Some("!~".to_string()),
NOT_TILDE_STAR => return Some("!~*".to_string()),
```

This unblocks type inference, which matches on the string returned by `operator()`.

### Step 2: Type inference

In `crates/smelt-db/src/type_inference.rs`, function `infer_binary_expr_type()` (line ~818), add a match arm alongside or after the existing comparison operators:

```rust
// Regex match operators - always return Boolean
"~" | "~*" | "!~" | "!~*" => Some(TypedColumn {
    data_type: DataType::Boolean,
    nullable: false,
}),
```

All four operators test whether a string matches a POSIX regex pattern and return true/false, never NULL (assuming non-NULL inputs, consistent with how comparison operators are handled here).

### Step 3: Property test generators

In `crates/smelt-db/tests/prop_helpers/generators.rs`:

1. Add a new variant to `ExprKind`:
   ```rust
   /// Regex match operator (~, ~*, !~, !~*).
   RegexOp,
   ```

2. Add `ExprKind::RegexOp` to the list of variants in `arb_expr_kind()` (the proptest strategy that picks a random expression kind).

3. Add a generation arm in the main `generate_typed_expr()` match:
   ```rust
   ExprKind::RegexOp => {
       // Find a varchar/text column for regex matching
       let str_col = columns.iter().find(|c| matches!(c.data_type, DataType::Varchar { .. } | DataType::Text))?;
       let ops = ["~", "~*", "!~", "!~*"];
       let op = ops[func_idx % ops.len()];
       Some(TypedExpr {
           sql: format!("{} {} '^[A-Z]'", str_col.name, op),
           alias,
           expected_smelt_type: DataType::Boolean,
       })
   }
   ```

   The pattern `'^[A-Z]'` is a safe POSIX regex that DuckDB supports. Using a fixed pattern avoids generating invalid regex strings.

### Step 4: Update TODO

Mark the regex operators item as complete in `docs/TODO.md`.

## Testing

1. **Unit tests** -- Existing parser tests (`test_regex_match` etc.) already pass. After step 1, verify with a new AST-level test that `operator()` returns the correct strings.
2. **Type inference tests** -- Add a test in `type_inference.rs` or existing test module that parses `SELECT col ~ '^A' FROM t` and confirms the inferred type is Boolean.
3. **Property tests** -- Run `cargo test -p smelt-db --test type_property_tests` and confirm regex expressions are generated and pass against DuckDB.
4. **Full suite** -- `cargo clippy --all-targets && cargo test` must pass with no warnings.

## Risks

- **DuckDB regex support**: DuckDB supports POSIX regex via `~` and `~*` (and their negations). This is confirmed by DuckDB documentation. If a future DuckDB version changes behavior, the property test will catch it.
- **NULL semantics**: If either operand is NULL, DuckDB returns NULL for regex operators. We mark `nullable: false` to match how other comparison operators are handled in type inference. If this diverges from DuckDB in property tests, we can adjust to `nullable: true` or add a known divergence.
