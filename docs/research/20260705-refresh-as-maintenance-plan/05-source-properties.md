# 05 — Source properties: the world-facts that license techniques

- **Date**: 2026-07-06
- **Status**: research (part 5 of [the refresh-as-maintenance-plan series](README.md))
- **Depends on**: [01-framework.md](01-framework.md) (theorem conditions), [02-loop-findings.md](02-loop-findings.md) (which declarations are consulted today), [04-knobs.md](04-knobs.md) (the model-side surface that consumes these)

Under the per-cell plan, **technique admission is a function of source properties**: whether a cell
may use fold-a-delta is decided by the theorem's conditions — replayable-at-current-`S` input,
faithful fold (the delta stream partitions the input multiset: no overlaps, no retractions) — and
those are facts about the *source*, not the model. This part specifies the source-declaration
surface those decisions need, as a diff against today's `sources.md`.

## What exists today (baseline)

`sources.md` already carries, per source: `columns:` (typed), target-aware `name:`, `timeseries:`
(the clock), `mutation_profile: append_only | mutable | change_feed` (default: undeclared =
conservative), `source_lateness:` (an interval; the declared term of the reach split), and
`key_recurrence: { key, window }` (runtime-checked recurrence bound). The loop confirmed how little
of this the execution path consumes today: `mutation_profile` reaches only the dormant
`input_delta_discovery` classifier (ledger FIX-2), and batched maintenance is unconditionally
recompute-region regardless of any profile (SC-2, G-01…G-09). The declarations below matter
*because* the framework will finally consume them.

## The trust rule (generalized from `key_recurrence`)

`sources.md` §Design already states the principle for one key; the framework makes it the governing
rule for every source declaration:

> A declaration that can only **widen** a scan (add margin, add caution) is safe against
> mis-statement and may be trusted as declared. A declaration that **narrows** what maintenance
> reads or licenses a cheaper technique — where an optimistic mis-statement silently corrupts state
> — is admitted only paired with a **verification mechanism**: a runtime tripwire that fails the
> consuming run loudly, or a scheduled probe that revokes the licence.

Every property below is classified under this rule. This is the production form of the loop's P0-4
lesson: a `MutationProfile` label on a *generated* schedule was never trusted unverified — the
self-check proved every declared-append-only schedule actually contained no mutation. Real sources
deserve the same discipline, mechanically weaker (smelt cannot see every write) but structurally
identical: verify what can be probed, bound the blast radius of what cannot.

---

## P1 — `mutation_profile`: the full enum

**Trust class:** narrows (licenses fold / window-forward) → verified. **Motivation:** theorem
conditions 2–3 (`01-framework.md` §4); ledger G-02/G-04/SC-2; `models.md` §"Input-consumption
axis".

Today's three values conflate delivery facts the theorem needs separated. Proposed:

```yaml
mutation_profile:
  kind: append_only            # append_only | mutable_snapshot | change_feed
  # append_only sub-facts:
  lateness: '7 days'           # optional; replaces top-level source_lateness (see P2)
  redelivery: none             # none | at_least_once   (default: at_least_once — conservative)
  # change_feed sub-facts:
  retractions: false           # does the feed carry deletes/updates as retraction events?
  ordered: true                # is the feed ordered by its offset column?
  delta_identity: [file_id]    # column(s) forming a stable per-delta identity (see P6)
```

with `mutation_profile: append_only` accepted as shorthand for the kind alone. Mapping to the
theorem:

