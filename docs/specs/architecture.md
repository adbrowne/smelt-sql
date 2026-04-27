---
feature: architecture
status: stable
last_reviewed: 2026-04-27
owners: [andrew]
---

# Architecture

> **Scope.** The system-level spec for smelt as a SQL-to-SQL compiler and orchestrator. Defines the compilation pipeline, crate boundaries, and the architectural invariants that all feature specs depend on. Feature specs (e.g., `incremental_models.md`, `lsp.md`) sit on top of this one.

## Surface

The system's surface is the contract between crates and the artifacts users interact with.

### Compilation pipeline

User-visible flow from source to executed SQL:

```
.sql / .py files → Parse → Analyze → Plan (optional) → Generate → Execute
```

Stages and their producers:

| Stage    | Crate                                          | Output                                |
|----------|------------------------------------------------|---------------------------------------|
| Parse    | `smelt-parser`                                  | Rowan `SyntaxNode` (CST)             |
| Analyze  | `smelt-db` (Salsa)                              | Refs, types, diagnostics keyed by file |
| Plan     | `smelt-planner`                                 | `Vec<Transformation>` + analyses     |
| Generate | `smelt-dialect`                                 | SQL `String` per dialect              |
| Execute  | `smelt-backend-*` (`-duckdb`, `-spark`)        | `ExecutionResult` (Arrow `RecordBatch`) |

### Crate responsibilities

Each crate has a single job; downstream crates depend on it but never the reverse.

| Crate                  | Sync/Async         | Owns                                                         |
|------------------------|--------------------|--------------------------------------------------------------|
| `smelt-types`          | sync               | `DataType`, `TypedColumn` — SQL type vocabulary              |
| `smelt-parser`         | sync               | Lexer + recursive-descent parser; Rowan CST; AST wrappers    |
| `smelt-core`           | sync               | `Config`, `ModelFile`, `DependencyGraph`, Python discovery   |
| `smelt-db`             | sync               | Salsa queries: `parse_file`, `model_refs`, `resolve_ref`, `file_diagnostics`, `model_schema` |
| `smelt-dialect`        | sync, lightweight  | `SqlDialect`, `BackendCapabilities`, dialect-aware printer   |
| `smelt-planner`        | sync               | Model-graph transforms; rule-based optimization              |
| `smelt-lsp`            | async (tower-lsp)  | Thin async shell over sync Salsa queries                     |
| `smelt-cli`            | sync (entry point) | CLI surface, model discovery, dialect selection              |
| `smelt-backend`        | async              | `Backend` trait, `ExecutionResult`                           |
| `smelt-backend-duckdb` | async              | DuckDB execution                                             |
| `smelt-backend-spark`  | async              | Spark execution                                              |
| `smelt-state`          | sync               | `RunManifest`, `IntervalStore`, `FileStore`                  |
| `smelt-ui`             | async              | Web dashboard, run execution, WebSocket streaming            |

### `Transformation` and `ExecutionStep` (planner output)

The planner outputs values, never mutations. The user-visible enum surface:

```rust
enum Transformation {
    CreateModel { name, sql, materialization },
    RedirectRef { model, old_ref, new_ref },
    RemoveModel { name },
    SetMaterialization { model, materialization },
    ReplaceWithPlan { model, steps: Vec<ExecutionStep> },
}

enum ExecutionStep {
    CreateTemp { name, sql },
    AppendToTemp { name, sql },
    FinalQuery { sql },
    DropTemp { name },
}
```

## Semantics

### Stage rules

1. **Parse** is total: every input produces a CST. Invalid input produces `ERROR` nodes; the parser does not abort.
2. **Analyze** is incremental: Salsa recomputes only affected queries when a file changes. Analysis must not mutate the CST.
3. **Plan** is optional and additive: the planner produces a `Vec<Transformation>`; it does not mutate existing CSTs. New SQL it emits is parsed normally by `smelt-parser`.
4. **Generate** is single-pass over the CST: each rewrite is a match arm in the recursive walk. Tokens not matched by a rewrite arm are emitted verbatim.
5. **Execute** is the only async stage. All cross-engine I/O happens here.

