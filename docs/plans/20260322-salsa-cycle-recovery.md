# Salsa Cycle Recovery for Circular Refs

**Date:** 2026-03-22

## Context

When model A references model B via `smelt.ref('model_b')` and model B references model A via `smelt.ref('model_a')`, the Salsa incremental computation framework panics with an unrecoverable cycle error. This happens because:

1. `type_context` for model A calls `db.resolved_model_schema(model_b_path)`
2. `resolved_model_schema` for model B calls `db.typed_model_schema(model_b_path)`
3. `typed_model_schema` for model B calls `db.type_context(model_b_path)`
4. `type_context` for model B calls `db.resolved_model_schema(model_a_path)`
5. This cycles back to step 1

The cycle can also be indirect (A -> B -> C -> A) or involve any combination of `resolved_model_schema`, `typed_model_schema`, and `type_context`.

Salsa 0.16 provides `#[salsa::cycle(recovery_fn)]` attributes on query group methods to handle cycles gracefully instead of panicking. When Salsa detects a cycle, it calls the recovery function which returns a fallback value.

## Key Files

- `crates/smelt-db/src/lib.rs` — Salsa query group definitions and implementations for `TypeChecking` trait (lines ~111-130), `type_context` (line 832), `typed_model_schema` (line 1023), `resolved_model_schema` (line 1068), `type_diagnostics` (line 1471)
- `crates/smelt-db/src/schema.rs` — `ModelSchema`, `ResolvedSchema`, `TypeContext` types (already have `empty()` / `Default` impls)
- `crates/smelt-db/src/type_inference.rs` — `TypeContext` definition (already implements `Default`)

## Implementation Steps

### 1. Add cycle recovery functions

Add three recovery functions in `crates/smelt-db/src/lib.rs`:

```rust
fn type_context_recover(
    _db: &dyn TypeChecking,
    _cycle: &salsa::Cycle,
    _path: PathBuf,
) -> Arc<TypeContext> {
    // Return empty context — upstream types will be Unknown
    Arc::new(TypeContext::new())
}

fn typed_model_schema_recover(
    db: &dyn TypeChecking,
    _cycle: &salsa::Cycle,
    path: PathBuf,
) -> Arc<ModelSchema> {
    // Fall back to the untyped schema (no upstream type info)
    db.model_schema(path)
}

fn resolved_model_schema_recover(
    db: &dyn TypeChecking,
    _cycle: &salsa::Cycle,
    path: PathBuf,
) -> Arc<ResolvedSchema> {
    // Return schema with columns but unresolved extensions
    let base = db.model_schema(path);
    Arc::new(ResolvedSchema {
        columns: base.columns.clone(),
        is_fully_resolved: false,
        unresolved_extensions: base.row_extensions.clone(),
    })
}
```

### 2. Annotate query group methods with `#[salsa::cycle(...)]`

In the `TypeChecking` query group definition (~line 111), add cycle recovery attributes:

```rust
#[salsa::query_group(TypeCheckingStorage)]
pub trait TypeChecking: Schema {
    #[salsa::cycle(typed_model_schema_recover)]
    fn typed_model_schema(&self, path: PathBuf) -> Arc<ModelSchema>;

    #[salsa::cycle(type_context_recover)]
    fn type_context(&self, path: PathBuf) -> Arc<TypeContext>;

    #[salsa::cycle(resolved_model_schema_recover)]
    fn resolved_model_schema(&self, path: PathBuf) -> Arc<ResolvedSchema>;

    // ... remaining methods unchanged
}
```

### 3. Add circular dependency diagnostics

Extend `file_diagnostics` in `crates/smelt-db/src/lib.rs` to detect circular ref chains and report them. The simplest approach: after resolving all refs for a model, do a DFS cycle check across the model dependency graph.

Add a helper function:

```rust
fn detect_ref_cycle(db: &dyn Semantic, start_path: &PathBuf) -> Option<Vec<String>> {
    // DFS from start_path following model_refs -> resolve_ref edges
    // Returns Some(cycle_path) if a cycle is found, None otherwise
}
```

Call this from `file_diagnostics` and emit an error diagnostic like:

> Circular dependency detected: model_a -> model_b -> model_a

The diagnostic should be attached to the `smelt.ref()` call site that closes the cycle, using the `RefLocation` range for accurate positioning.

### 4. Add tests

Add tests in the `#[cfg(test)] mod tests` section of `crates/smelt-db/src/lib.rs`:

**Test 1: Direct cycle (A refs B, B refs A)**
- Set up two model files in the Salsa database
- Call `typed_model_schema` on model A — should NOT panic
- Verify it returns a schema (with Unknown types for upstream columns)
- Verify `file_diagnostics` reports the circular dependency

**Test 2: Indirect cycle (A -> B -> C -> A)**
- Set up three model files
- Verify no panic and diagnostics are produced for all three models

**Test 3: Self-referencing model (A refs A)**
- Set up one model that refs itself
- Verify no panic and diagnostic is produced

**Test 4: Non-cyclic models still work correctly**
- Ensure existing behavior is preserved (A -> B with no cycle still infers types)

## Verification

```bash
# Ensure it compiles
cargo build

# Run all tests (including new cycle recovery tests)
cargo test -p smelt-db

# Clippy and format checks
cargo clippy --all-targets
cargo fmt --all -- --check
```

## Risks and Considerations

- **Salsa 0.16 cycle recovery signature**: The recovery function signature must exactly match what Salsa 0.16 expects: `fn(db: &dyn TraitName, cycle: &salsa::Cycle, ...key_args) -> ReturnType`. Verify against Salsa 0.16 docs/source if the build fails.
- **Fallback schema quality**: When a cycle is recovered, columns from the cyclic upstream will have `Unknown` types. This is acceptable — the diagnostic tells the user about the cycle, and the LSP remains responsive.
- **`model_schema` must not cycle**: The `model_schema` query (in `SchemaStorage`) only looks at the model's own SQL text and does not follow refs, so it is safe to call from recovery functions without risking a nested cycle.
- **Performance**: Cycle detection in `file_diagnostics` adds a DFS traversal per model. For typical project sizes (hundreds of models), this is negligible.
