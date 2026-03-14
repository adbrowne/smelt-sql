# Architecture Overview

## The Big Picture

smelt is a **SQL-to-SQL compiler and orchestrator** for data pipelines. It takes SQL models written in smelt's dialect (a PostgreSQL-base superset with cherry-picked features from DuckDB and Spark), resolves dependencies, optionally optimizes across model boundaries, and emits dialect-specific SQL for target execution engines.

Three ideas make smelt different from dbt:

1. **Logical/Physical Separation**: Users write WHAT to compute; the optimizer and dialect-aware printer decide HOW it executes on each backend.
2. **Cross-Model Optimization**: The optimizer operates at the model-graph level — creating shared materializations, redirecting refs, merging models — not at the expression level (that's the backend engine's job).
3. **First-Class Editor Support**: LSP + Salsa + Rowan for incremental compilation and real-time diagnostics, including dialect-specific hints.

## Compilation Pipeline

```
                  ┌──────────┐
                  │  .sql    │  smelt SQL models
                  │  files   │  (superset dialect)
                  └────┬─────┘
                       │
                  ┌────▼─────┐
                  │  Parse   │  smelt-parser (Rowan CST)
                  │          │  Error-recovery, lossless
                  └────┬─────┘
                       │ CST
                  ┌────▼─────┐
                  │ Analyze  │  smelt-db (Salsa queries)
                  │          │  Refs, types, diagnostics
                  └────┬─────┘
                       │ CST + semantic info
                  ┌────▼─────┐
                  │ Optimize │  smelt-optimizer (future)
                  │(optional)│  Model-graph transforms
                  └────┬─────┘
                       │ CST + graph edits
                  ┌────▼──────┐
                  │ Generate  │  smelt-dialect (dialect-aware printer)
                  │           │  CST walk → target SQL string
                  └────┬──────┘
                       │ SQL string per dialect
                  ┌────▼──────┐
                  │ Execute   │  smelt-backend-* (async)
                  │           │  Send SQL to engine
                  └───────────┘
```

**Key invariant**: The Rowan CST is the single representation from parse through generation. There is no intermediate IR like DataFusion LogicalPlan. This avoids two fidelity boundaries (CST → IR → SQL) and preserves comments, formatting, and smelt extensions throughout.

## Crate Dependency Graph

```
                          smelt-types          (sync, data types)
                            │
                          smelt-parser         (sync, Rowan CST)
                            │
                     ┌──────┼──────┐
                     │      │      │
                  smelt-core │   smelt-dialect  (sync, printer + capabilities)
                     │      │      │
                     │   smelt-db  │            (sync, Salsa queries)
                     │      │      │
              ┌──────┼──────┼──────┤
              │      │      │      │
           smelt-lsp │   smelt-optimizer        (sync, model-graph transforms)
              │      │      │      │             (future)
              │      │      │      │
              │   smelt-cli─┘      │
              │      │             │
              │   smelt-backend    │            (async, execution trait)
              │      │             │
              │   ┌──┴──────┐     │
              │   │         │     │
              │  duckdb   spark   │
              │  backend  backend │
              │      │      │     │
              │   smelt-transpiler┘             (async, wraps backend + dialect)
              │
           (LSP binary)
```

### Concern → Crate → Sync/Async

| Concern | Crate | Sync/Async | Notes |
|---------|-------|------------|-------|
| SQL data types | `smelt-types` | sync | `DataType`, `TypedColumn` |
| Parsing | `smelt-parser` | sync | Rowan CST, error recovery |
| Project config & discovery | `smelt-core` | sync | `Config`, `ModelFile`, `DependencyGraph` |
| Incremental queries | `smelt-db` | sync | Salsa: parse, refs, types, diagnostics |
| SQL dialects & printing | `smelt-dialect` | sync | `SqlDialect`, `BackendCapabilities`, dialect-aware printer |
| Model-graph optimization | `smelt-optimizer` | sync | Graph transforms, new model generation (future) |
| LSP server | `smelt-lsp` | async (tower-lsp) | Thin async shell over sync Salsa queries |
| Execution trait | `smelt-backend` | async | `Backend` trait, `ExecutionResult` |
| DuckDB execution | `smelt-backend-duckdb` | async | `DuckDbBackend` |
| Spark execution | `smelt-backend-spark` | async | `SparkBackend` |
| SQL rewriting for backends | `smelt-transpiler` | sync* | Wraps backend + dialect printer |

