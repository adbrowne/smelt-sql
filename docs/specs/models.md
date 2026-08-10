---
feature: models
status: experimental
last_reviewed: 2026-07-23
owners: [andrew]
---

# Models

> **What this is.** A normative spec for SQL model files — the core unit of computation in smelt. Covers file format, YAML frontmatter schema, model naming, materialization modes, the refresh axis (the `full | incremental | materialized_view` trichotomy with its declared shape facts and derived grain label), the shared Relation Contract, and the `smelt.<path>` reference surface as it applies to models. The universal addressing scheme is defined in `architecture.md` §"Resolution"; the derived per-cell maintenance plan an `incremental` model runs under is `incremental_models.md`.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### File format

A model is a `.sql` file discovered by recursively walking every non-excluded directory under the project root (`paths:` does not gate discovery — it only strips address prefixes; default `["models"]` strips a leading `models/`. See `smelt_yml.md` and `architecture.md` §"Resolution"). Files may be:

**Single-model** — the file contains one SQL query, optionally preceded by YAML frontmatter:

```sql
---
materialization: table
tags: [revenue, core]
---

SELECT order_date, SUM(amount) AS revenue
FROM smelt.orders
GROUP BY 1
```

**Multi-model** — the file defines multiple models using `--- name: model_name ---` section delimiters:

```sql
--- name: staging_orders ---
materialization: ephemeral
---
SELECT * FROM smelt.sources.raw.orders WHERE status != 'cancelled'

--- name: daily_revenue ---
materialization: table
---
SELECT DATE(order_time) AS order_date, SUM(amount) AS revenue
FROM smelt.staging_orders
GROUP BY 1
```

Each section delimiter must follow the exact form `--- name: <model_name> ---` (leading/trailing spaces around the name are trimmed). Any other `--- X ---` form is a hard parse error.

### Query body forms

A model's SQL body may be written either as a standard `SELECT` statement or as a **pipe query** — the FROM-first `FROM t |> WHERE … |> AGGREGATE …` form. A body that begins with a bare `FROM` (no leading `SELECT`) followed by `|>` stages is a pipe query and is lowered to standard SQL during code generation; all frontmatter (`materialization`, `refresh`, `grain`, `tags`, …) applies identically regardless of body form. See `pipe_sql.md` for the pipe operator set, scoping rules, and lowering.

### Model naming

| File type | Name source |
|-----------|-------------|
| Single-model | `file_stem()` — the filename without extension (e.g. `daily_revenue.sql` → `daily_revenue`) |
| Multi-model | The Layer 1 `--- name: <model_name> ---` section delimiter; the optional `name:` YAML key in the section body (Layer 2 frontmatter) is ignored |

The `name:` frontmatter key in single-model files is accepted by the parser but has no effect on the model's identity; the file stem is always authoritative. Identity in multi-model files comes from the Layer 1 section delimiter, never from Layer 2 (declaration frontmatter) `name:` keys — see `architecture.md` §"Resolution" for the two-layer file-format stack and the universal addressing rules.

### YAML frontmatter keys

All keys are optional. Unknown keys are a **hard error** (`deny_unknown_fields` — the parser rejects them rather than silently ignoring them).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | — | Accepted but ignored in single-model files; overridden by delimiter in multi-model files |
| `materialization` | enum | project default (`view`) | How to store the model's output (the storage axis). See Materialization (storage) modes. |
| `refresh` | enum | `full` | How stored output is recomputed across runs (the refresh axis): `full` \| `incremental` \| `materialized_view`. See Refresh axis. |
| `grain` | enum | — | **Check-only assertion**, never a driver: `partition` \| `key`. If written, it must match the grain derived from `(timeseries?, unique_key?, partition_column ∈ key?)`, else a hard error; it drives nothing. `key_per_partition` is a derived-only label — declaring it is a hard error naming the two facts that derive it and `key` as the closest supported declared shape. See Refresh axis. |
| `unique_key` | string[] | — | The output's declared **identity** — one of the two shape-defining Relation-Contract facts. Declaring it makes the output key-addressable; whether `partition_column` is a member distinguishes a trajectory from a keyed lookup. Composite (multi-column) keys are first-class. Declared-and-checked against the SQL for aggregated key bodies. |
| `timeseries` | object | — | The output's declared **clock** — the other shape-defining Relation-Contract fact (`event_time_column`, `partition_column`, `granularity`). See `timeseries.md`. Declared-and-checked on any `refresh: incremental` output that carries a time axis; with identity also declared, the clock is admitted iff key temporal locality is established (`incremental_models.md` §"Key temporal locality"). `granularity` is also the model's partition-axis grain for cross-model propagation (`incremental_models.md` §"The graph layer"). |
| `maintenance` | object | — | Per-cell technique preferences/pins and the scan-locality guardrail (`defaults.prefer`, `cells[]`, `scan_bounds`). Owned by `incremental_models.md` §Surface. |
| `safety_overrides` | object | all false | Named escape hatches for the partition-grain safety checks (e.g. `allow_having`). Admitted only on a partition-shaped output (a declared clock, no declared identity). See `incremental_models.md` §"Partition-grain frontmatter (in `.sql` files)". |
| `backfill` | enum (`cascade` \| `local`) | `cascade` where required | Backfill cascade policy for grains with cascading footprints. Owned by `incremental_models.md`. |
| `target` | string | — | Override execution target for this model (overrides `smelt.yml` and `--target`). Not valid on `ephemeral` models. |
| `tags` | string[] | `[]` | Organization labels. Merged with `smelt.yml` model config tags (union, deduplicated). |
| `owner` | string | — | Responsible team or person. Informational; surfaced in data catalog. |
| `description` | string | — | Human-readable model description. Surfaced in data catalog. |
| `columns` | object | `{}` | Per-column metadata. See Column metadata below. |
| `backend_hints` | object | `{}` | Freeform backend-specific hints (forward compatibility). Not validated. |
| `schema_evolution` | object | — | Schema evolution strategy. See `schema_evolution.md`. |
| `format` | enum (`delta` \| `parquet`) | target default | Override table format. Ignored for DuckDB targets; Spark targets default to `delta`. |

