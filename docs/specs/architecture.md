---
feature: architecture
status: stable
last_reviewed: 2026-05-04
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
| `smelt-cli`            | async (entry point) | CLI surface, model discovery, dialect selection — async because the entry point drives the async execution stage |
| `smelt-backend`        | async              | `Backend` trait (see "Backend trait surface" below), `ExecutionResult` |
| `smelt-backend-duckdb` | async              | DuckDB execution                                             |
| `smelt-backend-spark`  | async              | Spark execution                                              |
| `smelt-state`          | sync               | `RunManifest`, `IntervalStore`, `FileStore`                  |
| `smelt-ui`             | async              | Web dashboard, run execution, WebSocket streaming            |

### Project layout

A smelt workspace is rooted at a directory containing `smelt.yml`. **Directory layout is user-chosen** — the spec mandates only that `smelt.yml` exists at the root. The recommended layout (used in `examples/`) is:

| Path           | Convention                                                                        |
|----------------|-----------------------------------------------------------------------------------|
| `models/`      | `.sql` files containing bare-SELECT models (any number per file).                 |
| `functions/`   | `.sql` files containing `smelt.define` declarations.                              |
| `seeds/`       | Static `.csv` inputs.                                                             |
| `sources/`     | Per-source `.yml` declarations (schemas for external tables).                     |
| `tests/`       | `.sql` files containing `materialization: test` models (any number per file).    |
| `smelt.yml`    | Project-level configuration (backend selection, feature flags such as `unstable_schema`). |

A project may rename or reorganise these freely (`staging/`, `marts/`, `external/`, etc.). The kind of an entity is determined by the file's *format and content*, not by the name of its containing directory.

### Resolution: `smelt.<path>` is the universal addressing scheme

> The `smelt.<path>` migration completed across all feature specs on 2026-05-04. Earlier kind-prefixed forms (`smelt.models.<name>`, `smelt.sources.<schema>.<table>`, `smelt.fn.<path>`) are retired; references in older plans and research documents should be read as legacy.

Every project-defined entity — model, function, seed, source, test — is addressed by a single uniform syntax:

- `smelt.<path>` — value reference (used in `FROM` position, as a `TableExpr`-typed argument, etc.)
- `smelt.<path>(<args>)` — call (used for functions and parameterised models)

The path is the workspace-relative directory joined with the entity's leaf name, **with the matching `paths:` scan-root prefix stripped**, segments separated by `.`. Examples (assuming `paths: ["models"]` and the recommended layout):

| Filesystem location                                       | Reference syntax (scan-root stripped)             |
|-----------------------------------------------------------|---------------------------------------------------|
| `models/marts/customers.sql` (lone bare SELECT)           | `smelt.marts.customers`                           |
| `models/marts/file.sql` containing `name: customers`      | `smelt.marts.customers`                           |
| `functions/patterns/x.sql` declaring `session_rollup`     | `smelt.functions.patterns.session_rollup(...)`    |
| `seeds/raw/users.csv`                                     | `smelt.raw.users`                                 |
| `sources/raw/events.yml`                                  | `smelt.raw.events`                                |
| `tests/marts/customers.sql` (lone `materialization: test` model)        | `smelt.tests.marts.customers`                     |
| `tests/marts/file.sql` containing `name: customers_no_nulls` + `materialization: test` | `smelt.tests.marts.customers_no_nulls` |

Note: `functions/`, `sources/`, and `tests/` are discovered via their own dedicated scan paths and are **not** in the `paths:` list by default; their scan-root prefix is not stripped (they keep their full workspace-relative path as the address). Only directories listed in `paths:` have their prefix stripped.

**Kind is determined by file format and content, not by syntactic prefix.** When the resolver reaches a path:

- A `.sql` file with a bare SELECT (or one of multiple named bare SELECTs) → **model** (DAG-defaulted, `TableExpr`-valued). Tests are a model kind: a bare SELECT carrying `materialization: test` in its frontmatter is a **test** model — addressable for tooling but **not** valid in `TableExpr` positions, since a test never produces a database object (`testing.md`).
- A `.sql` file declaring `smelt.define <name>` → **function** (callable with arguments).
- A `.csv` file → **seed**.
- A `.yml` file with **no** sibling `.csv` (same directory, same stem) → **source** (externally-managed table; smelt declares its schema, does not load it).
- A `.yml` file **with** a sibling `.csv` (same directory, same stem) → **sidecar** to that seed; binds schema / column descriptions / `materialization` to the seed and is not itself an addressable entity.