### Identity property (printer)

For the native dialect (DuckDB, which supports the full smelt superset), the printer must emit SQL byte-identical to the input modulo smelt-extension resolution (`smelt.ref()` → `schema.model_name`). This is property-testable.

### Salsa purity rule (analysis)

Analysis logic in `smelt-db` (type inference, schema extraction, diagnostic checks) is implemented as **pure functions** that take AST nodes and plain data structures. Salsa queries are thin wrappers that build inputs, call the pure function, and return the result. This invariant exists so a future `smelt-check` crate can do batch compilation without Salsa as a mechanical extraction.

### Planner scope

The planner handles cross-model and execution-shape transforms only:

- **In scope**: shared materialization, model fusion, ref redirection, incremental detection, query splitting, temporal/batch-safety analysis.
- **Out of scope**: predicate pushdown, join reordering, cost-based optimization within a single query — these are the backend engine's job.

The planner's `detect` phase is sync and side-effect-free; the LSP may call it to surface code-action suggestions.

## Constraints & Invariants

These are normative and must be upheld across all features.

1. **Rowan CST is the single representation.** No intermediate IR (no DataFusion `LogicalPlan` analogue). The same CST flows from parse to generation.
2. **`smelt-db` analysis logic is pure.** Pure functions take AST + data; Salsa queries wrap them. Analysis never calls another Salsa query *inside* the pure function — inputs are gathered by the wrapper. Current acceptable exceptions: `file_diagnostics()` and `type_context()` orchestrate Salsa to gather inputs before calling the pure check.
3. **CSTs are not mutated.** Planner output is `Transformation` values. Generated SQL is a `String`. The original CST is unchanged.
4. **Sync core, async edges.** Parsing, analysis, planning, and printing are sync. Async is only at execution and at the LSP shell.
5. **`smelt-dialect` is lightweight.** No Arrow / Tokio / DuckDB dependencies. The LSP and CLI link it freely; backends do not flow back through it.
6. **No circular crate dependencies.** The dependency graph in the Surface table is total order modulo `smelt-types` (root) and `smelt-backend-*` (leaves).
7. **Parser produces a usable CST on invalid input.** No panics, no aborts, no truncated trees on syntax errors.

## Known Divergences / Open Questions

Update as part of any plan that touches architecture.

- **LSP dialect diagnostics are planned but not implemented.** `smelt-dialect` is in place; the LSP does not yet emit "QUALIFY will be rewritten" hints. Add to a future plan.
- **`smelt-check` crate not yet extracted.** The Salsa purity rule is currently upheld by convention; nothing prevents a regression. Once `smelt-check` is extracted, it becomes structurally enforced.
- **Planner cost estimation is future work.** Current rules are deterministic detectors with no statistics input.
- **Python model discovery** (`smelt-core` extracting SQL from `@model` decorators) is via subprocess/PyO3 — interface details are still in flux; no spec yet.

## References

- **Code**:
  - `crates/smelt-types/src/lib.rs`
  - `crates/smelt-parser/src/{lexer.rs, parser.rs, ast.rs}`
  - `crates/smelt-db/src/{lib.rs, type_inference.rs, schema.rs}`
  - `crates/smelt-dialect/src/{lib.rs, printer.rs, capabilities.rs}`
  - `crates/smelt-planner/src/lib.rs`
  - `crates/smelt-lsp/src/lib.rs`
  - `crates/smelt-backend/src/lib.rs` (trait), `crates/smelt-backend-{duckdb,spark}/src/lib.rs`
- **Tests**: dialect printer identity tests under `crates/smelt-dialect/tests/`; pure-function tests in `crates/smelt-db/tests/type_property_tests.rs`
- **User docs**: `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/developing/architecture.md`
- **Plans (history)**: see `docs/plans/` for area-specific implementation work
- **Related specs**: feature specs under `docs/specs/` extend this one (e.g., `incremental_models.md`)
- **Legacy reference (will thin out)**: `docs/architecture_overview.md` — superseded by this spec