| kind + sub-facts | Replayable at current `S`? | Faithful fold available? | Techniques licensed |
|---|---|---|---|
| `append_only, redelivery: none` | yes | yes — the delta stream partitions the input | fold-a-delta (any monoid); window-forward |
| `append_only, redelivery: at_least_once` | yes | idempotent combiners: yes. Additive: **only with the per-delta ledger** (never-fold-twice, §OQ4 design) | fold for idempotent; fold+ledger for additive |
| `mutable_snapshot` | yes (current content only) | **no** — mutation is not a partition of a multiset; folding successive snapshots is observer semantics (G-04's abstract REFUTED witness) | recompute-region only; snapshot-diff consumption |
| `change_feed, retractions: false` | yes (feed replay) | yes | fold; feed-driven targeted writes |
| `change_feed, retractions: true` | yes | invertible (group-rung) combiners: yes. Non-invertible (`MIN`/`MAX`/`BOOL_*`): **no** (P0-5's MIN-retraction witness) | fold only on group-rung cells; others recompute |

**Default:** undeclared stays the conservative fallback exactly as today (clocked → window-forward
consumption but *no fold licence*; unclocked → snapshot-diff). Every default row in this table is
the strictest one — a lazy user gets correct-but-expensive, never fast-but-wrong.

**Verification.** `append_only` is the dangerous declaration (it licenses folds that a silent
in-place UPDATE upstream would corrupt). Proposed tripwires, cheapest first, run as part of a
consuming maintenance run:

1. **Watermark monotonicity probe** (clocked sources, ~free): record `max(partition_column)` and
   row count per processed partition in the ledger; on a later run, re-count one recent processed
   partition. A count decrease proves deletion; an unchanged count with changed
   content needs (3). Catches deletes and most reload patterns.
2. **Frontier checksum** (cheap, opt-in): per processed partition, store a cheap aggregate
   fingerprint (`count`, `sum(hash(row))` over skeleton columns). Re-probing a sampled processed
   partition detects in-place updates SC-2-style — the exact hazard the loop showed forward-only
   consumption misses.
3. **Full re-scan comparison** — the degenerate case; only for audits.

On violation: fail the consuming run transactionally with a diagnostic naming the source, the
violated declaration, and the mitigation (`--full-refresh`, or correcting the profile) — the
`KeyedRecurrenceBoundViolated` pattern. A violated profile must never silently drop to recompute:
the declaration was load-bearing for already-materialized state, so past outputs are suspect and
the operator must know.

---

## P2 — Lateness and watermarks

**Trust class:** lateness *widens* → trusted as declared. A watermark *narrows* (it asserts
completeness) → checked. **Motivation:** `01-framework.md` §6 (settle bounds); ledger SC-1
(late-arrival within reach); `model_maintenance.md` §"Windowed maintenance and the horizon".

- **`lateness:`** (today's `source_lateness:`, folded into the profile block; the standalone key
  remains as an alias) — the declared bound on how far behind the clock a row can arrive. Widens
  scans and horizons; safe to trust. Its *new* consumer under the framework: it is what converts a
  derived watermark-relative settle bound into an **absolute** one (`04-knobs.md` §K4) — "settled
  when the conversions watermark ≥ event_ts + 7d" becomes "settled 9 days after event time" only
  because conversions declares `lateness: '2 days'`.
- **`watermark:`** (new, optional) — where the source's pipeline *publishes* a completeness marker
  (a column, or an external table/query returning "complete through T"):

  ```yaml
  watermark:
    complete_through: raw_meta.loads.complete_ts   # external completeness marker
  ```

  A declared watermark is a *narrowing* fact (it lets settle-reporting and GC treat data before T
  as final), so it is consumed checked: a row arriving with event time before the published
  watermark is a fail-loud violation (and, transitively, evidence against an `append_only`-derived
  fold — the P1 tripwires fire). Without a declared watermark, smelt's derived watermark is simply
  `max(partition_column)` processed so far, and settle bounds stay relative — honest but weaker.

**Rejected:** declaring lateness per *model* (which model reads the source doesn't change when the
source's data arrives — it is a world-fact of the feed, shared by all consumers).

---

## P3 — Unique keys on sources (composite)

**Trust class:** narrows (licenses 1:1 join proofs, dedup-free merges) → verified. **Motivation:**
ledger G-10 (composite keys inexpressible → over-conservative `OneToMany`); [03-design-forks.md](03-design-forks.md);
`model_properties.md` §"Fan-out / cardinality".

```yaml
unique_key: [user_id, dt]      # composite; a single column is the one-element list
```

What it licenses: the fan-out/cardinality proof (`OneToOne` join classification → enrichment
column-scoped re-derivation, the dimension-horizon MERGE), and dedup-free key-addressed merges from
this source. G-10 proved the ground truth (a composite equi-join covering the full declared key is
genuinely 1:1) and that the current `JoinContext` API cannot express it — the declaration surface
here is the input; the classifier extension is fork G-10 in [03-design-forks.md](03-design-forks.md).

**Verification:** a uniqueness probe — `SELECT 1 FROM src GROUP BY <key> HAVING COUNT(*) > 1
LIMIT 1` — scoped to the scan window of the consuming run (cheap: the run reads that window anyway),
full-table on demand (`smelt verify`, the source-existence pass `sources.md` already contemplates).
On violation, fail the consuming run: a duplicate key under a 1:1-licensed enrichment silently
fans out rows, the exact class of wrong-answer the trust rule exists to prevent.

---

## P4 — The clock (`timeseries:` on sources) and clocked dimensions

**Trust class:** structural (presence enables window-forward; absence = read-in-full). Monotonicity
of the clock is the narrowing assertion → already checked (the monotonicity trace + nullability
gate). **Motivation:** ledger G-05 (the "absent from the bound map" finding).

Unchanged in shape; two consequences of the loop's findings worth pinning:

- **Unclocked = lookup, structurally.** G-05 confirmed a non-timeseries source never enters
  `BoundContext` at all — read in full, re-read fresh on every recompute. This is *correct* and
  should be stated as the contract (not an accident): declaring no clock buys freshness-on-recompute
  at full-read cost.
- **Declaring a clock on a dimension** moves it into `source_bounds`' domain: its reads become
  clampable, which is an efficiency win but changes the enrichment cell's read scope from
  "current whole dimension" to "dimension slice" — a *semantic* change for mutable dimensions (a
  backfill would no longer see rows outside the slice). Admission rule: a clocked mutable dimension
  feeding an enrichment cell needs the cell's reach derivation to prove the slice covers the join's
  footprint, else the clock is ignored for that cell with a surfaced note. (G-06's related finding —
  the derived filter is emitted unqualified and collides when two clocked sources share a column
  name — is execution-layer, tracked with fork G-11.)

---

## P5 — Replayability and retention

**Trust class:** retention *narrows what backfill may claim* → checked at plan time (refuse, don't
probe). **Motivation:** theorem condition 2 — "replayable" means replayable **at the current
`Sᵢ`**; recompute-a-region is unconditionally valid *only* over replayable input.

```yaml
retention: '90 days'           # optional; absent = assumed fully replayable
```

- A **backfill** (region recompute) whose window reaches past the declared retention is **refused**
  at planning time — the recompute would silently re-derive from a partial input and *overwrite
  correct stored state with wrong state*. The stored output for that region is better than anything
  recomputable; refusing is the only move that preserves the invariant. Diagnostic points at the
  retention declaration and the stored-state provenance.
- Change feeds may carry `retention:` on the feed itself (offset horizon) with identical semantics
  for feed replay.
- Undeclared retention = assumed replayable (today's implicit posture). This default is trusting
  rather than conservative — flagged as the one place this part deviates from strictest-default,
  because refusing all backfills absent a declaration would make the common fully-retained case
  unusable. Mitigation: a backfill that reads an empty/short region *where the ledger says data was
  once processed* is a detectable anomaly (frontier row counts, P1 probe 1) and fails loud.

---

## P6 — Delta identity (change feeds; at-least-once appends)

**Trust class:** narrows (it is the dedup key for the never-fold-twice obligation) → checked
structurally. **Motivation:** the generalized ledger (`01-framework.md` OQ4 design): additive
groups need per-delta bookkeeping; idempotent groups need only a frontier.

```yaml
mutation_profile:
  kind: change_feed
  delta_identity: [_commit_version, _row_offset]   # e.g. Delta CDF columns / Kafka (partition, offset)
```

- Required when a fold-licensed cell has an **additive** (non-idempotent) combiner and the source is
  `at_least_once` or `change_feed`: the ledger records folded delta identities against `(region,
  column-group)` and refuses re-folding. Without a declared identity, additive fold is simply not
  admissible (the cell falls back to recompute) — no identity, no ledger, no licence.
- Idempotent-combiner cells ignore it (frontier-only bookkeeping — re-folding is harmless), which is
  the ladder-graded storage optimisation in the OQ4 design.
- Structural checks: the named columns must exist, be `NOT NULL`, and be (declared or probed)
  unique per delivered row — P3's probe machinery reused.

---

## P7 — Scan ceilings (source-side, symmetric to K8) — *deferred*

**Status: deferred (2026-07-06, `09-spec-readiness.md` decision 11).** The shipped scan ceiling is
the model-side, per-consumer `max_lookback` (K8); this source-owner variant is retained as a design
option and added only if owner-side governance proves needed. Documented here so the symmetry is on
record.

**Trust class:** neither widens nor narrows — a pure assertion → check-only, always safe.
**Motivation:** `01-framework.md` §5 (partition-local maintenance); the model-side guardrail
[`04-knobs.md`](04-knobs.md) §K8.

K8 lets a *model* assert that its maintenance stays partition-bounded in each source it reads. The
symmetric fact belongs to the source owner: a source may cap how much of itself any consumer's
maintenance is allowed to scan, so a downstream model that accidentally spells an unbounded
correlation against a 10-TB feed fails at *its* compile, not at 3 a.m. in production.

```yaml
# on the source
max_consumer_scan: '14 days'     # no consumer's per-run maintenance scan of this source may exceed 14 days
```

- Consumed exactly like K8's `max_lookback`, but authored once on the source and inherited by every
  consuming model's derived plan. A consumer whose derived scan clamp on this source exceeds the
  ceiling fails loud (`MaintenanceScanUnbounded`, citing both the source declaration and the
  offending cell); an unclocked full-read consumer must carry an explicit `allow_full_scan` (K8) to
  compile at all.
- **It never modifies any clamp** — same discipline as K8 and `horizon_ceiling:`. It is a governance
  assertion, not a maintenance modifier: the source owner states a cost expectation and the
  framework refuses plans that silently blow past it.
- Absent = no source-side ceiling (the model-side K8 default still applies). This is the one source
  property that constrains *consumers* rather than describing the feed, so it is deliberately opt-in.

---

## Summary table

| Property | Surface | Values / shape | Default | Trust class | Licenses | Verified by |
|---|---|---|---|---|---|---|
| Mutation kind | `mutation_profile.kind` | `append_only` \| `mutable_snapshot` \| `change_feed` | undeclared (strictest) | narrows | fold, window-forward, feed-driven writes | watermark/count probe; frontier checksum |
| Redelivery | `.redelivery` | `none` \| `at_least_once` | `at_least_once` | narrows (`none`) | additive fold without ledger | delta-identity collision check |
| Retractions | `.retractions` | bool | `true` (for feeds) | narrows (`false`) | fold on non-invertible combiners | feed event-type scan (cheap) |
| Lateness | `.lateness` / `source_lateness:` | interval | absent (0) | **widens** — trusted | absolute settle bounds; horizon margin | — (mis-statement only wastes scan) |
| Watermark | `watermark.complete_through` | column / external ref | absent (derived) | narrows | settle finality, GC | row-behind-watermark tripwire |
| Unique key | `unique_key:` | column list (composite) | absent | narrows | 1:1 proofs, dedup-free merge | uniqueness probe (windowed / on-demand) |
| Clock | `timeseries:` | existing shape | absent (lookup) | structural | window-forward, clamps | monotonicity trace + nullability gate (existing) |
| Retention | `retention:` | interval | absent (replayable) | narrows backfill | region recompute validity | plan-time refusal; ledger-anomaly probe |
| Delta identity | `.delta_identity` | column list | absent | narrows | additive fold + ledger | existence/NOT NULL/uniqueness probe |
| Key recurrence | `key_recurrence:` | existing shape | absent | narrows | locality-pruned merge scan | transactional check (existing) |
| Consumer scan ceiling *(deferred)* | `max_consumer_scan:` | interval | absent | **asserts** (neither) | — (caps consumers, never modifies clamp) | plan-time refusal (`MaintenanceScanUnbounded`) |

## Worked example: the paper's `conversions` source, fully annotated

```yaml
# models/sources/conversions.yml
description: >
  Conversion events from the attribution vendor. Landed hourly by the ingest job;
  a conversion is never retracted but can arrive up to 2 days after it occurred.
columns:
  - { name: conversion_id, type: BIGINT, nullable: false }
  - { name: user_id,       type: INTEGER, nullable: false }
  - { name: conversion_ts, type: TIMESTAMP, nullable: false }
  - { name: conversion_date, type: DATE, nullable: false }
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

With this, the `converted` cell of the worked model (`01-framework.md` §2) gets: a fold licence for
the late-conversion trigger (append-only, no redelivery, `BOOL_OR` idempotent — frontier-only
ledger), an **absolute** settle bound (event_ts + 7d window + 2d lateness = settled at +9d), a
backfill validity horizon of 400 days, and a probed uniqueness fact available to any enrichment
join. Delete the `mutation_profile` block and every one of those degrades to the conservative
default — recompute-only, watermark-relative settling — with the model still correct.

## Open questions this part leaves

- Probe cost governance: which tripwires run per-run vs sampled vs on-demand (`smelt verify`) —
  likely a project-level policy key, not per-source.
- ~~Whether `mutation_profile`'s structured block subsumes `key_recurrence`.~~ **Resolved 2026-07-06:
  subsume** — `key_recurrence` folds into the `mutation_profile` block (delivery-contract metadata of
  the same species); the standalone key is dropped (`09-spec-readiness.md` decision 9).
- Backend-published facts (Delta CDF presence, Iceberg snapshots) could *derive* `change_feed` +
  `delta_identity` instead of declaring them — a capability-flag question for `multi_backend.md`.
  **Left as a Known Divergence (decision 10):** tracked separately, not gating this spec.
