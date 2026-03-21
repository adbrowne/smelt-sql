# Plan: Cross-Model Type Mismatch Diagnostics

**Date:** 2026-03-22
**Status:** Proposed
**TODO ref:** "Cross-model type mismatch diagnostics" in `docs/TODO.md`

## Context

Today, `model_input_constraints` extracts what a downstream model *expects* from its upstream refs (e.g., `SUM(amount)` implies `amount` must be numeric), and `typed_model_schema` infers the actual output types of each model. However, these two pieces of information are never compared. If an upstream model outputs `amount` as `VARCHAR` and a downstream model calls `SUM(amount)`, no diagnostic is produced -- the user only discovers the error at query execution time.

This plan adds a new Salsa query that compares input constraints against upstream schemas and surfaces type mismatch warnings in the LSP.

## Key Files

- **`crates/smelt-db/src/lib.rs`** -- Salsa query definitions and implementations. This is where the new query will live, alongside existing `type_diagnostics`.
- **`crates/smelt-db/src/schema.rs`** -- `InputConstraint`, `ColumnConstraint` types (read-only for this work).
- **`crates/smelt-db/src/type_inference.rs`** -- `TypeContext` and inference helpers (read-only).
- **`crates/smelt-types/src/lib.rs`** -- `DataType` enum with `is_numeric()` and other classification methods.
- **`crates/smelt-lsp/src/main.rs`** -- Where diagnostics are collected and published to the editor.

## Design

### New Salsa query: `cross_model_diagnostics`

Add a new query to the `TypeChecking` trait:

```rust
fn cross_model_diagnostics(&self, path: PathBuf) -> Arc<Vec<Diagnostic>>;
```

This query:
1. Calls `model_input_constraints(path)` to get what the downstream model expects.
2. For each `InputConstraint` (one per referenced upstream model):
   a. Resolves the upstream model path via `resolve_ref(constraint.ref_name)`.
   b. Calls `typed_model_schema(upstream_path)` to get the upstream's actual output types.
   c. For each `ColumnConstraint` with an `expected_type`:
      - Looks up the column in the upstream schema.
      - If the column exists and has a known type, checks compatibility.
      - If incompatible (e.g., expected numeric but got VARCHAR), emits a `Warning` diagnostic.

### Type compatibility rules

The initial implementation uses a simple compatibility check. Two types are compatible when:

- Either type is `Unknown` or `Null` (not enough information to warn).
- The expected type is numeric (`is_numeric()`) and the actual type is also numeric.
- The expected type is a string type and the actual type is also a string type.
- The types are equal.

This is intentionally conservative -- we only warn when we are confident there is a mismatch. A helper function `types_compatible(expected: &DataType, actual: &DataType) -> bool` encapsulates this logic.

Add this helper to `crates/smelt-types/src/lib.rs` as a method on `DataType`:

```rust
impl DataType {
    /// Check if `self` is compatible with `other` for assignment/usage purposes.
    /// Returns true if the types are definitely compatible or if we can't be sure.
    /// Returns false only when there is a clear mismatch.
    pub fn is_usage_compatible(&self, other: &DataType) -> bool { ... }
}
```

### Diagnostic format

The diagnostic message should clearly identify both models and the mismatch:

```
Type mismatch: column 'amount' from 'raw_orders' is VARCHAR,
but this model uses it in a numeric context (SUM).
```

Severity: `Warning` (not Error, since runtime coercion may succeed in some backends).

The diagnostic range should point to the first `usage_site` from the `ColumnConstraint`, so the user sees the warning at the point where the column is used in the downstream model.

### LSP integration

In `crates/smelt-lsp/src/main.rs`, the `publish_diagnostics` method already chains `file_diagnostics` and `type_diagnostics`. Add `cross_model_diagnostics` to the chain:

```rust
let cross_diags = db.cross_model_diagnostics(path.clone());

let lsp_diagnostics: Vec<lsp_types::Diagnostic> = diagnostics
    .iter()
    .chain(type_diags.iter())
    .chain(cross_diags.iter())
    .map(|d| self.to_lsp_diagnostic(d))
    .collect();
```

This must be done in both `publish_diagnostics()` (line ~128) and the `did_change` handler (line ~310).

## Implementation Steps

1. **Add `DataType::is_usage_compatible` method** in `crates/smelt-types/src/lib.rs`
   - Add `is_string()` helper alongside existing `is_numeric()`.
   - Implement `is_usage_compatible(&self, other: &DataType) -> bool`.
   - Add unit tests for the compatibility matrix.

2. **Add `cross_model_diagnostics` query** in `crates/smelt-db/src/lib.rs`
   - Declare in `TypeChecking` trait.
   - Implement the query function.
   - Convert `TextRange` usage sites to `Range` (line/column) using `text_range_to_range`.

3. **Wire into LSP** in `crates/smelt-lsp/src/main.rs`
   - Chain `cross_model_diagnostics` in both diagnostic publishing paths.

4. **Add tests** in `crates/smelt-db/src/lib.rs` (in the existing `#[cfg(test)]` module)
   - Test: `SUM(col)` where upstream `col` is `VARCHAR` -- should produce warning.
   - Test: `SUM(col)` where upstream `col` is `INTEGER` -- no warning.
   - Test: upstream column not found (missing column) -- no type mismatch warning (this is a different error class).
   - Test: upstream type is `Unknown` -- no warning (insufficient information).
   - Test: multiple refs with mixed matches.

5. **Add LSP integration test** in `crates/smelt-lsp/tests/integration.rs`
   - Set up a two-model workspace where downstream uses `SUM` on a `VARCHAR` column.
   - Verify the warning diagnostic is published.

## Edge Cases

- **Upstream type is `Unknown`**: Skip the check -- we don't have enough info.
- **Column not found in upstream**: Don't emit a type mismatch diagnostic. This may be handled by a separate "missing column" diagnostic in the future.
- **`expected_type` is `None`**: No constraint to check -- skip.
- **Circular refs**: Salsa handles cycle detection; the query will return empty diagnostics if a cycle is detected.
- **CTE columns**: `model_input_constraints` only tracks ref-based inputs, not CTEs, so CTEs are not affected.
- **`SELECT *` (row extensions)**: Use `resolved_model_schema` instead of `typed_model_schema` when looking up upstream columns, to handle wildcard expansion. However, start with `typed_model_schema` for simplicity and add `resolved_model_schema` support as a follow-up if needed.

## Testing Strategy

- Unit tests in `smelt-db` exercise the Salsa query directly.
- Integration tests in `smelt-lsp` verify end-to-end diagnostic publishing.
- Manual testing with `test-workspace/` by creating a deliberate type mismatch.
- Property tests are not needed for this feature (it is deterministic given fixed schemas).

## Estimated Scope

- ~150 lines of new query logic in `lib.rs`
- ~30 lines in `smelt-types` for compatibility helpers
- ~10 lines in `smelt-lsp` for wiring
- ~200 lines of tests
- Total: ~400 lines, small-to-medium feature
