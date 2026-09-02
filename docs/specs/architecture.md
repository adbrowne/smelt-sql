---
feature: architecture
status: stable
last_reviewed: 2026-07-17
owners: [andrew]
---

# Architecture

> **What this is.** The system-level spec for smelt as a SQL-to-SQL compiler and orchestrator. Defines the compilation pipeline, crate boundaries, and the architectural invariants that all feature specs depend on. Feature specs (e.g., `incremental_models.md`, `lsp.md`) sit on top of this one.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

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

**Stages above are *building blocks*; `smelt-runtime` is the *driver*.** The Generate and Execute stages — together with the surrounding lifecycle work (selection/filter pass, function-body resolution, ephemeral inlining, type-cast wrapping, time-filter injection, per-model batch dispatch, manifest writes, interval-store updates, pre-execution diagnostic gate) — are composed by `smelt-runtime`. Both `smelt-cli` and `smelt-ui` consume `smelt-runtime`'s `execute_project(request, reporter)` entry point and contribute only surface concerns (argument parsing, progress reporting, HTTP serialization). `smelt-lsp` does *not* depend on `smelt-runtime`; its needs are met entirely by the analysis layer (`smelt-parser`, `smelt-db`, `smelt-core`, `smelt-planner`). See §"Run pipeline parity rule (CLI ↔ UI)" for the normative invariant.

### Crate responsibilities

Each crate has a single job; downstream crates depend on it but never the reverse.

| Crate                  | Sync/Async         | Owns                                                         |
|------------------------|--------------------|--------------------------------------------------------------|
| `smelt-types`          | sync               | `DataType`, `TypedColumn` — SQL type vocabulary              |
| `smelt-parser`         | sync               | Lexer + recursive-descent parser; Rowan CST; AST wrappers    |
| `smelt-core`           | sync               | `Config`, `ModelFile`, `DependencyGraph`; low-level Python execution helpers (`python_models.rs`, `python_utils.rs`) |
| `smelt-logical`        | sync               | Logical `Plan`/`LogicalNode` model; planner-rule interface (`RuleContext`, `detect_builtin_rules`, rule-data classifiers for cumulative/incremental/analysis). Sits above `smelt-core`/`smelt-parser`/`smelt-types` and **below** both `smelt-db` and `smelt-planner`; the rule surface is pure so the analysis layer can evaluate it incrementally for LSP diagnostics. |
| `smelt-db`             | sync               | Salsa queries: `parse_file`, `model_refs`, `resolve_ref`, `file_diagnostics`, `model_schema` |
| `smelt-dialect`        | sync, lightweight  | `SqlDialect`, `BackendCapabilities`, dialect-aware printer   |
| `smelt-planner`        | sync               | Model-graph transforms; rule-based optimization              |
| `smelt-lsp`            | async (tower-lsp)  | Thin async shell over sync Salsa queries                     |
| `smelt-cli`            | async (entry point) | CLI surface (argument parsing, stdout reporter, `commands/`), reporter adapter over `smelt-runtime` — async because the entry point drives the async execution stage |
| `smelt-backend`        | async              | `Backend` trait (see "Backend trait surface" below), `ExecutionResult` |
| `smelt-backend-duckdb` | async              | DuckDB execution                                             |
| `smelt-backend-spark`  | async              | Spark execution                                              |
| `smelt-state`          | sync               | `RunManifest`, `IntervalStore`, `FileStore`                  |
| `smelt-fingerprint`    | sync               | `output_fingerprint` — semantic output-equivalence oracle + determinism signal (see `output_fingerprint.md`) |
| `smelt-runtime`        | async              | Compile + execute driver: `compile`, `select_executable_models`, `execute_project`, `RunReporter` trait; Python model discovery (`python.rs`) and the combined SQL-generator ↔ Python fixed-point loop (`combined_loop.rs`). Composes the analysis-layer crates above; consumed by `smelt-cli` and `smelt-ui`. Not depended on by `smelt-lsp`. |
| `smelt-ui`             | async              | Web dashboard surface (HTTP / WebSocket), reporter adapter over `smelt-runtime` |

### Project layout

A smelt workspace is rooted at a directory containing `smelt.yml`. **Directory layout is user-chosen** — the spec mandates only that `smelt.yml` exists at the root. The recommended layout (used in `examples/`) is:

| Path           | Convention                                                                        |
|----------------|-----------------------------------------------------------------------------------|
| `models/`      | `.sql` files containing bare-SELECT models (any number per file) or generator files (`generates: models` frontmatter, body of type `List<ModelDef>`). |
| `functions/`   | `.sql` files containing `smelt.define` declarations.                              |
| `seeds/`       | Static `.csv` inputs.                                                             |
| `sources/`     | Per-source `.yml` declarations (schemas for external tables).                     |
| `tests/`       | `.sql` files containing `smelt.test` declarations (any number per file).          |
| `smelt.yml`    | Project-level configuration (backend selection, feature flags such as `unstable_schema`). |

A project may rename or reorganise these freely (`staging/`, `marts/`, `external/`, etc.). The kind of an entity is determined by the file's *format and content*, not by the name of its containing directory.

### Resolution: `smelt.<path>` is the universal addressing scheme

> The `smelt.<path>` migration completed across all feature specs on 2026-05-04. Earlier kind-prefixed forms (`smelt.models.<name>`, `smelt.sources.<schema>.<table>`, `smelt.fn.<path>`) are retired; references in older plans and research documents should be read as legacy.

Every project-defined entity — model, function, seed, source, test — is addressed by a single uniform syntax:

- `smelt.<path>` — value reference (used in `FROM` position, as a `TableExpr`-typed argument, etc.)
- `smelt.<path>(<args>)` — call (used for functions and parameterised models)

The path is the entity's location relative to the project root, joined with its leaf name and separated by `.`, **with any matching `paths:` prefix stripped**. A model in the project root (the directory containing `smelt.yml`) is addressed by its bare name (`smelt.<name>`). Examples (assuming `paths: ["models"]` and the recommended layout):

| Filesystem location                                       | Reference syntax (scan-root stripped)             |
|-----------------------------------------------------------|---------------------------------------------------|
| `models/marts/customers.sql` (lone bare SELECT)           | `smelt.marts.customers`                           |
| `models/marts/file.sql` containing `name: customers`      | `smelt.marts.customers`                           |
| `functions/patterns/x.sql` declaring `session_rollup`     | `smelt.functions.patterns.session_rollup(...)`    |
| `seeds/raw/users.csv`                                     | `smelt.seeds.raw.users`                           |
| `sources/raw/events.yml`                                  | `smelt.sources.raw.events`                        |
| `tests/marts/customers.sql` declaring `smelt.test customers`            | `smelt.tests.marts.customers`                     |
| `tests/marts/file.sql` declaring `smelt.test customers_no_nulls`        | `smelt.tests.marts.customers_no_nulls` |

**`paths:` is a strip-list, not a scan gate.** smelt discovers entities by walking **every non-excluded subdirectory** under the project root; a file's kind comes from its format and content (below), never from which directory it lives in. There are **no per-kind dedicated scan roots** — `functions/`, `sources/`, `tests/`, and `seeds/` are ordinary directories whose names appear in an address only because they are not stripped. The optional `paths:` list names directory prefixes to **strip** from the resulting addresses, so a generic-container layout stays ergonomic: a project that keeps its models, sources, and seeds together under a single `src/` (or `pipeline/`) directory — grouped by domain rather than by kind — sets `paths: ["src"]` and addresses `src/staging/orders.sql` as `smelt.staging.orders` rather than `smelt.src.staging.orders`. Directories not named in `paths:` keep their full relative path as address segments, so with `paths: ["models"]` a `sources/raw/events.yml` resolves to `smelt.sources.raw.events`. (Keeping the kind-named directory in the address is exactly what gives the recommended layout its free at-a-glance kind signal — `smelt.sources.raw.events` reads as "this is a source" — without the resolver ever treating the directory specially.)

**Kind is determined by file format and content, not by syntactic prefix.** When the resolver reaches a path:

- A `.sql` file with a bare SELECT (or one of multiple named bare SELECTs) → **model** (DAG-defaulted, `TableExpr`-valued).
- A `.sql` file declaring `smelt.test <name> AS (...)` → **test** (a unit test — addressable for tooling but **not** valid in `TableExpr` positions, since a test never produces a database object; `testing.md`).
- A `.sql` file declaring `smelt.check <name> AS (...)` → **check** (a data-quality assertion against real built data — addressable for tooling but **not** valid in `TableExpr` positions, since a check never produces a database object; `testing.md`).
- A `.sql` file declaring `smelt.define <name>` → **function** (callable with arguments).
- A `.csv` file → **seed**.
- A `.yml` file with **no** sibling `.csv` (same directory, same stem) → **source** (externally-managed table; smelt declares its schema, does not load it).
- A `.yml` file **with** a sibling `.csv` (same directory, same stem) → **sidecar** to that seed; binds schema / column descriptions / `materialization` to the seed and is not itself an addressable entity.

A `.sql` file may mix declaration kinds — a model with co-located tests (each declared via `smelt.test`), a function with co-located tests — provided names are unique within the file. A given path resolves to at most one kind. There is a single uniqueness rule, and it is address-based: two entities that resolve to the same `smelt.<path>` are a workspace-load error (the `project_address_collisions` rule below). A `data/users.csv` (seed → `smelt.data.users`) and a `data/users.sql` whose lone model is named `users` (→ `smelt.data.users`) collide on that address and are rejected; a same-stem file pair that registers *different* addresses (e.g. a function-only `data/users.sql` declaring `smelt.define helper`) does not collide and is allowed.

**Address uniqueness is global across `paths:`.** When `smelt.yml::paths` lists multiple prefixes (e.g. `paths: ["models", "fixtures"]`), each is stripped independently and all addresses share a single namespace. Two files that resolve to the same `smelt.<path>` — e.g. `models/users.csv` and `fixtures/users.csv` — are a hard workspace-load error. The rule is "one path → one entity" regardless of which directory the file lives under. Enforcement is the `project_address_collisions` Salsa query (see §"Workspace loading parity rule"), which emits the `DuplicateAddress` diagnostic; it is project-scoped, so the same address declared in two different projects is independent, not a collision (§"Project isolation rule").