### Three orthogonal axes

A model is described by three independent questions, each with its own surface:

| Axis | Question | Surface | Values |
|------|----------|---------|--------|
| **Kind** | What kind of node is this? | file format / `smelt.<noun>` keyword | `model` · `test` · `function` · `extern` · `seed` · `source` (see `architecture.md`) |
| **Storage** | How is a model's output stored? | `materialization:` | `view` · `table` · `ephemeral` |
| **Refresh** | How is stored output recomputed across runs? | `refresh:` (+ shape facts `timeseries:` / `unique_key:` for `incremental`) | `full` (default) · `incremental` · `materialized_view` |

`materialization` answers **only** the storage question: does the output persist data (`table`), re-evaluate on read (`view`), or inline (`ephemeral`)? Kind is determined elsewhere — a unit test is a `smelt.test` declaration (`testing.md`), not a `materialization` value. Refresh is a separate axis governing *how a stored table is kept current*. Every refresh mode pertains to a stored `table` — `refresh` applies only to `table` storage (on a `view`/`ephemeral` it is ignored with a warning; see Rules), and `table` is the default storage, so the modeller never restates `materialization: table` regardless of refresh mode.

### Materialization (storage) modes

| Value | Behavior |
|-------|----------|
| `view` | Creates a SQL view. No data stored; query re-evaluated on each read. Default if unset. |
| `table` | Persists the query result as a physical table. |
| `ephemeral` | Not materialized. SQL is inlined as a CTE into every downstream model that references it. |

An engine-maintained materialized view is **not** a storage value — it is `refresh: materialized_view` over an implied `table` (Refresh axis below). Storage answers only "does this persist data"; "who keeps it current, and how" is the refresh axis. The backend may physically emit `CREATE MATERIALIZED VIEW` to realize `refresh: materialized_view`, but that is a lowering choice (`multi_backend.md`), not a distinct storage mode the user selects.

### Refresh axis

The refresh axis is the **freshness-owner trichotomy**:

| `refresh:` | Who keeps it current | Contract |
|----------|----------------------|----------|
| `full` *(default)* | smelt, by recomputing everything each run | trivial (recompute) |
| `incremental` | smelt, by running the derived **maintenance plan** each run | processed-input equivalence, discharged per cell (`incremental_models.md`) |
| `materialized_view` | the **engine**, continuously, via native incremental-view maintenance | end-state; engine-owned (`materialized_view.md`) |

An `incremental` model additionally declares its **output shape** — what a stored row *is* — through the two orthogonal shape-defining facts of the Relation Contract (§"The Relation Contract"): the **clock** (`timeseries:`, or its absence) and the **identity** (`unique_key:`, or its absence — *including whether `partition_column` is a member of the key*). These two facts, both **declared-and-checked** on a model output (Design §"Declared shape facts, derived grain label and strategy"), fully determine what a stored row is. The four corners they inhabit:

| Declared facts | A stored row is… | Derived `grain` label |
|---|---|---|
| clock, no identity | one row of a complete, partition-addressed table | `partition` |
| identity, no clock | the end-state per key (a lookup, read in full) | `key` |
| clock + identity, **`partition_column` ∉ key** | the end-state per key with a fixed home slice (time-partitioned lookup) | `key` |
| clock + identity, **`partition_column` ∈ key** | the trajectory: one row per `(key, partition)` | `key_per_partition` (derived-only) |

`grain` is **not a declared driver** — it is a derived, reported classification computed from `(clock?, identity?, partition_column ∈ key?)`, printed by `smelt explain`, and computed for sources too (a source has an effective grain — clocked-fact, keyed-dimension, …). A modeller who wants the friendly name in frontmatter may write `grain: partition` or `grain: key` only as a **check-only assertion** (like `maintenance.scan_bounds`): it errors on mismatch with the derived facts (§"Constraint violations") and *drives nothing*. `key_per_partition` has no writable spelling — declaring `grain: key_per_partition` is a hard error naming the two facts that derive it (a `timeseries:` clock and `partition_column ∈ unique_key`) and `key` as the closest supported declared shape. This keeps a shared, human-readable shape name that can never disagree with the facts. `partition_column ∈ unique_key` is the single fact distinguishing a trajectory (the natural key recurs across partitions) from a keyed lookup whose key has a fixed home slice.

**Declaring `unique_key` is what makes an output key-shaped** — there is no "partition shape that also carries a secondary identity". A clocked output that declares an identity is a **composed clock-and-identity shape** (derived `grain: key`, time-partitioned, or `grain: key_per_partition`), never `grain: partition`. This is not a loss: the "region-addressed with respect to its main fact table, keyed for dimension changes" behaviour that once motivated an optional key on a partition grain is exactly **per-cell write addressing** on the composed shape — the fact-creation cell region-rewrites the touched partitions, the dimension-change cell merges by key (`incremental_models.md` §"Per-cell write addressing", §"What the composed shape uniquely enables"). The stored-row *shape* (one row per key, time-partitioned) is a keyed grain; how each *write* addresses rows is derived per cell. A model that wants whole-partition rewrites and never needs row identity simply declares no `unique_key` and stays `grain: partition`.

There is no history-keeping sub-declaration: retaining every version of a key (SCD2) is written as plain windowed SQL, not declared (`incremental_models.md` §Limitations).

