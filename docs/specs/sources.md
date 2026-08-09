---
feature: sources
status: experimental
last_reviewed: 2026-07-19
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
| `referential_integrity` | no | absent (unguaranteed) | Asserts that a consuming model's equi-join into the declared column(s) never drops a driving row — every value a consumer joins on is guaranteed present, so an inner/equi-join enrichment is as row-preserving as a `LEFT JOIN`. Composite-valued like `unique_key`. Narrowing (§Semantics "The trust rule") — paired with the count-preservation tripwire (§"Referential integrity"). |
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

### Referential integrity

```yaml
unique_key: [customer_id]
referential_integrity: [customer_id]
```

`referential_integrity` names a subset of `unique_key` (or, absent a declared `unique_key`, its own composite-valued column list) for which the source asserts **completeness relative to its consumers**: every value a consuming model's join column carries into these columns is guaranteed to find a matching row here — the dimension never has a gap a driving fact's foreign key can fall into. This is the fact an inner/equi-join enrichment needs to be as row-preserving as an unconditional `LEFT JOIN` (§"Row preservation" leg of skeleton-source closure, `model_properties.md`); without it, an inner join is only ever proven row-preserving by being spelled as a `LEFT JOIN`, since smelt cannot see the dimension's completeness from its own SQL.

This is a **narrowing** declaration under the trust rule (§Semantics "The trust rule") — a mis-declared `referential_integrity` would license a join-shape assumption (no row dropped) that a real gap in the dimension would silently violate, corrupting every skeleton-source-closure-licensed technique built on it. It is admitted only paired with the **count-preservation tripwire**: every consuming run that relies on the declaration re-checks, over the region it touched, that the enrichment join's row count equals the driving side's row count (no row lost to a missing key) — a violation fails the run loudly (`SourceCountPreservationViolated`) and marks prior output built on the declaration as suspect, per the trust rule's general shape. The tripwire is scoped to the touched region, not a full-table scan, the same way the other narrowing tripwires (§Semantics 4) are scan-window-scoped rather than full-table by default.

### Landed-delta (derived, recorded)

