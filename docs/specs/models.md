---
feature: models
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Models

> **What this is.** A normative spec for SQL model files — the core unit of computation in smelt. Covers file format, YAML frontmatter schema, model naming, materialization modes, and the `smelt.<path>` reference surface as it applies to models. The universal addressing scheme is defined in `architecture.md` §"Resolution".
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

A model's SQL body may be written either as a standard `SELECT` statement or as a **pipe query** — the FROM-first `FROM t |> WHERE … |> AGGREGATE …` form. A body that begins with a bare `FROM` (no leading `SELECT`) followed by `|>` stages is a pipe query and is lowered to standard SQL during code generation; all frontmatter (`materialization`, `refresh`, `batched`, `tags`, …) applies identically regardless of body form. See `pipe_sql.md` for the pipe operator set, scoping rules, and lowering.

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
| `refresh` | enum | `full` | How stored output is recomputed across runs (the refresh axis). See Refresh axis. |
| `timeseries` | object | — | Time-dimension declaration (`event_time_column`, `partition_column`, `granularity`). See `timeseries.md`. Required when `refresh: batched`. |
| `batched` | object | — | Batched-refresh configuration (`unique_key`, `nondeterministic_columns`, `safety_overrides`). See `batched_models.md`. Only valid with `refresh: batched`, which requires `timeseries:`. |
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
| **Refresh** | How is stored output recomputed across runs? | `refresh:` | `full` (default) · `batched` · `cumulative` · `versioned` · `latest_value` · `accumulating_snapshot` · `materialized_view` |

`materialization` answers **only** the storage question: does the output persist data (`table`), re-evaluate on read (`view`), or inline (`ephemeral`)? Kind is determined elsewhere — a unit test is a `smelt.test` declaration (`testing.md`), not a `materialization` value. Refresh is a separate axis governing *how a stored table is kept current*: a batched model is `refresh: batched` + a `timeseries:` block (`batched_models.md`); a cumulative aggregate is `refresh: cumulative` (`cumulative_aggregate.md`); an engine-maintained view is `refresh: materialized_view` (`materialized_view.md`). Every refresh mode pertains to a stored `table` — `refresh` applies only to `table` storage (on a `view`/`ephemeral` it is ignored with a warning; see Rules), and `table` is the default storage, so the modeller never restates `materialization: table` regardless of refresh mode.

### Materialization (storage) modes

| Value | Behavior |
|-------|----------|
| `view` | Creates a SQL view. No data stored; query re-evaluated on each read. Default if unset. |
| `table` | Persists the query result as a physical table. |
| `ephemeral` | Not materialized. SQL is inlined as a CTE into every downstream model that references it. |

An engine-maintained materialized view is **not** a storage value — it is `refresh: materialized_view` over an implied `table` (Refresh axis below). Storage answers only "does this persist data"; "who keeps it current, and how" is the refresh axis. The backend may physically emit `CREATE MATERIALIZED VIEW` to realize `refresh: materialized_view`, but that is a lowering choice (`multi_backend.md`), not a distinct storage mode the user selects.

### Refresh axis

A stored `table` is recomputed across runs according to the **refresh** axis. Every non-`full` mode upholds the same underlying guarantee — its result equals a full refresh restricted to the inputs it has processed so far (the *processed-input equivalence* invariant, stated formally — with the algebraic ladder that bounds what smelt can maintain itself — in `model_maintenance.md`) — but each names a **distinct contract with its own spec**. The values are peers, never a single value with a `strategy:` sub-knob that silently swaps invariants (Design §"Refresh modes are peers").

Two observable properties separate the modes: the **output shape** they produce, and who owns keeping them current.

**Partitioned output** — a plain complete table with a `partition_column`, built by processing new data forward in batches. Same *contents* as a `full` table; differs only in *build method*.

| `refresh:` | Surface | Contract | Output shape | Freshness owner | Spec |
|----------|---------|----------|--------------|-----------------|------|
| `full` | *(default; no key)* | trivial (recompute) | table | smelt (per run) | — |
| `batched` | `refresh: batched` + `timeseries:` (+ optional `batched:` block) | per-partition slice | partitioned | smelt (per run) | `batched_models.md` |

**Keyed output** — one row per key, no partition column; downstream treats it as a lookup. Each row reflects state across all processed inputs (order-independent end-state), kept behind the user-visible relation.