**Neither strategy nor addressing is surface.** There is no per-model value that says "delete+insert", "merge", or "fold", and no `strategy:` sub-knob. How each part of the output is maintained under each kind of change — the technique *and* its physical write addressing (region rewrite vs keyed merge) — is **derived per `(column-group × trigger × changed-input)` cell** — the maintenance plan (`incremental_models.md`) — reported by `smelt explain`, and steerable only through the `maintenance:` preference/pin block, whose choices are validated against the equivalence invariant. One model is routinely region-rewritten with respect to its main fact table yet keyed-merged when a different input changes; no single model-wide addressing verdict could describe it (Design §"Addressing is per-cell, not per-model").

**Declared facts shape downstream consumption.** An output that declares a **clock** carries a time axis and is windowed by consumers like any clocked source; an output that declares identity but no clock is a lookup, read in full. A trajectory output (`partition_column ∈ key`) stores a per-partition history whose late-data footprint cascades forward; it is admissible only with the `backfill:` cascade discipline or a declared lateness truncation (`incremental_models.md`).

`refresh` applies only to stored tables: `refresh` on a `view` or `ephemeral` model is a warning (the config is ignored). `refresh: materialized_view` on a backend without native incremental-view maintenance is a **hard error**, not a silent fallback (`materialized_view.md`).

### Input-consumption axis (derived, not declared)

The refresh axis answers *who keeps the table current* and the grain *what a row is*. A **second, orthogonal** question governs every `incremental` model before its plan ever runs: **which input rows are new since the last run?** This is the **input-consumption axis** (equivalently, *input-delta discovery*). It changes *what is scanned*, never *what the stored relation means*, and it is **derived from each source, never declared per model**. The axis has exactly three answers:

| Input discovery | How new rows are found | Cost | Where it applies |
|---|---|---|---|
| **window-forward** | the source carries a monotone clock (a `timeseries:` declaration); read the next window(s) forward | cheap — no change-tracking metadata; pays the monotonicity price | any grain over a clocked source |
| **snapshot-diff** | re-scan the source whole and compare against stored state | no clock required; pays a full source read per run | key grains over a mutable snapshot source |
| **change feed** | the source itself reports what changed (CDC / engine change-tracking) | engine-owned; on the smelt-driven side an update-events *table* is a change feed reified as a window-forward source | `materialized_view` (engine-owned); a CDC/update-events table consumed window-forward |

**Vertical is declared, horizontal is derived.** `refresh` + the shape facts (`timeseries:` / `unique_key:`) are the *vertical* choice — the physical commitment the modeller declares. The input-consumption cell is *horizontal* — derived from the driving source's shape: a clocked source is consumed window-forward; a mutable snapshot source is re-scanned and diffed. The one non-derivable world-fact on this axis — a source's mutation profile — is declared *on the source*, shared by every consumer (`sources.md`), never per model.

Concretely, for a key-grain model over a clocked source, consumption is window-forward: the `--event-time-start/-end` run window applies to the *driving source's* `partition_column` — never to a column on the keyed output, even when the output declares its own locality-admitted `timeseries:` — and the run steps over covered source partitions in temporal order, merging each window's delta into keyed state. Which techniques a column admits under which consumption cell is checked per cell by the plan's admission obligations (`incremental_models.md` §"Per-cell admission"). Moving horizontally never changes the equivalence contract or the grain; the derived cell is a physical fact surfaced by `smelt explain`, never declared.

At the graph level, the per-source deltas this axis discovers are what drive cross-model scheduling — which downstream partitions must run when a source lands (`incremental_models.md` §"The graph layer").

### The Relation Contract

Both a **source** and a **model output** are *a relation a downstream consumer reads*, and the graph layer already treats an upstream-model edge and a source edge as **the same standing** (`incremental_models.md` §"Upstream model edges"). smelt therefore defines **one named contract vocabulary** — the **Relation Contract** — that *both* providers fill. The two slots **both providers declare** — the clock (`timeseries:`) and identity (`unique_key:`) — carry **identical field paths**, which is what lets a consumer read them uniformly; the remaining slots are **source-declared / model-derived** (a model has no field path for its mutation, completeness, or replay facts — it proves them from its plan), so there is no path to reconcile and the source keeps its own spelling for them (e.g. `mutation_profile:`). A consumer reads one contract and never cares which mode filled a slot; this is the honest mechanism behind "an upstream maintained model is a plan edge of the same standing as a `sources.*` ref". Sources and models are **two providers, not a symmetric pair** — the asymmetries are explicit.

| Contract slot | Shared field | A source fills it by | A model output fills it by |
|---|---|---|---|
| **schema** (cols / types / nullability) | `columns:` | declared | derived (type inference — `types.md`) |
| **clock** | `timeseries:` (`event_time_column` / `partition_column` / `granularity`) | declared | **declared-and-checked** |
| **identity** | `unique_key:` (incl. whether `partition_column` ∈ key) | declared (trust rule) | **declared-and-checked** |
| **mutation / arrival** | the mutation slot (append/mutable/change_feed; lateness; redelivery; retractions; ordering; delta identity; key recurrence) — source spelling `mutation_profile:` (`sources.md`) | declared (trust rule) | **derived** from the SQL + upstream facts |
| **completeness / settle** | `watermark:` / settle bound | declared | **derived** from the plan |
| **replay bound** | replayability | `retention:` | always replayable (rebuildable) |
| *source-only* | external-name `name:` routing | declared | — |
| *model-only* | per-column `contract:`, `data_latency`, the definition-change trigger | — | declared / derived |

The **fill-modes** the table names:

- **Declared** — a source's world-facts: external, unprovable, governed by the **trust rule** (widening facts trusted; narrowing facts admitted only with a verification tripwire; undeclared → strictest-but-correct — `sources.md`).
- **Derived** — a model's facts proven from its SQL and plan: trusted because proven.
- **Declared-and-checked** — a model's *shape-defining* facts (the clock and identity) where pure derivation would let a projection refactor silently flip consumer semantics: declared, then checked against the SQL, error on mismatch (§"Refresh axis", §"Constraint violations").