For every source a maintenance run consumes, smelt records **what changed** since the source's delta was last consumed — the per-source delta. In its fullest form this is a **changed-row set with a partition projection** onto the source's own axis (which partition intervals contain at least one changed row); a delta the graph layer actually schedules against is always the *projected* form, never the raw row set. Where no row-level record exists for a source, the delta **widens** to the coarsest representation that source's shape supports — an append-only clocked source with no row-level tracking still resolves to the interval diff of processed partitions; a `change_feed` source's deltas come from the feed's offsets (already row-level); a `mutable_snapshot` source with no derived changed-row record resolves to "the whole table" (which propagates as whole-model dirt downstream). This is the input to cross-model forward propagation (`incremental_models.md` §"The graph layer": what landed decides which downstream partitions run), which applies the same rule to a **model** edge's delta (`incremental_models.md` §"The graph layer" — the observed output delta where recorded, else the run's written window). **Widen-never-narrow governs the whole hierarchy**: absent a recorded changed-row set, the coarser interval-or-whole-table form is always the correct fallback — a consumer may never assume a narrower delta exists than what was actually recorded. The recording is derived, never declared. Row-level recording for a `mutable_snapshot` source with no native change feed is synthesized by the **fingerprint sidecar** (§"The fingerprint sidecar", below); it is built for DuckDB as a standalone capability, not yet wired into this section's own live per-source delta consumption — see §Known Divergences. The record lives in smelt's run state (`run_state.md`), keyed by source address.

### The fingerprint sidecar

For a `mutable_snapshot` source with no native change feed, smelt can synthesize one: a
**row-content fingerprint sidecar** — a warehouse-resident table recording, per source row key, a
content digest last observed for that key — that a consuming run diffs the source's *current*
content against to derive an exact changed-key set, instead of treating every re-scan as a
whole-table delta (`incremental_models.md` §Future Extensions "Conditional maintenance without a
change feed", mechanism M3). This is a different **artifact class** from the semantic fingerprint
in `output_fingerprint.md` — see that spec's own boundary paragraph for why the two coexist
without contradiction.

**Digest.** A SHA-256-class digest over the row's content, restricted to the **fingerprint
projection** the consuming model actually reads (`model_properties.md` §"Fingerprint projection",
P4) — an irrelevant-column edit elsewhere in the row never dirties the key. The digest is a
*detection* mechanism only: the exact write-suppression compare a consuming write performs is
still `IS DISTINCT FROM` over the processed columns (`model_properties.md` §"Change
comparability"), never the digest itself. The **collision-soundness invariant** — two distinct row
contents never collide to the same digest at any practically observable rate — is an assumed
property of SHA-256, not something smelt proves; it is the oracle gate a synthesized-delta
conformance leg (`incremental_models.md`) exercises (real content edits ⇒ exactly the edited keys
detected; no false-negative "unchanged" verdict observed across the generated fixture space), not
a formally established fact.

**Naming and namespace.** The sidecar is a warehouse-resident table, `_smelt_fingerprint_sidecar`,
alongside the reconciliation ledger and the observed-delta table (`incremental_models.md`
§"The reconciliation ledger", §"Observed deltas on model edges") — the same excluded bookkeeping
class under §"Statement emission (single owner)"'s third exclusion, owned per dialect by
`smelt-state`, DuckDB-scoped today (matching the ledger's own posture; a non-DuckDB target fails
loud rather than silently skipping the sidecar). A row is namespaced by `(source address,
projection identity, source key)` — **projection identity**, not consumer identity: a canonical
hash of the P4 projection's column set. Two consumers of the same source whose derived projections
are byte-identical incidentally **share** one sidecar partition (a storage optimisation, never a
correctness dependency); consumers with differing projections never share, and a fail-closed
full-row digest (an unprojectable consumption, P4) is its own distinct projection identity — it is
never silently widened onto, or narrowed from, another consumer's own projection.

**Transactionality.** The sidecar upsert for a key runs in the **same backend transaction as the
consuming write** that reads the derived changed-key set — mirroring the observed-delta record's
own rule (`incremental_models.md` §"Observed deltas on model edges"): a failed write leaves no
digest update, so a re-run recomputes the same delta rather than silently treating a
half-committed key as already seen.

**First run and `--full-refresh`.** A source key absent from the sidecar is, by construction, a
changed key — the whole-table delta a first run against an unpopulated sidecar produces
(consistent with the landed-delta widen-never-narrow default, above) also populates the sidecar as
a byproduct. A `--full-refresh` run does not diff against the sidecar at all for the region it
rebuilds — trusting nothing stored is the whole point of a full refresh — and unconditionally
repopulates the sidecar for that region, so the next incremental run starts from a fresh baseline
rather than an inherited, possibly-stale one.

**GC.** A key's disappearance from the source is itself a change: a full re-scan diff observes a
sidecar key with no matching source row, emits it as a deletion in the changed-key set, and drops
its sidecar row. GC of a **projection-identity partition** whose owning consumer no longer exists
(a deleted or redefined model) is not performed — an orphaned partition is inert cost (never read
again, no soundness exposure), not a correctness hazard, and is left as an explicit Open Question
below rather than a silent leak masquerading as handled.

**Invalidation.** Every stored row carries an **identity stamp** — the digest-construction
version, the row's own projection identity, and a hash of the consuming model's SQL definition,
combined — that the next diff compares against a freshly computed one before trusting anything
stored. Three independent things can invalidate a partition, any one of which widens the next diff
to "every key in the source is changed", the same widen-never-narrow default every other
invalidation in this spec follows, never a narrower or partially-trusted comparison: the P4
fingerprint projection changing (a fresh, unpopulated projection-identity partition by
construction — no extra mechanism needed); the consuming model's own SQL definition changing while
the projection happens to stay the same (caught only by the stamp's model-hash component, since
`projection identity` alone cannot see it); or a stored stamp that fails to match for any other
reason, including on-disk corruption — detected and logged, never silently trusted. A row whose
stamp does not match is excluded from the comparison entirely, structurally identical to that key
having no sidecar row at all; the next refresh (which writes every currently-observed key's digest
unconditionally) re-stamps it with the current value, so the partition self-heals without a
separate "clear the stale partition" step. Invalidation is scoped to the changed consumer's own
partition; a sibling consumer's differently-identified partition, or one whose model definition did
not change, is unaffected.

**Partition grain.** The sidecar's partition grain is the *digested unit*, not necessarily one
source row: most consumers digest per source row (above), but a repair-family consumer
(`incremental_models.md` §"The repair family") partitions at **group** grain instead — one sidecar
row per output group key, its digest an order-insensitive aggregate over that group's contributing
source rows, so that adding, removing, or reordering a row within the group changes the group's
digest but the digest itself does not depend on the order the group's rows are read in. Same table,
same namespacing, stamp, and invalidation rules as the per-row grain; only what one sidecar row
represents differs. A group-grain partition is what makes a group's *disappearance* observable: a
row deleted from the source leaves nothing for a current-source scan to select, but the sidecar
still holds a stored comparandum for the group that row belonged to, so the group surfaces on the
next diff's "sidecar row with no matching source key" leg even though no source row survives to
name it.

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
| `MalformedSource` | Error | A source `.yml` parses as YAML but violates the shape above: missing `columns`; `materialization:` present; a malformed `name:` override; an unrecognised `mutation_profile.kind` or malformed sub-fact (bad interval, unknown column in `delta_identity`/`key_recurrence.key`, missing `key_recurrence.window`); both `source_lateness:` and `mutation_profile.lateness` declared; a change-feed sub-fact on a non-feed kind (and vice versa); a malformed `watermark`/`retention`/`unique_key`/`referential_integrity` (a column not declared in `columns:`, or a `referential_integrity` column set that is not a subset of a declared `unique_key` when one is declared). |
| `SourceTypeError` | Error | A `columns[].type` value is not a recognised smelt `DataType` (`types.md`). |
| `SourceMutationProfileViolated` | Error (fails the consuming run) | A verification tripwire disproved a declared narrowing fact: a processed partition's row count decreased or its fingerprint changed under `append_only`; a delta-identity collision under `redelivery: none`; a retraction event under `retractions: false`. Names the source, the violated declaration, and the mitigation. |
| `SourceWatermarkViolated` | Error (fails the consuming run) | A row arrived with event time before the source's published `watermark.complete_through`. |
| `SourceUniqueKeyViolated` | Error (fails the consuming run) | The uniqueness probe found duplicate rows for the declared `unique_key` within the consuming run's scan window (or on `smelt verify`). |
| `SourceRetentionExceeded` | Error (plan-time refusal) | A backfill window reaches past the declared `retention:` — the recompute would silently rebuild from partial input; points at the declaration and the stored-state provenance. |
| `KeyedRecurrenceBoundViolated` | Error (fails the consuming run, transactionally) | The `key_recurrence` bound was disproved by the consuming run's check (`incremental_models.md`). |
| `SourceCountPreservationViolated` | Error (fails the consuming run, transactionally) | The count-preservation tripwire disproved a declared `referential_integrity`: an enrichment join licensed by the declaration returned fewer rows than the driving side over the touched region. Names the source, the declared key, and the region checked. |

## Semantics

1. **Sources are never loaded.** `smelt seed`, `smelt build` (seed phase), and any other ingest path skip sources entirely. A `smelt seed --select <source-path>` invocation is a hard error ("not a seed").
2. **Schema is the contract.** When a model references a source column, the smelt type-checker uses the YAML's declared type. A column not declared in the YAML is undeclared and produces a diagnostic, even if the column exists in the upstream database.
3. **The trust rule.** Every world-fact declaration is classified by what a mis-statement could do:
   - A declaration that can only **widen** a scan (`lateness`) is safe against mis-statement and is **trusted as declared**.
   - A declaration that **narrows** what maintenance reads or licenses a cheaper technique (`mutation_profile.kind`, `redelivery`, `retractions`, `unique_key`, `referential_integrity`, `delta_identity`, `key_recurrence`, `watermark`, `retention`) is admitted only **paired with a verification mechanism**: a runtime tripwire that fails the consuming run loudly, a plan-time refusal, or a scheduled probe. A violated narrowing declaration must never silently degrade to the conservative technique — the declaration was load-bearing for already-materialized state, so past outputs are suspect and the operator must be told (the `Source*Violated` diagnostics above).
   - A pure **assertion** that neither widens nor narrows is check-only and always safe.
4. **Verification mechanisms** for `append_only` run as part of consuming maintenance runs, cheapest first: the watermark-monotonicity probe (per-partition row counts recorded and re-checked — catches deletes and reloads), the frontier checksum (a sampled per-partition fingerprint over skeleton columns — catches in-place updates), and full re-scan comparison (audit only). `unique_key` and `delta_identity` use the uniqueness probe scoped to the consuming run's scan window, full-table on demand via `smelt verify`. These are the source-side instances of the general probe-obligation rule (`model_properties.md` §"Probe obligation" generalizes this trust rule to model-scoped declarations) — its registry names each mechanism above by row, diagnostic, and cadence.
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
8. **No narrowing declaration is consumed without its verification mechanism.** A licence read from a declared world-fact must be revocable by a tripwire, probe, or plan-time check; wiring the licence without the check is a spec violation, not an optimisation. `model_properties.md` §"Probe obligation" states the same rule for model-scoped declarations and registers this spec's mechanisms by name.
9. **Undeclared is strictest** (except retention's trusted-replayable default, deliberately): absence of a world-fact must never license a cheaper technique than its most conservative value would.

## Known Divergences / Open Questions

- **The mutation slot keeps the `mutation_profile:` spelling.** The shared Relation Contract names the mutation slot `mutation:` (`models.md` §"The Relation Contract"); the source-side surface keeps the existing `mutation_profile:` key, because the slot is source-declared / model-derived and no model-side field path collides with it (§"The source as a Relation Contract provider"). Renaming the source key to `mutation:` for cosmetic vocabulary alignment is a possible future cleanup, deliberately not taken here to avoid churn without a field-path benefit — the reconciliation `docs/research/20260716-relation-contract-and-per-cell-addressing.md` §Open questions flagged.
- **The structured `mutation_profile` block parses; most licensing and runtime tripwires remain unbuilt — `key_recurrence` is the one exception.** `crates/smelt-core/src/sources.rs` parses both the bare-string shorthand and the structured block (`kind` + `lateness`/`redelivery`/`retractions`/`ordered`/`delta_identity`/`key_recurrence`), the `mutable_snapshot` wire name, `watermark:`, composite `unique_key:`, and `retention:`. A sub-fact declared for the wrong `kind`, and the `source_lateness`/`mutation_profile.lateness` double-declare, are `MalformedSource` errors. `key_recurrence` is now consumed exactly as this spec promises: key temporal locality's route 3 (`incremental_models.md` §"Key temporal locality") reads it as the declared fallback when no bound is statically derivable, admitted only when its `key` resolves exactly to the consuming model's `unique_key`, and paired with its verification mechanism — the checked route-3 merge's out-of-slice match probe, failing the run transactionally on any violation (`KeyedRecurrenceBoundViolated`). What remains open for the rest of the block: cross-referencing `delta_identity`/`key_recurrence.key` column names against `columns:` is not yet validated at parse time (surfaces later, at admission or runtime); the per-cell admission that reads the other sub-facts (`lateness`/`redelivery`/`retractions`/`ordered`/`delta_identity`) and their own runtime verification mechanisms are still unbuilt.
- **Declared profiles license almost nothing yet.** `mutation_profile` reaches only the input-delta classifier (whose only wired consumer distinction is `change_feed`) — every partition-grain cell is served by unconditional recompute regardless of profile, and the fold/ledger techniques the licence table describes are the unbuilt machinery of `incremental_models.md` §Known Divergences. `SourceMutationProfileViolated` now exists and dispatches at every run that consumes an `append_only` source with a recorded baseline (`model_properties.md` §"Probe obligation"); `SourceWatermarkViolated`, `SourceUniqueKeyViolated`, and `SourceRetentionExceeded` remain unbuilt; `smelt verify` does not exist.
- **`referential_integrity` parses and validates; P1 closure runs live over external sources, and the count-preservation tripwire is dispatched at its one live consumer.** `crates/smelt-core/src/sources.rs` parses the bare-string and list forms and enforces the subset rule against a declared `unique_key` (`MalformedSource`), so a source can now declare the world-fact `model_properties.md` §"Skeleton-source closure" (P1) consumes for its row-preservation conjunct. An `UpstreamMutation` cell driven by a `mutation_profile: mutable_snapshot` source that declares `referential_integrity` now closes P1 for its own inner-join enrichment the same way a `LEFT JOIN` does (`smelt_logical::maintenance::derive::mutation_enrichment_closure`), naming the route `Closed { DeclaredReferentialIntegrity { source } }` (`model_properties.md` §"Skeleton-source closure"), and the closed verdict licenses the SAME delta-restricted recompute a maintained-model edge's own closure does (`smelt_logical::maintenance::choice::resolve_recompute_restriction`) — restricting the recompute to the fingerprint sidecar's synthesized changed-key set (the fixture: `examples/timeseries/models/daily_events_enriched.sql`, whose `raw.users` dimension declares both `unique_key` and `referential_integrity`; the undeclared inner-join shape stays `Open`, `crates/smelt-logical/tests/skeleton_closure_pinned.rs`). The count-preservation tripwire (`smelt_logical::maintenance::emit::emit_count_preservation_probe`/`emit_count_preservation_probe_from_body`) is dispatched by `smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction` whenever the restriction it is about to take rests on a `DeclaredReferentialIntegrity` route — before any write, over the touched region, failing the run transactionally (`SourceCountPreservationViolated`) on a violation, and falling back to the widened scan if the probe cannot be built from the model's own body. What remains open: the Salsa production derivation (`smelt-db`) now threads a source's declared `referential_integrity` into its own plan (`derive_model_maintenance_plan`'s `source_referential_integrity` parameter, `build_source_referential_integrity`), so `smelt explain`/diagnostics see the real declared-route verdict — but *which* live maintenance cells consult it at all remains narrow: today only the source-enrichment `UpstreamMutation` route ever derives a declared-route closure; a model-edge creation cell's own closure is always derived against an empty referential-integrity map. Widening that reach is separate follow-on work. Tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md` (E2, F5) and `docs/outcomes/20260809-probe-backed-facts/outcome.md`.
- **Landed-delta recording is v1 (append-only interval diff only) for the live graph-layer consumption path; the changed-row-set form (§"Landed-delta (derived, recorded)") is not yet wired in for sources.** The per-source delta intervals the graph layer consumes are recorded per source address in the run state (`smelt_state::landed_deltas`), no longer model-only: an append-only clocked source's landing is interval-diffed against prior coverage; a `mutable_snapshot` or unclocked source still resolves to the whole-table delta on this path. `change_feed` offset-based delta detection is unbuilt. The fingerprint sidecar (below) now derives an exact changed-row set for a `mutable_snapshot` source as a standalone, independently-tested capability; wiring its output into the graph layer's own per-source delta so a live run actually consumes it in place of the whole-table fallback is separate follow-on work. Tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md` (M3-input, §Future Extensions of `incremental_models.md`).
- **The fingerprint sidecar is built for DuckDB; other backends fail loudly.** `_smelt_fingerprint_sidecar` (table DDL, the digest-refresh upsert, and GC of a deleted source key's row) lives in `smelt_state::ddl_duckdb`, matching the reconciliation ledger's own DuckDB-scoped posture; the synthesized change-feed diff query — comparing the source's current row-content digest, computed via DuckDB's native `sha256()`, against the sidecar's stored digest over the P4 projection and the current identity stamp (§"The fingerprint sidecar" — "Invalidation") — is emitter-authored (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`). The sidecar upsert commits in the same backend transaction as the write it rides with (`Backend::execute_write_and_refresh_fingerprint_sidecar`), so a failed write leaves the sidecar untouched rather than half-committed. A target backend other than DuckDB fails loudly at the call site (`crates/smelt-runtime/src/maintenance_driver.rs`) rather than silently falling back to a whole-table delta or being handed DuckDB-flavored SQL it cannot run. Invalidation (§"The fingerprint sidecar" — "Invalidation") is built: a projection change, a model-definition edit holding the projection fixed, or a corrupted/mismatched stamp all degrade the next diff to the same whole-table delta an absent sidecar produces, logged loudly on a detected mismatch (never silently trusted, never silently skipped). The point-lookup delta-restricted recompute over an external source this capability enables is built and proven directly against a real fixture and backend (the P1 closure + restriction-gate licence union described above); what remains unbuilt is wiring it into a live run's own trigger/technique selection — `crates/smelt-runtime/src/execute.rs`'s regular incremental batch loop does not yet consult it, so a live run still takes the ordinary unrestricted recompute. Tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md` (F3–F5 built; live dispatch remains).
- **Two sidecar lifecycle questions are open, deliberately not ruled on above.** (1) GC of an orphaned projection-identity partition (a deleted or redefined consumer's stale digests) is not specified — the sidecar section leaves it unswept, reasoning that inert storage is not a soundness hazard, but a project running long enough could accumulate meaningful dead weight; a future sweep (e.g. keyed to model-address existence at plan time) is unruled-on future work. A stamp-invalidated partition's stale rows are handled the same way — excluded from comparison immediately, physically overwritten only when a subsequent refresh next touches that key — so this same inert-but-swept-lazily posture now also covers invalidated (not just orphaned) rows. (2) Cross-project sharing of a sidecar for a source declared identically in two smelt projects against the same warehouse is out of scope — the namespace above is implicitly single-project (no project identifier in the key), which is fine under project isolation (`architecture.md` §"Project isolation rule") but is recorded here as a boundary, not an oversight.
- **Aggregate `sources.yml` presence is not yet a migration error (Constraint 6).** Still parsed as a legacy type-information fallback when a project declares no per-entity sources. Tracked as BUG-078 in `docs/bug-hunt/2026-05-30-findings.md`.
- **Backend-derived source facts are a Known Divergence by decision** (`09-spec-readiness.md` decision 10): a backend capability (Delta CDF presence, Iceberg snapshots) could *derive* `change_feed` + `delta_identity` instead of requiring declaration — a `multi_backend.md` capability-flag question, tracked separately.
- **Probe cost governance is open**: which tripwires run per-run vs sampled vs on-demand — likely a project-level policy key, not per-source (`docs/research/20260705-refresh-as-maintenance-plan/05-source-properties.md` §Open questions).
- **Column-level tests on sources.** Same status as for seeds — per-column assertions on the shared YAML grammar are not yet defined.
- **Co-location with seeds.** A `.yml` declaring a source can be co-located with seed CSVs in the same directory (different stems); style guides may discourage mixing, the resolver does not.
- **The source-side derived grain is landed.** `SourceInfo::resolved_grain` derives the effective grain label from a source's declared clock/identity facts via the same pure derivation a model output's `grain` reads, and `smelt explain <model>` prints it for every source edge alongside the model's own contract (`models.md` §"The Relation Contract"). Only the clock/identity/derived-grain slots render this way; the mutation/completeness/replay slots remain readable only from the source YAML itself.

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
  - `model_properties.md` — §"Skeleton-source closure" (P1), the proof `referential_integrity` licenses the row-preservation conjunct of; §"Fingerprint projection" (P4), the proof the fingerprint sidecar digests against.
  - `output_fingerprint.md` — the semantic model-SQL fingerprint; §"Two fingerprint artifact classes" draws the boundary against the row-content sidecar.
  - `models.md` — the input-consumption axis these declarations decide, and §"The Relation Contract" (the shared vocabulary this source YAML is the declared-provider fill of).
  - `run_state.md` — where landed-delta intervals and probe records live.
  - `types.md` — `DataType` vocabulary used by `columns[].type`.