A `.sql` file may mix declaration kinds — a model with co-located tests (each declared via `materialization: test`), a function with co-located tests — provided names are unique within the file. A given path resolves to at most one kind. Names must be unique within a directory across all kinds — a `data/users.csv` and `data/users.sql` is a workspace-load error.

**Address uniqueness is global across `paths:`.** When `smelt.yml::paths` lists multiple roots (e.g. `paths: ["models", "fixtures"]`), the scan-root prefix is stripped from each independently and addresses share a single namespace. Two files that resolve to the same `smelt.<path>` — e.g. `models/users.csv` and `fixtures/users.csv` — are a hard workspace-load error. The rule is "one path → one entity" regardless of which scan root the file lives under.

**Externs are flat.** `smelt.extern` declarations register a bare name in the workspace-wide builtin/extern namespace and are *not* addressed via `smelt.<path>`. A `smelt.extern read_parquet(...)` declared in `functions/io/parquet.sql` is callable everywhere as `read_parquet(...)`. The path of the declaring file affects only navigation (jump-to-definition), never the call surface. This is the one documented exception to the universal addressing scheme; externs exist precisely to extend the bare-name builtin namespace, and a path-prefixed extern would defeat that purpose.

**File kind is grammar, not location.** `data/foo.sql` containing a bare SELECT is a model addressable as `smelt.data.foo`; `random/x.sql` declaring `smelt.define helper(...)` is callable as `smelt.random.helper(...)` (the filename stem is not a path component). The recommended layout is convention; the resolver only cares about path and content.

**Bare-model naming.** A `.sql` file may contain any number of bare-SELECT models. A file's *lone* bare SELECT takes its leaf name from the filename and **must not** declare `name:` in its frontmatter. In a file with two or more bare SELECTs, each bare SELECT **must** declare `name:` in its frontmatter; the filename ceases to register as a model name and becomes purely a container. The model's full path is the directory path joined with the leaf name. Names within a file must be unique across bare SELECTs (including `materialization: test` ones), `smelt.define`s, and `smelt.extern`s.

### Default materialization name mapping

A persisted entity addressed as `smelt.<path>` materialises by default at:

- **Schema** = the active target's `schema:` (from `smelt.yml::targets.<name>.schema`, default `main`).
- **Table** = address path joined with `_`. Underscores already in path components are preserved as-is (no escaping, since path components come from filesystem-safe identifiers already).

| Address | Default DB location (target schema = `main`) |
|---|---|
| `smelt.users` | `main.users` |
| `smelt.staging.orders` | `main.staging_orders` |
| `smelt.payments.seeds.lookup.regions` | `main.payments_seeds_lookup_regions` |

This rule provides the **default** DB location for every entity referenced by a path that needs to resolve to a table-like database object: models with `materialization: table` / `view` / `materialized_view`, seeds with `materialization: table`, and sources (which need a target name for `FROM`-clause emission even though smelt does not load them).

It does **not** apply to entities that never resolve to a database object:

- `smelt.define` functions (inlined; never persisted).
- Ephemeral models / seeds (inlined as CTE / `VALUES`; never persisted).
- Externs and built-ins (no path).

Per-entity overrides are kind-specific:

- **Sources** may override the default via the YAML `name:` key (`sources.md`) — the external pipeline names the table, smelt records it.
- **Seeds and models** cannot override individually today; the rule is mandatory. A future configurable mapping (per-entity overrides, an analogue of dbt's `generate_schema_name` / `generate_alias_name`) lifts this restriction.

### Unified frontmatter rule

A YAML frontmatter block (between `---` fences) attaches to the **immediately following declaration**: a model `SELECT` (including a test, declared via `materialization: test` on the SELECT's frontmatter), a `smelt.define`, or a `smelt.extern`. Each declaration may carry its own block; there is no file-level frontmatter, and no inheritance across declarations in the same file. (Research §16 #22.)

The frontmatter parser is shared across all four declaration kinds; the parsing contract is identical. Property semantics differ — per-feature key catalogues live in the relevant feature spec:

- **Function and extern keys** (`deterministic`, `idempotent`, `append_only`, `backends`, gated `joins` / `provenance`): see `functions.md`.
- **Model materialization keys** (`materialization`, `incremental`, …): see `incremental_models.md`.

This spec does not duplicate those catalogues; it only fixes the attachment rule and the parser-sharing invariant.

### Models as functions

A smelt **model** is a bare `SELECT` (in a `.sql` file) consuming `TableExpr` inputs (via `smelt.<path>` references) and producing a `TableExpr` output. A smelt **function** is a `smelt.define` declaration consuming typed fragment inputs and producing a typed fragment output. **They are the same concept with different parameter-binding defaults.**

The model

```sql
-- models/margins.sql
---
materialization: table
---
SELECT revenue - cost AS margin
FROM smelt.product_summary
```

is equivalent to the parameterised form

```sql
-- models/margins.sql
smelt.define margins(
    product_summary: TableExpr = smelt.product_summary
) -> TableExpr AS (
    SELECT revenue - cost AS margin
    FROM product_summary
)
```

(both are addressable as `smelt.margins` when `paths: ["models"]`)

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
| Source                     | black-box    | external        | schema from per-entity source YAML or catalog |

#### Normative rules

1. **Materialization is orthogonal to transparency.** Choosing `materialization: table` versus `view` versus `ephemeral` does not change whether the body is visible to the planner — every `smelt.define` and every model body is transparent. Materialization controls only how output is persisted and scheduled.
2. **Parameter-binding style is sugar.** The model/function distinction reduces to "DAG-default `TableExpr` parameters" versus "explicitly-passed parameters." A workspace may mix both forms in one declaration: a model with explicit `TableExpr` parameters alongside its DAG-default refs is a **parameterised model** and is well-formed. Callers (or tests) override the defaults at the call site.
3. **`smelt.<path>` references are parameters with DAG-supplied defaults.** Each `smelt.<path>` reference inside a model body desugars to a `TableExpr` parameter whose default expression is resolved against the workspace by the `smelt.<path>` resolver — the file at the path is consulted, and its kind (model, source, seed) determines how the default is satisfied. Tests and parameterised callers may override these defaults; the type-system contract is "any `TableExpr` whose schema satisfies the columns the body actually touches" — row polymorphism, in the same sense as `TableExpr<{…}>` parameters in `functions.md`.
4. **The planner's optimization boundary aligns with transparency, not materialization.** The planner reasons across every transparent boundary — model-to-model and call-to-callee — and treats every black-box boundary (`smelt.extern`, built-ins, sources) as atomic. See `planner_integration.md` for how frontmatter properties drive that reasoning.

The function half of the equivalence — `smelt.define` grammar, frontmatter keys, `PASSING`, `smelt.as_struct`, fragment-sort parameters, the cycle/recursion/overload rules — is specified in `functions.md` and is not duplicated here.

### Backend trait surface

The `Backend` trait is the contract every execution backend implements. The minimal surface every backend must provide:

| Method | Purpose |
|---|---|
| `execute_sql(sql) -> RecordBatch[]` | Run an arbitrary SQL statement; return Arrow batches. The primary execution path. |
| `drop_table_if_exists(schema, name)` / `drop_view_if_exists(schema, name)` | Type-aware drops. DuckDB rejects `DROP TABLE` against a view (and vice-versa); these helpers paper over that. |
| `load_table(schema, name, arrow_schema, batches)` | Cross-backend Arrow ingest path. Used by seed loading and (future) any other "build a table from in-memory data" surface. DuckDB implements via `Appender`; Spark via `createDataFrame(...).saveAsTable(...)`. |

Trait methods grow as new ingest / introspection paths land; the minimum a backend has to implement is the four above. The trait is `async`; backends are responsible for their own connection lifecycle.

#### Cross-engine data exchange

When a model on DuckDB references a model pinned to Spark (via `smelt.<path>`), smelt resolves the reference to a `read_parquet()` call against the Spark model's Parquet files in the `warehouse` directory. No explicit copy step is needed; DuckDB reads the files natively. This requires the Spark model to have `materialization: table` and the Spark target to have a `warehouse` path configured. (A full multi-backend execution model — capability negotiation, cross-engine reference rules, Databricks-specific features — is deferred to a future `multi_backend.md` spec; see Known Divergences.)

### `Transformation` and `ExecutionStep` (planner output)

The planner outputs values, never mutations. The user-visible enum surface:

```rust
enum Transformation {
    // Single-model transformations
    ReplaceWithPlan   { model, steps: Vec<ExecutionStep> },
    SetIncremental    { model, event_time_column, partition_column, granularity },
    SetMaterialization { model, materialization },

    // Graph-level transformations
    CreateNode        { name, sql, dependencies, origin, materialization },
    RemoveNode        { model },
    RedirectRef       { from, to },
}

enum ExecutionStep {
    CreateTemp   { name, sql },
    AppendToTemp { name, sql },
    FinalQuery   { sql },
    DropTemp     { name },
}
```

## Semantics

### Stage rules

1. **Parse** is total: every input produces a CST. Invalid input produces `ERROR` nodes; the parser does not abort.
2. **Analyze** is incremental: Salsa recomputes only affected queries when a file changes. Analysis must not mutate the CST.
3. **Plan** is optional and additive: the planner produces a `Vec<Transformation>`; it does not mutate existing CSTs. New SQL it emits is parsed normally by `smelt-parser`.
4. **Generate** is single-pass over the CST: each rewrite is a match arm in the recursive walk. Tokens not matched by a rewrite arm are emitted verbatim.
5. **Execute** is the only async stage. All cross-engine I/O happens here.

### Identity properties

Two identity properties hold across the pipeline. Both are property-testable.

**1. Parse-level semantic anchor (PostgreSQL via pg_query).** smelt's grammar tracks PostgreSQL semantics. For SQL containing no smelt extensions, the parser's pretty-printed output is fingerprint-equivalent to the input under pg_query. This is the parser-validation oracle in `crates/smelt-parser-compat/` — it lets the grammar drift from PostgreSQL only deliberately, never by accident, and is the canonical reference for "is this construct syntactically valid smelt SQL?". Equivalence on the Spark side is checked separately against `sqlparser-rs DatabricksDialect` (and optionally `sqlglot`); pg_query is the primary anchor.

**2. Print-level identity for the DuckDB dialect.** The DuckDB printer emits SQL byte-identical to the input, modulo:

- **Smelt-extension resolution** (universal across dialects): `smelt.<path>` → backend-resolvable identifier (`<schema>.<emitted_name>` for models and seeds, source-declared name for sources); `smelt.<path>(<args>)` → expanded function body; `smelt.as_struct(alias [EXCEPT …])` → DuckDB struct literal; bare-name `smelt.extern` calls are emitted verbatim (or with the configured backend-namespace remap from `functions.md`).
- **Cross-dialect function-name normalization** for the short list of constructs where smelt's accepted surface diverges from DuckDB's surface:
  - `EXPLODE(x)` → `UNNEST(x)`
  - `EVERY(b)` → `BOOL_AND(b)`

  Input that already uses the DuckDB-flavoured spellings (`UNNEST`, `BOOL_AND`, `BOOL_OR`, `QUALIFY`, `DATE '…'`, `x::T`, `ARRAY[…]`, trailing commas) round-trips byte-identically.

For the Spark and PostgreSQL printers, additional rewrites apply (QUALIFY → subquery, `ARRAY[…]` → `ARRAY(…)`, `x::T` → `CAST(x AS T)`, `DATE 'lit'` → `DATE('lit')`, trailing-comma stripping, further function-name remaps such as `UNNEST` ↔ `EXPLODE`, `BOOL_OR` → `SOME`). Identity for those dialects holds only *modulo those rewrites*; they are not the byte-identity target. The capability matrix lives in `crates/smelt-dialect/src/dialect.rs::BackendCapabilities`.

### Salsa purity rule (analysis)

Analysis logic in `smelt-db` (type inference, schema extraction, diagnostic checks) is implemented as **pure functions** that take AST nodes and plain data structures. Salsa queries are thin wrappers that build inputs, call the pure function, and return the result. This invariant exists so a future `smelt-check` crate can do batch compilation without Salsa as a mechanical extraction.

### Planner scope

The planner handles cross-model and execution-shape transforms only:

- **In scope**: shared materialization, model fusion, ref redirection, incremental detection, query splitting, temporal/batch-safety analysis.
- **Out of scope**: predicate pushdown, join reordering, cost-based optimization within a single query — these are the backend engine's job.

The planner's `detect` phase is sync and side-effect-free; the LSP may call it to surface code-action suggestions.

## Design

This section captures the load-bearing rationale behind the pipeline, the crate boundaries, and the project-layout / models-as-functions framings above. It does not restate the rules; it explains why those rules are shaped this way and what was rejected.

**One Rowan CST flows from parse to generation — no intermediate IR.** The conventional shape (Calcite, DataFusion, Spark Catalyst) is parse → AST → logical plan → physical plan. We reject that shape because it produces *two-IR drift*: the planner's IR slowly diverges from the parser's, dialect printers must walk a structure the user never wrote, and roundtrip identity becomes impossible. Keeping the CST as the single representation lets the dialect printer be authored as one recursive walk over the user's tokens, makes both identity properties (Semantics §"Identity properties") testable — print-level byte-identity for DuckDB and parse-level fingerprint equivalence against pg_query — and means an error-recovery CST node remains visible end-to-end. The trade-off is that planner rewrites are CST→CST rather than `LogicalPlan`→`LogicalPlan` — more verbose for cross-cutting transforms — which is paid back via the `Transformation` value vocabulary (see below) and the per-node frontmatter that drives planner reasoning (`planner_integration.md`).

**`smelt-db` analysis logic is pure (Salsa purity rule).** Salsa is the right tool for *incrementality* (cache invalidation across edits) and the wrong tool for *batch compilation* (CLI builds, planner runs, future test runners). Embedding analysis logic inside Salsa queries ties every consumer to the Salsa runtime; pulling it out so queries are thin wrappers around `fn check_x(ast, ctx) -> Result` lets a future `smelt-check` crate do batch compilation as a mechanical extraction. This invariant is upheld by convention today and will be structurally enforced once `smelt-check` exists (see Known Divergences). Rejected alternative: ship LSP-only analysis and rebuild it for batch — guaranteed drift between editor and CLI diagnostics.

**CSTs are not mutated; the planner outputs `Transformation` values.** A mutating planner (`rule.apply(&mut cst)`) is harder to debug — the diff between "before" and "after" only exists in the rule's head — and forecloses speculative planning (try a rewrite, measure, discard). Returning `Vec<Transformation>` makes rules composable (stack them, inspect them, render them in `--show-plan`), unit-testable as plain values, and reversible. See `planner_integration.md` for how rules consume frontmatter to decide which transformations to emit.

**Sync core, async edges.** Parsing, analysis, planning, and printing are CPU-bound; the per-task overhead of `tokio::spawn` would slow incremental compilation in `smelt-db` and add no parallelism (each query is small and sequential under Salsa's invalidation graph). Async lives at execution (where I/O against backends dominates) and at the process entry points that drive execution — the LSP server (where the protocol demands it), the CLI, and the UI. Crate-level async/sync labelling in the Surface table is the contract; a sync crate may not transitively depend on an async runtime.

**`smelt-dialect` is lightweight.** Both `smelt-lsp` (which surfaces "this construct is unsupported on Spark" diagnostics) and `smelt-cli` (which selects a backend dialect) must link the dialect crate. If `smelt-dialect` pulled in Arrow / Tokio / DuckDB, every consumer would inherit those dependencies — including the planner, which has no business compiling DuckDB. Keeping the dialect crate to `SqlDialect`, `BackendCapabilities`, and the printer means it sits cleanly between analysis and execution without becoming a fan-in chokepoint.

**Single addressing scheme `smelt.<path>` for all project-defined entities.** Earlier shapes used kind-specific prefixes — `smelt.ref('m')` for models, `smelt.source('raw.x')` for sources, `smelt.fn.<path>(...)` for functions, with externs flat. That asymmetry forced users to know an entity's kind before referencing it, conflated the *what* with the *where*, and made cross-kind refactors (a seed promoted to a model, a model factored as a parameterised function) churn every callsite. Collapsing every project-defined entity into `smelt.<path>` makes resolution uniform: the path locates the entity, the file format/content determines the kind, and the resolver dispatches accordingly. A reader who *wants* the kind-signal at the callsite gets it for free if the project follows the recommended layout (`smelt.sources.raw.events` reads as "this is a source"); a reader who doesn't can name their directory whatever they like. Externs remain the documented exception (flat, ambient, callable by bare name) because their job is to extend the built-in namespace — a path-prefixed extern would defeat the ergonomics that motivate them. (Research §16 #22; addressing redesigned 2026-05-01.)

**Directory layout is user-chosen; kind is determined by file format/content.** The recommended `models/` / `functions/` / `seeds/` / `sources/` layout is convention, not spec-mandated structure. Forcing kind-by-directory was rejected because it forecloses meaningful per-project organisation — a project that prefers `staging/` / `marts/` / `external/` should not have to fight the framework. Forcing kind-by-syntactic-prefix was rejected for the addressing-scheme reason above. The resolver examines the file at a given path — bare SELECT → model (a SELECT carrying `materialization: test` is a test model), `smelt.define` → function, `.csv` → seed, source `.yml` → source — which means a user can refactor across kinds without changing call sites, and `smelt.yml` stays as project-level configuration rather than a directory-type registry. The spec mandates only that `smelt.yml` exists at the workspace root.

A consequence worth naming: multi-team or multi-domain workspaces can co-locate everything for a domain — sources, seeds, tests, and models — under a single directory tree (`payments/`, `inventory/`, `support/`), with the namespace falling out of the path automatically. The kind-axis and the domain-axis stay independent. A kind-by-directory rule would have collapsed them, forcing every team to scatter their entities across `models/payments/`, `seeds/payments/`, `tests/payments/` instead of holding `payments/` together.

**Unified frontmatter attaches to the immediately following declaration.** Visually, a YAML block "introduces" what comes after it; that is the natural binding for human readers and for editors annotating it. The alternative — file-level frontmatter only — falls apart the moment a file mixes multiple bare-SELECT models with multiple `smelt.define`s, because per-declaration metadata (a model's `materialization: incremental`, one function's `deterministic: true`, another function's `backends: [duckdb]`) has nowhere to live. Per-declaration attachment with a shared parser keeps the grammar uniform across all three declaration kinds (model, `smelt.define`, `smelt.extern`) while letting feature specs catalogue their own keys (`functions.md`, `incremental_models.md`). Research §16 #22.

**Bare-model naming: lone-anonymous OR all-named, never mixed.** Two simpler-looking rules were rejected. *Always-fall-back-to-filename* (use frontmatter `name:` when present, filename otherwise) lets one file mix anonymous-named-by-filename and named-by-frontmatter SELECTs, so a reader has to scan each declaration's YAML to know what it is called. *Always-frontmatter-when-present* has the same failure mode in reverse. The all-or-nothing rule pays a one-line YAML key in multi-model files for a structural unambiguity: the *presence* of `name:` on a bare SELECT is the signal that the file is multi-model and the filename is a container; its absence is the signal that the filename is the canonical name. Multi-model files exist because some helpers (a small staging variant, a debug projection) deserve to live next to the model they serve without earning a separate file — but they must declare themselves to do so.

**Models are functions with DAG-defaulted parameters.** The Surface "Models as functions" table is a normative reframing, not a documentation flourish. Treating `smelt.<path>` references as parameters with DAG-supplied default expressions means one type system handles every surface (row-polymorphic `TableExpr` everywhere), one set of planner rules reasons across every surface (transparent boundaries are transparent regardless of whether the parameter binding came from a DAG default or an explicit caller), and parameterised models — models that take additional `TableExpr` parameters beyond their refs — fall out for free. The alternative (separate model and function pipelines, distinct call surfaces per kind) was rejected because the planner-boundary mismatch it creates would force every cross-cutting rule to be implemented twice; see `functions.md` "Models-as-functions equivalence" and research §4.

**Materialization is orthogonal to transparency.** Two genuinely independent axes — *can the planner see the body?* (transparency) and *how is the output persisted?* (materialization) — must not be collapsed onto one. Conflation forces false trade-offs: a user wanting a view-materialized model loses planner visibility, or a user wanting an ephemeral model gains nothing because materialization is doing the visibility work. Keeping them orthogonal means `materialization: table` and `materialization: ephemeral` differ only in execution scheduling — both bodies remain transparent to the planner. The planner's optimization boundary aligns with transparency, not materialization (Semantics §"Planner scope"); this is the contract `planner_integration.md` consumes.

## Constraints & Invariants

These are normative and must be upheld across all features.

1. **Rowan CST is the single representation.** No intermediate IR (no DataFusion `LogicalPlan` analogue). The same CST flows from parse to generation.
2. **`smelt-db` analysis logic is pure.** Pure functions take AST + data; Salsa queries wrap them. Analysis never calls another Salsa query *inside* the pure function — inputs are gathered by the wrapper. Current acceptable exceptions: `file_diagnostics()` and `type_context()` orchestrate Salsa to gather inputs before calling the pure check.
3. **CSTs are not mutated.** Planner output is `Transformation` values. Generated SQL is a `String`. The original CST is unchanged.
4. **Sync core, async edges.** Parsing, analysis, planning, and printing are sync. Async is at execution and at the process entry points that drive execution (LSP server, CLI, UI).
5. **`smelt-dialect` is lightweight.** No Arrow / Tokio / DuckDB dependencies. The LSP and CLI link it freely; backends do not flow back through it.
6. **No circular crate dependencies.** The dependency graph in the Surface table is total order modulo `smelt-types` (root) and `smelt-backend-*` (leaves).
7. **Parser produces a usable CST on invalid input.** No panics, no aborts, no truncated trees on syntax errors.
8. **Unknown-key doctrine: user-authored content is strict; project-level config is lenient with warnings.** Frontmatter on a model `SELECT`, a `smelt.define`, or a `smelt.extern`, plus type annotations and per-entity source / seed-sidecar YAML, are user-authored under direct review and reject unknown keys (`deny_unknown_fields`) so typos surface immediately. Project-level configuration in `smelt.yml` is reviewed less often, edited cross-team, and read by tools that pre-date keys they encounter; it warns on unknown top-level keys instead of erroring, so forward-compatible configs work across smelt versions. Per-feature specs that catalogue keys (`models.md`, `smelt_yml.md`, `functions.md`, `sources.md`) reference this doctrine rather than restating the rule.

## Known Divergences / Open Questions

Update as part of any plan that touches architecture.

- **Namespace decoupled from directory path is future work.** Today `smelt.<path>` is the literal workspace-relative directory path. A future extension could let projects declare a namespace alias (per-directory `package.yml`, top-level `smelt.yml` mapping, or a `smelt.package <name>` declaration at file scope) so deeply nested directories can present a flatter namespace — useful when an organisation's filesystem hierarchy is richer than the desired call-surface depth (e.g., `models/teams/payments/marts/balances.sql` exposed as `smelt.payments.balances`). Deferred until concrete need emerges; the literal-path rule is the default and removes one layer of indirection.
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
- **Tests**: dialect printer identity tests under `crates/smelt-dialect/tests/`; parse-level pg_query / Spark equivalence tests in `crates/smelt-parser-compat/tests/`; pure-function tests in `crates/smelt-db/tests/type_property_tests.rs`
- **User docs**: `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/developing/architecture.md`
- **Plans (history)**: see `docs/plans/` for area-specific implementation work
- **Related specs**: feature specs under `docs/specs/` extend this one — `functions.md` (the function half of the models-as-functions equivalence), `incremental_models.md` (model materialization keys), `types.md` (type vocabulary), `planner_integration.md` (planner consumption of frontmatter properties)
- **Research**: `docs/research/20260413-smelt-functions.md` §4 (the unified-model framing)
- **Legacy reference (will thin out)**: `docs/architecture_overview.md` — superseded by this spec