\* The transpiler's core logic (CST walk + emit) is sync. The `TranspilingBackend<B>` wrapper is async because it delegates to an async `Backend`.

### Why extract `smelt-dialect`?

The current `smelt-backend` crate contains `SqlDialect` and `BackendCapabilities` alongside async execution code that depends on Arrow, Tokio, and DuckDB. The LSP needs dialect information (to show "QUALIFY will be rewritten for PostgreSQL") but must not link against heavy async/native dependencies.

`smelt-dialect` is a lightweight sync crate that holds:
- `SqlDialect` enum (DuckDB, SparkSQL, PostgreSQL)
- `BackendCapabilities` struct (feature flags per dialect)
- The dialect-aware printer (CST → SQL string)

Both `smelt-lsp` and `smelt-transpiler` depend on it. Neither needs to depend on `smelt-backend`.

## Data Flow

### 1. Parse (smelt-parser)

**Input**: SQL source text
**Output**: Rowan CST (`SyntaxNode`)
**Representation**: Lossless concrete syntax tree — every byte of input is represented, including whitespace, comments, and smelt extensions (`smelt.ref()`, `smelt.metric()`, `=>` named parameters).

The parser uses recursive descent with error recovery at sync points (semicolons, keywords). Invalid input produces `ERROR` nodes in the CST rather than aborting — critical for LSP support where code is almost always incomplete.

### 2. Analyze (smelt-db via Salsa)

**Input**: CST + project config
**Output**: Semantic information layered on the CST
**Representation**: Salsa query results keyed by file/model — refs with positions, resolved types, diagnostics.

Salsa provides automatic incremental recomputation. When a file changes, only affected queries are re-evaluated. The CST itself is not mutated; semantic information is computed as derived queries.

Key queries:
- `parse_file()` → CST
- `model_refs()` → ref names + positions
- `resolve_ref()` → target model
- `file_diagnostics()` → errors and warnings
- `model_schema()` → column types

### 3. Optimize (smelt-optimizer, future)

**Input**: Model dependency graph + CSTs
**Output**: Graph edits (new models, redirected refs, removed models)
**Representation**: A set of `Transformation` instructions, not mutated CSTs.

