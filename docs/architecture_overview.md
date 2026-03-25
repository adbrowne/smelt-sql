# Architecture Overview

## The Big Picture

smelt is a **SQL-to-SQL compiler and orchestrator** for data pipelines. It takes SQL models written in smelt's dialect (a PostgreSQL-base superset with cherry-picked features from DuckDB and Spark), resolves dependencies, optionally optimizes across model boundaries, and emits dialect-specific SQL for target execution engines.

Three ideas make smelt different from dbt:

1. **Logical/Physical Separation**: Users write WHAT to compute; the optimizer and dialect-aware printer decide HOW it executes on each backend.
2. **Cross-Model Optimization**: The optimizer operates at the model-graph level — creating shared materializations, redirecting refs, merging models — not at the expression level (that's the backend engine's job).
3. **First-Class Editor Support**: LSP + Salsa + Rowan for incremental compilation and real-time diagnostics, including dialect-specific hints.

## Compilation Pipeline

```
          ┌──────────┐     ┌──────────┐
          │  .sql    │     │  .py     │  Python models
          │  files   │     │  files   │  (@model decorator)
          └────┬─────┘     └────┬─────┘
               │                │
               │           ┌────▼──────┐
               │           │ Discover  │  smelt-core (subprocess/PyO3)
               │           │ Python    │  Extract SQL from decorators
               │           └────┬──────┘
               │                │ SQL strings
               └───────┬───────┘
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
                  │ Optimize │  smelt-optimizer
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
              │      │      │
              │   smelt-cli─┘
              │      │
              │   smelt-backend                 (async, execution trait)
              │      │
              │   ┌──┴──────┐
              │   │         │
              │  duckdb   spark
              │  backend  backend
              │
           (LSP binary)

  Supporting crates (not in main pipeline):
    smelt-state          (sync, run manifests + interval tracking)
    smelt-ui             (async, web dashboard + run execution)
    smelt-datagen        (standalone, test data generation)
    smelt-bench          (standalone, benchmarks)
    smelt-parser-compat  (testing, pg_query/sqlparser-rs/sqlglot compat)
```

### Concern → Crate → Sync/Async

| Concern | Crate | Sync/Async | Notes |
|---------|-------|------------|-------|
| SQL data types | `smelt-types` | sync | `DataType`, `TypedColumn` |
| Parsing | `smelt-parser` | sync | Rowan CST, error recovery |
| Project config & discovery | `smelt-core` | sync | `Config`, `ModelFile`, `DependencyGraph`, Python model discovery |
| Incremental queries | `smelt-db` | sync | Salsa: parse, refs, types, diagnostics |
| SQL dialects & printing | `smelt-dialect` | sync | `SqlDialect`, `BackendCapabilities`, dialect-aware printer |
| Model-graph optimization | `smelt-optimizer` | sync | Cube split, incremental detection, temporal analysis, batch safety |
| LSP server | `smelt-lsp` | async (tower-lsp) | Thin async shell over sync Salsa queries |
| Execution trait | `smelt-backend` | async | `Backend` trait, `ExecutionResult` |
| DuckDB execution | `smelt-backend-duckdb` | async | `DuckDbBackend` |
| Spark execution | `smelt-backend-spark` | async | `SparkBackend` |
| Run state & history | `smelt-state` | sync | `RunManifest`, `IntervalStore`, `FileStore` |
| Web dashboard | `smelt-ui` | async | React frontend, run execution, WebSocket streaming |
| Compatibility testing | `smelt-parser-compat` | sync | pg_query, sqlparser-rs, sqlglot verification |

### Why `smelt-dialect` is separate

The LSP needs dialect information (to show "QUALIFY will be rewritten for PostgreSQL") but must not link against heavy async/native dependencies like Arrow, Tokio, and DuckDB.

`smelt-dialect` is a lightweight sync crate that holds:
- `SqlDialect` enum (DuckDB, SparkSQL, PostgreSQL)
- `BackendCapabilities` struct (feature flags per dialect)
- The dialect-aware printer (CST → SQL string)

Both `smelt-lsp` and `smelt-cli` depend on it. Neither needs to depend on `smelt-backend`.

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

### 3. Optimize (smelt-optimizer)