| `refresh:` | Surface | Contract | Output shape | Freshness owner | Spec |
|----------|---------|----------|--------------|-----------------|------|
| `cumulative` | `refresh: cumulative` | end-state | keyed (one row per key) | smelt (per run) | `cumulative_aggregate.md` |
| `versioned` | `refresh: versioned` | end-state (interval-keyed) | keyed + validity interval | smelt (per run) | `versioned_models.md` |
| `latest_value` | `refresh: latest_value` | end-state | keyed (current row per key) | smelt (per run) | `latest_value_models.md` |
| `accumulating_snapshot` | `refresh: accumulating_snapshot` | end-state | keyed (one row per key) + bounded forward horizon | smelt (per run) | `accumulating_snapshot.md` |
| `materialized_view` | `refresh: materialized_view` | end-state | keyed | **engine** (continuous) | `materialized_view.md` |

`cumulative` and `materialized_view` produce the same-shaped result but differ in **freshness owner** — smelt-pull (correct as of the last `smelt build`) vs engine-push (kept current continuously by the backend's native incremental-view maintenance). That different operational commitment is why they are peers rather than one mode with a hidden maintainer flag. `versioned` keeps every version of a key with a validity interval; `latest_value` keeps only the current row per key (the SCD Type-2 / Type-1 patterns, named without the vendor jargon). `accumulating_snapshot` keeps one row per key filled in once by a later event (a milestone / retroactive-enrichment pattern) within a bounded forward-attribution horizon.

`refresh` applies only to stored tables: `refresh` on a `view` or `ephemeral` model is a warning (the config is ignored). `refresh: materialized_view` on a backend without native incremental-view maintenance is a **hard error**, not a silent fallback (`materialized_view.md`).

### Input-consumption axis (derived, not declared)

The refresh axis (above) answers *what a stored relation means* — its equivalence contract and output shape. A **second, orthogonal** question governs every non-`full` mode before that contract is ever applied: **which input rows are new since the last run?** This is the **input-consumption axis** (equivalently, *input-delta discovery*). It is a different kind of fact from refresh — it changes *what is scanned*, never *what the stored relation means* — and, unlike the refresh axis, it is **derived from the source, never declared per model**. There is no `strategy:` knob for it; adding one would re-introduce exactly the contract-swapping footgun the refresh peers exist to avoid (Design §"The declaration law"). Moving along this axis never touches the *processed-input equivalence* invariant (`model_maintenance.md`) — only what is scanned.

The axis has exactly three answers:

| Input discovery | How new rows are found | Cost | Where it applies |
|---|---|---|---|
| **window-forward** | the source carries a monotone clock (a `timeseries:` declaration); read the next window(s) forward | cheap — no change-tracking metadata; pays the monotonicity price (`batched_models.md` §"Event-time monotonicity trace") | `batched`; every keyed mode over a clocked source |
| **snapshot-diff** | re-scan the source whole and compare against stored state | no clock required; pays a full source read per run | keyed modes over a mutable snapshot source |
| **change feed** | the source itself reports what changed (CDC / engine change-tracking) | engine-owned; on the smelt-driven side an update-events *table* is a change feed reified as a window-forward source | `materialized_view` (engine-owned); a CDC/update-events table consumed window-forward |

**Vertical is declared, horizontal is derived.** The refresh mode is the *vertical* choice — an irreversible physical commitment the modeller declares. The input-consumption cell is the *horizontal* choice — **derived from the driving source's shape**: a source carrying a `timeseries:` clock is consumed **window-forward**; a mutable snapshot source (no clock) is re-scanned and diffed. The one non-derivable world-fact on this axis — whether a source is append-only or mutable — belongs to a declaration *on the source*, shared by every consumer, not a per-model knob (see Known Divergences §"Source mutation profile").

Concretely, for a keyed mode (`cumulative`, `latest_value`, `versioned`, `accumulating_snapshot`) over a source that carries a `timeseries:` clock, the model is consumed **window-forward**: the `--event-time-start/-end` run window applies to the *driving source's* `partition_column` — **not** to any column on the keyed output, which has no partition column — and the run steps over the covered source partitions in temporal order, merging each window's delta into keyed state. This is precisely why "forbids a `timeseries:` block" on a keyed mode forbids *output* partitioning, never event-time-aware *consumption* (`cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`).

Moving horizontally along this axis **never** changes the equivalence contract or the output shape: a `latest_value` model over a mutable snapshot and one over an append-only update stream uphold the same end-state contract; only the delta-discovery mechanics differ. The chosen cell is therefore a physical fact to be *surfaced* (via `smelt explain`, alongside batched's per-source clamp), never a semantic one the user declares.

### Materialization precedence (highest to lowest)

1. YAML frontmatter in the `.sql` file
2. `models.<name>.materialization` in `smelt.yml`
3. `default_materialization` in `smelt.yml` (fallback: `view`)

### `columns:` — column metadata

> **Canonical home.** This section pins the full shape of the per-model `columns:` frontmatter map. Adjacent specs (`schema_evolution.md`, `data_catalog.md`, `testing.md`) reference this section rather than duplicating the schema; they only define keys they normatively own (e.g. evolution semantics, catalog rendering).

Under `columns:`, each key is a column name. Each value is an object with the keys below. All keys are optional; omitted keys have no effect. The map is read by the type checker, the data catalog, the schema-evolution path, and the LSP — each consumes the keys it cares about and ignores the rest.

| Key | Type | Description | Owning spec |
|-----|------|-------------|-------------|
| `description` | string | Human-readable column description. Rendered in the data catalog. | `data_catalog.md` |
| `tests` | list | Column-level test constraints (`not_null`, `unique`, `{accepted_values: [...]}`, etc.). | `testing.md` |
| `data_latency` | object | Late-arrival configuration consumed by batched-refresh batch-safety analysis. | `batched_models.md` |
| `default` | string | SQL literal used as the DEFAULT expression when adding a NOT NULL column under schema evolution. | `schema_evolution.md` |
| `backfill` | string | SQL expression applied in an UPDATE statement after the column is added, to populate existing rows. | `schema_evolution.md` |

Column **types** are not declared in this map — they are derived by the type-inference system from the model's SQL (see `types.md`). Catalog output and LSP hover both read inferred types. A future per-column `type:` annotation has not been specified.

Columns named in `columns:` but absent from the inferred schema are silently dropped from catalog output (`data_catalog.md` Semantics §"Column description sources"). Columns present in the inferred schema but absent from `columns:` appear in the catalog without per-column metadata.

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
| `ephemeral` + `batched:` block | Hard error |
| `ephemeral` + `timeseries` | Hard error (see `timeseries.md`) |
| `ephemeral` + `target` override | Hard error |
| `refresh: batched` without `timeseries` | Hard error (`TimeseriesRequiredForBatched`) |
| `batched:` block without `refresh: batched` | Hard error |
| `refresh: cumulative` \| `versioned` \| `latest_value` \| `accumulating_snapshot` \| `materialized_view` + `timeseries` | Hard error (keyed-output modes have no partition column) |
| `refresh: cumulative` (or any keyed-output mode) + `batched:` block | Hard error (different refresh contracts) |
| `view` + non-`full` `refresh:` | Warning (stderr); refresh config ignored |
| `refresh: materialized_view` on a backend without native IVM | Hard error (`materialized_view.md`) |
| Unknown frontmatter key | Hard error (`deny_unknown_fields`) |

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

### Unknown frontmatter fields

The YAML frontmatter parser uses `serde`'s `deny_unknown_fields` mode. Any key not in the table above produces a parse error that prevents the model from loading. This is intentional: typos in frontmatter keys (e.g., `materialized: table` instead of `materialization: table`) surface immediately rather than silently using defaults.

## Design

**File stem as identity.** Model names derive from file paths so the filesystem is the source of truth. Names don't need to be declared; they fall out of where you put the file. This aligns with the `smelt.<path>` addressing principle (`architecture.md`): identity is structural, not declared. The `name:` frontmatter key is accepted (so YAML frontmatter round-trips cleanly) but is not authoritative.

**Multi-model files.** The `--- name: model_name ---` syntax allows logically-related models to live in one file without requiring a directory hierarchy. This is useful for staging + mart pairs or small pipelines that belong together conceptually. The name must be in the delimiter (not just YAML body) so the file can be scanned without full YAML parsing.

**`materialization` is the storage axis only.** dbt's `materialized` value conflates three questions — what kind of node this is (`test`), how output is stored (`table`/`view`), and how it is refreshed (`incremental`, `materialized_view`). smelt keeps these on three orthogonal axes (see "Three orthogonal axes"). `materialization` answers only "how is output stored", with three storage modes: `view` (no data, recompute on read), `table` (persisted), and `ephemeral` (inlined, no stored object). This matches the backend's own storage-only notion of materialization. A unit test is a different *kind* of node — it produces no output and nothing depends on it — so it is a `smelt.test` declaration on the kind axis (`testing.md`), not a `materialization` value. A refresh mode (`batched`, `cumulative`, `versioned`, `latest_value`, `accumulating_snapshot`, `materialized_view`) is a different *axis* — `materialization: table` (implied) + `refresh:` — because two models can share a storage mode while differing in how they are recomputed.

**`materialized_view` is a refresh mode, not a storage mode.** A backend-managed materialized view persists data (so its *storage* is `table`) and is kept current *by the engine* (so its distinguishing property is on the *refresh* axis, the engine-owned peer of the smelt-owned `cumulative`). Making it a fourth `materialization` value — as an earlier design did — repeated exactly the dbt conflation this axis split exists to avoid: it put "who owns freshness" on the storage axis. Relocating it to `refresh: materialized_view` (implied `table` storage) keeps `materialization` answering only "does this persist data" and keeps the freshness-owner question on the refresh axis where `cumulative` already lives. The physical `CREATE MATERIALIZED VIEW` DDL a backend may emit is a lowering detail (`multi_backend.md`), not a user-selected storage kind.

**Refresh modes are peers, grouped by statefulness — never a strategy sub-knob.** The refresh values are distinct named peers, each naming exactly one equivalence contract (Surface §"Refresh axis"). A single `refresh:` value with a `strategy:` sub-knob (dbt's `materialized='incremental'` + `incremental_strategy`) was rejected: the sub-knob silently swaps the equivalence contract under one name, the single most common source of dbt confusion. The peers divide by two observable properties — output shape and freshness owner (Surface §"Refresh axis"): `batched` produces a partitioned table built forward in batches; `cumulative`/`versioned`/`latest_value`/`accumulating_snapshot`/`materialized_view` produce a keyed lookup whose rows carry state across all processed inputs. `cumulative` and `batched` are not variants of one strategy — they differ in output shape (keyed vs partitioned), equivalence (end-state vs per-partition), and execution (ordered vs window-independent). Each keyed mode has its own spec (`cumulative_aggregate.md`, `versioned_models.md`, `latest_value_models.md`, `accumulating_snapshot.md`, `materialized_view.md`); deeper rationale in `docs/research/20260703-model-updates.md` (Parts 1, 16, 17).

**The declaration law: declared, derived, implied by the mode.** The features that *feel* combinable — "process it day by day", "read a CDC stream", "diff a snapshot" — do not multiply the refresh enum, because they live on a *second* axis (Surface §"Input-consumption axis"): how input deltas are discovered, orthogonal to what the stored relation means. Keeping the two axes separate is what stops the refresh enum from blowing up into a pattern×discovery grid. Sorting every fact about a model by *who fixes it* is what keeps that separation honest — each fact is **declared**, **derived**, or **implied by the mode**:

- **Declared** — the `refresh:` mode itself (the one *selector*: it chooses execution), plus a handful of *assertions* that bound or widen what the machinery may do without ever picking the strategy — source-lateness, `nondeterministic_columns`, a bounded-domain budget, a cost ceiling, the declared-monotonicity escape hatch, and (where smelt cannot derive it) the source mutation profile.
- **Derived** — read off the model's SQL and its driving source, never declared per model: the algebraic rung, the lookback/horizon, ordering, partition alignment, input-delta discovery (window-forward vs snapshot-diff vs change feed), and monotonicity *where statically decidable*.
- **Implied by the mode** — a consequence of the selected `refresh:` value, neither declared nor derived-from-SQL: the **output shape**, and the **freshness owner** (smelt-pull for every smelt-driven mode, engine-push for `materialized_view`).

The selector/assertion split is the crux: only the mode picks the strategy; every other declaration merely constrains it. The analysis machinery *validates* the declared mode against the derived facts; it never chooses the mode. The formal equivalence invariant and the algebraic ladder those derived facts feed live in `model_maintenance.md`.

A single litmus rule keeps the declared/derived/composed boundary honest against every future "can these combine?" — this is its single home; `model_maintenance.md` and the individual mode specs reference it rather than restating it:

- Changes the **equivalence contract or output shape** → a new **peer** refresh value (`materialized_view` earned its peer name this way; a maintained-trajectory or observation-series shape would too).
- Changes only **how deltas are discovered or how much is scanned** → **derived** from the source (window-forward vs snapshot-diff vs change feed; direct vs decomposed state), surfaced in `smelt explain`, never declared per model.
- Wants **two contracts at two grains** → **compose two models** in the DAG, not one mode with a sub-knob.

Modelling window-forward-vs-snapshot-diff as a `strategy:` selector was rejected for the same reason a refresh `strategy:` was: a selector that silently swaps *what is scanned* invites the reader to believe it swaps *what the relation means*. The derivation is instead read off the driving source — a source with a `timeseries:` clock is consumed forward; a mutable one is diffed — so a `latest_value` model over an append-only stream and one over a snapshot table are the same mode, differing only in a physical fact `smelt explain` reports. Full derivation: `docs/research/20260703-model-updates.md` Part 19.

**Tag union, not override.** Tags accumulate across config layers rather than overriding. This lets a project-level `smelt.yml` add organization-wide tags (e.g., `pii`, `sla`) to specific models without preventing model authors from adding their own. Override semantics would require model authors to re-declare all project-level tags whenever they add their own.

**`deny_unknown_fields`.** Model frontmatter is the user-authored side of the unknown-key doctrine in `architecture.md` §"Constraints & Invariants" §8: user-authored content rejects unknown keys so typos surface immediately. The alternative — silently ignoring unknown keys — hides configuration mistakes and makes frontmatter edits feel non-deterministic. The error message from `serde` names the offending field, which is sufficient for the user to correct it.

**Named parameters parsed but deferred.** `smelt.<path>(filter => ...)` syntax is parsed to avoid breaking the grammar if/when full parameterised-model execution is implemented (see `architecture.md` §"Models as functions"). DAG-defaulted bare references work today; parameter-binding overrides at call sites are pre-execution. The note in user docs is the authoritative statement of current status.

## Constraints & Invariants

1. **Every model file is pure SQL.** No Jinja, no conditionals, no `is_incremental()`-style build-mode branching. The framework injects time filters and other execution-time rewrites; the logical SQL is static.
2. **Ephemeral models have no database object.** They produce no `CREATE TABLE`, `CREATE VIEW`, or DDL of any kind. Their SQL exists only as text substituted into downstream models.
3. **Tests are not models and have no database object.** A unit test is a `smelt.test` declaration (`testing.md`), not a model and not a `materialization` value. Tests are executed in-memory against a mock dataset; they never produce persistent state.
4. **Canonical addresses are unique within a project.** The discovery pass must not yield two `ModelFile` entries with the same canonical `smelt.<path>` address. Uniqueness is keyed on the full canonical address, not the bare leaf model name — `models/users.sql` (address `users`) and `models/archive/users.sql` (address `archive.users`) are distinct and legal.
5. **Unknown frontmatter keys are rejected.** The parser must not silently accept and ignore unknown YAML keys.
6. **Tags are additive.** No frontmatter tag can remove a tag assigned by `smelt.yml`.

## Known Divergences / Open Questions

- **`name:` in single-model frontmatter is ignored but accepted.** This is technically inconsistent (the field is silently dropped). A future cleanup could either remove support for it or make it an alias for renaming the model (which would conflict with file-stem identity).
- **Named parameter syntax in `smelt.<path>(...)`.** Parsed, not executed. Tracked in user docs as a note; no implementation timeline.
- **`backend_hints` is completely unvalidated.** Any freeform YAML is accepted. No backend currently reads it. It is a forward-compatibility escape hatch.
- **Keyed refresh modes beyond `cumulative` are not fully built.** The refresh axis names `versioned`, `latest_value`, `accumulating_snapshot`, and `materialized_view` as normative peers, each with its own spec, but only `full`, `batched`, and `cumulative` are built today. `versioned`, `latest_value`, and `accumulating_snapshot` do **not parse** — `RefreshStrategy` accepts only `full`/`batched`/`cumulative`/`materialized_view`, so declaring them produces an unknown-refresh-value error. `materialized_view` parses but hard-errors on every current backend (none has native IVM; `materialized_view.md`). Delivery: `versioned` / `latest_value` / `materialized_view` by `docs/plans/20260704-model-updates.md`; `accumulating_snapshot` by `docs/plans/20260704-accumulating-snapshot.md` (worked design `docs/research/20260703-model-updates.md` Part 20).
- **Source mutation profile has a first-class declaration, but only `change_feed` currently changes the derived verdict.** The input-consumption cell (Surface §"Input-consumption axis") reads a declared `mutation_profile:` (`sources.md` §"Source YAML shape" — `append_only` / `mutable` / `change_feed`): a declared `change_feed` yields the change-feed verdict; absent that, the *presence* of a `timeseries:` clock is the fallback (clocked ⇒ window-forward, otherwise ⇒ snapshot-diff). A declared `append_only` or `mutable` on an unclocked source does **not yet** change the verdict — it still falls back to snapshot-diff, same as leaving the field undeclared; only the change-feed case is a wired discovery path today. The *observation-series* rejection (a non-replayable input under a partitioned output, `docs/research/20260703-model-updates.md` §19.6 cell 2) is also not yet wired to this declaration. Tracked in `docs/plans/20260704-model-updates.md`.
- **Snapshot-diff mechanics for keyed modes are under-specified.** What the `--event-time` run flags mean for a snapshot-diff run (there is no window), whether the diff is bounded, and how `--auto` staleness fires for a clock-less source are open (`docs/research/20260703-model-updates.md` §19.8). Window-forward consumption is the specified path today; the sibling keyed specs (`latest_value_models.md`, `versioned_models.md`) note snapshot-diff as the identical-contract, full-rescan alternative.
- **`smelt explain` does not yet report the input-consumption cell.** The derived cell (window-forward vs snapshot-diff) is a physical fact the Design section says belongs in `smelt explain` alongside batched's `source_bounds`, but explain does not emit it yet (`batched_models.md` Known Divergences records the parallel gap for the per-source clamp).

## References

- **Code**:
  - `crates/smelt-core/src/metadata.rs` — `ModelMetadata`, `FileMetadata`, `extract_file_metadata()`
  - `crates/smelt-core/src/model_id.rs` — `ModelId`, name-from-path derivation
  - `crates/smelt-core/src/discovery.rs` — `ModelDiscovery`, model file walking
  - `crates/smelt-core/src/config.rs` — `Materialization`, `ModelConfig`, `validate_model_configs()`, tag-merge logic
- **Tests**:
  - `crates/smelt-core/src/metadata.rs` (inline `#[cfg(test)]`) — frontmatter parsing, multi-model, unknown fields, format, schema evolution
  - `crates/smelt-core/src/config.rs` (inline `#[cfg(test)]`) — materialization validation, tag merging, ephemeral/test constraints
- **User docs**:
  - `docs-site/docs/guide/sql-models.md`
  - `docs-site/docs/guide/materializations.md`
- **Related specs**:
  - `architecture.md` — `smelt.<path>` addressing scheme and identity-from-structure principle
  - `model_maintenance.md` — the processed-input equivalence invariant, the algebraic ladder, and the composition contract that consume this spec's declaration law and litmus rule
  - `model_properties.md` — the derived proofs a model's SQL can carry and the model-scoped declarations
  - `model_transforms.md` — the physical transforms a property licenses
  - `timeseries.md` — `timeseries:` frontmatter block
  - `batched_models.md` — the `refresh: batched` mode and the `batched:` frontmatter block
  - `cumulative_aggregate.md` — the `refresh: cumulative` mode
  - `versioned_models.md` — the `refresh: versioned` mode (SCD Type 2)
  - `latest_value_models.md` — the `refresh: latest_value` mode (SCD Type 1)
  - `accumulating_snapshot.md` — the `refresh: accumulating_snapshot` mode (keyed milestone / late-enrichment peer)
  - `materialized_view.md` — the `refresh: materialized_view` mode (engine-owned incremental-view maintenance)
  - `testing.md` — the `smelt.test` declaration kind
  - `schema_evolution.md` — `schema_evolution:` and `columns.default/backfill` frontmatter keys (forthcoming)
  - `pipe_sql.md` — the FROM-first pipe-query body form a model may use