The **shape-defining slots — clock and identity — are exactly the two facts that fix a model output's shape** (§"Refresh axis"), so they are the declared-and-checked ones; the derived `grain` label is their summary and is computed for sources too. `sources.md` owns the source-side declaration surface and trust rules; `timeseries.md` owns the clock slot's grammar for both providers; `incremental_models.md` owns how a model *derives* the mutation, completeness, and replay slots from its plan.

### Materialization precedence (highest to lowest)

1. YAML frontmatter in the `.sql` file
2. `models.<name>.materialization` in `smelt.yml`
3. `default_materialization` in `smelt.yml` (fallback: `view`)

### `columns:` — column metadata

> **Canonical home.** This section pins the full shape of the per-model `columns:` frontmatter map. Adjacent specs (`schema_evolution.md`, `data_catalog.md`, `testing.md`, `incremental_models.md`) reference this section rather than duplicating the schema; they only define keys they normatively own (e.g. evolution semantics, catalog rendering).

Under `columns:`, each key is a column name. Each value is an object with the keys below. All keys are optional; omitted keys have no effect. The map is read by the type checker, the data catalog, the schema-evolution path, the maintenance plan, and the LSP — each consumes the keys it cares about and ignores the rest.

| Key | Type | Description | Owning spec |
|-----|------|-------------|-------------|
| `description` | string | Human-readable column description. Rendered in the data catalog. | `data_catalog.md` |
| `tests` | list | Column-level test constraints (`not_null`, `unique`, `{accepted_values: [...]}`, `{relationships: {to: ..., field: ...}}`). Unlike every other key in this table, a `tests` entry on a column absent from the inferred schema is a hard diagnostic, not a silent drop — see `data_tests.md`. | `data_tests.md` |
| `data_latency` | object | Late-arrival configuration consumed by partition-grain batch-safety analysis. | `incremental_models.md` |
| `default` | string | SQL literal used as the DEFAULT expression when adding a NOT NULL column under schema evolution. | `schema_evolution.md` |
| `backfill` | string | SQL expression applied in an UPDATE statement after the column is added, to populate existing rows. | `schema_evolution.md` |
| `contract` | enum (`exact` \| `plausible`) | The column's equivalence contract (default `exact`). `plausible` admits non-determinism in a payload column; barred from every skeleton position, with cross-model fail-loud propagation. | `incremental_models.md` |

Column **types** are not declared in this map — they are derived by the type-inference system from the model's SQL (see `types.md`). Catalog output and LSP hover both read inferred types. A future per-column `type:` annotation has not been specified.

Columns named in `columns:` but absent from the inferred schema are silently dropped from catalog output (`data_catalog.md` Semantics §"Column description sources"). Columns present in the inferred schema but absent from `columns:` appear in the catalog without per-column metadata. The `tests` key is the one exception to the silent-drop rule: a `tests` entry on a column absent from the inferred schema is a hard diagnostic rather than a silent drop, because a stale or misspelled test is a test that was never running (`data_tests.md` §Surface "Fail-loud validation").

### Reference syntax

Within model SQL, every project-defined entity — model, seed, source — is referenced using the universal `smelt.<path>` form (see `architecture.md` §"Resolution"). The path is the entity's workspace location with the matching `paths:` scan-root prefix stripped:

```sql
FROM smelt.upstream_model         -- model at models/upstream_model.sql
FROM smelt.staging.cleaned        -- model at models/staging/cleaned.sql
FROM smelt.my_seed                -- seed at seeds/my_seed.csv (seeds are valid ref targets)
FROM smelt.sources.raw.events     -- source at sources/raw/events.yml
```

Models can also be invoked as parameterised functions when they declare `TableExpr` parameters (`architecture.md` §"Models as functions"). Named-argument syntax binds parameters by name; bare `smelt.<path>` (without parens) is shorthand for the DAG-default binding:

```sql
FROM smelt.events(filter => date > '2024-01-01', limit => 1000)
```

Calling a non-parameterised model with arguments, or omitting required parameters of a parameterised model without DAG defaults, is a hard error.

### Constraint violations

