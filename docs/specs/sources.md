---
feature: sources
status: experimental
last_reviewed: 2026-07-17
owners: [andrew]
---

# Sources

> **What this is.** Normative spec for source declarations: externally-managed tables that smelt does not load but can type-check, route in `FROM` positions, and — through the declared **world-facts** (mutation profile, lateness, keys, retention) — admit maintenance techniques for (`incremental_models.md` consumes these facts; this spec owns their declaration surface and trust rules). Sources share their YAML grammar with seed sidecars (`seeds.md`); this spec owns that shared grammar and the source-only semantics.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### What a source is

A **source** is an external table that already exists in the target database, populated by some pipeline outside smelt. Smelt declares the source's schema, type-checks references, surfaces the columns in the LSP, and routes `smelt.<path>` references to the underlying `<schema>.<table>` — but it never runs `CREATE TABLE` or `INSERT` for the source. `smelt seed` does not touch sources.

Beyond the schema, a source YAML declares the source's **world-facts** — delivery-contract properties of the feed (how rows change, how late they arrive, what identifies them) that no analysis of consumer SQL can derive. These facts are what license the cheaper maintenance techniques (`incremental_models.md` §"Per-cell admission"); they are declared once on the source and shared by every consumer, never per model.

### The source as a Relation Contract provider

A source and a model output are two **providers** of the same **Relation Contract** — one named vocabulary a downstream consumer reads without caring which provider filled it (`models.md` §"The Relation Contract"). A source fills the contract's slots by **declaration** (its world-facts are external and unprovable — the trust rule below governs them); a model output fills the same slots by **derivation** or as **declared-and-checked** shape facts. The shared slots carry **identical field paths** across both providers:

| Contract slot | Source fills it with | Fill mode |
|---|---|---|
| **schema** | `columns:` | declared |
| **clock** | `timeseries:` (`timeseries.md`) | declared |
| **identity** | `unique_key:` | declared (trust rule) |
| **mutation / arrival** | `mutation_profile:` (the mutation slot — kind, lateness, redelivery, retractions, ordering, delta identity, key recurrence) | declared (trust rule) |
| **completeness / settle** | `watermark:` | declared |
| **replay bound** | `retention:` | declared (trusted-replayable default) |
| *source-only* | `name:` external routing | declared |