**Input**: Model dependency graph + CSTs + YAML frontmatter config
**Output**: Graph edits (new models, redirected refs, execution plans) + analysis results
**Representation**: A set of `Transformation` instructions, not mutated CSTs.

Current capabilities:
- **Cube split**: Detects models with multiple `COUNT(DISTINCT)` and splits into parallel sub-queries
- **Incremental materialization**: Detects time-partitioned GROUP BY and generates DELETE+INSERT execution plans
- **Temporal dependency inference**: Analyzes window functions, LAG/LEAD, JOIN intervals, WHERE interval patterns to determine lookback/lookahead requirements
- **Batch safety analysis**: Classifies models as `FullyBatchSafe`, `BoundedSafe`, or `PerPartitionOnly` for backfill planning

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

The dialect-aware printer in `smelt-dialect` walks the CST in a single forward pass and emits dialect-specific SQL. Each construct that needs translation is a match arm in the recursive walk — no multi-pass rewrites, no byte offset arithmetic.

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

The optimizer is smelt's key differentiator. It inspects model SQL (via the CST) and the model dependency graph, then produces transformations: new models, redirected refs, changed materialization strategies, and multi-step execution plans.

**What the optimizer does vs. what it doesn't**: The optimizer does not replicate work that backend engines already do well — predicate pushdown, join reordering, cost-based plan selection within a single query. DuckDB, Spark, and BigQuery all have mature query optimizers for that. Instead, smelt's optimizer handles patterns that no single engine can optimize because they span multiple models or require restructuring how a query is executed.

### What the optimizer does

The optimizer reads CST structure to detect patterns, then emits new SQL as strings (parsed normally) and execution instructions. It does **not** mutate existing CSTs.

#### Cross-model graph transforms

1. **Shared materialization**: Multiple models compute the same intermediate result (e.g., session computation). Create a single shared model and redirect refs.
2. **Model fusion**: Two models where one is a trivial SELECT from the other can be merged.
3. **Ref redirection**: Point a model's ref to a materialized intermediate instead of recomputing from source.

#### Single-model execution transforms

The optimizer also inspects a model's SQL to decide *how* it should be executed:

4. **Incremental materialization detection**: A model that aggregates by a time window (e.g., `GROUP BY date_trunc('day', timestamp)`) can be converted to an incremental model that only processes new partitions instead of full-refreshing.

5. **Query splitting**: A large cube query with many COUNT DISTINCT expressions causes massive memory pressure on engines like Spark and DuckDB (hash tables for each distinct spill to disk). The optimizer can split it into N smaller queries, each computing a subset of the distincts, appending results to a temp table. This keeps intermediate state in memory and avoids spills.

These transforms read CST structure (detecting GROUP BY patterns, counting DISTINCT aggregations) but output new SQL strings and execution plans — they don't modify the original CST in place.

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
    /// Replace a model's single-query execution with a multi-step plan
    ReplaceWithPlan {
        model: String,
        steps: Vec<ExecutionStep>,
    },
}

enum ExecutionStep {
    /// Create a temp table from a query
    CreateTemp { name: String, sql: String },
    /// Append query results to an existing temp table
    AppendToTemp { name: String, sql: String },
    /// The final query that produces the model's output
    FinalQuery { sql: String },
    /// Clean up a temp table
    DropTemp { name: String },
}
```

Transformations produce new SQL strings (parsed normally by `smelt-parser`) and execution instructions. They do **not** mutate existing CSTs. This keeps the optimizer's output easy to inspect: "the optimizer split `big_cube` into 4 queries" or "the optimizer added model `_session_summary` and redirected 3 refs to it."

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
- Perform expression-level algebraic optimization (predicate pushdown, join reordering) — that's the backend engine's job
- Perform cost estimation (future work, requires backend statistics)
- Mutate existing CSTs — it reads them to detect patterns, then produces new SQL strings

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

## Architecture Status

The architecture described above is fully implemented. Key milestones:

- **`smelt-dialect`** extracted as lightweight sync crate — LSP can check dialect capabilities without linking to backends
- **Dialect-aware printer** handles QUALIFY, array literals, DATE literals, JSON function remapping via single-pass CST walk
- **`smelt-optimizer`** implements cube split, incremental materialization, temporal analysis, and batch safety
- **LSP dialect diagnostics** planned but not yet implemented (the infrastructure is in place via `smelt-dialect`)

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