**Externs are flat.** `smelt.extern` declarations register a bare name in the workspace-wide builtin/extern namespace and are *not* addressed via `smelt.<path>`. A `smelt.extern read_parquet(...)` declared in `functions/io/parquet.sql` is callable everywhere as `read_parquet(...)`. The path of the declaring file affects only navigation (jump-to-definition), never the call surface. This is the one documented exception to the universal addressing scheme; externs exist precisely to extend the bare-name builtin namespace, and a path-prefixed extern would defeat that purpose.

**File kind is grammar, not location.** `data/foo.sql` containing a bare SELECT is a model addressable as `smelt.data.foo`; `random/x.sql` declaring `smelt.define helper(...)` is callable as `smelt.random.helper(...)` (the filename stem is not a path component). The recommended layout is convention; the resolver only cares about path and content.

**Bare-model naming.** A `.sql` file may contain any number of bare-SELECT models. A file's *lone* bare SELECT takes its leaf name from the filename and the file uses no section delimiter. In a file with two or more bare SELECTs, each bare SELECT **must** declare itself with a `--- name: <name> ---` section delimiter line (Layer 1, see "Two-layer multi-model file format" below); the filename ceases to register as a model name and becomes purely a container. The model's full path is the directory path joined with the leaf name. Names within a file must be unique across bare SELECTs, `smelt.test`s, `smelt.check`s, `smelt.define`s, and `smelt.extern`s. The canonical syntax for the section delimiter and worked examples live in `models.md` §"File format".

**Generator files (`generates: models`).** A `.sql` file whose YAML frontmatter declares `generates: models` is a **generator file**: its body is a meta-language expression of type `List<ModelDef>`, and each emitted `ModelDef` becomes a model in the workspace. Generator files are mutually exclusive with bare-model identity — `generates: models` cannot coexist with a `name:` field or with `--- name: ---` Layer-1 delimiters in the same file. The number of emitted models is **statically computable** from the file's body during workspace-shape resolution; the result is deterministic over a given workspace input. Each emitted model's `smelt.<path>` is `<dir_with_dots>.<file_stem>.<modeldef.name>`, where `<file_stem>` is the generator file's stem (with `.gen.sql` or `.sql` stripped). The full normative surface for generator files — the frontmatter directive, the `ModelDef` record type, the workspace-shape resolution pipeline, collision handling, and the LSP support — lives in `meta_language.md` §"Multi-model production". This section names the file kind so the `smelt.<path>` resolver and the `paths:` strip rules apply uniformly; the meta-language spec owns the per-emission semantics.

**Two-layer multi-model file format.** Multi-model `.sql` files use a two-layer stack with distinct grammars:

- **Layer 1 — section delimiter (`--- name: <name> ---`).** A line that introduces a new model section within a multi-model file. Owned by `smelt-core`; splits the file into independent model sections. The `name:` carried on this line is the **source of identity** for the model — it is the address-component used by the `smelt.<path>` resolver. A bare `--- ---` form (no `name:`) on a delimiter line is a hard parse error in a multi-model file. The lone-bare-SELECT case in a single-model file uses no Layer 1 delimiter at all.
- **Layer 2 — declaration frontmatter (`---` / `---` fences).** A YAML frontmatter block enclosed by bare `---` fences that attaches to the **immediately following declaration** (a model `SELECT`, a `smelt.define`, or a `smelt.extern`) within a section. Owned by `smelt-parser`; supplies per-declaration metadata (`materialization:`, `tags:`, `deterministic:`, etc.). A `name:` key inside Layer 2 frontmatter is **ignored** — identity comes from Layer 1 (in multi-model files) or the filename (in single-model files), never from Layer 2.

The two layers compose: Layer 1 splits a file into sections; within each section, Layer 2 frontmatter attaches to the declaration that follows it. Single-model files have no Layer 1 delimiter at all; their identity comes from the filename, and any Layer 2 `name:` key is ignored.

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

**Emitted-name collisions are caught loudly.** The `_`-join is intentionally readable rather than injective: distinct addresses can map to the same emitted name — `smelt.staging.orders` and `smelt.staging_orders` both target `main.staging_orders`. The `project_address_collisions` rule (which enforces *address* uniqueness) does not catch this, because the two addresses genuinely differ. A second, separate check therefore enforces **emitted-name uniqueness**: across every entity in a project that persists to a database object, the resolved `(target schema, joined table name)` pair must be unique, evaluated per active target. Two entities that resolve to the same `(schema, table)` are a hard error (`DuplicateEmittedName`, Error severity — see `diagnostics.md`), not a silent overwrite; this upholds the fail-loud discipline (a clobber would otherwise mean "wrong data, exit 0"). The check is structural — it depends only on each entity's address and the active target's schema, never on row contents — so it shares the identity-projection home of `project_address_collisions` (the address-identity authority, described under §"Workspace loading parity rule (CLI ↔ LSP)"). Once per-entity name overrides land, this rule is what makes a manual override the escape hatch for a genuine collision.

It does **not** apply to entities that never resolve to a database object:

- `smelt.define` functions (inlined; never persisted).
- Ephemeral models / seeds (inlined as CTE / `VALUES`; never persisted).
- Externs and built-ins (no path).

Per-entity overrides are kind-specific:

- **Sources** may override the default via the YAML `name:` key (`sources.md`) — the external pipeline names the table, smelt records it.
- **Seeds and models** cannot override individually today; the rule is mandatory. A future configurable mapping (per-entity overrides, an analogue of dbt's `generate_schema_name` / `generate_alias_name`) lifts this restriction.

### Unified frontmatter rule

A YAML frontmatter block (between `---` fences) attaches to the **immediately following declaration**: a model `SELECT`, a `smelt.test`, a `smelt.define`, or a `smelt.extern`. Each declaration may carry its own block; there is no file-level frontmatter, and no inheritance across declarations in the same file. (Research §16 #22.)

The frontmatter parser is shared across all four declaration kinds; the parsing contract is identical. Property semantics differ — per-feature key catalogues live in the relevant feature spec:

- **Function and extern keys** (`deterministic`, `idempotent`, `append_only`, `backends`, gated `joins` / `provenance`): see `functions.md`.
- **Time-dimension keys** (`timeseries`): see `timeseries.md`.
- **Model materialization keys** (`materialization`, `incremental`, …): see `incremental_models.md`.

This spec does not duplicate those catalogues; it only fixes the attachment rule and the parser-sharing invariant.

**One parser over a key catalogue.** "Shared" means *literally one* parser, not two implementations kept in step by hand. Every declaration kind's frontmatter is parsed by a single routine over a **key catalogue** — the one place a key's value-shape, the declaration kinds it applies to, and its owning feature are declared. The per-feature key sets above are entries in that catalogue; the catalogue is the sole authority on which keys exist. It is open by construction: a planner rule may contribute its own keys (its own schema entry) the way the built-in `incremental` / `timeseries` rules do, so the catalogue is a registry rather than a closed enumeration. (The mechanism by which a *non-built-in* rule registers a schema is a separate concern — see `planner_rule_api_design.md`; the parser depends only on the composed catalogue, however it was populated.)

**Error-handling is part of that contract.** Parsing a block against the catalogue yields diagnostics through the same `file_diagnostics` surface the analysis layer and the build share (per §"Diagnostic parity rule (analysis ↔ build)"), anchored at the offending declaration, with these rules:

- A key **unknown to the whole catalogue** (a typo such as `detrministic`), malformed YAML, or a value that violates its declared shape (e.g. a `granularity` outside the closed enum) is an **`Error`**.
- A key **known to the catalogue but not applicable to this declaration kind** (e.g. a function/extern key like `deterministic` on a model) is a **`Warning`**: the block is retained, the rest of its keys take effect, and the author is told the key is a no-op here. It is never an error, and never silently ignored.
- A block is **never silently discarded.** Falling back to default materialization so a declared `materialization: table` is built as a `view`, or dropping a typo'd key without a diagnostic, is forbidden — it is a silent correctness loss of exactly the kind the parity rule exists to prevent.

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

**1. Parse-level semantic anchor (PostgreSQL via pg_query).** smelt's grammar tracks PostgreSQL semantics. For SQL containing no smelt extensions, the parser's pretty-printed output is fingerprint-equivalent to the input under pg_query. This is the parser-validation oracle in `crates/smelt-parser-compat/` — it lets the grammar drift from PostgreSQL only deliberately, never by accident, and is the canonical reference for "is this construct syntactically valid smelt SQL?". Equivalence on the Spark side is checked separately against `sqlparser-rs DatabricksDialect` (and optionally `sqlglot`); pg_query is the primary anchor. This "fingerprint-equivalence" is *syntactic* (does the parsed structure round-trip?) and is distinct from the *semantic output-fingerprint* of `output_fingerprint.md`, which hashes a canonical form to decide whether two model versions compute the same relation.

**2. Print-level identity for the DuckDB dialect.** The DuckDB printer emits SQL byte-identical to the input, modulo:

- **Smelt-extension resolution** (universal across dialects): `smelt.<path>` → backend-resolvable identifier (`<schema>.<emitted_name>` for models and seeds, source-declared name for sources); `smelt.<path>(<args>)` → expanded function body; `smelt.as_struct(alias [EXCEPT …])` → DuckDB struct literal; bare-name `smelt.extern` calls are emitted verbatim (or with the configured backend-namespace remap from `functions.md`).
- **Pipe-query lowering** (universal across dialects unless the backend reports `supports_pipe_syntax`): a FROM-first `FROM t |> …` pipe query (`pipe_sql.md`) is lowered to standard SQL — collecting contiguous stages into a query level, nesting on each `AGGREGATE`, and mapping post-aggregate/post-window `|> WHERE` to `HAVING`/`QUALIFY`. The lowered SQL is a generated artifact and is not a byte-identity target. `BackendCapabilities::supports_pipe_syntax` (false for all current backends) reserves native pipe emission for capable backends.
- **Cross-dialect function-name normalization** for the short list of constructs where smelt's accepted surface diverges from DuckDB's surface:
  - `EXPLODE(x)` → `UNNEST(x)`
  - `EVERY(b)` → `BOOL_AND(b)`

  Input that already uses the DuckDB-flavoured spellings (`UNNEST`, `BOOL_AND`, `BOOL_OR`, `QUALIFY`, `DATE '…'`, `x::T`, `ARRAY[…]`, trailing commas) round-trips byte-identically.

For the Spark and PostgreSQL printers, additional rewrites apply (QUALIFY → subquery, `ARRAY[…]` → `ARRAY(…)`, `x::T` → `CAST(x AS T)`, `DATE 'lit'` → `DATE('lit')`, trailing-comma stripping, further function-name remaps such as `UNNEST` ↔ `EXPLODE`, `BOOL_OR` → `SOME`). Identity for those dialects holds only *modulo those rewrites*; they are not the byte-identity target. The capability matrix lives in `crates/smelt-dialect/src/dialect.rs::BackendCapabilities`.

### Salsa purity rule (analysis)

Analysis logic in `smelt-db` (type inference, schema extraction, diagnostic checks) is implemented as **pure functions** that take AST nodes and plain data structures — not Salsa database references. Salsa queries are thin wrappers that build inputs, call the pure function, and return the result; they exist only to provide incrementality (caching, dependency tracking, change detection). This invariant exists so a future `smelt-check` crate can do batch compilation without Salsa as a mechanical extraction.

**The rule in practice:**
- **DO** write analysis as `fn check_something(ast: &Expr, ctx: &TypeContext) -> Result`
- **DO** have Salsa queries build the inputs, call the pure function, and return the result
- **DON'T** use `db.some_query()` calls inside analysis logic — pass the data in as parameters instead
- **DON'T** make `TypeContext`, `ModelSchema`, or diagnostic functions depend on Salsa traits

Canonical examples of this pattern: `type_inference.rs` (pure functions, zero Salsa imports), `schema.rs` (pure data structures), `check_expression_types()` in `lib.rs` (pure diagnostic check). Acceptable current exceptions: `file_diagnostics()` orchestrates multiple Salsa queries to gather inputs before running checks; `type_context()` calls Salsa to resolve upstream model schemas. Both are thin orchestration wrappers — they do not embed analysis logic themselves.

### Workspace loading parity rule (CLI ↔ LSP)

Eager init-time workspace discovery — every non-excluded subdirectory under the project root walked for SQL models, function definitions (any `.sql` declaring `smelt.define`, wherever it lives — there is no hardcoded `functions/` gate), multi-model frontmatter expansion, YAML/JSON/TOML loader files for `smelt.config.load_yaml`, and deployed-schema snapshots for `docs/specs/definition_deltas.md` §"Detection" (every `.smelt/targets/<target>/schemas/<model>.json` on disk, registered as a `DeployedSchemaInput` world fact) — lives in **exactly one place**: `smelt_core::workspace::load_workspace`. Both the CLI's `init_db` and the LSP's `Backend::initialize` consume it, and the Salsa-ingest sequence (`set_project_input` → `set_source_file` → `register_loader_files_from_disk` → `register_deployed_schemas_from_disk`) is centralised in `smelt_db::workspace_ingest::ingest_loaded_workspace`. New eager-discovery steps land in `load_workspace`; both sides pick them up automatically. *Lazy* discovery (seeds via `project_seeds`, per-entity sources via `project_sources`) lives inside Salsa queries keyed on `ProjectInput` and is already shared by construction — these don't need the loader.

The invariant exists because the asymmetric-discovery bug class is the load-bearing failure mode of having two independent reimplementations of "load a smelt project from disk". Two real instances: (i) the LSP shipping without `functions/` discovery surfaced `unknown-smelt-fn` in VSCode while `cargo test -p smelt-cli --test example_diagnostics` stayed green because that test populates the DB via the CLI's discovery; (ii) the LSP not calling `set_loader_file` meant `smelt.config.load_yaml(...)` in generator files silently failed to resolve in the editor while the CLI's `init_db` worked fine. Routing every consumer through the same loader makes this class structurally impossible.

Address identity has a single owner on the same principle. The set of `smelt.<path>` addresses a project declares — across models, functions, seeds, and sources — and the detection of two files resolving to one address (the *one-path-one-entity* rule, §Resolution) are computed by a single Salsa query keyed on `ProjectInput` (`project_address_collisions`, reading a `project_address_map`). The CLI build gate and the LSP both consume its diagnostics, so collision enforcement is identical on both surfaces by construction rather than by keeping two discovery passes in sync. Because a collision is a purely *structural* fact about the set of addresses, that query depends only on each entity's address, never on its schema — editing a seed's CSV contents or a source's column types does not recompute collisions. This is the durable form of the eager/lazy distinction: not a `smelt-core` (sync) vs `smelt-db` (Salsa) crate split, but an **identity** projection (address-only, the collision authority) separated from **schema** derivation (per-entity, on demand) by Salsa query granularity and backdating.

**The rule in practice:**
- **DO** add new eager-discovery steps to `smelt_core::workspace::load_workspace`.
- **DO** route new lazy-discovery steps through Salsa queries keyed on `ProjectInput`.
- **DON'T** walk the filesystem from inside `Backend::initialize` or from CLI commands directly — call `load_workspace` instead.
- **DON'T** add a sibling discovery helper that only one side calls.

The standing CI gate is `cargo test -p smelt-lsp --test example_workspaces`, which drives the real `Backend` against every non-broken example workspace and asserts zero diagnostics. Always run this when touching LSP startup, CLI discovery, or `smelt-core::workspace`.

### Project isolation rule

A workspace folder may contain **multiple smelt projects**. A *project* is a directory containing `smelt.yml` (or `smelt.yaml`); `find_smelt_projects` discovers them recursively from the workspace folder root, and every editor workspace folder, CLI invocation, and test harness may produce 0..N projects. Each project is a **closed resolution scope**: a `smelt.<path>` reference declared inside project A resolves only against entities (models, functions, seeds, sources, tests, externs) declared inside project A. Same-name collisions across projects are not errors — they're independent, and each side sees only its own declarations. Cross-project references are not supported in the language today; if a future feature needs them, it will be opt-in via explicit declaration (e.g. a `dependencies:` block in `smelt.yml`) rather than ambient global lookup.

The invariant applies to every Salsa query that takes `workspace: Workspace` and walks `workspace.files(db)` to resolve a name or path. `resolve_ref_path` already obeys it — it iterates `workspace.projects(db)` and uses `file_path_tuple(&project_root, …)` to filter files to the project being checked. The function-resolution layer historically did not: `resolve_function` (and the closures in `function_diagnostics::sig_lookup`) walked every file in the workspace and returned the first match in sorted-path order, so a `smelt.functions.sessionize(...)` call in `examples/web_analytics/models/silver/sessions.sql` would resolve to `examples/functions_demo/functions/sessionize.sql`'s signature — which has different parameter names (`user_col` vs `partition_col`) — producing a spurious `Missing required argument` diagnostic on the call site. The CLI test suite never observed this because each `cargo test -p smelt-cli --test example_diagnostics` case ingests one project at a time; VSCode hit it the moment a user opened a monorepo folder containing both example workspaces.

The structural fix is that **every workspace-scoped resolver becomes project-scoped**: `resolve_function`, `resolve_function_path`, the `sig_lookup` closures in `workspace_function_diagnostics` and `function_body_check`, and function-call cycle detection all accept (or derive) a `ProjectInput` and only consider files whose `project_root` matches. Callers thread the project through from the file under analysis (its `source_file.project_root(db)` is the project key). The LSP's goto-def derives the project from the cursor file's project root before calling `resolve_function_path`.

**The rule in practice:**
- **DO** thread `ProjectInput` through any Salsa query that takes `workspace: Workspace` and walks `workspace.files(db)` to resolve a name.
- **DO** derive the project from the file under analysis: `source_file.project_root(db)` → `find_project(workspace, root)` → `ProjectInput`.
- **DON'T** add a workspace-flat resolver helper; if you need cross-project information later, make it opt-in (future `dependencies:` declaration in `smelt.yml`).
- **DON'T** assume a workspace folder is one project — `find_smelt_projects` may return 0..N projects.

The standing CI gate is a multi-project case in `cargo test -p smelt-lsp --test example_workspaces` that opens the entire `examples/` directory as one workspace folder and asserts no diagnostics on `web_analytics/models/silver/sessions.sql` — this case fails before the fix and passes after.

### Run pipeline parity rule (CLI ↔ UI)

The compile + execute pipeline lives in **exactly one place**: `smelt-runtime`. Both `smelt-cli` (via `commands/run.rs`) and `smelt-ui` (via `run_manager.rs`) consume it through a single `execute_project(request, reporter)` entry point. Consumer crates contribute only surface concerns — argument parsing, stdout/HTTP serialization, `RunReporter` implementations — and never reimplement compile or execute logic.

The pipeline that `smelt-runtime` owns covers the full lifecycle above the analysis layer: the selection/filter pass (resolve selectors, apply excludes, drop tests, drop generator files, expand emitted models, per-model target assignment), the compile pipeline (function-body resolution via `build_fn_body_map`, `SqlCompiler` with its `smelt_fn` / `smelt_as_struct` / `smelt_path_ref` / `smelt_path_call` emitters, ephemeral CTE inlining, `apply_type_casts`, time-filter injection), the pre-execution diagnostic gate (fail-fast on any `Error`-severity analysis diagnostic — see §"Diagnostic parity rule (analysis ↔ build)"), and the per-model execute loop (full refresh, incremental batches, cumulative dispatch, `RunManifest` writes, interval-store updates, cancellation handling). The `RunReporter` trait abstracts progress: `smelt-cli` implements it as a stdout/spinner reporter; `smelt-ui` implements it as a broadcast reporter over `RunProgressEvent`; tests implement a captured-log reporter.

The invariant exists because incidents trace to two failure modes at different layers. **Mode A** (a consumer reimplements *analysis* logic instead of calling the shared analysis layer) is the bug class the Workspace Loading Parity Rule and Project Isolation Rule address — the LSP `functions/`-discovery miss, the `set_loader_file` miss, the flat-resolver multi-project leak. **Mode B** (a consumer reimplements *compile or execute* logic because there is no shared layer to call) is the bug class this rule addresses — the UI executing `smelt.test` declarations, the UI silently passing `smelt.fn.*` calls through unexpanded because its `PrintContext` was constructed with `smelt_fn: None`, the UI skipping `apply_type_casts` and ephemeral inlining, the UI not running the pre-execution `UnknownSmeltFn` gate, the UI not expanding `*.gen.sql` generators, **the UI's backend factory supporting only DuckDB while the CLI's supported Spark — so a Spark project ran from `smelt run` but could not run from the UI at all**. Layered single-ownership closes both modes: every shared lifecycle stage has exactly one owning crate, and consumers may only depend downward.

**Backend selection is part of the parity contract — with a dependency caveat.** `smelt-runtime` stays backend-agnostic: `execute_project` receives a `&dyn BackendFactory` and never constructs a backend, so the core pipeline does not depend on the concrete backend crates (`smelt-backend-duckdb`'s C++ build, `smelt-backend-spark`'s PyO3 client). But *which engine runs a model* is a lifecycle concern, not a surface concern: the **selection** logic — mapping a target's `type:` to a constructed `Box<dyn Backend>` — must live in **exactly one** shared place that both consumers depend on, namely a backend-aggregator crate (`smelt-backends`) that depends on the concrete backend crates and is feature-gated per backend. Each consumer's `BackendFactory` impl is a thin delegate to that shared factory; it must not reimplement the `type → backend` match. A consumer that hand-rolls its own selection drifts out of parity — a backend the CLI can run, the UI must be able to run. The current duplication (`smelt-cli/src/backend_registry.rs` vs. `smelt-ui/src/run_manager.rs`'s DuckDB-only `UiBackendFactory`) is the live instance of this Mode-B drift; its removal is tracked in `docs/plans/20260628-spark-parity.md`.

**The rule in practice:**
- **DO** place new lifecycle logic at the *lowest* layer that needs it. If the LSP needs it (parsing, analysis, type inference, schemas, diagnostics, workspace discovery, planning), it lives in `smelt-parser` / `smelt-db` / `smelt-core` / `smelt-planner`. If only CLI and UI need it (compile, execute, manifests, selection/filter), it lives in `smelt-runtime`. Surface concerns live in the consumer crate.
- **DO** make `smelt-runtime`'s internals `pub(crate)` so consumers cannot construct a `CompiledModel` half-way (e.g. with type casts but no fn expansion). The constructors of `SqlCompiler`, `PrintContext`, and their emitter factories are `pub(crate)`; `compile_with_sql` (the no-ephemerals variant) is `pub(crate)`. A consumer can obtain a compiled model only through `execute_project` or `CompilerRegistry::get(...).compile_with_sql_and_ephemerals(...)`. This structural enforcement is in place and verified by the `surface_audit` test in `smelt-runtime`.
- **DON'T** add a parallel compile or execute helper inside `smelt-cli` or `smelt-ui`. If `smelt-runtime` doesn't expose the shape you need, change `smelt-runtime`.
- **DO** route backend selection through the one shared `smelt-backends` factory. Each consumer's `BackendFactory` impl only injects credentials / feature-gating and delegates the `type → Box<dyn Backend>` mapping to the shared crate — it never carries its own `match` over target types.
- **DON'T** reimplement backend selection in `smelt-cli` or `smelt-ui`. A backend one consumer can construct, the other must construct identically; divergence (e.g. the UI lacking a backend the CLI has) is a parity break the dual-consumer factory test must catch.
- **DON'T** move analysis logic *up* into `smelt-runtime`. Diagnostic checks, type inference, schema extraction, and workspace ingest must remain in `smelt-db` / `smelt-core` so the LSP can continue to consume them and a future `smelt-language-service` extraction (see Known Divergences) remains mechanical.

The shared pre-execution gate (§"Diagnostic parity rule (analysis ↔ build)") enforces parity structurally: both `smelt-cli` and `smelt-ui` must pass the same `Error`-severity `file_diagnostics` check before any model is compiled or executed, making it impossible for a consumer to silently skip a lifecycle stage the other one runs. The standing dual-consumer fixture test (`cargo test -p smelt-runtime --test execute_parity`) runs the same project through both CLI and UI entry points and asserts identical model outputs, manifest contents, and selection sets. Always run `cargo test -p smelt-runtime` when touching the compile pipeline, the execute loop, or either consumer's run path.

### Diagnostic parity rule (analysis ↔ build)

The build refuses to execute anything the analysis layer rejects. Before any model is compiled to engine SQL or executed, the pre-execution diagnostic gate runs the **same** analysis-layer diagnostics the LSP surfaces (`smelt_db::file_diagnostics`) over the selected models and their in-DAG dependencies, and **fails fast — with a non-zero exit and no execution** — if any diagnostic of `Error` severity is present. The set of build-blocking diagnostics is exactly "`severity == Error`"; it is not a hand-maintained allow-list of individual codes. `Warning` / `Info` / `Hint` diagnostics are reported but never block.

The invariant exists because the editor and the build sharing a parser but not a *verdict* is its own failure-mode-B class: a model can be clean in the LSP and in the `example_diagnostics` / `example_workspaces` gates (which assert over the analysis layer) yet silently misbuild — emit invalid engine SQL, drop a model, or, worst, materialize wrong data with exit 0. Gating the build on the analysis layer's error verdict closes the gap: anything the editor flags red cannot be built, and the two surfaces can only disagree on advisory (non-error) diagnostics.

**Scope includes built-in planner-rule diagnostics.** The contract is defined over the analysis layer — `smelt_db::file_diagnostics`, the diagnostics the editor and the build share (parsing, resolution, typing, schema, frontmatter, function/meta validation). It is **exact** for everything the analysis layer checks: the build must never miss an error the editor reports, and must never reject a model the editor considers clean. Crucially, the analysis layer also covers the built-in planner rules: every rule exposes its checks through the uniform rule → diagnostics interface (§"Planner scope"), and `file_diagnostics` aggregates those alongside its own checks. The rule-data classifiers and the `detect` interface live in `smelt-logical` (below both `smelt-db` and `smelt-planner`), so `file_diagnostics` calls them directly via the `smelt-db → smelt-logical` dependency edge — no `smelt-db → smelt-planner` production edge required. Rule *application* (`Planner`, `logical_plan_rules.rs`, etc.) remains in `smelt-planner`; both `smelt-db` and `smelt-planner` depend downward on `smelt-logical`. A rule's verdict therefore reaches the editor and the build identically: the cumulative-aggregate classifier and the incremental batch-safety/bounds analyzers are surfaced this way, so a model the build would refuse is flagged red in the editor too. The same path is the contract for a future user-authored rule: it implements the same interface and is treated identically to a built-in one. The only thing outside the guarantee is an error a planner rule raises *outside* the rule → diagnostics interface (e.g. a panic or an `execute`-stage failure with no `detect` counterpart); rules must route any condition the build should gate on through `detect` so it is visible to both surfaces.

Two properties make the rule sound rather than merely strict:
- **Error severity is the contract, and it is load-bearing.** Once the build gates on it, every `Error`-severity diagnostic is a build blocker, so a diagnostic that is *conservatively* an error (flags valid SQL the engine would accept) becomes a false build failure. Codes whose verdict the build cannot stand behind must be `Warning`, not `Error`. Precision of the error set is therefore a standing obligation, not a nicety.
- **The gate runs on what executes.** The diagnostics must reflect the SQL that will actually run, i.e. *after* function-call expansion, fragment substitution, and meta-language evaluation (see §"Models as functions" and the meta-language spec). A construct the analysis layer accepts must compile to valid engine SQL; an expansion/codegen step that cannot honor an accepted construct is itself the defect, not a reason to weaken the gate.

**The rule in practice:**
- **DO** route both consumers' run paths through one shared gate so the CLI and UI reach the identical verdict (per §"Run pipeline parity rule").
- **DO** keep the diagnostic checks themselves in the analysis layer (`smelt-db`); the gate only *consumes* `file_diagnostics`, it never reimplements a check.
- **DON'T** filter the gate to a subset of codes. The build blocks on the whole `Error` set; narrowing it is what let the misbuilds through.
- **DON'T** demote a real error to `Warning` to get a model to build. If the model is wrong, the build should refuse it; if the diagnostic is wrong, fix the check.

The standing CI gate is a "build every example" pass (`cargo test -p smelt-cli --test example_builds`) that compiles **and executes** every `examples/` workspace on DuckDB — not merely analyzing it as `example_diagnostics` does. A workspace that is analysis-clean but unbuildable fails this gate; a workspace intended to be rejected carries a `*_broken_*` marker and asserts the expected `Error` code at the gate instead.

### Diagnostic range encoding rule

Diagnostics carry **byte-offset `TextRange` values internally**, never `(line, column)` form. Conversion to `(line, column)` happens **exactly once, at the boundary** between smelt's analysis layer and an external consumer — the LSP protocol, the CLI's human-readable terminal output, or any other surface. The boundary converter is backed by a per-file `LineIndex` (the `line-index` crate from `rust-lang/rust-analyzer`) that maps `TextSize` ↔ `(line, column)` in the encoding the consumer requested.

The invariant exists because mid-flight `(line, column)` conversion has two failure modes that compound:

- **Encoding ambiguity.** `(line, column)` is meaningless without an encoding: a column index in bytes, in UTF-16 code units (LSP's default), and in Unicode codepoints diverge for any non-ASCII text. Converting early forces a choice the analysis layer cannot make — only the consumer knows whether it's serving LSP (UTF-16 by default, configurable via `positionEncodingKind`), a Unix terminal (codepoints), or a binary protocol (bytes).
- **Repeat conversion cost.** Re-running a `TextSize → (line, column)` scan at every diagnostic emission site is O(N) per call. A `LineIndex` built once per file is O(log N) per lookup with the binary-search cache. The "convert at the boundary" pattern is what rust-analyzer / rustc / Roslyn / TypeScript all do, and is the only design that doesn't degrade quadratically on large files.

The rule reaches every type in `smelt-db`'s diagnostic surface, every diagnostic-producing function, every consumer of `Diagnostic`. `smelt_db::Diagnostic::range` is a `TextRange`. The four LSP / CLI emission points (`smelt-lsp::backend::publish_diagnostics`, `smelt-cli::commands::type_::run`, `smelt-cli::commands::run::report_diagnostic`, `smelt-runtime::reporter::emit_diagnostic`) each build a `LineIndex` for the source file and convert at the protocol/terminal boundary, in the encoding negotiated with the client (LSP `positionEncodingKind`) or fixed by the surface (terminal: codepoints, matching rustc).

**The rule in practice:**
- **DO** carry `rowan::TextRange` through every analysis function, every `Diagnostic`, every Salsa query that returns positions.
- **DO** construct the `LineIndex` once per file at the boundary and reuse it across every diagnostic for that file.
- **DO** consult the LSP `positionEncodingKind` capability when constructing diagnostics for an LSP client; default to UTF-16 (LSP default) when the client advertises no preference.
- **DON'T** convert `TextRange` to `(line, column)` inside `smelt-db`, `smelt-parser`, or any other analysis crate.
- **DON'T** store `Position` / `Range` fields on `Diagnostic` or any intermediate. The `text_range_to_range` / `offset_to_position` family of helpers exist only inside boundary converters.

The standing CI gate is `cargo test -p smelt-lsp --test position_encoding`, which opens a workspace fixture containing non-ASCII identifiers and asserts that LSP diagnostics report the correct UTF-16 column at the diagnostic site under both the default UTF-16 and the negotiated UTF-8 (byte) encoding. ASCII-only fixtures continue to pass before and after this rule lands (since byte / UTF-16 / codepoint columns are identical for ASCII), so the existing `example_diagnostics` and `example_workspaces` gates are the regression baseline. Run `rg 'offset_to_position|text_range_to_range' crates/smelt-db/src/ crates/smelt-parser/src/` to confirm no analysis-crate violations were reintroduced.

### Property composition walk rule

A composition-relevant model-property verdict — bound/reach derivation, partition-alignment admission, the event-time monotonicity trace, grain/functional-dependency/determinism folding — is produced by the shared bottom-up property walk in `smelt-logical::analysis::walk` (`crates/smelt-logical/src/analysis/walk.rs`: the normalized `QueryTree`, the `Transfer` trait, per-node scope enumeration and column lineage), never by an ad hoc scan over the model's raw SQL text. Per-clause or substring scans are admissible only in two shapes: as **leaf classifiers** invoked by the walk over one already-bounded node's own region (a single window frame, one already-isolated expression, the text the walk itself carved out for that node) — never over the whole model — or as **advisory heuristics** that never feed a composition-relevant verdict (an estimate consumed only for presentation, batch-sizing, or other non-correctness-bearing output). A scan that gates admission or a derived bound while operating on the whole model's text, outside the walk, is exactly the shape this rule forbids: it under-derives nested composition (a stacked window frame or a chained join band collapses to the same margin as a single one) and skips scopes the flat scan never visits (CTE-internal `HAVING`/`DISTINCT`/`OVER`/`LIMIT`).

This mirrors `docs/specs/model_properties.md` §Constraints "Composition happens in the walk, not in scans" — same rule, authoritative there for property semantics; this section is the architectural (cross-cutting, `smelt-logical`-wide) statement of it.

The rule is crate-wide, and it binds *every* proof layer in `smelt-logical`, not only `analysis/`. Definition-diff classification (the backbuild option catalogue) and maintenance-plan derivation are consumers of the same walk substrate: a backbuild admission proof — column provenance, "derivable from stored data", discriminated-union branch recognition, functional-dependency uniqueness — reuses the walk's `ColumnLineage`, skeleton-closure, discriminant, and FD verdicts rather than re-deriving them from a flat scan over one `SELECT` list. Two recognizers for the same property (for example, two constant-literal classifiers that disagree on a typed `DATE '…'` literal) are a correctness bug, not a style issue: the moment both feed admission decisions, the same model can be proven maintainable by one layer and refused by the other.

**The rule in practice:**
- **DO** add new composition-relevant logic as a `Transfer` impl (or an extension of an existing one) in `analysis/walk.rs`, so it folds bottom-up with exhaustive scope enumeration — CTE bodies, set-operation branches, derived tables — included by construction.
- **DO** write a leaf classifier as a pure function over one AST node's own text/subtree, invoked by a `Transfer::leaf`/`Transfer::operator` implementation — never called directly against the whole model's SQL from a proof path.
- **DO** tag every surviving non-walk text scan in `smelt-logical` with a doc comment classifying it `Leaf classifier` or `Advisory heuristic`, naming what invokes it (or, for an advisory heuristic, that no admission/bound path consumes its output).
- **DON'T** add a new uppercase/lowercase-substring scan over the full model SQL to gate admission, a bound, or a monotonicity verdict. If the walk cannot yet see the construct you need, extend the walk — a scan bolted on beside it re-opens the coverage hole the walk exists to close.
- **DON'T** let an advisory heuristic's output reach an admission gate or a pushdown-eligibility proof — the moment it does, it is no longer advisory and must be migrated onto the walk (or promoted to a leaf classifier invoked by it).

The standing CI gate is `cargo test -p smelt-logical --test walk_coverage`, which scans the admission/proof modules under `crates/smelt-logical/src/{analysis,rules,maintenance,backbuild}` for raw substring text-scans (`.contains("…")` on case-folded free text) and fails on any that lack a `Leaf classifier`/`Advisory heuristic` doc-comment tag on the enclosing function (or a file-wide tag on the module's `//!` doc, used where an entire module is a deliberate advisory divergence, e.g. `analysis/temporal.rs`'s `EffectiveWindow` estimate). Run it whenever adding or modifying a text-scanning helper in `smelt-logical`.

### Planner scope

The planner handles cross-model and execution-shape transforms only:

- **In scope**: shared materialization, model fusion, ref redirection, incremental detection, query splitting, temporal/batch-safety analysis.
- **Out of scope**: predicate pushdown, join reordering, cost-based optimization within a single query — these are the backend engine's job.

The planner's `detect` phase is sync and side-effect-free; the LSP may call it to surface code-action suggestions.

**Rules expose diagnostics through one uniform interface.** Every planner rule — built-in today, user-authored in future — surfaces the conditions it rejects as diagnostics through a single rule → diagnostics interface, evaluated in the sync, side-effect-free `detect` phase. This is the seam the Diagnostic parity rule consumes: `smelt_db::file_diagnostics` runs the built-in rules through this interface and merges their diagnostics into the analysis-layer set, so a rule's verdict is visible to the editor and the build identically (§"Diagnostic parity rule (analysis ↔ build)"). The interface is uniform by design: built-in and user rules return diagnostics the same way, so adding a user rule later is mechanical and inherits parity for free. A rule must route every condition the build should refuse through this interface (a value-returning `detect`), never as a panic or an `execute`-only failure — a condition reachable only at execute time is invisible to the editor and breaks parity.

## Design

This section captures the load-bearing rationale behind the pipeline, the crate boundaries, and the project-layout / models-as-functions framings above. It does not restate the rules; it explains why those rules are shaped this way and what was rejected.

**One Rowan CST flows from parse to generation — no intermediate IR.** The conventional shape (Calcite, DataFusion, Spark Catalyst) is parse → AST → logical plan → physical plan. We reject that shape because it produces *two-IR drift*: the planner's IR slowly diverges from the parser's, dialect printers must walk a structure the user never wrote, and roundtrip identity becomes impossible. Keeping the CST as the single representation lets the dialect printer be authored as one recursive walk over the user's tokens, makes both identity properties (Semantics §"Identity properties") testable — print-level byte-identity for DuckDB and parse-level fingerprint equivalence against pg_query — and means an error-recovery CST node remains visible end-to-end. The trade-off is that planner rewrites are CST→CST rather than `LogicalPlan`→`LogicalPlan` — more verbose for cross-cutting transforms — which is paid back via the `Transformation` value vocabulary (see below) and the per-node frontmatter that drives planner reasoning (`planner_integration.md`).

**`smelt-db` analysis logic is pure (Salsa purity rule).** Salsa is the right tool for *incrementality* (cache invalidation across edits) and the wrong tool for *batch compilation* (CLI builds, planner runs, future test runners). Embedding analysis logic inside Salsa queries ties every consumer to the Salsa runtime; pulling it out so queries are thin wrappers around `fn check_x(ast, ctx) -> Result` lets a future `smelt-check` crate do batch compilation as a mechanical extraction. This invariant is upheld by convention today and will be structurally enforced once `smelt-check` exists (see Known Divergences). Rejected alternative: ship LSP-only analysis and rebuild it for batch — guaranteed drift between editor and CLI diagnostics.

**CSTs are not mutated; the planner outputs `Transformation` values.** A mutating planner (`rule.apply(&mut cst)`) is harder to debug — the diff between "before" and "after" only exists in the rule's head — and forecloses speculative planning (try a rewrite, measure, discard). Returning `Vec<Transformation>` makes rules composable (stack them, inspect them, render them in `--show-plan`), unit-testable as plain values, and reversible. See `planner_integration.md` for how rules consume frontmatter to decide which transformations to emit.

**Sync core, async edges.** Parsing, analysis, planning, and printing are CPU-bound; the per-task overhead of `tokio::spawn` would slow incremental compilation in `smelt-db` and add no parallelism (each query is small and sequential under Salsa's invalidation graph). Async lives at execution (where I/O against backends dominates) and at the process entry points that drive execution — the LSP server (where the protocol demands it), the CLI, and the UI. Crate-level async/sync labelling in the Surface table is the contract; a sync crate may not transitively depend on an async runtime.

**`smelt-dialect` is lightweight.** Both `smelt-lsp` (which surfaces "this construct is unsupported on Spark" diagnostics) and `smelt-cli` (which selects a backend dialect) must link the dialect crate. If `smelt-dialect` pulled in Arrow / Tokio / DuckDB, every consumer would inherit those dependencies — including the planner, which has no business compiling DuckDB. Keeping the dialect crate to `SqlDialect`, `BackendCapabilities`, and the printer means it sits cleanly between analysis and execution without becoming a fan-in chokepoint.

**Single addressing scheme `smelt.<path>` for all project-defined entities.** Earlier shapes used kind-specific prefixes — `smelt.ref('m')` for models, `smelt.source('raw.x')` for sources, `smelt.fn.<path>(...)` for functions, with externs flat. That asymmetry forced users to know an entity's kind before referencing it, conflated the *what* with the *where*, and made cross-kind refactors (a seed promoted to a model, a model factored as a parameterised function) churn every callsite. Collapsing every project-defined entity into `smelt.<path>` makes resolution uniform: the path locates the entity, the file format/content determines the kind, and the resolver dispatches accordingly. A reader who *wants* the kind-signal at the callsite gets it for free if the project follows the recommended layout (`smelt.sources.raw.events` reads as "this is a source"); a reader who doesn't can name their directory whatever they like. Externs remain the documented exception (flat, ambient, callable by bare name) because their job is to extend the built-in namespace — a path-prefixed extern would defeat the ergonomics that motivate them. (Research §16 #22; addressing redesigned 2026-05-01.)

**Directory layout is user-chosen; kind is determined by file format/content.** The recommended `models/` / `functions/` / `seeds/` / `sources/` layout is convention, not spec-mandated structure. Forcing kind-by-directory was rejected because it forecloses meaningful per-project organisation — a project that prefers `staging/` / `marts/` / `external/` should not have to fight the framework. Forcing kind-by-syntactic-prefix was rejected for the addressing-scheme reason above. The resolver examines the file at a given path — bare SELECT → model, `smelt.test` → test, `smelt.define` → function, `.csv` → seed, source `.yml` → source — which means a user can refactor across kinds without changing call sites, and `smelt.yml` stays as project-level configuration rather than a directory-type registry. The spec mandates only that `smelt.yml` exists at the workspace root.

A consequence worth naming: multi-team or multi-domain workspaces can co-locate everything for a domain — sources, seeds, tests, and models — under a single directory tree (`payments/`, `inventory/`, `support/`), with the namespace falling out of the path automatically. The kind-axis and the domain-axis stay independent. A kind-by-directory rule would have collapsed them, forcing every team to scatter their entities across `models/payments/`, `seeds/payments/`, `tests/payments/` instead of holding `payments/` together.

**Unified frontmatter attaches to the immediately following declaration.** Visually, a YAML block "introduces" what comes after it; that is the natural binding for human readers and for editors annotating it. The alternative — file-level frontmatter only — falls apart the moment a file mixes multiple bare-SELECT models with multiple `smelt.define`s, because per-declaration metadata (a model's `materialization: table`, one function's `deterministic: true`, another function's `backends: [duckdb]`) has nowhere to live. Per-declaration attachment with a shared parser keeps the grammar uniform across all three declaration kinds (model, `smelt.define`, `smelt.extern`) while letting feature specs catalogue their own keys (`functions.md`, `incremental_models.md`). Research §16 #22.

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
9. **Canonical-address invariant: `smelt.<path>` is the only resolution key in non-display layers.**
10. **Layered single-ownership: `smelt-logical` is the home of the logical model and rule interface.** The dependency graph is strictly downward: `smelt-db` and `smelt-planner` both depend on `smelt-logical`; `smelt-logical` depends only on `smelt-core`, `smelt-parser`, and `smelt-types`; `smelt-db` has no production dependency on `smelt-planner`. Rule *application* (`Planner`, rule files that call `apply`) lives in `smelt-planner`; the logical `Plan`/`LogicalNode` model, `RuleContext`, `detect_builtin_rules`, and the pure rule-data classifiers live in `smelt-logical` so the Salsa analysis layer can evaluate them incrementally without pulling in rule-application code. The structural assertion is `cargo tree -p smelt-db -i smelt-planner` showing no production path. Inside model SQL, every `smelt.<path>` reference is fully qualified and resolved through `resolve_ref_path` against the path tuple defined in §"Resolution"; the leaf-only-name match is never accepted. The `DependencyGraph`, selection engine, run manifest, execution-step graph, and every consumer downstream of analysis key on the canonical dot-path. Leaf-only model names exist only as a parsed-out field of `ModelFile` for diagnostic context, not as a resolution key. The CLI / UI may accept shorthand at their input boundary (`cli.md` §"Argument resolution and `--scope`"), but they must expand to the canonical path before calling any resolution or graph API, and every identifier they emit is the canonical path. This rule closes the bug class where a leaf-only resolution silently shadowed the spec-mandated path resolution (e.g. `smelt.events_parsed` resolving to `silver.events_parsed` by leaf coincidence).
11. **Fail-loud discipline.** Every path that can encounter unrecognisable user input must emit a diagnostic rather than silently falling back to a default or inferred value. The diagnostic-code catalogue lives in `docs/specs/diagnostics.md`. Four CI gates enforce this rule; they must not be lowered without a reviewer sign-off note:
    - **`unwrap`/`expect` ratchet** (`cargo test -p smelt-core --test hardening_budget`) — per-crate production `unwrap` and `.expect("` counts are frozen in `.claude/hardening-baseline.txt`; the gate fails if any count rises above baseline. New production `unwrap`/`expect` must be justified as infallible (annotated with `// infallible: …`) or converted to `Result`. Test-support crates are outside the budget, and which crates those are is derived from the workspace rather than listed: a crate qualifies when some crate dev-depends on it, no crate depends on it normally, and it produces no binary target. The binary condition is what keeps a shipped CLI in the budget when another crate reaches back to it through a test-only dependency edge. The ratchet is two-sided on both axes: a count below baseline fails as a stale baseline, and a baseline entry naming a crate the gate no longer counts fails as orphaned — without the latter, a crate dropping out of the budget would be indistinguishable from one still being measured.
    - **`println!` gate** (`cargo test -p smelt-core --test hardening_budget::no_println_in_libraries`) — production `println!` in all library crates is frozen at zero; use `tracing::{debug,info,warn}` instead. Legitimate user-facing stdout in `smelt-cli` and `smelt-ui` (command output, progress reporting) is excluded from the gate.
    - **`error`-Unknown guard** (`cargo test -p smelt-types --test unknown_census::every_unknown_site_is_classified`) — every `DataType::Unknown` construction site in production code must appear in `.claude/unknown-census.toml` with a `legitimate` or `error` classification. A new unclassified site fails the test; an `error`-classified site must be accompanied by the diagnostic that covers it.
    - **`MetadataError` exhaustiveness gate** (enforced by the Rust compiler) — `map_metadata_error_to_diagnostic` in `smelt-db/src/lib.rs` contains an exhaustive `match` over every `MetadataError` variant. A new variant added to `MetadataError` in `smelt-core/src/metadata.rs` will not compile until it is explicitly listed in this function; returning `None` is only permitted for variants demonstrably handled by a dedicated arm elsewhere in `check_file_diagnostics`. This rule closes the bug class where structural parse errors (e.g. `MalformedDelimiter`, `UnclosedFrontmatter`) were silently swallowed by a `_ => {}` catch-all, causing the LSP to show no diagnostic for a structurally invalid model file.
12. **Maintenance-plan purity: the plan is pure data, derived by pure functions, owned by `smelt-logical`.** The maintenance plan (`incremental_models.md`) — the per-cell technique assignment, clamps, ledger grading, and propagation edges — is derived once as a pure data structure; every consumer (`smelt-db` diagnostics, `smelt-planner` rule application, `smelt-runtime` lowering, the graph/propagation layer) reads that data, never re-derives it by re-running the underlying proof or admission logic itself. This mirrors rule 10's layered-ownership shape one level up the stack: one derivation, many read-only consumers. The structural assertion — a single production entry point (`derive_maintenance_plan` or equivalent) is the only caller of the admission/technique-selection logic, checked mechanically once consumers exist — is tracked as a CI gate in `docs/plans/20260707-maintenance-plan-impl.md`; today the rule is upheld by convention only, the same interim state rule 2 (Salsa purity) was in before `smelt-check` existed. The rule extends one level down to the statements themselves: every maintenance statement a run executes (region `DELETE`+`INSERT`, keyed fold `MERGE`, column-scoped `MERGE`, in-place `UPDATE`, first-run `CREATE TABLE … AS`) is the output of a pure emitter in `smelt-logical`'s maintenance layer; backends execute emitted statements and never author maintenance-statement text (`incremental_models.md` §"Statement emission (single owner)"; ledger DDL/DML in `smelt-state` is bookkeeping, explicitly excluded). The same single-owner shape applies to backbuild migration statements: every statement a backbuild script contains is the output of a pure emitter in `smelt-logical`'s backbuild layer, and those emitters share statement families with the maintenance emitters rather than forking them — an unregioned `UPDATE` is the maintenance in-place-update emitter with an absent region, not a sibling implementation. The rule also covers the diff that triggers those statements: the model definition diff has exactly one engine (token-stream comparison, whitespace-insensitive), consumed by both the maintenance `ColumnAdded` trigger and the backbuild option catalogue — a second text-equality notion beside it (for example, raw trimmed-text comparison that calls a pure reformat a semantic change) is the same two-recognizers bug named under §"Property composition walk rule". Standing CI gate: `crates/smelt-runtime/tests/statement_parity.rs` diffs `execute_project`'s executed statements against the emitters' output per family, and its structural leg (`no_maintenance_statement_authoring_outside_the_emitter`) fails the build on any production maintenance-statement text found outside the emitter module.

    **The write-pattern set is a backend-filled capability registry.** Which physical write patterns a maintenance cell may use is governed by the available-addressings rule (`incremental_models.md` §"Per-cell write addressing"), whose fourth admission factor is **backend capability**: each backend fills a **write-pattern capability registry** naming the patterns it can execute (atomic partition swap, true `UPDATE`, merge-on-read, `MERGE … WHEN NOT MATCHED BY SOURCE`, …). This registry is the architectural extension point for backend-specific write optimisations to be *contributed* rather than special-cased in the planner, and it keeps a portable project from silently depending on a primitive only one engine has. It is consistent with this invariant's single-author discipline: a backend **declares** which registered patterns it supports and **executes** the emitted statements for them; it never **authors** maintenance-statement text of its own, and a pattern the target cannot execute — or a `write:` pin naming an unrecognised pattern — fails loud (`MaintenanceWritePatternUnavailable`), never a silent downgrade. A new pattern is admitted by declaring the contract facts it requires and discharging the equivalence proof obligation, so the durable contract is the admission function, not the pattern enumeration. Whether new patterns may be registered out-of-tree or must land in `smelt-logical`'s maintenance layer to keep the emitter single-owner is an open question (§Known Divergences). Design derivation: `docs/research/20260716-relation-contract-and-per-cell-addressing.md`.

13. **SQL dialect conformance testing.** The parser's dialect claims are verified differentially against real engines, in both directions, rather than only by hand-written unit tests:
    - **Accept direction** — SQL that the target dialect (DuckDB first) accepts must either parse cleanly in smelt or appear in the known-gaps registry (`crates/smelt-parser-compat/src/gaps.rs`). The registered-gap count is a ratchet: it may go down freely, and any increase must be an explicit registry entry, never a silent skip.
    - **Fidelity direction** — any statement smelt parses with zero errors must round-trip through the printer to SQL the target engine still accepts and, where executable, evaluates. This closes the silent-mis-parse class (a query that "parses" into the wrong tree).
    - **Corpus grounding** — generated-SQL property tests are complemented by a vendored corpus of statements extracted from external engine test suites (DuckDB sqllogictest, PostgreSQL regression), filtered to the SELECT-only subset smelt targets, with a checked-in ledger of known failures.
    - **Oracle strictness (types)** — the type-inference property oracle compares inferred types against the engine's reported schema exactly (integer width and decimal precision/scale included); every tolerated difference is an explicit entry in the divergence registry (`crates/smelt-db/tests/prop_helpers/divergences.rs`), and every expression the oracle skips because smelt inferred `Unknown` is an explicit entry in a known-unknowns ledger, not a silent skip.
    - **Cross-engine emission audit** — the built-in registry's per-dialect emission verdicts are
      verified against real engines in two legs (schema and values) via the `smelt-oracle-testkit`
      suite; the standing coverage table (`docs/reference/dialect-coverage.md`) is derived from
      registry data and ledger verdicts and is gated per-PR for DuckDB and nightly for Spark
      (`multi_backend.md` §"Cross-engine emission audit").

14. **Function-registry single ownership.** A built-in SQL function's *name*, *classification* (aggregate / window / scalar), and — for functions on the registry-driven typing path — its *inferred type* **and its per-dialect, per-position emission** have exactly one authoritative home: the `BuiltinRegistry` in `crates/smelt-types/src/signatures.rs`. Recognition (whether a call names a known SQL function), the `ExprKind` seed used by the expression-kind checker, registry-first type inference, and per-dialect, per-position emission all derive from that one table, so a name added to the registry is automatically recognised, classified, typed (once migrated), and emitted correctly — and a name absent from it is diagnosed (`UnrecognizedFunction`), never half-known. Emission verdicts are keyed on `(DialectId, Position)` — `Any`/`Scalar`/`Aggregate`/`WholePartitionWindow`/`Window` — because a backend's support for a built-in routinely differs between positions, and lookup never falls back from one position to another (`multi_backend.md` §"Emission is scoped to call position"). CI gates enforce this; they must not be lowered without a reviewer sign-off note:
    - **Consistency gate** (`cargo test -p smelt-db --test integration registry_consistency::every_recognized_function_is_registry_backed`) — every name the callable-function surface (`SqlFunction`) recognises resolves in the registry with a matching classification, and every non-operator registry entry is a recognised function. A name that lives in one list but not the other fails with the missing side named. (SQL *operators* / dedicated-syntax forms — `CAST`, `LIKE`, `ILIKE`, `GLOB`, `IN`, `BETWEEN`, `IS [NOT] NULL`, `EXISTS`, `DATE_ADD`/`DATE_SUB` — carry registry entries for hover/completion but are exempt from the function-consistency check because `sig.syntax_form != SyntaxForm::Call`; the exemption is derived from the `SyntaxForm` field, not from a named exclusion list.)
    - **Migration ratchet** (`cargo test -p smelt-db --test integration registry_consistency::legacy_match_ratchet`) — the count of recognised functions still typed by the hand-written `match` in `type_inference/function_call.rs` (rather than registry-first via `try_registry_inference`) is frozen at an upper bound in `.claude/registry-migration-baseline.txt` and may only shrink. The residual functions are the ones whose return type or nullability depends on argument types/values in a way a static `Signature` cannot yet express (see Known Divergences).
    - **Alias consistency gate** (`cargo test -p smelt-db --test integration registry_consistency::every_alias_is_registry_backed`) — dialect-specific alternate spellings (`NVL`, `JSON_BUILD_OBJECT`, `GET_JSON_OBJECT`, …) are also registry-owned: each canonical `Signature` in `BuiltinRegistry` carries an `aliases: &'static [&'static str]` table, and `SqlFunction::from_name` resolves every name — canonical or alias — through `BuiltinRegistry::canonical_name`, never a second hand-written alias match. The gate asserts every registered alias is recognised by `SqlFunction::from_name`, resolves to the same canonical function the registry names, and classifies consistently with it.

    The `SqlFunction` enum is retained because it is the shared vocabulary for downstream *combiner semantics* (`smelt-logical` monotonicity/decomposition classifiers, `smelt-db` maintenance folding) — a concern distinct from recognition. Its canonical surface *and* its dialect-alias surface are both registry-derived (via `BuiltinRegistry::resolve` / `canonical_name`), so the consistency gates guarantee neither can drift from the registry.

## Known Divergences / Open Questions

Update as part of any plan that touches architecture.

- **Namespace decoupled from directory path is future work.** Today `smelt.<path>` is the literal workspace-relative directory path. A future extension could let projects declare a namespace alias (per-directory `package.yml`, top-level `smelt.yml` mapping, or a `smelt.package <name>` declaration at file scope) so deeply nested directories can present a flatter namespace — useful when an organisation's filesystem hierarchy is richer than the desired call-surface depth (e.g., `models/teams/payments/marts/balances.sql` exposed as `smelt.payments.balances`). Deferred until concrete need emerges; the literal-path rule is the default and removes one layer of indirection.
- **Dialect conformance gates (§Constraints #13) are partly implemented.** Both DuckDB directions are enforced against a real in-memory DuckDB execution oracle (`crates/smelt-parser-compat/src/duckdb_oracle.rs`): the **accept-direction** gate runs a seed corpus (`tests/corpus/duckdb_seed.sql`) — every statement DuckDB accepts must parse cleanly in smelt or match a `gaps.rs` entry — and the **fidelity-direction** gate prints every clean parse back to SQL and *executes* it on DuckDB, closing the silent-mis-parse class. A ratchet (`.claude/parser-gaps-baseline.txt`) pins the registered seed-gap count so it may only shrink. Of the seed corpus's original registered gaps, only one parse-level divergence remains: DuckDB's `POSITION(a = 1 IN b)` form parses at DuckDB's own parser but fails at its binder, so no accept-direction pressure exists to close it from the seed corpus — it is registered purely so the divergence stays fail-loud-visible rather than silent. (`TRY_CAST`, `GROUP BY ALL`, `ORDER BY ALL`, `IGNORE`/`RESPECT NULLS`, `E'…'`/`B'…'` strings, dollar-quoted strings (`$$…$$` and `$tag$…$tag$`), `INTERVAL 3 MONTH`, the `GLOB` operator, underscore digit separators, the SQL-standard function forms `trim(BOTH … FROM …)`, `substring(x FROM i FOR n)`, and `position(s IN x)`, the `MAP {key: value, …}` literal, and list comprehensions (`[expr FOR x IN list (IF cond)?]`) now parse and round-trip cleanly; `TRY_CAST` also infers its target type as always-nullable; `MAP {…}` infers as `Map(key_type, value_type)`, unifying key and value types across entries the same way array-literal elements unify; a list comprehension whose element expression is exactly its loop variable infers the source list's element type, any other element expression infers as `Unknown` since binding a scoped loop-variable type is out of scope for the current inference machinery.)
  Corpus grounding is also implemented: a vendored, SELECT-only sample drawn from DuckDB's sqllogictest suite and PostgreSQL's regression suite (`crates/smelt-parser-compat/tests/corpus/external/{duckdb,postgres}.sql`, refreshed via `scripts/extract-sql-corpus.py`) runs the same parse-or-registered check with its own shrink-only-checked failure ledger (`tests/corpus/external_ledger.toml`). Because these upstream suites intentionally probe the full breadth of each dialect's grammar, most ledger entries are bucketed by an automated pattern match against known grammar-gap signatures rather than individually hand-triaged; a residual with no matching signature is recorded generically with the actual smelt parser error as its note.
  Type-oracle strictness is also implemented: the property comparator (`crates/smelt-db/tests/prop_helpers/type_comparison.rs`) compares integer widths and Decimal precision/scale exactly, with the string family (Text/Varchar/Char) as the single named-`ByDesign` leniency; every other tolerated difference is an explicit `divergences.rs` entry (integer-width, FLOAT/DOUBLE normalisation, and the Spark-vs-DuckDB `decimal_arithmetic_model` class), and every column smelt infers as `Unknown` fails the property unless its generating expression matches a `known_unknowns.rs` entry (JSON extraction, non-portable decimal division, decimal-multiply overflow), reported at warn level when an entry goes stale. The remaining gap in §Constraints #13 is grammar support for the registered parser gaps listed above (not test strictness). Tracked in `docs/plans/20260711-parser-type-testing-hardening.md`.
- **Some built-in functions are still typed by the hand-written match, not the registry.** The registry is the single authoritative home for function *recognition* and *classification* (§Constraints #14), and for the typing of every function on the registry-driven path. A residual set — pinned by the migration ratchet (`.claude/registry-migration-baseline.txt`) — still computes its return type or nullability in `type_inference/function_call.rs` because the value depends on argument types/values in a way a static `Signature` cannot yet express: precision/width widening (`SUM`, `CEIL`/`CEILING`/`FLOOR`, `ROUND`/`TRUNC`/`TRUNCATE`, `MOD`, `MEDIAN`), argument-wrapping (`ARRAY_AGG`), argument-axis-mirroring (`DATE_TRUNC`), first-concrete-of-N with derived nullability (`COALESCE`, `IFNULL`, `NULLIF`, `GREATEST`, `LEAST`), first-argument identity with optional trailing arguments (`MODE`, `ANY_VALUE`, `ARG_MAX`, `FIRST`, `LAST`, `LAG`, `LEAD`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`, `BIT_AND`/`BIT_OR`/`BIT_XOR`), and the `EXTRACT` syntax form. Closing this requires extending the signature language (argument-dependent return types, per-signature nullability) so these move onto the registry path too; the ratchet makes the shrink durable. Tracked in `docs/plans/20260711-parser-type-testing-hardening.md`.
- **PostgreSQL emission verdicts are unverified.** `PostgreSql` is a `DialectId` variant and the
  `BuiltinRegistry` carries emission verdicts for it, but there is no backend crate and no oracle,
  so no leg of the cross-engine emission audit exercises them. The coverage table
  (`docs/reference/dialect-coverage.md`) marks every `(entry, PostgreSql)` pair as *unverified*
  rather than as *passing* until a PostgreSQL backend exists and a test leg runs against it.
- **LSP dialect diagnostics are planned but not implemented.** `smelt-dialect` is in place; the LSP does not yet emit "QUALIFY will be rewritten" hints. Add to a future plan.
- **`smelt-check` crate not yet extracted.** The Salsa purity rule is currently upheld by convention; nothing prevents a regression. Once `smelt-check` is extracted, it becomes structurally enforced.
- **Maintenance-plan purity: the *plan* half has no structural gate yet; the *statement-emission* half does.** The rule (§"Constraints & Invariants" #12) covers two things — the derived `MaintenancePlan` data structure and the statements a run executes from it. The *plan* half is upheld by convention today: it has production consumers (`smelt-db` diagnostics, `smelt-runtime` technique lowering, the propagation layer — all reading `derive_maintenance_plan`'s output), but no mechanical check yet asserts they are the only admission callers. The *statement-emission* half is gated: the region `DELETE`+`INSERT` pair, the keyed fold `MERGE` (plus its first-run `CREATE TABLE … AS`), and the column-scoped `MERGE` are each produced by a single pure emitter in `smelt-logical`'s maintenance layer (`incremental_models.md` §"Statement emission (single owner)"). `crates/smelt-runtime/tests/statement_parity.rs` proves both directions for all three families against a real `execute_project` run: the executed statements are byte-identical to a direct emitter call over the same inputs, and the resulting table state is multiset-equal to a full refresh. Its `no_maintenance_statement_authoring_outside_the_emitter` structural gate scans `smelt-backend*/src`, `smelt-runtime/src`, and `smelt-logical/src` production code (excluding the two single-owner emitter modules themselves) for the forbidden statement shapes, failing the build if one is reintroduced outside the emitters. One pre-existing gap the gate allowlists rather than closes: `Backend::delete_partitions`/`insert_overwrite` (a per-partition materialization capability that predates the maintenance layer; `IncrementalStrategy` itself has one dispatchable variant, `DeleteInsert`) still construct a `DELETE FROM` range predicate independently of the emitter — no live maintenance-plan derivation selects `insert_overwrite` today, so the duplication is dormant, but it is a second author of the same statement shape and remains open work.
- **Backbuild's executed-statement parity leg awaits wiring.** Backbuild's emitters (`crates/smelt-logical/src/backbuild/emit.rs`) share the single-owner emitter families with maintenance (`crates/smelt-logical/src/maintenance/emit.rs`) and are covered by the crate-wide `walk_coverage`/`statement_parity` structural gates, same as every other admission/proof surface. What remains is the *executed-statement* half of the parity contract §"Constraints & Invariants" #12 already proves for maintenance: no CLI/runtime consumer drives a backbuild script through a real backend yet (`.smelt/`-sourced before-SQL, CLI invocation), so there is no executed statement to diff against a direct emitter call. Tracking plan: `docs/plans/20260808-substrate-unification.md`.
- **The write-pattern capability registry is spec, not code.** Invariant #12's fourth admission factor — a backend-filled registry of executable write patterns queried by the available-addressings rule (`incremental_models.md` §"Per-cell write addressing") — does not exist as a registry today. Backend write-capability is expressed as ad-hoc trait flags (`BackendCapabilities::supports_merge`, `Backend::supports_column_scoped_merge`-style predicates), the write-pattern set is a closed `Technique` / `IncrementalStrategy` enum, and there is no per-pattern declaration of required contract facts or equivalence proof obligation. The open question of out-of-tree vs `smelt-logical`-resident pattern registration is unresolved. Design derivation: `docs/research/20260716-relation-contract-and-per-cell-addressing.md`.
- **Language-service slot is empty.** Editor-relevant analysis features (diagnostics, completions, hover, goto-def, document-version tracking) live inside `smelt-lsp::backend` mixed with tower-lsp transport. A future `smelt-language-service` crate would extract the transport-agnostic portion so that `smelt-lsp` (JSON-RPC adapter) and `smelt-ui` (HTTP/WebSocket adapter, for eventual in-browser editing) consume one shared service. The Run Pipeline Parity Rule forbids analysis logic from moving up into `smelt-runtime`, which keeps this option open without committing to its shape — extraction is triggered by UI-editor feature work, not by the runtime extraction. Until then, the LSP is the only consumer of editor-relevant analysis, and the UI does not surface live diagnostics in its model viewer.
- **Planner cost estimation is future work.** Current rules are deterministic detectors with no statistics input.
- **User-authored planner rules cannot yet be registered.** Today every planner rule is built into the binary; there is no mechanism for a project to register its own rule. When that lands it uses the same rule → diagnostics interface (§"Planner scope"), so user rules inherit Diagnostic-parity coverage by construction — this is a missing *extensibility* feature, not a gap in the parity contract.
- **Python model discovery** is specified in `python_models.md` and implemented in `smelt-runtime` (`python.rs`, `combined_loop.rs`). The PyO3 vs subprocess behavior parity (both execution paths should produce identical results) is not systematically tested — edge cases in SDK path resolution may differ.
- **Multi-backend execution model not specified beyond trait surface.** Capability negotiation (incremental support, MERGE support, ALTER COLUMN support), cross-engine reference resolution rules (when does `read_parquet()` substitution apply?), and target precedence will land in `multi_backend.md` (or an expansion of §"Backend trait surface"). Today, capability claims are scattered across `incremental_models.md`, `schema_evolution.md`, `testing.md`, and `smelt_yml.md`.
- **User journey integrity matrix open.** The cross-product of testing × incremental × schema-evolution × multi-backend is not pinned end-to-end. Pinning depends on `run_state.md` and the multi-backend spec landing first.
- **dbt comparison and migration story not specified.** Expected home: a `migration_from_dbt.md` spec or a dedicated docs-site/ guide. Until authored, the gap is a known limitation for adopters migrating from dbt.
- **Schema-inference subsystem still uses leaf names for column-origin tracking.** `RowExtension.ref_name` and `InputConstraint.ref_name` in `smelt-db` carry the leaf segment of an upstream model (the file stem) rather than the canonical `smelt.<path>` tuple. As a result, the LSP's column-goto-definition (via `smelt_db::resolve_ref_leaf`) and `smelt-db`'s own schema inference resolve upstreams by leaf name. This is structurally invisible to users today — leaf collisions across layers do not occur in the column subsystem because every column lookup is already scoped to a single project — but it preserves a leaf-only resolution path inside the schema layer that is not shared by the canonical-path resolver (`resolve_ref_path`). Migrating to canonical paths requires changing the column-origin schema types (`RowExtension`, `InputConstraint`) and threading the full path tuple through schema inference; this is a separate refactor tracked in `docs/plans/20260527-canonical-addressing-and-scope.md`.

- **Diagnostic parity (analysis ↔ build) — residual gaps after `docs/plans/20260531-diagnostic-parity.md`.** The plan closed the primary parity drift (P2–P7d, June 2026): the CLI run/build path now runs the full `Error`-severity `file_diagnostics` gate via the shared `gate_diagnostics` helper (not just `UnknownSmeltFn`); `execute_project` runs the same gate; the built-in planner rules (cumulative classifier, incremental batch-safety/bounds) surface through a uniform rule → diagnostics interface and are visible to both the editor and the build; in-model meta constructs (spread, HOFs, `columns_of`, `config.var`, wide reflection, config loaders) are evaluated at compile time and lowered to plain SQL. Residual gap: user-authored planner rule registration (extensibility) is future work — built-in rules already use the uniform interface and any new rule inherits parity by construction.

### Specs not yet authored

The spec set has explicit gaps that the following entries claim space for. Each names the in-scope future spec and which existing specs will pull content out of it:

- **Multi-backend execution model** — likely an expansion of §"Backend trait surface" or a dedicated `multi_backend.md`. Today scattered across `incremental_models.md`, `schema_evolution.md`, `testing.md`, `smelt_yml.md`.
- **`planner_api.md`** — owns the user-authored planner-rule surface. Working design at `docs/planner_rule_api_design.md`; needs review against the 2026-05-01 universal-addressing rework before becoming normative.
- **`migration_from_dbt.md`** *(or docs-site guide)* — owns the dbt analogue mapping and migration story. No content today.

Each in-spec Known Divergence cross-references this anchor.

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
- **Related specs**: feature specs under `docs/specs/` extend this one — `functions.md` (the function half of the models-as-functions equivalence), `timeseries.md` (time-dimension declaration), `incremental_models.md` (model materialization keys), `types.md` (type vocabulary), `planner_integration.md` (planner consumption of frontmatter properties), `diagnostics.md` (diagnostic-code catalogue and fail-loud discipline), `pipe_sql.md` (FROM-first pipe-query body form and its lowering at the dialect-printer seam)
- **Research**: `docs/research/20260413-smelt-functions.md` §4 (the unified-model framing)
- **Legacy reference (will thin out)**: `docs/architecture_overview.md` — superseded by this spec