The **clock** and **identity** slots share their field paths (`timeseries:`, `unique_key:`) with the model-output contract verbatim — which is exactly what lets a downstream consumer treat an upstream maintained model and a `sources.*` ref as the same standing (`incremental_models.md` §"Upstream model edges"). The **mutation**, **completeness**, and **replay** slots are **source-declared, model-derived**: a source declares them (it has world-knowledge of its own feed's physics; a model has none), a model proves them from its plan — so there is no field-path to reconcile, and the source keeps the `mutation_profile:` spelling of the mutation slot (§Known Divergences records the shared-name reconciliation). A source has an **effective grain** too (clocked-fact, keyed-dimension, …), derived from its clock and identity the same way a model's is (`models.md` §"Refresh axis") and reported by `smelt explain`.

### Filesystem layout

A source is declared by a `.yml` file in any non-excluded directory under the project root. (Discovery is project-wide; `smelt.yml::paths` only strips address prefixes, it does not gate which directories are scanned — see `architecture.md` §"Resolution".) The file must **not** have a sibling `.csv` with the same stem in the same directory — that would make the YAML a seed sidecar instead (`architecture.md` §"Resolution").

| File on disk (with `paths: ["models"]`) | Address |
|---|---|
| `models/sources/raw/users.yml` | `smelt.sources.raw.users` |
| `models/external/api/orders.yml` | `smelt.external.api.orders` |

The address path follows universal addressing (`architecture.md` §"Resolution"). The mapping from `smelt.<path>` to `<db_schema>.<db_table>` follows the default rule in `architecture.md` §"Default materialization name mapping" — `<target_schema>.<path-joined-by-_>` — unless the YAML provides a `name:` override (recommended whenever the external pipeline named the table differently). The override is **target-aware** (see §"Target-aware `name:` override").

### Source YAML shape

```yaml
description: >
  Conversion events from the attribution vendor. Landed hourly; a conversion is
  never retracted but can arrive up to 2 days after it occurred.
columns:
  - { name: conversion_id,   type: BIGINT,    nullable: false }
  - { name: user_id,         type: INTEGER,   nullable: false }
  - { name: conversion_ts,   type: TIMESTAMP, nullable: false }
  - { name: conversion_date, type: DATE,      nullable: false }
timeseries:
  event_time_column: conversion_ts
  partition_column: conversion_date
  granularity: day
mutation_profile:
  kind: append_only
  lateness: '2 days'
  redelivery: none
unique_key: [conversion_id]
retention: '400 days'
```

| Key | Required | Default | Meaning |
|-----|----------|---------|---------|
| `description` | no | absent | Free-text description, surfaced in LSP hover. |
| `columns` | yes | — | Column declarations. Sources without a column list are not useful — type-checking has no contract to enforce. |
| `columns[].name` | yes | — | Column name as it appears in the database. |
| `columns[].type` | yes | — | Smelt `DataType` (`types.md`). Same vocabulary as model type-checking and seed sidecars. |
| `columns[].nullable` | no | `true` | Whether the column may contain NULL in the upstream database. Type-checking respects this. |
| `columns[].description` | no | absent | Free-text description, surfaced in LSP hover. |
| `name` | no | derived | Override the database-side name. **Target-aware** (see §"Target-aware `name:` override"). |
| `timeseries` | no | absent | Declares the source's clock (`event_time_column`, `partition_column`, `granularity`). See `timeseries.md`. Presence makes the source **clocked** (window-forward consumption, clampable reads); absence makes it an **unclocked lookup**, read in full on every recompute — a structural contract, not an accident. `granularity` is also the source's partition-axis grain for cross-model propagation (`incremental_models.md` §"The graph layer"). The named columns must appear in `columns:` with date/timestamp-compatible types. |
| `mutation_profile` | no | absent (undeclared — strictest) | The structured mutation block (see §"`mutation_profile` — the structured block"). The bare string form `mutation_profile: append_only` (or `mutable_snapshot` / `change_feed`) is shorthand for `{ kind: <value> }`. |
| `source_lateness` | no | absent (zero) | Alias for `mutation_profile.lateness` (the standalone key is retained as shorthand). Declaring both is a `MalformedSource` error. |
| `watermark` | no | absent (derived) | Where the source's pipeline publishes a completeness marker: `watermark: { complete_through: <schema.table.column or column> }`. When absent, the derived watermark is `max(partition_column)` processed so far, and settle bounds stay watermark-relative. |
| `unique_key` | no | absent | Row identity of the source, **composite-valued** (a single column is the one-element list). Licenses 1:1 join-cardinality proofs and dedup-free key-addressed merges. Verified, never trusted (§Semantics "The trust rule"). |
| `retention` | no | absent (**trusted replayable**) | How far back the source can be re-read: an interval. A backfill window reaching past it is refused at plan time. Absent = assumed fully replayable. |
| `materialization` | — | — | **Not allowed on a source.** Sources are externally managed; declaring a materialization is a hard error pointing at the seed sidecar shape. |

The YAML grammar is shared with the seed sidecar (`seeds.md` §"Sidecar YAML — seed-specific keys"). Differences: a source must declare `columns:`; a source must not declare `materialization:`; a source supports the `name:` override; a source may declare `timeseries:` and the world-fact keys above (a seed is loaded by smelt, so its world-facts are smelt's own doing).

### `mutation_profile` — the structured block

```yaml
mutation_profile:
  kind: append_only              # append_only | mutable_snapshot | change_feed
  # append_only sub-facts:
  lateness: '7 days'             # optional; how far behind the clock a row can arrive
  redelivery: none               # none | at_least_once      (default: at_least_once)
  # change_feed sub-facts:
  retractions: false             # does the feed carry deletes/updates as retraction events?  (default: true)
  ordered: true                  # is the feed ordered by its offset column?
  delta_identity: [_commit_version, _row_offset]   # stable per-delta identity column(s)
  # delivery-contract bound (any kind):
  key_recurrence:                # every pair of rows sharing the key lies within `window`
    key: [user_id]               #   of each other on the event-time axis
    window: '3 days'
```

- `kind: append_only` — rows are only ever appended; an existing row never changes or disappears. `redelivery: at_least_once` (the conservative default) states a delivered row may arrive again; `none` states each row arrives exactly once.
- `kind: mutable_snapshot` — rows may be updated or deleted in place; only a full re-scan sees every change.
- `kind: change_feed` — the source itself reports what changed (CDC/CDF). `retractions` states whether delete/update events appear; `delta_identity` names the column(s) forming a stable identity per delivered delta (e.g. Delta CDF's commit version + row offset, Kafka's partition + offset) — required for any additive fold over a redeliverable or feed source, since it is the dedup key of the ledger's never-fold-twice obligation (`incremental_models.md` §"The reconciliation ledger"). The named columns must exist, be `NOT NULL`, and be unique per delivered row (probed).
- `key_recurrence` — the delivery-contract recurrence bound (e.g. an at-least-once feed whose redeliveries land within three days). It lives inside the block because it is delivery-contract metadata of the same species as the other sub-facts. Consumed by key temporal locality (`incremental_models.md` §"Key temporal locality") when a consuming model's `unique_key` resolves exactly to the declared columns; **always runtime-checked, never trusted** (`KeyedRecurrenceBoundViolated`).

What each posture licenses (the mapping consumed by per-cell admission, `incremental_models.md`):

| kind + sub-facts | Replayable at current `S`? | Faithful fold? | Techniques licensed |
|---|---|---|---|
| `append_only, redelivery: none` | yes | yes | fold-a-delta (any monoid); window-forward |
| `append_only, redelivery: at_least_once` | yes | idempotent: yes; additive: only with the per-delta ledger | fold for idempotent; fold+ledger for additive |
| `mutable_snapshot` | yes (current content only) | **no** — folding successive snapshots is observer semantics | recompute-region only; snapshot-diff consumption |
| `change_feed, retractions: false` | yes (feed replay) | yes | fold; feed-driven targeted writes |
| `change_feed, retractions: true` | yes | invertible combiners only | fold on group-rung cells; others recompute |
| *undeclared* | assumed | **no fold licence** | clocked → window-forward recompute; unclocked → snapshot-diff |

Every undeclared/default row is the strictest: a lazy declaration gets correct-but-expensive, never fast-but-wrong.

### Landed-delta intervals (derived, recorded)

For every source a maintenance run consumes, smelt records **which partition intervals of that source landed** — the per-source delta, on the source's own partition axis. This is the input to cross-model forward propagation (`incremental_models.md` §"The graph layer": what landed decides which downstream partitions run). The recording is derived, never declared: for an append-only clocked source it is the interval diff of processed partitions; a change feed's deltas come from the feed's offsets; a `mutable_snapshot` source has no interval representation — its delta is "the whole table" (which propagates as whole-model dirt downstream). The record lives in smelt's run state (`run_state.md`), keyed by source address.

### Source with `timeseries:` declaration

A source declaring a time dimension opts in to being a pushdown target for downstream planner rules — incremental models reading the source receive source-filter pushdown based on the declared partition column. Declaring `timeseries:` does not change how the source is loaded — sources remain externally managed. It only declares the partition shape downstream consumers may rely on.

Declaring a clock on a **mutable dimension** moves its reads into the clampable domain — an efficiency win, but a semantic change for enrichment cells (a slice, not the whole current dimension). Admission rule: a clocked mutable dimension feeding an enrichment cell is sliced only where the cell's reach derivation proves the slice covers the join's footprint; otherwise the clock is ignored for that cell with a surfaced note.

### Target-aware `name:` override

The `name:` override is resolved against the **active target** so a single source declaration can point at different external tables in different environments. Two forms are accepted:

**Literal form** — one `<schema>.<table>` string, applied to every target:

```yaml
name: raw_cdc.users        # all targets read raw_cdc.users
```

**Per-target map** — keys are target names from `smelt.yml::targets`, values are `<schema>.<table>` literals:

```yaml
name:
  dev:  raw_cdc_dev.users
  prod: raw_cdc.users
```

Resolution rules:

- The map is keyed by the active `--target` name. When the active target has an entry, its `<schema>.<table>` is used verbatim (the literal's schema is **not** overridden by the target's `schema:`).
- A target with no entry in the map falls back to the default mapping (`<target_schema>.<address-path-joined-by-_>`) — the map only overrides the targets it names.
- The literal form is shorthand for a map whose single value applies to every target.
- A map value that is not a `<schema>.<table>` literal, or a map key that names no declared target, is a `MalformedSource` error.

The grammar lives here in `sources.md`; `smelt.yml` carries no source-name config key. `smelt_yml.md` references this section for the target-aware behaviour.

### Discovery and addressing

Sources are discovered alongside every other project file by walking `paths:`. Resolution rules (`architecture.md` §"Resolution"):

- A `.yml` file with no sibling `.csv` of the same stem → source.
- A `.yml` file with a sibling `.csv` → sidecar to that seed (not a source). See `seeds.md`.
- Two files resolving to the same address (anywhere in the project) → workspace-load error.

### LSP surface

- **Hover** on a `smelt.<path>` reference to a source → table description + column list with types and descriptions.
- **Goto-definition** → opens the source `.yml`.
- **Diagnostics** — references to columns not declared in the source YAML produce an "undeclared column" diagnostic, same as for any other typed table reference.
- **No "Pin schema" code action.** Sources have no data file to infer from; the YAML is hand-written.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `MalformedSource` | Error | A source `.yml` parses as YAML but violates the shape above: missing `columns`; `materialization:` present; a malformed `name:` override; an unrecognised `mutation_profile.kind` or malformed sub-fact (bad interval, unknown column in `delta_identity`/`key_recurrence.key`, missing `key_recurrence.window`); both `source_lateness:` and `mutation_profile.lateness` declared; a change-feed sub-fact on a non-feed kind (and vice versa); a malformed `watermark`/`retention`/`unique_key`. |
| `SourceTypeError` | Error | A `columns[].type` value is not a recognised smelt `DataType` (`types.md`). |
| `SourceMutationProfileViolated` | Error (fails the consuming run) | A verification tripwire disproved a declared narrowing fact: a processed partition's row count decreased or its fingerprint changed under `append_only`; a delta-identity collision under `redelivery: none`; a retraction event under `retractions: false`. Names the source, the violated declaration, and the mitigation. |
| `SourceWatermarkViolated` | Error (fails the consuming run) | A row arrived with event time before the source's published `watermark.complete_through`. |
| `SourceUniqueKeyViolated` | Error (fails the consuming run) | The uniqueness probe found duplicate rows for the declared `unique_key` within the consuming run's scan window (or on `smelt verify`). |
| `SourceRetentionExceeded` | Error (plan-time refusal) | A backfill window reaches past the declared `retention:` — the recompute would silently rebuild from partial input; points at the declaration and the stored-state provenance. |
| `KeyedRecurrenceBoundViolated` | Error (fails the consuming run, transactionally) | The `key_recurrence` bound was disproved by the consuming run's check (`incremental_models.md`). |

## Semantics

1. **Sources are never loaded.** `smelt seed`, `smelt build` (seed phase), and any other ingest path skip sources entirely. A `smelt seed --select <source-path>` invocation is a hard error ("not a seed").
2. **Schema is the contract.** When a model references a source column, the smelt type-checker uses the YAML's declared type. A column not declared in the YAML is undeclared and produces a diagnostic, even if the column exists in the upstream database.
3. **The trust rule.** Every world-fact declaration is classified by what a mis-statement could do:
   - A declaration that can only **widen** a scan (`lateness`) is safe against mis-statement and is **trusted as declared**.
   - A declaration that **narrows** what maintenance reads or licenses a cheaper technique (`mutation_profile.kind`, `redelivery`, `retractions`, `unique_key`, `delta_identity`, `key_recurrence`, `watermark`, `retention`) is admitted only **paired with a verification mechanism**: a runtime tripwire that fails the consuming run loudly, a plan-time refusal, or a scheduled probe. A violated narrowing declaration must never silently degrade to the conservative technique — the declaration was load-bearing for already-materialized state, so past outputs are suspect and the operator must be told (the `Source*Violated` diagnostics above).
   - A pure **assertion** that neither widens nor narrows is check-only and always safe.
4. **Verification mechanisms** for `append_only` run as part of consuming maintenance runs, cheapest first: the watermark-monotonicity probe (per-partition row counts recorded and re-checked — catches deletes and reloads), the frontier checksum (a sampled per-partition fingerprint over skeleton columns — catches in-place updates), and full re-scan comparison (audit only). `unique_key` and `delta_identity` use the uniqueness probe scoped to the consuming run's scan window, full-table on demand via `smelt verify`.
5. **Retention refusal.** A region recompute (backfill) whose window reaches past declared `retention:` must be refused at plan time (`SourceRetentionExceeded`): stored output for that region is better than anything recomputable, and overwriting correct state with a partial re-derivation is the one unrecoverable move. Undeclared retention is trusted replayable; the blast-radius bound is the ledger-anomaly probe — a backfill reading an empty or short region where the recorded state says data was once processed fails loud.
6. **Smelt does not validate that the source exists.** A reference to a non-existent source surfaces only at execution time as a backend error; `smelt verify` is the on-demand pass that checks declared sources (existence, columns, probes) against the live database.
7. **Address-only references.** A source has no body for the planner to inspect — it is black-box, like an `extern`, but addressable by path rather than by bare name (`architecture.md` §"Two orthogonal axes").
8. **Discovery and uniqueness.** A source's address is its workspace path under `paths:`, with the scan-root prefix stripped. The cross-path uniqueness rule (`architecture.md` §"Resolution") applies.

## Design

**One contract, two providers.** Casting the source YAML as the source-side fill of the shared Relation Contract (§"The source as a Relation Contract provider") is what makes "an upstream maintained model is a plan edge of the same standing as a `sources.*` ref" honest rather than asserted: the clock and identity slots are literally the same field paths on both providers, and the mutation / completeness / replay slots are source-declared / model-derived, so no path collides. Sources and models are two providers, **not a symmetric pair** — the asymmetries (source-only `name:` routing and `retention:`; model-only per-column `contract:`) are explicit, not accidental. The derived `grain` label is surfaced by `smelt explain` for **sources as well as models**: the derivation (`(clock?, identity?, partition_column ∈ key?)`) is identical for both providers, so surfacing it symmetrically costs nothing and keeps the shared vocabulary honest. Full derivation: `docs/research/20260716-relation-contract-and-per-cell-addressing.md`.

**World-facts live on the source, never per model.** How a feed changes, how late it arrives, what identifies a delta — these do not vary by which model reads the feed. Declaring them per consumer would let two models assert contradictory physics about one table; declaring them once makes every consumer's admission consistent and every tripwire shared. (Rejected: per-model lateness/mutation declarations — `docs/research/20260705-refresh-as-maintenance-plan/05-source-properties.md` P2.)

**The trust rule generalizes `key_recurrence`'s original discipline.** The spec's earlier posture — one runtime-checked narrowing key among otherwise-trusted declarations — is now the governing classification for every world-fact: widening declarations trusted, narrowing declarations verified, assertions check-only. This is the production form of the property-loop's lesson that a declared `append_only` on a generated schedule was never trusted unverified; real sources get the same discipline, mechanically weaker (smelt cannot see every write) but structurally identical — verify what can be probed, bound the blast radius of what cannot.

**The structured `mutation_profile` block, subsuming `key_recurrence`.** The flat three-value enum conflated delivery facts that admission needs separated: append-only-ness, redelivery, retraction-carrying, ordering, and delta identity are independent axes of one delivery contract, and `key_recurrence` is metadata of the same species — so they share one block (`09-spec-readiness.md` decision 9), with the bare-string shorthand preserving the simple case. `mutable` is named `mutable_snapshot` to say *what a read observes* (a snapshot), which is exactly the fact that refuses folding (observer semantics).

**Trusted-replayable retention default** (`09-spec-readiness.md` decision 6): refusing all backfills absent a declaration would make the common fully-retained case unusable; the deviation from strictest-default is deliberate and bounded by the ledger-anomaly probe (Semantics 5).

**Composite `unique_key` from day one** (`09-spec-readiness.md` decision 2): the common real-world dimension key is composite (`(entity_id, valid_from)`, `(user_id, dt)`); a single-column surface would bake the known-wrong shape into declarations that later have to migrate.

**A source-side consumer-scan ceiling is deferred, on record.** A `max_consumer_scan:` key — the source-owner mirror of the model-side `maintenance.scan_bounds.per_source.max_lookback` guardrail — was designed and deliberately not adopted (`09-spec-readiness.md` decision 11): the shipped ceiling is model-side per-consumer; the source-side variant is the one property that constrains *consumers* rather than describing the feed, and is added only if owner-side governance proves needed (`docs/research/20260705-refresh-as-maintenance-plan/05-source-properties.md` P7).

**Two concepts (seed and source), one YAML grammar.** Seeds and sources have different lifecycles — smelt loads a seed; an external pipeline owns a source — and that distinction is real to users. But every other concern overlaps: column types, descriptions, hover, goto-definition, future tests. Sharing the YAML grammar means one parser, one schema-resolution path, and one set of LSP affordances. The kind is determined structurally (sibling CSV present?), not by a configuration toggle.

**Per-entity YAML, not aggregate `sources.yml`.** The aggregate file violates universal addressing — every project entity should live at its addressed path. `models/sources/raw/users.yml` *is* `smelt.sources.raw.users`.

**Why `name:` is allowed on sources but not on seeds.** A seed's identity *is* its workspace path — smelt picks the database name. A source's identity is the external table the pipeline produces; smelt only declares it. The external name is not a function of the workspace layout, so the YAML must be able to override the default mapping.

**Why `name:` is target-aware.** Real pipelines stage the same logical feed in different schemas per environment (`raw_cdc_dev` vs `raw_cdc`). A table-only override (schema always from the target) was rejected: it cannot express the common case where the *schema* differs per environment, which is exactly where source portability breaks.

**`materialization:` not allowed on sources.** Sources are external by definition — there is no smelt-controlled materialization.

## Constraints & Invariants

1. A `.yml` file with no sibling `.csv` is a source; with a sibling `.csv` it is a sidecar. The kinds are disjoint.
2. Sources are never loaded by `smelt seed` or `smelt build`.
3. `materialization:` on a source YAML is a hard error.
4. The source YAML grammar is a strict superset of the seed sidecar grammar limited to the source-only keys (`name:`, `timeseries:`, the world-fact keys); the shared core is identical.
5. The cross-path uniqueness rule (`architecture.md` §"Resolution") applies — a source's address is unique across all `paths:` roots.
6. Aggregate `sources.yml` at the project root is being retired in favour of per-entity source YAMLs. Once the legacy fallback is removed, its presence will produce a clear migration error; until then it is still parsed as a fallback (see Known Divergences).
7. **A `smelt.sources.<path>` reference resolves by its path prefix, not a separate namespace.** Addressing is the single `smelt.<path>` scheme (`architecture.md`); the path prefix is dispositive over any same-named model.
8. **No narrowing declaration is consumed without its verification mechanism.** A licence read from a declared world-fact must be revocable by a tripwire, probe, or plan-time check; wiring the licence without the check is a spec violation, not an optimisation.
9. **Undeclared is strictest** (except retention's trusted-replayable default, deliberately): absence of a world-fact must never license a cheaper technique than its most conservative value would.

## Known Divergences / Open Questions

- **The mutation slot keeps the `mutation_profile:` spelling.** The shared Relation Contract names the mutation slot `mutation:` (`models.md` §"The Relation Contract"); the source-side surface keeps the existing `mutation_profile:` key, because the slot is source-declared / model-derived and no model-side field path collides with it (§"The source as a Relation Contract provider"). Renaming the source key to `mutation:` for cosmetic vocabulary alignment is a possible future cleanup, deliberately not taken here to avoid churn without a field-path benefit — the reconciliation `docs/research/20260716-relation-contract-and-per-cell-addressing.md` §Open questions flagged.
- **The structured `mutation_profile` block parses; licensing and runtime tripwires remain unbuilt.** `crates/smelt-core/src/sources.rs` parses both the bare-string shorthand and the structured block (`kind` + `lateness`/`redelivery`/`retractions`/`ordered`/`delta_identity`/`key_recurrence`), the `mutable_snapshot` wire name, `watermark:`, composite `unique_key:`, and `retention:`. A sub-fact declared for the wrong `kind`, and the `source_lateness`/`mutation_profile.lateness` double-declare, are `MalformedSource` errors. What remains open: cross-referencing `delta_identity`/`key_recurrence.key` column names against `columns:` is not yet validated at parse time (surfaces later, at admission or runtime); the per-cell admission that reads these facts and the runtime verification mechanisms below are still unbuilt.
- **Declared profiles license almost nothing yet.** `mutation_profile` reaches only the input-delta classifier (whose only wired consumer distinction is `change_feed`) — every partition-grain cell is served by unconditional recompute regardless of profile, and the fold/ledger techniques the licence table describes are the unbuilt machinery of `incremental_models.md` §Known Divergences. None of the verification tripwires (`SourceMutationProfileViolated`, `SourceWatermarkViolated`, `SourceUniqueKeyViolated`, `SourceRetentionExceeded`) exist; `smelt verify` does not exist.
- **Landed-delta recording is v1 (append-only interval diff only).** The per-source delta intervals the graph layer consumes are recorded per source address in the run state (`smelt_state::landed_deltas`), no longer model-only: an append-only clocked source's landing is interval-diffed against prior coverage; a `mutable_snapshot` or unclocked source always resolves to the whole-table delta. `change_feed` offset-based delta detection and snapshot diffing are not yet built — every source still resolves through the append-only-or-whole-table path regardless of a declared `change_feed` profile.
- **Aggregate `sources.yml` presence is not yet a migration error (Constraint 6).** Still parsed as a legacy type-information fallback when a project declares no per-entity sources. Tracked as BUG-078 in `docs/bug-hunt/2026-05-30-findings.md`.
- **Backend-derived source facts are a Known Divergence by decision** (`09-spec-readiness.md` decision 10): a backend capability (Delta CDF presence, Iceberg snapshots) could *derive* `change_feed` + `delta_identity` instead of requiring declaration — a `multi_backend.md` capability-flag question, tracked separately.
- **Probe cost governance is open**: which tripwires run per-run vs sampled vs on-demand — likely a project-level policy key, not per-source (`docs/research/20260705-refresh-as-maintenance-plan/05-source-properties.md` §Open questions).
- **Column-level tests on sources.** Same status as for seeds — per-column assertions on the shared YAML grammar are not yet defined.
- **Co-location with seeds.** A `.yml` declaring a source can be co-located with seed CSVs in the same directory (different stems); style guides may discourage mixing, the resolver does not.

## References

- **Code**:
  - `crates/smelt-core/src/sources.rs` — source discovery, YAML loader, `SourceInfo`, `MutationProfile`.
  - `crates/smelt-db/src/schema.rs` — source YAML → `ModelSchema` (shared with seed sidecars).
  - `crates/smelt-logical/src/analysis/input_delta.rs` — the input-delta classifier that reads the profile.
  - `crates/smelt-lsp/src/lib.rs` — hover, goto-definition for source references.
- **Tests**:
  - `crates/smelt-core/tests/source_yaml.rs` — schema validation, `name:` override, kind tiebreaker against seed sidecars.
- **User docs**:
  - `docs-site/docs/guide/sources.md` — user-facing source guide.
  - `docs-site/docs/reference/sources-yml.md` — per-key YAML reference (to be reconciled with this spec by the migration plan).
- **Plans (history)**: `docs/plans/20260403-sources-yml-live-updates.md` (prior aggregate-shape work, superseded); `docs/plans/20260704-model-updates.md`.
- **Related specs**:
  - `architecture.md` §"Resolution" — kind-determination, sidecar tiebreaker, cross-path uniqueness.
  - `seeds.md` — shares the YAML grammar; the load-side complement of this spec.
  - `smelt_yml.md` — `paths:` key the discovery layer consumes.
  - `timeseries.md` — the `timeseries:` block grammar this spec hosts on external sources.
  - `incremental_models.md` — the per-cell admission and graph layer these world-facts license and feed; consumer of `timeseries:` on sources via source-filter pushdown, and of `mutation_profile`/`key_recurrence` (key temporal locality).
  - `models.md` — the input-consumption axis these declarations decide, and §"The Relation Contract" (the shared vocabulary this source YAML is the declared-provider fill of).
  - `run_state.md` — where landed-delta intervals and probe records live.
  - `types.md` — `DataType` vocabulary used by `columns[].type`.
