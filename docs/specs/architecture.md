---
feature: architecture
status: stable
last_reviewed: 2026-04-29
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

### Project layout

A smelt workspace is rooted at a directory containing `smelt.yml`. The standard subdirectories are:

| Path           | Role                                                                              |
|----------------|-----------------------------------------------------------------------------------|
| `models/`      | Model `.sql` files. Each may contain at most one bare model `SELECT` plus zero or more `smelt.define` / `smelt.extern` declarations. |
| `functions/`   | Function `.sql` files. Each may contain zero or more `smelt.define` / `smelt.extern`; a bare model `SELECT` is grammatically allowed but unusual. |
| `seeds/`       | Static CSV inputs (loaded as tables before model execution).                      |
| `tests/`       | Unit-test SQL for models and functions.                                           |
| `sources.yml`  | External-source declarations (schemas for `smelt.source(...)` references).        |
| `smelt.yml`    | Project-level configuration (backend selection, feature flags such as `unstable_schema`). |

**File kind is grammar, not directory.** A `models/foo.sql` may legally contain `smelt.define`s, and a `functions/bar.sql` may legally contain a bare model `SELECT`. Directory placement drives **only** `smelt.fn.*` namespacing: a `smelt.define session_rollup` in `functions/patterns/session_rollup.sql` is callable as `smelt.fn.patterns.session_rollup(...)`. The rest of a file's behaviour comes from the items it contains.

### Unified frontmatter rule

A YAML frontmatter block (between `---` fences) attaches to the **immediately following declaration**: a model `SELECT`, a `smelt.define`, or a `smelt.extern`. Each declaration may carry its own block; there is no file-level frontmatter, and no inheritance across declarations in the same file. (Research §16 #22.)

The frontmatter parser is shared across all three declaration kinds; the parsing contract is identical. Property semantics differ — per-feature key catalogues live in the relevant feature spec:

- **Function and extern keys** (`deterministic`, `idempotent`, `append_only`, `backends`, gated `joins` / `provenance`): see `functions.md`.
- **Model materialization keys** (`materialization`, `incremental`, …): see `incremental_models.md`.

This spec does not duplicate those catalogues; it only fixes the attachment rule and the parser-sharing invariant.

### Models as functions

A smelt **model** is a `.sql` file whose top-level item is a bare `SELECT` consuming `TableExpr` inputs (via `smelt.ref(...)` and `smelt.source(...)`) and producing a `TableExpr` output. A smelt **function** is a `smelt.define` declaration consuming typed fragment inputs and producing a typed fragment output. **They are the same concept with different parameter-binding defaults.**

The model

```sql
-- models/margins.sql
---
materialization: table
---
SELECT revenue - cost AS margin
FROM smelt.ref('product_summary')
```

is equivalent to the parameterised form

```sql
smelt.define margins(
    product_summary: TableExpr = smelt.ref('product_summary')
) -> TableExpr AS (
    SELECT revenue - cost AS margin
    FROM product_summary
)
```

#### Two orthogonal axes

Every transformation in a smelt workspace lies on two independent axes:

| Axis             | Values                                  | What it controls                                                  |
|------------------|-----------------------------------------|-------------------------------------------------------------------|
| **Transparency** | transparent / black-box                 | Whether the planner can see and rewrite across the boundary.      |
| **Materialization** | persisted (table / view / mat-view) / inline (CTE / expansion) | How the output is realised at execution time.        |

The taxonomy of current concepts:

| Concept                    | Transparency | Materialization | Parameters              |
|----------------------------|--------------|-----------------|-------------------------|
| Table / view model         | transparent  | persisted       | DAG-default (refs/sources) |
| Materialized-view model    | transparent  | persisted       | DAG-default             |
| Ephemeral model            | transparent  | inline (CTE)    | DAG-default             |
| `smelt.define` function    | transparent  | inline (expansion) | explicit             |
| `smelt.extern` / built-in  | black-box    | inline          | engine- / user-declared signature |
| Source                     | black-box    | external        | schema from `sources.yml` or catalog |

#### Normative rules

1. **Materialization is orthogonal to transparency.** Choosing `materialization: table` versus `view` versus `ephemeral` does not change whether the body is visible to the planner — every `smelt.define` and every model body is transparent. Materialization controls only how output is persisted and scheduled.
2. **Parameter-binding style is sugar.** The model/function distinction reduces to "DAG-default `TableExpr` parameters" versus "explicitly-passed parameters." A workspace may mix both forms in one declaration: a model with explicit `TableExpr` parameters alongside its DAG-default refs is a **parameterised model** and is well-formed. Callers (or tests) override the defaults at the call site.
3. **`smelt.ref(...)` and `smelt.source(...)` are parameters with DAG-supplied defaults.** Each call inside a model body desugars to a `TableExpr` parameter whose default expression is resolved against the workspace dependency graph (`smelt.ref`) or the source catalogue (`smelt.source`). Tests and parameterised callers may override these defaults; the type-system contract is "any `TableExpr` whose schema satisfies the columns the body actually touches" — row polymorphism, in the same sense as `TableExpr<{…}>` parameters in `functions.md`.
4. **The planner's optimization boundary aligns with transparency, not materialization.** The planner reasons across every transparent boundary — model-to-model and call-to-callee — and treats every black-box boundary (`smelt.extern`, built-ins, sources) as atomic. See `planner_integration.md` for how frontmatter properties drive that reasoning.

The function half of the equivalence — `smelt.define` grammar, frontmatter keys, `PASSING`, `smelt.as_struct`, fragment-sort parameters, the cycle/recursion/overload rules — is specified in `functions.md` and is not duplicated here.

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
  - `crates/smelt-core/src/{discovery.rs, graph.rs}` — `ModelFile`, `DependencyGraph` (project layout, ref resolution)
  - `crates/smelt-db/src/{lib.rs, type_inference.rs, schema.rs}` — `resolved_model_schema`, `resolve_ref`
  - `crates/smelt-dialect/src/{lib.rs, printer.rs, capabilities.rs}`
  - `crates/smelt-planner/src/lib.rs`
  - `crates/smelt-lsp/src/lib.rs`
  - `crates/smelt-backend/src/lib.rs` (trait), `crates/smelt-backend-{duckdb,spark}/src/lib.rs`
- **Tests**: dialect printer identity tests under `crates/smelt-dialect/tests/`; pure-function tests in `crates/smelt-db/tests/type_property_tests.rs`
- **User docs**: `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/developing/architecture.md`
- **Plans (history)**: see `docs/plans/` for area-specific implementation work
- **Related specs**: feature specs under `docs/specs/` extend this one — `functions.md` (the function half of the models-as-functions equivalence), `incremental_models.md` (model materialization keys), `types.md` (type vocabulary), `planner_integration.md` (planner consumption of frontmatter properties)
- **Research**: `docs/research/20260413-smelt-functions.md` §4 (the unified-model framing)
- **Legacy reference (will thin out)**: `docs/architecture_overview.md` — superseded by this spec