| Combination | Result |
|-------------|--------|
| `ephemeral` + any non-`full` `refresh:` | Hard error |
| `ephemeral` + `timeseries` | Hard error (see `timeseries.md`) |
| `ephemeral` + `target` override | Hard error |
| `refresh: incremental` with neither `timeseries:` nor `unique_key:` | Hard error (no shape-defining fact declared — nothing maintainable) |
| `grain:` (assertion) / `maintenance` / `safety_overrides` / `backfill` without `refresh: incremental` | Hard error |
| `timeseries:` + `unique_key:` on a model, but key temporal locality cannot be established | Hard error (`incremental_models.md` §"Key temporal locality") |
| `safety_overrides` on a key-shaped output (one that declares a `unique_key:`) | Hard error (partition-shaped output only — no declared identity) |
| Declared `grain:` assertion contradicted by the derived facts — e.g. `grain: partition` with a declared `unique_key`, or `grain: key` with no `unique_key` | Hard error naming the asserted and derived grains |
| Declared `grain:` assertion contradicted by the derived plan (e.g. asserting `partition` where a cell's only admissible addressing is keyed) | Hard error naming the cell and both candidate grains (`incremental_models.md`) |
| A `plausible` contract, or a non-deterministic expression, on an `event_time_column`, `partition_column`, or `unique_key` column | Hard error naming the offending column and the role it holds (skeleton positions must stay deterministic) |
| For aggregated key bodies: `unique_key` ≠ the `GROUP BY` column set | Hard error (checked restatement) |
| `view` + non-`full` `refresh:` | Warning (stderr); refresh config ignored |
| `refresh: materialized_view` on a backend without native IVM | Hard error (`materialized_view.md`) |
| `refresh: batched` \| `keyed` \| `cumulative` \| `versioned` | Hard error: unknown refresh value, with a fix-it naming the `refresh: incremental` + shape-facts (`timeseries:` / `unique_key:`) replacement (`smelt migrate` applies it) |
| A `.sql` frontmatter `batched:` sub-block (`batched.unique_key` / `batched.safety_overrides` / `batched.nondeterministic_columns`) | Hard error naming each sub-block key the caller declared, with a fix-it giving the top-level replacement and the caller's own value under the new spelling (`unique_key:`, `safety_overrides:`, or `columns.<c>.contract: plausible` respectively) |
| A `smelt.yml` `models.<name>.batched.nondeterministic_columns` key | Hard error, regardless of value: `nondeterministic_columns` has no `smelt.yml` spelling — the fix-it names `columns.<c>.contract: plausible` in the model's `.sql` frontmatter |
| Unknown frontmatter key | Hard error (`deny_unknown_fields`) |

### Batched sub-block retirement

A `.sql` frontmatter that still declares a `batched:` sub-block is refused (see Constraint violations above): the sub-block's three keys are only ever a nested surface for options that already have top-level homes. `batched.unique_key` → top-level `unique_key:`; `batched.safety_overrides` → top-level `safety_overrides:` — both carry the same precedence as every other Relation-Contract fact (SQL frontmatter over the `smelt.yml` model override; `smelt_yml.md` §"Precedence rules" rule 2, "Incremental"). `batched.nondeterministic_columns` has no replacement inside `batched:` at all, and no `smelt.yml` spelling either: the per-column `columns.<c>.contract: plausible` declaration (`incremental_models.md` §"Non-determinism and the payload rule") is its sole replacement, declarable only in the model's own `.sql` frontmatter. The `smelt.yml` model override's `batched:` sub-block still parses its remaining two keys (`unique_key`, `safety_overrides` — §Known Divergences); a `models.<name>.batched.nondeterministic_columns` key there is refused the same way, regardless of its value, with a fix-it pointing at the `.sql`-frontmatter-only replacement. A `plausible` contract on a column that also serves as the model's `event_time_column`, `partition_column`, or a `unique_key` member is refused — those skeleton positions govern windowing, partition placement, or dedup identity and must stay deterministic, the same bar the retired list form enforced.

## Semantics

### Model discovery

smelt recursively walks each path in `paths:` (resolved relative to the project root), following symlinks, and collects all `.sql` files. Files are parsed independently — multi-model files yield multiple `ModelFile` entries, one per section.

Canonical `smelt.<path>` addresses must be unique across the project. Two files with different filesystem paths may coexist when they produce distinct addresses: `models/users.sql` (address `users`) and `models/archive/users.sql` (address `archive.users`) are legal. A `DuplicateAddress` error is raised when two declarations claim the same full canonical address — for example, two `--- name: dup ---` sections in one file, or a Python `@model` whose function name matches a SQL model's canonical address.

### Ephemeral inlining

An `ephemeral` model's SQL is substituted as a CTE at each point a downstream model references it via `smelt.<path>`. The ephemeral model itself is never materialized. If multiple downstream models reference the same ephemeral, each gets an independent copy of the CTE; there is no shared materialization.

Transitive ephemeral chains are resolved: if `a` references ephemeral `b`, which references ephemeral `c`, then `a` gets both `b` and `c` as CTEs.

### Tag merging

A model's effective tags are the deduplicated union of:
1. Tags listed under `models.<name>.tags` in `smelt.yml`
2. Tags listed in the model's `tags:` frontmatter

Order within the merged list is smelt.yml tags first, frontmatter tags second, with duplicates removed on second occurrence.

**Tag case-sensitivity.** Tag string comparison is case-sensitive throughout: `Revenue` and `revenue` are different tags. The merged set treats them as distinct entries; selectors (`tag:Revenue` vs `tag:revenue` per `model_selection.md`) match the exact case that appears in the merged set.

### Materialization change

When a model's effective materialization type changes between runs (e.g., `view` → `table`), smelt drops the existing database object (whatever type it currently is) and creates the new one. No manual `DROP` is needed.

### Shape is stable; a shape change is a new table

The declared shape facts — the clock (`timeseries:`) and identity (`unique_key:`, including whether `partition_column` is a member) — are the contract every technique writing the table agrees on, and the derived `grain` label is their summary. Changing any of them between runs is a **rebuild**, never an in-place migration: the stored rows become the wrong rows (a field entering a grouping/identity position changes *which rows exist* — `incremental_models.md` §"The definition-change trigger"). smelt must refuse to maintain a table whose derived grain (or its underlying facts) differs from the one it was built with and direct the operator to a full rebuild.

### Unknown frontmatter fields

The YAML frontmatter parser uses `serde`'s `deny_unknown_fields` mode. Any key not in the table above produces a parse error that prevents the model from loading. This is intentional: typos in frontmatter keys (e.g., `materialized: table` instead of `materialization: table`) surface immediately rather than silently using defaults.

## Design

**File stem as identity.** Model names derive from file paths so the filesystem is the source of truth. Names don't need to be declared; they fall out of where you put the file. This aligns with the `smelt.<path>` addressing principle (`architecture.md`): identity is structural, not declared. The `name:` frontmatter key is accepted (so YAML frontmatter round-trips cleanly) but is not authoritative.

**Multi-model files.** The `--- name: model_name ---` syntax allows logically-related models to live in one file without requiring a directory hierarchy. This is useful for staging + mart pairs or small pipelines that belong together conceptually. The name must be in the delimiter (not just YAML body) so the file can be scanned without full YAML parsing.

**`materialization` is the storage axis only.** dbt's `materialized` value conflates three questions — what kind of node this is (`test`), how output is stored (`table`/`view`), and how it is refreshed (`incremental`, `materialized_view`). smelt keeps these on three orthogonal axes (see "Three orthogonal axes"). `materialization` answers only "how is output stored". A unit test is a different *kind* of node (`testing.md`); refresh is a different *axis*, because two models can share a storage mode while differing in how they are recomputed.

**`materialized_view` is a refresh mode, not a storage mode.** A backend-managed materialized view persists data (so its *storage* is `table`) and is kept current *by the engine* (so its distinguishing property is on the *refresh* axis). Making it a fourth `materialization` value — as an earlier design did — repeated exactly the dbt conflation this axis split exists to avoid. The physical `CREATE MATERIALIZED VIEW` DDL a backend may emit is a lowering detail (`multi_backend.md`).

**The trichotomy is the peers argument's honest survivor.** An earlier cut of this spec named five refresh peers — `full`, `batched`, `keyed`, `versioned`, `materialized_view` — arguing each named a distinct contract. The maintenance-plan analysis (`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §13, normative conflict 1) showed the trio's *strategy content* was a lossy projection: one model legitimately needs different techniques for different inputs and different column groups, so "which technique" is a property of `(column-group × trigger)` cells, not of the model. What genuinely separates refresh values is the **freshness owner** (nobody / smelt-per-run / engine-continuous) — an operational commitment no derivation can change — so the enum is the trichotomy, and the trio's *residue* — what they actually pinned down once strategy content is derived — is the declared **shape facts** (the clock and the identity), which the derived `grain` label summarizes. The old names were **removed outright rather than kept as sugar** (`09-spec-readiness.md` decision 5): keeping them would imply the old strategy semantics; pre-1.0, the honest move is the hard cut with a `smelt migrate` assist. The former mode specs survive as **shape profiles** of the grains — now sections of `incremental_models.md` (§"The partition grain" ≈ `grain: partition`, §"The key grain" ≈ `grain: key`) — their admission matrices re-derived as instances of the plan's per-cell obligations. The `versioned` shape (SCD2) is not carried forward as a smelt-maintained profile at all: history-keeping is plain windowed SQL (`incremental_models.md` §Limitations).

**Declared shape facts, derived grain label and strategy.** What fixes "what a stored row is" stays **declared-and-checked** — but the declared facts are the clock (`timeseries:`) and identity (`unique_key:`), not a separate `grain:` token. Deriving those *facts* from the plan was considered and rejected: they govern downstream consumption (windowed source vs read-in-full lookup) and the identity requirement, so deriving them would let a projection refactor silently flip a downstream contract with no diagnostic — the exact silent-swap the declaration law exists to prevent. The `grain` *label*, by contrast, is a pure function of those already-declared facts `(clock?, identity?, partition_column ∈ key?)`, so it is **derived-and-reported** and, where the modeller writes it, admitted only as a check-only assertion that can never disagree with the facts. Strategy content — the technique **and** its physical write addressing (region rewrite vs keyed merge) — is likewise derived, proven contract-preserving per cell (the interchangeability conditions and the available-addressings rule, `incremental_models.md`), so deriving it swaps nothing observable at a fixed processed-input set. Hence: **shape facts (clock, identity) declared-and-checked; grain label and strategy derived-and-reported.** Identity stays declared even where a targeted-write cell would reveal it is *needed*, including the checked restatement against `GROUP BY` for aggregated key bodies: identity is the one thing every technique writing the table must agree on, so it stays visible in review. The trajectory (`partition_column ∈ key`) earns its own derived label because the end-state/trajectory distinction changes what a stored row *is* — and its unbounded late-data footprint is a fact the modeller should have to look at; it falls out of one declared fact (partition membership in the key), not a separate token.

**Addressing is per-cell, not per-model.** How a write physically addresses rows — a whole-partition region rewrite versus a keyed merge targeting specific rows — is a property of *a write*, not of *a model*. The same output is legitimately region-addressed with respect to its main fact table (new fact rows rewrite the touched partitions) yet keyed-addressed when a *different* input changes (a late dimension correction targets specific rows across many partitions). Pinning one addressing model-wide — which is what a declared `grain: partition` once did, carrying both "a stored row is one row of a complete clocked table" *and* "addressed by whole-partition rewrite" — contradicts the per-cell plan beneath it. The fix puts each half on its correct side of the litmus line: *what a row is* stays declared (as clock + identity facts); *addressing* moves fully into derived-per-cell, where the litmus rule already places technique (`incremental_models.md` §"Per-cell write addressing"). Deriving addressing changes nothing observable at a fixed processed-input set, so it swaps no contract — while a model-wide addressing token would invite the reader to believe it swaps what the relation means, the dbt-`incremental_strategy` footgun. Full derivation: `docs/research/20260716-relation-contract-and-per-cell-addressing.md`.

**The declaration law: declared, derived, implied.** Every fact about a model is sorted by *who fixes it*:

- **Declared** — the `refresh:` trichotomy value and the **shape facts** (`timeseries:`, `unique_key:`) — the selectors of shape and ownership — plus a bounded set of *assertions* that constrain or widen what the machinery may do without ever picking a strategy or an addressing: the check-only `grain:` label, per-column `contract`, `data_latency`, a bounded-domain budget, `horizon_ceiling`, `maintenance.scan_bounds`, the per-cell `maintenance.cells[].write` addressing pin, `safety_overrides`, the declared-monotonicity escape hatch, and (where smelt cannot derive it) source world-facts, declared on the source (`sources.md`).
- **Derived** — read off the model's SQL, its sources, and the DAG, never declared per model: the maintenance plan itself (cells, techniques, **physical write addressing**, scan clamps, partition-locality), the `grain` label, the algebraic rung, lookback/horizon, ordering, input-delta discovery, cross-model dirty-set propagation, and monotonicity where statically decidable.
- **Implied by the refresh value** — the freshness owner, and nothing else.

The selector/assertion split is the crux: only `refresh` + the shape facts (`timeseries:` / `unique_key:`) pick anything; every other declaration merely constrains. The machinery *validates* declarations against derivations and refuses on contradiction; it never chooses (`incremental_models.md` §"Validator, not chooser"; per-cell choice among proven-interchangeable techniques and admissible addressings is the one sanctioned freedom, `incremental_models.md`).

The litmus rule keeps this boundary honest against every future "can these combine?" — this is its single home; other specs reference it:

- Changes the **freshness owner or the equivalence contract itself** → a new **peer** refresh value (`materialized_view` earned its name this way; a future as-of-run/prefix-consistency contract would too).
- Changes **what a stored row is** → a declared **shape fact** — the `timeseries:` clock or the `unique_key:` identity (including whether `partition_column` is a member) — declared and checked, never a new refresh value and never a standalone `grain:` token (the `grain` *name* is *derived* from these facts; the trajectory shape falls out of `partition_column ∈ key`).
- Changes only **which technique serves a cell** → **derived** per `(column-group × trigger × changed-input)` cell, reported by `smelt explain`, steerable via `maintenance:` within proven interchangeability (`incremental_models.md`).
- Changes only **how a write physically addresses rows** (whole-partition region rewrite vs keyed merge) → **derived** per `(column-group × trigger × changed-input)` cell and validated against the equivalence invariant, reported by `smelt explain`, steerable via the `maintenance.cells[].write` pin — never a model-wide addressing token (`incremental_models.md` §"Per-cell write addressing").
- Changes only **how deltas are discovered or how much is scanned** → **derived** from the source (window-forward vs snapshot-diff vs change feed), surfaced, never declared per model.
- Wants **two contracts at two shapes** → **compose two models** in the DAG, not one mode with a sub-knob.
- **Names a reusable combiner/table shape without changing contract or shape** → a **function**, not a mode (e.g. `smelt.latest(value, ordering)` expanding to `MAX_BY` — `incremental_models.md` §"The column-family catalogue").

Modelling any of the derived rows as a declared selector was rejected for one shared reason: a selector that silently swaps *what is scanned* or *how a cell is served* invites the reader to believe it swaps *what the relation means* — dbt's `incremental_strategy` footgun. Full derivations: `docs/research/20260703-model-updates.md` Part 19; `docs/research/20260705-refresh-as-maintenance-plan/` (ratification records in 09 §1, 10 §11).

**Tag union, not override.** Tags accumulate across config layers rather than overriding. This lets a project-level `smelt.yml` add organization-wide tags (e.g., `pii`, `sla`) to specific models without preventing model authors from adding their own. Override semantics would require model authors to re-declare all project-level tags whenever they add their own.

**`deny_unknown_fields`.** Model frontmatter is the user-authored side of the unknown-key doctrine in `architecture.md` §"Constraints & Invariants" §8: user-authored content rejects unknown keys so typos surface immediately. The alternative — silently ignoring unknown keys — hides configuration mistakes and makes frontmatter edits feel non-deterministic. The error message from `serde` names the offending field, which is sufficient for the user to correct it.

**Named parameters parsed but deferred.** `smelt.<path>(filter => ...)` syntax is parsed to avoid breaking the grammar if/when full parameterised-model execution is implemented (see `architecture.md` §"Models as functions"). DAG-defaulted bare references work today; parameter-binding overrides at call sites are pre-execution. The note in user docs is the authoritative statement of current status.

## Constraints & Invariants

1. **Every model file is pure SQL.** No Jinja, no conditionals, no `is_incremental()`-style build-mode branching. The framework injects time filters and other execution-time rewrites; the logical SQL is static.
2. **Ephemeral models have no database object.** They produce no `CREATE TABLE`, `CREATE VIEW`, or DDL of any kind. Their SQL exists only as text substituted into downstream models.
3. **Tests are not models and have no database object.** A unit test is a `smelt.test` declaration (`testing.md`), not a model and not a `materialization` value. Tests are executed in-memory against a mock dataset; they never produce persistent state.
4. **Canonical addresses are unique within a project.** The discovery pass must not yield two `ModelFile` entries with the same canonical `smelt.<path>` address. Uniqueness is keyed on the full canonical address, not the bare leaf model name.
5. **Unknown frontmatter keys are rejected.** The parser must not silently accept and ignore unknown YAML keys.
6. **Tags are additive.** No frontmatter tag can remove a tag assigned by `smelt.yml`.
7. **The refresh surface carries no strategy or addressing content.** No frontmatter key may select a maintenance technique or a physical write addressing outright except the per-cell `maintenance.cells[].technique` / `maintenance.cells[].write` pins, each constrained to the cell's admitted set and validated against the equivalence invariant (`incremental_models.md`). The shape facts (clock, identity) are declared-and-checked; the `grain` label, techniques, and addressing are derived.
8. **A declared `grain:` assertion contradicted by the derived facts or plan is an error, never a silent re-grain.** The `grain:` key drives nothing — it only asserts; the machinery validates it against the derived facts and plan and never substitutes a value for them.

## Known Divergences / Open Questions

- **The refresh trichotomy is parsed and enforced; the demotion of `grain:` to a derived label has landed for the top-level surface, with one narrow gap remaining.** `RefreshStrategy` is the `full`/`incremental`/`materialized_view` trichotomy; `batched`/`keyed`/`cumulative`/`versioned` as `refresh:` values are hard errors with a fix-it. Top-level `unique_key:` (list or single-string sugar) now parses in `.sql` frontmatter and as a `smelt.yml` model override (frontmatter wins); `refresh: incremental` is admitted on the two shape-defining facts alone — `timeseries:` (clock) and/or top-level `unique_key:` (identity) — with no `grain:` required, and neither fact declared (with no `grain:` to fall back on) is the hard error this spec names (§"Constraint violations"). `grain:`, when written, is a check-only assertion validated against `derive_grain(clock?, identity?, partition_column ∈ key?)` whenever a top-level `unique_key:` is declared; it errors on mismatch, naming both labels. The narrow gap: a `grain: key` model that declares no top-level `unique_key:` (deriving its identity from the SQL body's own `GROUP BY` instead — the pre-existing surface) is not yet checked against that GROUP-BY-derived key at the frontmatter level, only at plan derivation (`smelt-db::queries::maintenance`) when a top-level `unique_key:` *is* also declared. `maintenance:` and `columns.<c>.contract` are parsed frontmatter keys (`crates/smelt-core/src/metadata.rs`). Top-level `safety_overrides:` parses (frontmatter and `smelt.yml` model override; dual-declaration with the retired sub-block spelling refuses). The `.sql` frontmatter `batched:` sub-block is retired outright — a hard error naming each declared sub-key's top-level replacement and the caller's own value (`unique_key` → top-level `unique_key:`, `safety_overrides` → top-level `safety_overrides:`, `nondeterministic_columns` → `columns.<c>.contract: plausible`). The `smelt.yml` model override's own `batched:` sub-block is a **separate parsing path** that still carries `unique_key` / `safety_overrides` — it remains the only surface for a partition-shaped model's MERGE-dedup-only `unique_key` (a row identity for its column-scoped `MERGE` technique that must not confer the identity-conferring, grain-changing meaning top-level `unique_key:` carries; a row-shaped body with no `GROUP BY` cannot occupy the composed key+clock shape top-level `unique_key:` would derive); its `nondeterministic_columns` key is retired the same as the `.sql`-frontmatter form (§"Constraint violations"). Still missing: the per-cell `maintenance.cells[].write` addressing pin and a top-level `backfill:` block do not yet exist as frontmatter keys; the derived `key_per_partition` grain (declaring it is refused at config parse) has no dedicated execution path yet, refused fail-loud at plan derivation. The `smelt migrate` assist for the hard cut does not exist. Migration ordering: `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md` §2.8; `docs/research/20260716-relation-contract-and-per-cell-addressing.md`; `docs/plans/20260719-prod-w8-composed-axes-followups.md`.
- **The shape-profile demotion of the mode specs has landed.** The grain shape profiles (`incremental_models.md` §"The partition grain", §"The key grain") are composition table + local machinery only; the maintenance contract they compose is owned by the same spec's contract and plan sections.
- **`name:` in single-model frontmatter is ignored but accepted.** This is technically inconsistent (the field is silently dropped). A future cleanup could either remove support for it or make it an alias for renaming the model (which would conflict with file-stem identity).
- **Named parameter syntax in `smelt.<path>(...)`.** Parsed, not executed. Tracked in user docs as a note; no implementation timeline.
- **`backend_hints` is completely unvalidated.** Any freeform YAML is accepted. No backend currently reads it. It is a forward-compatibility escape hatch.
- **Source mutation profile has a first-class declaration, but only `change_feed` currently changes the derived verdict.** The input-consumption cell reads a declared `mutation_profile:` (`sources.md`): a declared `change_feed` yields the change-feed verdict; absent that, the *presence* of a `timeseries:` clock is the fallback (clocked ⇒ window-forward, otherwise ⇒ snapshot-diff). A declared `append_only` or `mutable` on an unclocked source does **not yet** change the verdict. Tracked in `docs/plans/20260704-model-updates.md`.
- **`smelt explain` does not yet report the input-consumption cell or the maintenance plan** (`incremental_models.md` §Known Divergences).
- **The Relation Contract's read side is landed for the clock/identity/derived-grain slots only.** `smelt explain <model>` prints the model's own contract (clock, identity, derived `grain`) and one contract block per inbound edge — a declared source or an upstream model, rendered through the identical rows and labelled by provider. The mutation/completeness/replay slots (`sources.md` §"The Relation Contract") are not yet part of this report; a source's `mutation_profile:`, `watermark:`, and `retention:` remain readable only from the source YAML itself, not surfaced per edge.

## References

- **Code**:
  - `crates/smelt-core/src/metadata.rs` — `ModelMetadata`, `FileMetadata`, `extract_file_metadata()`
  - `crates/smelt-core/src/model_id.rs` — `ModelId`, name-from-path derivation
  - `crates/smelt-core/src/discovery.rs` — `ModelDiscovery`, model file walking
  - `crates/smelt-core/src/config.rs` — `Materialization`, `RefreshStrategy`, `ModelConfig`, `validate_model_configs()`, tag-merge logic
- **Tests**:
  - `crates/smelt-core/src/metadata.rs` (inline `#[cfg(test)]`) — frontmatter parsing, multi-model, unknown fields, format, schema evolution
  - `crates/smelt-core/src/config.rs` (inline `#[cfg(test)]`) — materialization validation, tag merging, ephemeral/test constraints
- **User docs**:
  - `docs-site/docs/guide/sql-models.md`
  - `docs-site/docs/guide/materializations.md`
- **Plans (history)**:
  - `docs/plans/20260704-model-updates.md`
  - `docs/plans/20260705-keyed-collapse.md`
- **Related specs**:
  - `architecture.md` — `smelt.<path>` addressing scheme and identity-from-structure principle
  - `incremental_models.md` — the derived per-cell maintenance plan, the `maintenance:` block, and the cross-model propagation graph; the processed-input equivalence invariant, the algebraic ladder, and the composition contract; the grain shape profiles (partition, key)
  - `model_properties.md` — the derived proofs a model's SQL can carry and the model-scoped declarations
  - `model_transforms.md` — the physical transforms a property licenses
  - `timeseries.md` — `timeseries:` frontmatter block
  - `materialized_view.md` — the `refresh: materialized_view` mode (engine-owned incremental-view maintenance)
  - `sources.md` — source world-fact declarations the input-consumption axis reads
  - `testing.md` — the `smelt.test` declaration kind
  - `data_tests.md` — the `columns.<c>.tests` grammar and derived-property-aware resolution order
  - `schema_evolution.md` — `schema_evolution:` and `columns.default/backfill` frontmatter keys
  - `pipe_sql.md` — the FROM-first pipe-query body form a model may use