See [Optimizer Design](#optimizer-design) below.

### 4. Generate (smelt-dialect, dialect-aware printer)

**Input**: CST + target `SqlDialect`
**Output**: SQL string valid for the target engine
**Representation**: Plain `String`.

See [Dialect-Aware Printer](#dialect-aware-printer-design) below.

### 5. Execute (smelt-backend-*)

**Input**: SQL string
**Output**: `ExecutionResult` (duration, row count, optional preview)
**Representation**: Arrow `RecordBatch` for data interchange.

The `Backend` trait is async because execution involves network I/O (Spark connect, future cloud backends). Each backend implementation handles DDL (CREATE TABLE AS, views, incremental inserts) and returns results.

## Dialect-Aware Printer Design

The dialect-aware printer replaces the current transpiler's text-range string replacement approach. Instead of detecting byte offsets in the CST and splicing replacement strings from end to start, the printer walks the CST in a single forward pass and emits dialect-specific SQL.

### Current approach (smelt-transpiler, to be replaced)

```
Parse → detect SyntaxKind nodes → extract byte ranges → compute replacement text
→ sort replacements descending → apply via String::replace_range()
```

Problems:
- **Multi-pass fragility**: Statement-level rewrites (QUALIFY) run first, then the output is re-parsed for expression-level rewrites (array literals). Rewrites can interact.
- **Offset arithmetic**: Replacements must be sorted and applied end-to-start to avoid invalidating byte positions. Nested constructs require care.
- **Not composable**: Each rewrite is an independent module that doesn't know about other rewrites. Combining them requires the multi-pass orchestrator.

### New approach (dialect-aware printer)

```rust
fn print(node: &SyntaxNode, dialect: &SqlDialect, caps: &BackendCapabilities) -> String {
    let mut out = String::new();
    print_node(node, dialect, caps, &mut out);
    out
}

fn print_node(node: &SyntaxNode, dialect: &SqlDialect, caps: &BackendCapabilities, out: &mut String) {
    match node.kind() {
        // smelt extension: resolve ref to schema.model_name
        SyntaxKind::REF_CALL => {
            let model_name = extract_ref_name(node);
            write!(out, "{schema}.{model_name}", schema = dialect.default_schema());
        }

        // Dialect rewrite: QUALIFY → subquery wrapper
        SyntaxKind::SELECT_STMT if has_qualify(node) && !caps.supports_qualify => {
            print_qualify_as_subquery(node, dialect, caps, out);
        }

        // Dialect rewrite: array literal syntax
        SyntaxKind::ARRAY_LITERAL if !caps.supports_array_literal => {
            // ARRAY[1, 2, 3] → ARRAY(1, 2, 3)
            out.push_str("ARRAY(");
            print_array_elements(node, dialect, caps, out);
            out.push(')');
        }

        // Default: recursively print children
        _ => {
            for child in node.children_with_tokens() {
                match child {
                    NodeOrToken::Node(n) => print_node(&n, dialect, caps, out),
                    NodeOrToken::Token(t) => out.push_str(t.text()),
                }
            }
        }
    }
}
```

**Advantages**:
- **Single pass**: No re-parsing, no offset arithmetic, no multi-pass coordination.
- **Composable**: Adding a new rewrite is adding a new match arm. Nested rewrites compose naturally because the recursive walk handles inner nodes.
- **Preserves formatting**: The default arm emits tokens verbatim, preserving whitespace and comments. Only matched constructs are rewritten.
- **Testable**: Each match arm can be unit-tested by constructing a CST subtree and checking the emitted string.

### Identity property

For the "native" dialect (DuckDB, which supports the full smelt superset), the printer should emit SQL identical to the input (modulo smelt extension resolution). This is a strong correctness invariant that can be property-tested.

## Optimizer Design

The optimizer is smelt's key differentiator. It works at the **model-graph level**, not at the expression level within a single query. Expression-level optimization (predicate pushdown, join reordering, cost-based plan selection) is the backend engine's job — DuckDB, Spark, and BigQuery all have mature optimizers for this.

### What the optimizer does

The optimizer detects patterns across multiple models and produces **graph transformations**:

1. **Shared materialization**: Multiple models compute the same intermediate result (e.g., session computation). Create a single shared model and redirect refs.
2. **Dimensional splitting**: A large GROUP BY with many dimensions can be split into smaller, cheaper aggregations when downstream models only need subsets.
3. **Model fusion**: Two models where one is a trivial SELECT from the other can be merged.
4. **Ref redirection**: Point a model's ref to a materialized intermediate instead of recomputing from source.

### Transformation enum

```rust
enum Transformation {
    /// Create a new model with the given SQL and config
    CreateModel {
        name: String,
        sql: String,
        materialization: Materialization,
    },
    /// Redirect a ref in an existing model to point to a different target
    RedirectRef {
        model: String,
        old_ref: String,
        new_ref: String,
    },
    /// Remove a model (after all refs redirected away)
    RemoveModel {
        name: String,
    },
    /// Change materialization strategy for an existing model
    SetMaterialization {
        model: String,
        materialization: Materialization,
    },
}
```

Transformations produce new SQL strings (parsed normally by `smelt-parser`) and graph edits. They do **not** mutate existing CSTs. This keeps the optimizer's output easy to inspect: "the optimizer added model `_session_summary` and changed `user_sessions` to ref it."

### Optimizer rules

Rules are explicit, named, and user-controllable:

```rust
struct OptimizationRule {
    name: &'static str,
    description: &'static str,
    /// Detect whether this rule applies to the current model graph
    detect: fn(&ModelGraph) -> Vec<Opportunity>,
    /// Produce transformations for a detected opportunity
    rewrite: fn(&Opportunity) -> Vec<Transformation>,
}
```

The optimizer runs all rules, collects opportunities, and applies transformations. Users can enable/disable rules and inspect what was applied.

### Scope boundary

The optimizer does **not**:
- Parse or understand SQL semantics beyond what the CST provides
- Perform cost estimation (future work, requires backend statistics)
- Rewrite SQL expressions within a model (that's the dialect-aware printer or the backend engine)

## LSP Integration

### Current state

The LSP (`smelt-lsp`) depends on `smelt-db` (Salsa queries) and `smelt-parser` (CST). It provides:
- Parse error diagnostics with accurate positions
- Undefined ref diagnostics
- Go-to-definition for `smelt.ref()`
- Hover with type information
- Column completions (including table alias completions)
- Model name completions in `smelt.ref()`

### Dialect diagnostics (via smelt-dialect)

With `smelt-dialect` extracted as a lightweight sync crate, the LSP can check constructs against `BackendCapabilities` without linking to any backend:

```
smelt-lsp → smelt-dialect (sync, no Arrow/Tokio/DuckDB)
         → smelt-db      (sync, Salsa)
         → smelt-parser  (sync, Rowan)
```

This enables dialect-specific informational hints in the editor:

- `ℹ️ QUALIFY will be rewritten to a subquery for PostgreSQL`
- `ℹ️ ARRAY[...] will be rewritten to ARRAY(...) for Spark SQL`
- `⚠️ DATE literal syntax not supported by Spark — will be rewritten to DATE() function`

These are informational (not errors) because the dialect-aware printer handles the rewriting. They help developers understand what will happen when their code runs on a different backend.

### Optimizer suggestions (future)

When the optimizer detects opportunities, the LSP can surface them as code actions:

```
ℹ️ Optimization available: 3 models share session computation
   → Create shared materialization 'session_summary'
```

The LSP calls the optimizer's `detect` phase (sync, no execution needed) and presents `Opportunity` values as diagnostics. If the user accepts, the LSP applies the `Transformation` list as workspace edits.

## Migration Path

The architecture described above is the target state. Here is the phased migration from the current codebase:

### Phase 1: Extract `smelt-dialect` (no behavior change)

Move `SqlDialect`, `BackendCapabilities` from `smelt-backend` into a new `smelt-dialect` crate. Both `smelt-backend` and `smelt-transpiler` re-export or depend on `smelt-dialect`. No functional changes — pure crate extraction.

### Phase 2: Dialect-aware printer in `smelt-dialect`

Implement the CST-walking printer in `smelt-dialect`. Port the three existing rewrite rules (QUALIFY, array literals, DATE literals) from the transpiler's text-range approach to match arms in the printer. Verify with the transpiler's existing test suite.

### Phase 3: Replace transpiler internals

Replace `smelt-transpiler`'s multi-pass replacement logic with calls to the dialect-aware printer. `TranspilingBackend<B>` calls `smelt_dialect::print()` instead of the old `transpile()` function. The transpiler crate may shrink to just the `TranspilingBackend` wrapper or be absorbed into `smelt-cli`.

### Phase 4: LSP dialect diagnostics

Add `smelt-dialect` as a dependency of `smelt-lsp`. Walk the CST checking constructs against `BackendCapabilities` for the configured target. Emit informational diagnostics for constructs that will be rewritten.

### Phase 5: Optimizer (future)

Introduce `smelt-optimizer` crate. Implement the first optimization rule (shared materialization). Integrate with LSP for opportunity detection and code actions.

## Key Technical Decisions

### Rowan CST as single representation

The CST is the single source of truth from parse to generation. This avoids the fidelity loss inherent in converting to and from an intermediate representation like DataFusion LogicalPlan. The CST preserves:
- Comments and whitespace (important for readable generated SQL)
- smelt extensions (`smelt.ref()`, `smelt.metric()`, named parameters)
- Original formatting (the printer's default arm emits tokens verbatim)
- Error nodes (the LSP works with incomplete/invalid code)

### Salsa for incremental compilation

Salsa tracks query dependencies automatically. When a file changes, only affected queries are recomputed. This is critical for LSP responsiveness — parsing 1000 models once takes ~1s, but re-analyzing a single changed file takes ~50ms.

### Rowan for error recovery

Developers write invalid code most of the time while editing. The parser must produce a usable CST even with syntax errors. Rowan's `ERROR` nodes allow the LSP to provide diagnostics, completions, and go-to-definition on incomplete code.

### Expression optimization is the engine's job

smelt does not attempt predicate pushdown, join reordering, or cost-based optimization within a single query. DuckDB, Spark, and BigQuery all have mature query optimizers. smelt's value is in cross-model optimization that no single engine can do because it doesn't see the full pipeline.

### Sync core, async edges

All core logic (parsing, analysis, optimization, printing) is synchronous. Async is only at the execution boundary where network I/O happens. This keeps the codebase simple, testable, and compatible with Salsa (which is sync).
