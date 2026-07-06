# Example catalogue — what works, what trades, what refuses

- **Date**: 2026-07-06
- **Status**: research (part 7 of [`README.md`](README.md); framework in [`01-framework.md`](01-framework.md))
- **Depends on**: proposed source surface in [`05-source-properties.md`](05-source-properties.md), knobs in [`04-knobs.md`](04-knobs.md), empirical verdicts in [`02-loop-findings.md`](02-loop-findings.md)

This is the worked-example sweep: combinations of **SQL construct × upstream property ×
output shape × technique**, each landing on a verdict, each tied to a real ingest /
enrichment / transform use case. Verdicts are stated **under the proposed framework**
([`01-framework.md`](01-framework.md)); where today's surface differs, the entry says so
(the paper's §9 discipline). Examples marked `unprobed-candidate` are formatted so they can
be lifted into the property-discovery loop's catalog
(`docs/research/property-discovery/catalog.md`) as new cells — see the closing section.

## Entry schema

```
### EX-nn — <short name>
- grid: construct=<...> | sources=<property, per source> | output-shape=<...> | technique(s)=<per column-group × trigger>
- expected: HOLDS | CONDITIONAL(<named traded guarantee>) | REFUSED(<why>) | UNSUPPORTED-TODAY(<what's missing>)
- probe-status: probed(<ledger cell>) | unprobed-candidate | not-probe-worthy(<why>)
- use-case: <real-world scenario>
```

**Verdict legend** (aligned with the ledger's vocabulary, `property-discovery/ledger.md`):

| verdict | meaning |
|---|---|
| **HOLDS** | The maintenance plan is admissible and equals full refresh at every processed-input set `S` (skeleton exact; payload per its declared contract). |
| **CONDITIONAL(g)** | Admissible only under the **named traded guarantee** `g`, recorded per-column in the §6 ledger of `01-framework.md` — allowed, never silent. |
| **REFUSED(why)** | The framework must fail loud: no technique satisfies the invariant and no honest weaker contract is on offer (or the needed contract is deliberately deferred — OQ2). |
| **UNSUPPORTED-TODAY(gap)** | The framework admits it, but a named piece of machinery is missing from today's smelt (these are the implementation backlog, cross-referenced in [`09-spec-readiness.md`](09-spec-readiness.md)). |

Source YAML uses today's declared surface (`mutation_profile`, `source_lateness`,
`key_recurrence`, `timeseries:` — `docs/specs/sources.md`); keys that exist only as
proposals carry a `# PROPOSED` comment pointing at
[`05-source-properties.md`](05-source-properties.md). Column lists are elided (`columns: …`)
except where load-bearing. Model frontmatter likewise uses **today's** `refresh:` modes
(`batched`/`keyed`/`versioned`/`materialized_view`), not [`04-knobs.md`](04-knobs.md) K1's
proposed `refresh: incremental` + `grain:` surface — K1 is itself pending ratification
([`09-spec-readiness.md`](09-spec-readiness.md) decision 5), and writing the examples in the
shipped surface keeps them checkable against real smelt; each entry's *verdict* is under the
proposed framework regardless.

## Physical-maintenance notation

Each example carries a `**Physical maintenance**` block: the actual `DELETE`+`INSERT` /
`MERGE` an admissible plan would emit per trigger, with its clamps. The conventions,
stated once:

- **input clamp** = the scan window on each *read* source; **output clamp** = the write
  window/keys on the *target*. In both, the **partition-column predicate is always shown
  explicitly** — it is what lets the engine prune. A finer, non-prunable predicate
  (`user_id = u`, an event-time `BETWEEN`) narrows *within* the pruned partitions and is
  shown after it.
- **2×2 corner** tags name the cell each trigger lands in
  ([`01-framework.md`](01-framework.md) §3): *top-left* fold-a-delta (delta+state,
  targeted), *top-right* read-modify-write region (delta+state, region-overwrite),
  *bottom-left* column-scoped `MERGE` / re-derivation (full-input, targeted),
  *bottom-right* recompute-region (full-input, region-overwrite).
- **`DELETE`+`INSERT`** realizes a region-overwrite; **`MERGE`** realizes a targeted
  write. Where the §4 interchangeability theorem applies, **both** are shown.
- **`Partition-local:`** records, *per source*, whether the maintenance is partition-local
  ([`01-framework.md`](01-framework.md) §5 "Partition-local maintenance"): ✅ = scan and
  footprint both project onto a bounded partition interval (all ops run partition-by-
  partition, no full-table scan/shuffle); ⚠️ = only under a declared bound/horizon; ❌ =
  the footprint chains across unbounded partitions (a full-table operation is the only
  honest option — several REFUSED/UNSUPPORTED verdicts refuse *because* of this).
- **`Amplification (secondary):`** appears only where the merge key is *orthogonal* to the
  partition key; it states what copy-on-write actually rewrites, and that deletion vectors
  / `OPTIMIZE` absorb the within-partition scatter. The *partition bound* is what the plan
  guarantees; file-level rewrite minimization is the engine's job.

## Coverage matrix

Rows = construct, columns = dominant upstream property. Cells name the examples; **bold** =
probed by the loop.

| construct \ source | append-only | append-only + lateness | mutable snapshot | change feed (retractions) | at-least-once redelivery | unclocked lookup/dim | composite key |
|---|---|---|---|---|---|---|---|
| pass-through projection | EX-02 | EX-03 | **EX-04** | EX-14† | EX-05† | — | — |
| additive agg (SUM/COUNT) | **EX-13** | EX-18 | **EX-04**† | EX-14 | **EX-20** | — | — |
| idempotent agg (MIN/MAX/BOOL_OR) | **EX-15** | EX-15† | **EX-16** | EX-16† | EX-15† | — | — |
| holistic agg (MEDIAN/COUNT DISTINCT) | **EX-17** | EX-17† | EX-17† | — | **EX-17** | — | — |
| inner-join enrichment | EX-08 | EX-08† | **EX-07** | EX-26† | — | **EX-07**, EX-08 | EX-10 |
| LEFT JOIN (null-preservation) | **EX-09** | **EX-09** | — | — | — | — | — |
| join fan-out (1:N / N:1 proof) | **EX-10** | — | EX-10† | — | — | EX-10 | **EX-10** |
| correlated EXISTS / scalar subquery | **EX-01**, EX-11 | **EX-01** | EX-16† | — | — | — | — |
| correlated first-value pick (MIN_BY / first) | EX-35 | EX-35† | — | — | — | — | — |
| window: running total (trajectory) | EX-22 | EX-23 | — | — | — | — | — |
| window: LAG/LEAD | EX-25 | EX-25 | — | — | — | — | — |
| window: ROW_NUMBER dedup | EX-27 | EX-27 | — | — | EX-27 | — | — |
| UNION ALL | **EX-05** | EX-05† | EX-06 | — | — | — | — |
| self-referential model | **EX-21** | **EX-21** | — | — | — | — | — |
| GROUP BY coarser than partition | EX-18 | EX-18 | — | — | — | — | — |
| multi-input column group (merge) | — | — | EX-12 | EX-12 | — | EX-12 | — |
| dedup-to-latest (keyed collapse) | EX-27 | EX-27 | — | — | EX-27 | — | — |
| keyed end-state fold | EX-19, EX-24 | EX-24 | — | EX-26 | — | — | — |
| SCD2 / versioned intervals | — | — | EX-29 | EX-28 | — | — | — |
| engine-maintained (MV) | EX-32 | — | EX-32 | — | — | — | — |
| cross-model DAG propagation | EX-31, EX-33, EX-34 | EX-34 | — | — | — | — | — |

† = the property appears in that example's discussion/variant, not its headline cell.

**Family G (EX-36–39) is trigger-orthogonal** and so occupies no matrix cell: column
addition is a *definition-change* trigger ([`01-framework.md`](01-framework.md) §5) that can
apply to **any** construct row above. Its examples reuse EX-02 / 07 / 08 / 13's shapes
rather than inhabiting new cells.

---

## Family A — Ingest (raw → bronze landing)

The simplest cells: one source, pass-through or near-pass-through projection,
time-partitioned output. Everything here turns on the **source's mutation profile and
lateness**, because the SQL contributes no interesting reach of its own. This family is
where the loop's universal finding bites hardest: today's batched maintenance recovers any
upstream change **only on an explicit backfill of the affected window** — a forward-only
advance never revisits a processed partition.

### EX-02 — clickstream landing
- grid: construct=pass-through projection | sources=events: append-only, clocked | output-shape=time-partitioned (day) | technique(s)=all-columns × new-day → delta-read/region-append; backfill → recompute-region
- expected: HOLDS
- probe-status: probed(P0-1, G-02 mechanism)
- use-case: raw web clickstream landed into a typed bronze table, one partition per day.

```yaml
# sources/events.yml
description: web click events, at-most-once, event-time stamped
mutation_profile: append_only
timeseries: { event_time_column: event_ts, partition_column: event_date, granularity: day }
columns: …
```

```sql
---
refresh: batched
timeseries: { event_time_column: event_ts, partition_column: event_date, granularity: day }
---
SELECT event_id, user_id, event_date, event_ts, page, referrer
FROM smelt.sources.events
```

One column group (`{}` mutation-sensitivity — append-only upstream never rewrites a row),
one trigger that matters. New-day maintenance is the degenerate top-right corner (the new
region has no prior contents, so read-modify-write reduces to an insert); backfill is
bottom-right. Nothing to declare beyond the clock. The framework derives everything;
`refresh:` here is pure output-shape assertion.

**Physical maintenance.** One source, partition `event_date`. **Partition-local:** ✅ `events`.
- *New day `D`* — *top-right* region-append (no prior contents → degenerate `INSERT`). input clamp: `events WHERE event_date = D`; output clamp: partition `event_date = D`.
  ```sql
  INSERT INTO clickstream SELECT event_id, … FROM events WHERE event_date = D;
  ```
- *Backfill `[t₀,tₙ)`* — *bottom-right* recompute-region. input clamp: `events WHERE event_date >= t0 AND event_date < tn`; output clamp: same partitions.
  ```sql
  DELETE FROM clickstream WHERE event_date >= t0 AND event_date < tn;
  INSERT INTO clickstream SELECT event_id, … FROM events WHERE event_date >= t0 AND event_date < tn;
  ```

### EX-03 — IoT uplinks with 48-hour lateness
- grid: construct=pass-through projection | sources=readings: append-only + `source_lateness: '48 hours'` | output-shape=time-partitioned (day) | technique(s)=all-columns × new-day → region-append; late-uplink-within-48h → read-modify-write region **or** recompute-region (interchangeable, cost-chosen); beyond-horizon → excluded (data-quality flag)
- expected: CONDITIONAL(bounded-lateness truncation — rows later than the declared 48h margin are outside the maintained window; surfacing them is a data-quality concern, `model_maintenance.md` §horizon)
- probe-status: unprobed-candidate (the RMW-region technique specifically; the recompute arm is covered by G-06's late-append shape)
- use-case: IoT sensor fleet whose devices buffer readings offline and uplink up to two days late.

```yaml
# sources/readings.yml
mutation_profile: append_only
source_lateness: '48 hours'
timeseries: { event_time_column: reading_ts, partition_column: reading_date, granularity: day }
columns: …
```

```sql
---
refresh: batched
timeseries: { event_time_column: reading_ts, partition_column: reading_date, granularity: day }
---
SELECT device_id, reading_date, reading_ts, temperature_c, battery_pct
FROM smelt.sources.readings
```

| column group | new day | late uplink ≤48h | uplink >48h |
|---|---|---|---|
| all (append-only pass-through) | region-append | RMW-region *(read stored partition + delta, rewrite)* or recompute-region — §4 interchangeable | excluded; flag via DQ check |

The interesting cell is the middle one: because the columns are pass-through and the source
append-only, the late rows can be **appended into the stored partition without re-reading
the upstream day** (top-right, read-modify-write) — a genuinely cheaper alternative to the
bottom-right recompute, and the two are interchangeable by the §4 theorem (idempotent
skeleton, faithful "fold" = multiset insert). This is the cleanest bake-off candidate for
the offline cost-measurement principle (§11 of 01-framework.md): same contract, measurably different cost when
the upstream day is expensive to re-scan.

**Physical maintenance.** One source, partition `reading_date`. **Partition-local:** ✅ `readings` (the `source_lateness` horizon bounds how far back a delta reaches). The late-uplink cell is the §4 interchangeable pair — both shown.
- *New day `D`* — *top-right* region-append. input clamp: `readings WHERE reading_date = D`; output clamp: partition `reading_date = D`.
- *Late uplink ≤48h into stored day `P`* — **either** *top-right* read-modify-write **or** *bottom-right* recompute (§4-interchangeable). input clamp (RMW): the delta rows, `reading_date = P`; (recompute): `readings WHERE reading_date = P`. output clamp: partition `reading_date = P`.
  ```sql
  -- RMW: append the newly-arrived rows into the stored partition (faithful fold = multiset insert)
  INSERT INTO uplinks SELECT device_id, … FROM readings_delta WHERE reading_date = P;
  -- recompute: replace the partition from upstream
  DELETE FROM uplinks WHERE reading_date = P;
  INSERT INTO uplinks SELECT device_id, … FROM readings WHERE reading_date = P;
  ```
- *Uplink >48h* — excluded by the horizon clamp; surfaced by a DQ check, no write.

### EX-04 — hand-corrected operational table
- grid: construct=pass-through + additive agg | sources=adjustments: **mutable snapshot**, clocked | output-shape=time-partitioned | technique(s)=all-columns × any-change → recompute-region of the touched window (requires knowing which window was touched)
- expected: CONDITIONAL(backfill-recovers / forward-advance-stale: an in-place edit to an already-processed partition is only reflected when that window is explicitly re-run — nothing detects the edit)
- probe-status: probed(SC-2)
- use-case: a finance ops team hand-corrects rows in an adjustments table (backed by a spreadsheet sync); downstream daily totals must eventually reflect the fix.

```yaml
# sources/adjustments.yml
mutation_profile: mutable
timeseries: { event_time_column: adj_ts, partition_column: adj_date, granularity: day }
columns: …
```

```sql
---
refresh: batched
timeseries: { event_time_column: adj_ts, partition_column: adj_date, granularity: day }
---
SELECT adj_date, cost_center, SUM(amount) AS total_amount
FROM smelt.sources.adjustments
GROUP BY adj_date, cost_center
```

SC-2 established this precisely: a mutable clocked source is (mis)classified window-forward
by the dormant `input_delta_discovery`, but the *shipped* behaviour is simply "recompute
whatever window you ask for" — the edit is recovered by an explicit backfill and by nothing
else. The framework's honest offer for `mutable` without a change feed is exactly this
CONDITIONAL, plus (proposed) an operational knob for periodic re-scan windows
([`04-knobs.md`](04-knobs.md)). The durable fix for the use case is upgrading the source to
a change feed (EX-14).

**Physical maintenance.** One source, partition `adj_date`, `GROUP BY adj_date` = partition-aligned. **Partition-local:** ✅ `adjustments` *for the op* (a requested window recomputes in bounds); the CONDITIONAL is about *detection*, not locality — nothing tells the plan which window a silent edit touched.
- *Backfill of window `W`* (the only trigger that recovers an edit) — *bottom-right* recompute-region. input clamp: `adjustments WHERE adj_date IN W`; output clamp: partitions `W`.
  ```sql
  DELETE FROM daily_adjustments WHERE adj_date >= w0 AND adj_date < w1;
  INSERT INTO daily_adjustments
  SELECT adj_date, cost_center, SUM(amount) AS total_amount
  FROM adjustments WHERE adj_date >= w0 AND adj_date < w1 GROUP BY adj_date, cost_center;
  ```

### EX-05 — unified web + mobile event stream
- grid: construct=UNION ALL (two arms) | sources=web_events, mobile_events: both append-only, clocked | output-shape=time-partitioned | technique(s)=all-columns × new-day-either-arm → region-append; late row either arm → recompute-region on backfill
- expected: HOLDS
- probe-status: probed(G-09)
- use-case: web and mobile client telemetry, separately landed, unified into one canonical events table.

```sql
---
refresh: batched
timeseries: { event_time_column: event_ts, partition_column: event_date, granularity: day }
---
SELECT 'web' AS src, event_id, user_id, event_date, event_ts FROM smelt.sources.web_events
UNION ALL
SELECT 'mobile' AS src, event_id, user_id, event_date, event_ts FROM smelt.sources.mobile_events
```

G-09 confirmed the load-bearing mechanism: the derived window clamp applies to the **outer**
query, so a backfill re-reads both arms symmetrically — there is no per-arm bound to
under-cover one side. The `src` discriminator keeps the two arms' `(event_id)` domains from
colliding in the skeleton. Delta discovery is per-arm (per-input `S` vector, §4): a new day
in the mobile arm alone still triggers the region's maintenance.

**Physical maintenance.** Two sources, shared partition `event_date`. **Partition-local:** ✅ `web_events`, ✅ `mobile_events`. Each source changing is its own trigger; delta discovery is per-arm.
- *New web day `D`* (`web_events` Δ) — *top-right* region-append, reads the web arm only. input clamp: `web_events WHERE event_date = D`; output clamp: partition `event_date = D`.
  ```sql
  INSERT INTO events_unified SELECT 'web' AS src, event_id, … FROM web_events WHERE event_date = D;
  ```
- *New mobile day `D`* (`mobile_events` Δ) — symmetric on the mobile arm (the `src` discriminator keeps `event_id` domains disjoint).
- *Backfill `[t₀,tₙ)`* — *bottom-right* recompute both arms symmetrically (the clamp is on the *outer* query, G-09). `DELETE` the partitions, `INSERT … UNION ALL …` restricted to `event_date ∈ [t₀,tₙ)`.

### EX-06 — live stream + mutable history arm
- grid: construct=UNION ALL | sources=live: append-only; history: **mutable snapshot** (one-off corrections) | output-shape=time-partitioned | technique(s)=per-arm column groups share the region → recompute-region; history edits recovered only on backfill
- expected: CONDITIONAL(backfill-recovers, scoped to the history arm's partitions)
- probe-status: unprobed-candidate (named as G-09's uncovered variant in its coverage caveat)
- use-case: a vendor-supplied historical backfill table (occasionally re-issued with corrections) unioned under a live append-only stream at the cutover date.

Same SQL shape as EX-05. The framework's refinement over today: the per-input `S` vector
lets the plan state that partitions before the cutover are settled **relative to `history`'s
snapshot at last backfill**, while post-cutover partitions are settled immediately — two
different settle-bound entries in the §6 ledger for the *same* columns, split by region.
Today's surface can't say this; it just recomputes what you ask.

**Physical maintenance.** Two sources, shared partition `event_date` (cutover date splits the arms). **Partition-local:** ✅ `live`, ✅ `history` *for the op* (bounded to the pre-cutover partitions); the CONDITIONAL is detection of history edits, as in EX-04.
- *Live append, new day `D`* (`live` Δ, post-cutover) — *top-right* region-append. input clamp: `live WHERE event_date = D`; output clamp: partition `event_date = D`.
- *History edit* (`history` Δ, pre-cutover, backfill only) — *bottom-right* recompute of the affected pre-cutover partitions. input clamp: both arms restricted to the touched `event_date` window; output clamp: those partitions. `DELETE`+`INSERT … UNION ALL …`.

---

## Family B — Enrichment (joins, subqueries, attribution)

Where the plan first genuinely **factors by column group**: the driving fact's pass-through
columns and the enrichment columns have different mutation-sensitivity, so different cells
get different techniques. This family contains the paper's flagship inexpressible cell and
most of the UNSUPPORTED-TODAY backlog.

### EX-01 — 7-day conversion attribution (the paper's flagship)
- grid: construct=correlated EXISTS over a second append-only source | sources=bronze.events: append-only, clocked; conversions: append-only **with unbounded arrival lateness**, clocked | output-shape=time-partitioned, column-scoped enrichment | technique(s)=see matrix
- expected: UNSUPPORTED-TODAY(the `converted × late-conversion` fold cell — a delta-directed, column-scoped, key-and-window-bounded MERGE; today only the recompute column exists)
- probe-status: probed(SC-1 — **recompute technique only**: the 7-day reach *is* derived (post-FIX-1, soundly) and a backfill recovers a late conversion; the targeted fold cell has never executed because no machinery emits it)
- use-case: marketplace ad-click → purchase attribution; conversions arrive days after the click.

```yaml
# sources/conversions.yml
mutation_profile: append_only
timeseries: { event_time_column: conversion_ts, partition_column: conversion_date, granularity: day }
# no source_lateness: arrival lateness is unbounded — settle bound is watermark-relative (§6)
columns: …
```

```sql
---
refresh: batched
timeseries: { event_time_column: event_ts_utc, partition_column: event_date, granularity: day }
---
SELECT
    e.event_id, e.user_id, e.event_date, e.event_ts_utc,
    EXISTS (
        SELECT 1 FROM smelt.sources.conversions c
        WHERE c.user_id = e.user_id
          AND c.conversion_ts BETWEEN e.event_ts_utc AND e.event_ts_utc + INTERVAL '7 days'
    ) AS converted
FROM smelt.sources.bronze_events e
```

| column group | new event day | late conversion (append) | backfill `[t₀,tₙ)` |
|---|---|---|---|
| `{event_id, user_id, event_date, event_ts_utc}` | region-append | untouched | recompute-region |
| `{converted}` | bounded read of conversions `[D, D+7d]` → region-append | **delta-read → MERGE `converted` for that user's events in `[conv_ts−7d, conv_ts]`** | recompute-region (read widened `+7d`) |

The bolded cell is the whole motivation: pure fold corner, `BOOL_OR` monotone flip
`false→true`, cost ∝ conversion delta. Missing machinery: conversion-side delta discovery
driving a **column-scoped MERGE** keyed on `(user_id, event window)` — no emitter exists
(`dimension_horizon_merge` is the nearest dormant relative, [`03-design-forks.md`](03-design-forks.md)).
`converted`'s settle bound is watermark-relative (conversions watermark ≥ `event_ts + 7d`),
absolute only if `conversions` declared a `source_lateness`.

**Physical maintenance.** Three triggers → three 2×2 corners; the two sources each drive their own. **Partition-local:** ✅ `bronze_events`, ✅ `conversions` (every op bounded to `event_date` partitions).
- *New event day `D`* (`bronze_events` Δ) — skeleton → *top-right* region-append; `{converted}` → *off-diagonal* bounded read. input clamp: `bronze_events WHERE event_date = D`; `conversions WHERE conversion_ts >= D AND conversion_ts < D + INTERVAL '8 days'`. output clamp: partition `event_date = D`.
  ```sql
  INSERT INTO silver_event_conversions
  SELECT e.event_id, e.user_id, e.event_date, e.event_ts_utc,
         EXISTS (SELECT 1 FROM conversions c
                 WHERE c.user_id = e.user_id
                   AND c.conversion_ts BETWEEN e.event_ts_utc AND e.event_ts_utc + INTERVAL '7 days'
                   AND c.conversion_ts < D + INTERVAL '8 days')     -- input clamp (conversions)
         AS converted
  FROM bronze_events e WHERE e.event_date = D;                       -- output clamp
  ```
- *Late conversion (append) at `conv_ts`, user `u`* (`conversions` Δ) — `{converted}` → *top-left* fold; `BOOL_OR` monotone `false→true`, idempotent. **UNSUPPORTED-TODAY** (no emitter). input clamp: `event_date BETWEEN date(conv_ts−7d) AND date(conv_ts)` **(partition-local bound)** `AND user_id = u AND event_ts_utc BETWEEN conv_ts−7d AND conv_ts`. output clamp: `converted` on those partitions, user `u`.
  ```sql
  MERGE INTO silver_event_conversions m
  USING (SELECT event_id, event_date FROM bronze_events
         WHERE event_date BETWEEN CAST(conv_ts - INTERVAL '7 days' AS DATE)   -- partition-local scan bound
                              AND CAST(conv_ts AS DATE)
           AND user_id = u
           AND event_ts_utc BETWEEN conv_ts - INTERVAL '7 days' AND conv_ts) s
  ON m.event_id = s.event_id AND m.event_date = s.event_date                  -- partition-pruned target
  WHEN MATCHED THEN UPDATE SET converted = TRUE;
  ```
  *Amplification (secondary):* `user_id` ⟂ `event_date`, so the write touches ~8 date-partitions' files; deletion vectors / `OPTIMIZE` absorb the within-partition scatter — the *partition* bound (8 days, not the whole table) is what the plan guarantees.
- *Backfill `[t₀,tₙ)`* — both groups → *bottom-right* recompute; conversions scan widened `+7d`. input clamp: `bronze_events WHERE event_date >= t0 AND event_date < tn`; `conversions WHERE conversion_ts >= t0 AND conversion_ts < tn + INTERVAL '7 days'`. output clamp: partitions `[t₀,tₙ)`.
  ```sql
  DELETE FROM silver_event_conversions WHERE event_date >= t0 AND event_date < tn;
  INSERT INTO silver_event_conversions SELECT … FROM bronze_events e WHERE e.event_date >= t0 AND e.event_date < tn;
  ```

### EX-07 — orders enriched with customer tier
- grid: construct=inner-join enrichment (fact × dim) | sources=orders: append-only, clocked; customers: **mutable, unclocked lookup** | output-shape=time-partitioned + column-scoped enrichment | technique(s)=pass-through × new-day → region-append; `{tier}` × dim-churn → recompute-region on backfill (today) / column-scoped re-derivation (framework)
- expected: CONDITIONAL(enrichment staleness bounded by dimension churn + backfill cadence: `tier` reflects the dimension as-of the partition's last materialization)
- probe-status: probed(G-05)
- use-case: e-commerce orders enriched with the customer's loyalty tier for margin reporting; tiers change occasionally.

```yaml
# sources/customers.yml
mutation_profile: mutable      # no timeseries: — an unclocked lookup, read in full
columns: …
```

```sql
---
refresh: batched
timeseries: { event_time_column: order_ts, partition_column: order_date, granularity: day }
---
SELECT o.order_date, o.order_id, o.user_id, o.amount, c.tier
FROM smelt.sources.orders o
JOIN smelt.sources.customers c ON c.user_id = o.user_id
```

G-05 confirmed both directions: a backfill re-reads the dimension's *current* contents
(no snapshot pinning anywhere), and a forward-only advance leaves old partitions'
`tier` permanently stale. Under the framework the `{tier}` group's mutation-sensitivity is
`{customers}` alone, so the honest ledger entry is per-column staleness — and the upgrade
path is EX-08. Note the two possible *intents*: "tier as it was when the order happened"
(then `tier` should come from an SCD2 dim, EX-28, and this plan is *wrong by design*) vs
"current tier" (then EX-08's targeted re-derivation is what you want). The framework forces
that choice into the open instead of leaving it to backfill accidents.

**Physical maintenance.** Two sources: `orders` (partition `order_date`), `customers` (unclocked mutable dim, *no partition*). **Partition-local:** ✅ `orders`; ❌ `customers` — a tier change has **no clock**, so its footprint is every past order of that user across *all* partitions. This ❌ is the honest diagnosis behind the CONDITIONAL.
- *New order day `D`* (`orders` Δ) — pass-through → region-append; `{tier}` joined at creation. input clamp: `orders WHERE order_date = D`; `customers` read in full (unclocked — not partition-bounded, but a small dim). output clamp: partition `order_date = D`.
  ```sql
  INSERT INTO orders_tiered
  SELECT o.order_date, o.order_id, o.user_id, o.amount, c.tier
  FROM orders o JOIN customers c ON c.user_id = o.user_id WHERE o.order_date = D;
  ```
- *Tier change for user `u`* (`customers` Δ) — `{tier}` → *bottom-left* column re-derivation **in principle**, but the footprint (all of `u`'s orders) has **no partition bound** → ❌ the "targeted" `MERGE` scatters across the whole table:
  ```sql
  MERGE INTO orders_tiered m USING (SELECT user_id, tier FROM customers WHERE user_id = u) s
  ON m.user_id = s.user_id                              -- ❌ no partition predicate exists → full-table target scan
  WHEN MATCHED THEN UPDATE SET tier = s.tier;
  ```
  Today this is recovered only by an explicit whole-history backfill; the framework's move is to **warn/refuse** rather than run this silently (upgrade path: EX-08's clocked change feed + a horizon).

### EX-08 — catalog re-pricing propagated to recent order lines
- grid: construct=inner-join enrichment | sources=order_lines: append-only, clocked; products: **change feed**, unclocked, `unique_key: [product_id]` | output-shape=time-partitioned + column-scoped enrichment | technique(s)=`{unit_price, margin}` × product-delta → **column-scoped region re-derivation** (dimension-driven horizon-bounded MERGE: re-derive only affected products' rows within the derived horizon)
- expected: UNSUPPORTED-TODAY(`dimension_horizon_merge` exists but is dormant — zero production call sites; wiring it is design fork FIX-2/G-10 territory, [`03-design-forks.md`](03-design-forks.md))
- probe-status: probed(G-10 — the classification half; the execution half has no live path)
- use-case: retailer re-prices SKUs; recent order-line margins must be re-derived for the affected SKUs only, not the whole history.

```yaml
# sources/products.yml
mutation_profile: change_feed
unique_key: [product_id]        # PROPOSED as a source-level key declaration (05-source-properties.md)
columns: …
```

| column group | new order day | product-price delta | backfill |
|---|---|---|---|
| `{order_id, qty, order_date}` | region-append | untouched | recompute-region |
| `{unit_price, margin}` | joined at creation | **delta-read (changed products) → MERGE those products' rows in `[now−horizon, now]`** | recompute-region |

Broadcast shape (§12 of 01-framework.md: breaks A per-row, keeps B): a dimension delta touches many fact rows,
but each row's read stays bounded. Bounded write requires either a horizon ("only re-derive
the last 90 days' lines") or dimension-churn locality. Needs: change-feed delta discovery on
an unclocked source, the one-to-one join-cardinality proof (composite keys included — G-10),
and the MERGE emitter wired. All three exist as dormant or partial code.

**Physical maintenance.** Two sources: `order_lines` (partition `order_date`), `products` (change feed, unclocked). **Partition-local:** ✅ `order_lines`; ⚠️ `products` — a price delta's footprint is all order-lines with that `product_id`, which spans unbounded partitions **unless a horizon** (`[now−90d, now]`) bounds it. The horizon *is* what buys partition-locality here.
- *New order day `D`* (`order_lines` Δ) — `{order fields}` region-append; `{unit_price, margin}` joined at creation. input clamp: `order_lines WHERE order_date = D`. output clamp: partition `order_date = D`.
- *Product-price Δ (products)* — `{unit_price, margin}` → *bottom-left* column `MERGE`. input clamp: changed products (delta); `order_lines WHERE order_date >= now − INTERVAL '90 days'` **(horizon = partition bound)** `AND product_id IN (changed)`. output clamp: `{unit_price, margin}` on partitions `[now−90d, now]`.
  ```sql
  MERGE INTO order_line_margin m
  USING (SELECT ol.order_id, ol.order_date, p.unit_price, p.margin
         FROM order_lines ol JOIN products_changed p ON p.product_id = ol.product_id
         WHERE ol.order_date >= now - INTERVAL '90 days') s          -- horizon → partition-local scan
  ON m.order_id = s.order_id AND m.order_date = s.order_date         -- partition-pruned target
  WHEN MATCHED THEN UPDATE SET unit_price = s.unit_price, margin = s.margin;
  ```
  *Amplification (secondary):* `product_id` ⟂ `order_date` → within the 90-day partitions the changed-product rows scatter; deletion vectors / `OPTIMIZE` absorb it. Without the horizon, ⚠️ becomes ❌ (unbounded, EX-07's failure).

### EX-09 — orders LEFT JOIN refunds
- grid: construct=LEFT JOIN, null-preserving | sources=orders: append-only; refunds: append-only, late-arriving, clocked (own clock `refund_date`) | output-shape=time-partitioned | technique(s)=`{refund_amt}` × late-refund → recompute-region on backfill (today); fold corner = NULL→value flip MERGE (framework, same shape as EX-01's)
- expected: HOLDS (recompute arm, probed); the targeted arm shares EX-01's UNSUPPORTED-TODAY gap
- probe-status: probed(G-06)
- use-case: order-level finance table where refunds may arrive weeks later; `refund_amt` starts NULL and fills in.

The null-preservation subtlety G-06 pinned: the unmatched row *exists* with `refund_amt =
NULL` (skeleton decided by `orders` alone — a LEFT JOIN's row set is driving-side-only, so
`refunds` is **not** in the skeleton's mutation-sensitivity), and a late refund is a
payload-cell flip. That makes the fold corner *cleaner* than an inner join's (where a late
right row would create a row — skeleton mutation). Contrast deliberately with EX-01: same
fold shape, different construct.

**Physical maintenance.** Two sources: `orders` (partition `order_date`), `refunds` (own clock `refund_date`, late). **Partition-local:** ✅ `orders`; ⚠️ `refunds` — a late refund's footprint is the one order it matches (by `order_id`); bounding it to a partition needs the order's date, i.e. a declared refund-lateness horizon so `order_date ∈ [refund_date−N, refund_date]`.
- *New order day `D`* (`orders` Δ) — *top-right* region-append; `refund_amt = NULL` initially. input clamp: `orders WHERE order_date = D`; output clamp: partition `order_date = D`.
- *Late refund (refunds Δ)* — `{refund_amt}` → *top-left* fold, `NULL→value` flip (skeleton untouched — LEFT JOIN row set is orders-only). input clamp: `orders WHERE order_date BETWEEN date(refund_date−N) AND refund_date` **(horizon = partition bound)** `AND order_id = r.order_id`. output clamp: `{refund_amt}` those rows.
  ```sql
  MERGE INTO orders_refunds m
  USING (SELECT order_id, order_date, refund_amt FROM refunds_delta) s
  ON m.order_id = s.order_id AND m.order_date = s.order_date         -- partition-pruned via horizon
  WHEN MATCHED THEN UPDATE SET refund_amt = s.refund_amt;
  ```
  *Amplification (secondary):* `order_id` ⟂ `order_date` → scatter across the horizon partitions; deletion vectors absorb.

### EX-10 — point-in-time feature join on a composite key
- grid: construct=join fan-out proof (N:1 on composite key) | sources=events: append-only; features: append-only snapshot-per-day, **composite key `(user_id, dt)`** | output-shape=time-partitioned enrichment | technique(s)=column-scoped re-derivation, admissible **iff** the join is proven one-to-one on the composite key
- expected: UNSUPPORTED-TODAY(`join_shape::JoinContext` declares uniqueness per single column only; a genuine composite key is misclassified `OneToMany` and would refuse the targeted technique — over-conservative, G-10)
- probe-status: probed(G-10)
- use-case: ML feature-store point-in-time join — each event enriched with that user's feature vector *as of that day*; `(user_id, dt)` is unique, neither column alone is.

```sql
SELECT e.event_id, e.event_date, e.user_id, f.churn_score, f.ltv_bucket
FROM smelt.sources.events e
JOIN smelt.sources.features f ON f.user_id = e.user_id AND f.dt = e.event_date
```

Ground truth (G-10's proptest) proves one-to-one; the classifier can't express it. The
composite natural key is arguably the *common* real-world case (daily dim snapshots, SCD2
`(key, valid_from)`), so this expressiveness gap gates most of Family B's targeted
techniques. Recommendation in [`03-design-forks.md`](03-design-forks.md).

**Physical maintenance.** Two sources: `events` (partition `event_date`), `features` (snapshot-per-day, composite key `(user_id, dt)`). **Partition-local:** ✅ `events`, ✅ `features` — a feature delta for day `dt` touches only events of `event_date = dt`, a single partition. The tightest targeted case in Family B.
- *New event day `D`* (`events` Δ) — region-append, join `features WHERE dt = D`. input clamp: `events WHERE event_date = D`; `features WHERE dt = D`. output clamp: partition `event_date = D`.
- *Feature Δ for day `dt` (features Δ)* — `{churn_score, ltv_bucket}` → *bottom-left* column `MERGE`, one partition. input clamp: `events WHERE event_date = dt AND user_id IN (changed)`; output clamp: those rows.
  ```sql
  MERGE INTO event_features m
  USING (SELECT e.event_id, e.event_date, f.churn_score, f.ltv_bucket
         FROM events e JOIN features_changed f ON f.user_id = e.user_id AND f.dt = e.event_date
         WHERE e.event_date = dt) s
  ON m.event_id = s.event_id AND m.event_date = s.event_date         -- prunes to the single partition dt
  WHEN MATCHED THEN UPDATE SET churn_score = s.churn_score, ltv_bucket = s.ltv_bucket;
  ```
  *Amplification (secondary):* `user_id` ⟂ `event_date` → scatter *within the single partition `dt`*; deletion vectors absorb. (UNSUPPORTED-TODAY is the composite-key *classifier* gap G-10, not locality — the op is ideal.)

### EX-11 — support-ticket count within 30 days of order
- grid: construct=correlated **scalar** subquery, additive (`COUNT`) | sources=orders: append-only; tickets: append-only, late | output-shape=time-partitioned enrichment | technique(s)=`{ticket_count}` × late-ticket → fold (+1 increment MERGE) **with per-delta ledger**, or bounded recompute of affected rows
- expected: UNSUPPORTED-TODAY(same emitter gap as EX-01, **plus** the additive fold needs the generalized ledger's per-delta bookkeeping — re-delivering a ticket delta must not double-increment)
- probe-status: unprobed-candidate
- use-case: CX analytics — orders annotated with how many support tickets the buyer filed within 30 days.

```sql
SELECT o.order_id, o.order_date,
       (SELECT COUNT(*) FROM smelt.sources.tickets t
         WHERE t.user_id = o.user_id
           AND t.ticket_ts BETWEEN o.order_ts AND o.order_ts + INTERVAL '30 days') AS ticket_count
FROM smelt.sources.orders o
```

Deliberate contrast with EX-01: swap `EXISTS` (idempotent `BOOL_OR`, frontier-only ledger)
for `COUNT` (additive, per-delta ledger). Same reach derivation, same footprint reflection
(`(0, 30d)` scan ↔ `(30d, 0)` footprint), strictly stronger ledger obligation — the pair is
the cleanest empirical probe of the ledger design's additive/idempotent grading
([`01-framework.md`](01-framework.md) OQ4 design).

**Physical maintenance.** Two sources: `orders` (partition `order_date`), `tickets` (append, late). **Partition-local:** ✅ `orders`, ✅ `tickets` (the 30-day window bounds the footprint to `[ticket_ts−30d, ticket_ts]`).
- *New order day `D`* (`orders` Δ) — region-append, count clamp `tickets [order_ts, +30d]`. input clamp: `orders WHERE order_date = D`; `tickets WHERE ticket_ts >= D AND ticket_ts < D + INTERVAL '31 days'`. output clamp: partition `order_date = D`.
- *Ticket Δ at `ticket_ts`, user `u` (tickets Δ)* — `{ticket_count}` → *top-left* fold, `+1` increment with a **per-delta ledger** (redelivery must not double-increment). input clamp: `orders WHERE order_date BETWEEN date(ticket_ts−30d) AND date(ticket_ts) AND user_id = u`. output clamp: `{ticket_count}` those rows.
  ```sql
  MERGE INTO order_ticket_count m
  USING (SELECT order_id, order_date FROM orders
         WHERE order_date BETWEEN CAST(ticket_ts - INTERVAL '30 days' AS DATE) AND CAST(ticket_ts AS DATE)
           AND user_id = u AND order_ts BETWEEN ticket_ts - INTERVAL '30 days' AND ticket_ts) s
  ON m.order_id = s.order_id AND m.order_date = s.order_date
  WHEN MATCHED THEN UPDATE SET ticket_count = ticket_count + 1;      -- ledger guards against re-delivery
  ```
  *Amplification (secondary):* `user_id` ⟂ `order_date` → scatter across ~30 partitions; deletion vectors absorb.

### EX-35 — score of the first conversion within 7 days
- grid: construct=correlated **first-value pick** (`MIN_BY(score, conversion_ts)`) | sources=events: append-only, clocked; conversions: append-only **with unbounded arrival lateness**, clocked, carries `score` | output-shape=time-partitioned, column-scoped enrichment | technique(s)=see below
- expected: UNSUPPORTED-TODAY(order-sensitive fold — the combiner is *first-writer-wins within window*, which a later append overturns only if it lands **earlier**; needs the ledger to store the current winner's `conversion_ts`, strictly more state than EX-01's monotone bit and EX-11's counter)
- probe-status: unprobed-candidate
- use-case: attribution that records not *whether* a click converted but the **quality signal** (`score`) of the first conversion it drove — e.g. first-purchase basket size within the 7-day window.

```sql
---
refresh: batched
timeseries: { event_time_column: event_ts_utc, partition_column: event_date, granularity: day }
---
SELECT e.event_id, e.event_date,
       (SELECT c.score FROM smelt.sources.conversions c
         WHERE c.user_id = e.user_id
           AND c.conversion_ts BETWEEN e.event_ts_utc AND e.event_ts_utc + INTERVAL '7 days'
         ORDER BY c.conversion_ts LIMIT 1) AS first_conv_score        -- = MIN_BY(score, conversion_ts)
FROM smelt.sources.events e
```

**Physical maintenance.** Same reach as EX-01 (`(0,+7d)` scan ↔ `(+7d,0)` footprint), a *harder ledger grade*. Two sources: `events` (partition `event_date`), `conversions`. **Partition-local:** ✅ `events`, ✅ `conversions`.
- *New event day `D`* / *Backfill* (`events` Δ) — identical corners & SQL shape to EX-01 (the scalar subquery replaces `EXISTS`); eliding.
- *Late conversion at `conv_ts`, score `v`, user `u`* (`conversions` Δ) — `{first_conv_score}` → *top-left* fold, **conditional overwrite**: replaces the value **only if** `conv_ts` precedes the stored winner (or none exists). input clamp: `event_date BETWEEN date(conv_ts−7d) AND date(conv_ts) AND user_id = u AND event_ts_utc BETWEEN conv_ts−7d AND conv_ts`. output clamp: `{first_conv_score}` those rows, gated on `conv_ts < stored winner ts`.
  ```sql
  MERGE INTO first_conv m
  USING (SELECT event_id, event_date FROM events
         WHERE event_date BETWEEN CAST(conv_ts - INTERVAL '7 days' AS DATE) AND CAST(conv_ts AS DATE)
           AND user_id = u AND event_ts_utc BETWEEN conv_ts - INTERVAL '7 days' AND conv_ts) s
  ON m.event_id = s.event_id AND m.event_date = s.event_date
  WHEN MATCHED AND (m.first_conv_ts IS NULL OR conv_ts < m.first_conv_ts)
    THEN UPDATE SET first_conv_score = v, first_conv_ts = conv_ts;   -- needs stored winner ts in the ledger
  ```
  *Amplification (secondary):* `user_id` ⟂ `event_date` → ~8 date-partitions' files; deletion vectors / `OPTIMIZE` absorb.

The deliberate three-way contrast now completes: **EX-01** `EXISTS`→`BOOL_OR` (frontier bit) · **EX-11** `COUNT`→additive (per-delta counter) · **EX-35** `MIN_BY`→order-sensitive pick (stored winner + timestamp). Same window derivation, three distinct ledger obligations, ascending in stored state.

### EX-12 — currency-converted revenue (two mutable inputs, one projection)
- grid: construct=multi-input column group (`o.amount * fx.rate`) | sources=orders: **mutable** (order amendments); fx_rates: **mutable**, unclocked | output-shape=time-partitioned | technique(s)=merged column group → recompute-region only (factoring degenerates)
- expected: CONDITIONAL(backfill-recovers, now for the *whole row*: the merged group has mutation-sensitivity `{orders, fx_rates}`, so no targeted technique isolates either input)
- probe-status: not-probe-worthy(the verdict is definitional — group merging is a classification fact, not a runtime behaviour to falsify; the *classifier* producing it is what needs a Link-B probe)
- use-case: multi-currency revenue reporting where finance amends orders and refreshes FX fixings retroactively.

```sql
SELECT o.order_date, o.order_id, o.amount * fx.rate AS amount_usd
FROM smelt.sources.orders o
JOIN smelt.sources.fx_rates fx ON fx.ccy = o.ccy AND fx.fixing_date = o.order_date
```

The §5 limit case: one projection mutation-sensitive to two inputs merges their groups, and
the plan collapses to today's per-model story. The design-guidance answer (OQ5 resolution):
split the model — land `amount` and `ccy` (sensitivity `{orders}`), keep the conversion in a
downstream view or a separate maintained column fed by the fx feed. The framework's value
here is the *diagnostic that explains the degeneration*, so the user can restructure.

**Physical maintenance.** Two sources: `orders` (mutable, partition `order_date`), `fx_rates` (mutable, unclocked; join `fx.fixing_date = o.order_date`). **Partition-local:** ✅ `orders`, ✅ `fx_rates` — both project onto `order_date` (an fx fixing for date `d` touches exactly orders of `order_date = d`), the exact contrast with EX-07 where the dim's key (`user_id`) is *orthogonal* to the partition. The *op* is bounded once triggered; detecting *which* fx fixing changed is the CONDITIONAL's detection gap (unclocked mutable, as EX-04), not a locality failure. Partition-local holds, yet the merged column group still forces recompute (no targeted isolation) — a clean demonstration that **partition-local ≠ foldable**.
- *Order amendment on day `d`* (`orders` Δ) — *bottom-right* recompute partition `d`. input clamp: `orders WHERE order_date = d`, `fx_rates WHERE fixing_date = d`; output clamp: partition `order_date = d`.
- *FX fixing change for day `d`* (`fx_rates` Δ) — *bottom-right* recompute partition `d` (same partition, via `fixing_date = order_date`).
  ```sql
  DELETE FROM revenue_usd WHERE order_date = d;
  INSERT INTO revenue_usd
  SELECT o.order_date, o.order_id, o.amount * fx.rate AS amount_usd
  FROM orders o JOIN fx_rates fx ON fx.ccy = o.ccy AND fx.fixing_date = o.order_date
  WHERE o.order_date = d;
  ```
  No `MERGE` arm: the group `{amount_usd}` is mutation-sensitive to both inputs, so it recomputes wholesale. Partition-aligned (`order_date`) — no amplification.

---

## Family C — Aggregation

Aggregates are where combiner algebra decides the read axis: additive folds need the
ledger's per-delta memory, idempotent folds need only a frontier, holistic aggregates have
no bounded state at all. Today every one of these runs recompute-region (the loop's
headline mechanism finding) — the fold column of each matrix is the build-out.

### EX-13 — daily revenue
- grid: construct=additive agg (SUM), GROUP BY = partition col | sources=payments: append-only | output-shape=time-partitioned | technique(s)=new-day → recompute-region (today, HOLDS) or fold-delta (framework, needs ledger)
- expected: HOLDS
- probe-status: probed(G-01, G-02)
- use-case: the canonical daily-revenue rollup.

The happy-path control. G-02's finding worth restating: today re-delivery is safe **because
of the mechanism** (DELETE+INSERT window replace), not the algebra — the moment a true
fold-delta path exists, re-delivery safety becomes a *ledger* obligation (EX-20).

**Physical maintenance.** One source `payments`, partition = day, `GROUP BY` day = partition-aligned. **Partition-local:** ✅ `payments`.
- *New day `D`* — today *bottom-right* recompute-region (DELETE+INSERT window replace, G-02-safe); the framework's *top-left* fold arm needs the ledger. input clamp: `payments WHERE pay_date = D`; output clamp: partition `D`.
  ```sql
  DELETE FROM daily_revenue WHERE pay_date = D;                       -- today: window replace
  INSERT INTO daily_revenue SELECT pay_date, SUM(amount) AS revenue FROM payments WHERE pay_date = D GROUP BY pay_date;
  -- framework fold arm (needs ledger): MERGE … SET revenue = revenue + Δ  keyed on pay_date
  ```
  Partition-aligned — no amplification.

### EX-14 — order totals over an OLTP CDC feed with cancellations
- grid: construct=additive agg (SUM of signed deltas) | sources=order_lines_cdc: **change feed with retractions** (insert/update/delete images), `unique_key: [line_id]` | output-shape=keyed end-state per order, or time-partitioned by order_date | technique(s)=fold-delta with **invertible** combiner (SUM has an inverse: fold `+new − old`), per-delta ledger
- expected: UNSUPPORTED-TODAY(change-feed-driven fold: `mutation_profile: change_feed` parses today but no execution path consumes deltas as deltas — everything still re-scans; needs the ledger + a delta-apply emitter)
- probe-status: unprobed-candidate
- use-case: order-management OLTP replicated via Debezium-style CDC; lines are added, edited, and cancelled; the mart must track live order totals without full re-scans.

```yaml
# sources/order_lines_cdc.yml
mutation_profile: change_feed
unique_key: [line_id]                    # PROPOSED (05-source-properties.md)
change_feed: { op_column: _op, ops: [insert, update, delete], before_image: true }   # PROPOSED
columns: …
```

The framework's condition 3 (faithful fold) is satisfiable here **because SUM is
invertible**: a retraction folds as subtraction. Contrast EX-16, where the same source
property meets a non-invertible combiner and is refused. This is the single most valuable
UNSUPPORTED-TODAY cell for CDC-ingest users, and the reason `change_feed` needs the richer
declaration shape (op column, before-images) proposed in 05.

**Physical maintenance.** One source `order_lines_cdc` (change feed, key `line_id`), output keyed per `order_id` (or partitioned by `order_date`). **Partition-local:** ✅ keyed end-state (targeted per `order_id`); ⚠️ if time-partitioned by `order_date` (the delta must carry `order_date` to prune).
- *CDC delta (insert / update / delete image)* — *top-left* fold, **invertible** SUM (`+new − old`), per-delta ledger keyed on `line_id`. input clamp: the delta rows; output clamp: the affected `order_id` rows.
  ```sql
  MERGE INTO order_totals m
  USING (SELECT order_id, SUM(CASE _op WHEN 'delete' THEN -amount
                                       WHEN 'update' THEN amount - before_amount
                                       ELSE amount END) AS delta
         FROM order_lines_cdc_delta GROUP BY order_id) s
  ON m.order_id = s.order_id
  WHEN MATCHED THEN UPDATE SET total = total + s.delta                -- retraction folds as subtraction
  WHEN NOT MATCHED THEN INSERT (order_id, total) VALUES (s.order_id, s.delta);
  ```
  *Amplification (secondary):* only if partitioned by `order_date` with the key orthogonal; clustered/keyed by `order_id` it is minimal.

### EX-15 — first/last login per user-day
- grid: construct=idempotent agg (MIN/MAX) | sources=logins: append-only (re-delivery tolerated) | output-shape=time-partitioned | technique(s)=fold-delta, frontier-only ledger (idempotent: re-fold harmless)
- expected: HOLDS
- probe-status: probed(G-03; abstract arm P0-5)
- use-case: auth telemetry — first and last login timestamp per user per day, tolerant of duplicate log shipping.

The idempotent control. Ledger grading pays off: no per-delta identity needed, just a
frontier watermark per input — the cheap end of the generalized-ledger design.

**Physical maintenance.** One source `logins`, partition = day, `GROUP BY user, day`. **Partition-local:** ✅ `logins`.
- *New day `D`* — *top-left* fold, frontier-only ledger (idempotent: re-fold harmless) or *bottom-right* recompute. input clamp: `logins WHERE login_date = D`; output clamp: partition `D`.
  ```sql
  MERGE INTO login_bounds m
  USING (SELECT user_id, login_date, MIN(login_ts) mn, MAX(login_ts) mx FROM logins WHERE login_date = D GROUP BY user_id, login_date) s
  ON m.user_id = s.user_id AND m.login_date = s.login_date
  WHEN MATCHED THEN UPDATE SET first_login = LEAST(m.first_login, s.mn), last_login = GREATEST(m.last_login, s.mx)
  WHEN NOT MATCHED THEN INSERT VALUES (s.user_id, s.login_date, s.mn, s.mx);   -- idempotent under redelivery
  ```
  Target partition = `login_date` — no amplification.

### EX-16 — lowest-price tracker over a mutable price table
- grid: construct=idempotent agg (MIN) **as fold** | sources=prices: **mutable snapshot** | output-shape=keyed per product | technique(s)=fold-delta — refused; recompute admissible
- expected: REFUSED(observer semantics: fold = "min ever observed", recompute = "min in the current snapshot"; MIN is non-invertible, a raised price can never un-fold — the admission matrix's `KeyedSnapshotSourceUnsupportedColumn`)
- probe-status: probed(G-04 recompute arm HOLDS; P0-5 holds the deterministic fold REFUTED witness)
- use-case: competitor price monitoring — "lowest price we have ever seen" vs "lowest current price" are *different products*; the refusal forces the modeller to say which they mean.

The refusal is the feature: if the user wants min-ever-observed, that is an append-only
*observation log* they should land first (EX-02 shape) and fold over; if they want
min-current, that is a recompute over the snapshot. Both are expressible — just not by
silently reinterpreting one as the other.

**Physical maintenance.** One source `prices` (mutable snapshot, unclocked), keyed per `product_id`. **Partition-local:** ❌ `prices` — this is *why* the fold is refused **and** why the recompute is a full read: a mutable snapshot has no clock/partition, so a raised price's footprint (min-ever) chains across the entire observation history, and min-current re-reads the whole snapshot. No bounded op exists.
- *Fold (min-ever)* — REFUSED (non-invertible MIN can't un-fold a raised price; `KeyedSnapshotSourceUnsupportedColumn`).
- *Recompute (min-current)* — *bottom-right* full snapshot scan per key; not partition-bounded.
  ```sql
  DELETE FROM lowest_price;                                           -- ❌ no partition clamp available
  INSERT INTO lowest_price SELECT product_id, MIN(price) AS lowest FROM prices GROUP BY product_id;
  ```
  The escape that restores locality: land an append-only *observation log* (EX-02 shape, clocked) and fold over it.

### EX-17 — daily p50 latency and unique users
- grid: construct=holistic agg (MEDIAN, COUNT(DISTINCT)) | sources=requests: append-only, at-least-once shipping | output-shape=time-partitioned | technique(s)=recompute-region only; fold-delta REFUSED(no bounded combiner state)
- expected: HOLDS (recompute); the plan simply has no fold column for these cells
- probe-status: probed(G-07)
- use-case: SRE golden-signals rollup — per-day p50/p95 latency and unique-caller counts.

Where the algebraic ladder ends: holistic combiners keep the read axis pinned at full-input.
The per-column-group framing earns its keep when a model mixes them — `SUM(bytes)` folds
while `MEDIAN(latency)` recomputes, in the *same* model, rather than the whole model being
demoted to the weakest column's technique.

**Physical maintenance.** One source `requests`, partition = day. **Partition-local:** ✅ `requests` *at region granularity* — but holistic combiners keep the read axis pinned at full-input, so there is no fold arm (another instance of partition-local ≠ foldable).
- *New day `D`* / *Backfill* — *bottom-right* recompute-region only. input clamp: `requests WHERE req_date ∈ region`; output clamp: those partitions.
  ```sql
  DELETE FROM golden_signals WHERE req_date = D;
  INSERT INTO golden_signals
  SELECT req_date, MEDIAN(latency_ms) AS p50, COUNT(DISTINCT caller) AS uniq FROM requests WHERE req_date = D GROUP BY req_date;
  ```
  Partition-aligned — no amplification.

### EX-18 — weekly finance rollup over daily partitions
- grid: construct=additive agg, GROUP BY **coarser** than the source clock (week over days) | sources=daily_revenue (a smelt model): append-only-by-partition, late days within the horizon | output-shape=time-partitioned (week) | technique(s)=new-day → RMW-region of the containing week (read stored week row + day delta, fold) or recompute-region of the week
- expected: HOLDS (recompute of the containing week today, provided the write window is widened to whole weeks); the RMW/fold arm needs the ledger
- probe-status: unprobed-candidate (the write-window coarsening interaction specifically)
- use-case: finance closes weeks; a day landing mid-week must update its week's row without touching closed weeks.

The region/granularity interaction: a day-grain delta's footprint is its containing week, so
the write region must round up to week boundaries (`batched_models.md`'s write-window
widening, now derived from the footprint map). The additive fold arm (add the new day into
the stored week row) is the textbook per-delta-ledger case: fold a day twice and the week
double-counts — exactly the G-02 hazard, but on a path where a real fold would exist.

**Physical maintenance.** One source `daily_revenue` (partition = day), output partition = week. **Partition-local:** ✅ — a day-grain delta's footprint is its containing week; the write region rounds up to week boundaries. §4-interchangeable pair.
- *New day mid-week `W`* — **either** *top-right* RMW-week (read stored week row + day delta, fold) **or** *bottom-right* recompute-week. input clamp: `daily_revenue WHERE week(d) = W` (partition rounds to the week); output clamp: partition `week = W`.
  ```sql
  -- RMW-week: read the stored week row, add the day, replace one row
  UPDATE weekly_finance SET revenue = revenue + (SELECT revenue FROM daily_revenue WHERE d = new_day) WHERE week = W;
  -- recompute-week: replace the whole week from its days
  DELETE FROM weekly_finance WHERE week = W;
  INSERT INTO weekly_finance SELECT date_trunc('week', d) AS week, SUM(revenue) FROM daily_revenue WHERE week(d) = W GROUP BY 1;
  ```
  Write-window widening (day → week) is the footprint-map rounding; the fold arm carries the double-count hazard (ledger). Partition-aligned (week) — no amplification.

### EX-19 — lifetime GMV counter
- grid: construct=additive agg, **no GROUP BY** (global scalar) | sources=payments: append-only | output-shape=unpartitioned single-row rollup | technique(s)=fold-delta into keyed state (grain = the empty key) — framework; full-refresh-only today
- expected: UNSUPPORTED-TODAY(a `refresh: keyed` model requires a GROUP BY key; the empty-key grain — one global row, folded — has no home, so today this is `refresh: full` and re-scans all history every run)
- probe-status: unprobed-candidate
- use-case: the company-dashboard "total GMV all-time" tile, updated hourly without scanning years of payments.

The degenerate-but-common cell: grain = `()`. Everything the keyed mode needs exists (an
additive fold, a merge target of one row); only the surface refuses the empty key. Cheap
win; noted in [`09-spec-readiness.md`](09-spec-readiness.md).

**Physical maintenance.** One source `payments`, output = single unpartitioned row (grain `()`). **Partition-local:** ✅ trivially under the framework fold (reads only the delta into one row); ❌ *today* (`refresh: full` re-scans all history).
- *Payment Δ (framework)* — *top-left* fold into single-row state. input clamp: the delta; output clamp: the one row.
  ```sql
  MERGE INTO lifetime_gmv m USING (SELECT SUM(amount) AS delta FROM payments_delta) s
  ON TRUE WHEN MATCHED THEN UPDATE SET gmv = gmv + s.delta;           -- one-row merge target
  ```
- *Today* — full re-scan: `INSERT OVERWRITE … SELECT SUM(amount) FROM payments` (no empty-key home). Single row — no amplification.

### EX-20 — billing events on at-least-once delivery
- grid: construct=additive agg (SUM of charges) | sources=charges: append-only, **at-least-once (re-delivered)**, `key_recurrence: {key: [charge_id], window: '3 days'}` | output-shape=time-partitioned | technique(s)=recompute-region (HOLDS today, mechanism-level); fold-delta requires either an upstream dedup (EX-27) or per-delta ledger identity = `charge_id`
- expected: CONDITIONAL(under fold: re-delivery safety is a ledger obligation keyed on `charge_id` within the declared recurrence window; under today's recompute: HOLDS unconditionally, G-02)
- probe-status: probed(G-02 for the recompute arm; the fold-with-ledger arm is the loop's next natural cell)
- use-case: usage-based billing where the event bus re-delivers; double-counting a charge is a customer-facing incident.

The example that makes the ledger design concrete: the delta identity the additive entry
must record is exactly the business key + recurrence window the source already declares.
Best practice remains dedup-first (EX-27 upstream, then EX-13 shape downstream) — the plan
matrix makes the cost of skipping that layer legible.

**Physical maintenance.** One source `charges`, partition = day, `key_recurrence: {charge_id, 3d}`. **Partition-local:** ✅ `charges` (the recurrence window bounds redelivery locality).
- *New day `D`* — *bottom-right* recompute-region today (DELETE+INSERT window replace, G-02-safe unconditionally); the *top-left* fold arm needs a per-delta ledger keyed on `charge_id`. input clamp: `charges WHERE charge_date = D`; output clamp: partition `D`.
  ```sql
  DELETE FROM billing_totals WHERE charge_date = D;                   -- today, re-delivery-safe by mechanism
  INSERT INTO billing_totals SELECT charge_date, SUM(amount) FROM charges WHERE charge_date = D GROUP BY charge_date;
  -- fold arm: MERGE … SET total = total + Δ  with ledger dedup on charge_id within the 3-day recurrence window
  ```
  Partition-aligned — no amplification.

---

## Family D — Trajectory & state

Running values are where output **grain** decides everything (§7): the end-state projection
folds cheaply; the stored trajectory has an irreducibly unbounded forward footprint under
late data. This family is the boundary between HOLDS, CONDITIONAL, and the deferred
as-of-run contract.

### EX-21 — wallet running balance (self-referential)
- grid: construct=self-referential batched model (prior-day balance + today's transactions) | sources=txns: append-only, late rows | output-shape=key×partition trajectory | technique(s)=new-day → ordered region-append reading own prior partition; late row → recompute mutated partition **plus cascade over every later partition, in order**
- expected: CONDITIONAL(cascade-required: nothing detects that a backfilled partition's stored value changed, nothing schedules downstream partitions; a lone backfill leaves every later balance silently stale)
- probe-status: probed(G-08; G-11 blocks the *direct-join* spelling — the model below uses the subquery-wrapped workaround)
- use-case: fintech wallet ledger — per-account daily closing balance, auditable at every day, late-posted transactions must repair history.

```sql
---
refresh: batched
timeseries: { event_time_column: d, partition_column: d, granularity: day }
---
SELECT t.account_id, t.d,
       COALESCE(bal.balance, 0) + SUM(t.amount) AS balance
FROM smelt.sources.txns t
LEFT JOIN (SELECT account_id, d, balance FROM smelt.wallet_balance) bal
  ON bal.account_id = t.account_id AND bal.d = t.d - INTERVAL '1 day'
GROUP BY t.account_id, t.d, bal.balance
```

G-08's trap, verbatim: backfill day-1 and days 2..n stay wrong until each is re-run in
order. The knob this motivates ([`04-knobs.md`](04-knobs.md)): a declared cascade policy
(`on_backfill: cascade | warn | forbid`) so the operator chooses eager repair, a loud
diagnostic, or refusal — never silent staleness. Also the G-11 execution bug: the natural
direct-join spelling of this model doesn't compile today.

**Physical maintenance.** One source `txns`, partition = day, self-referential (reads own prior partition). **Partition-local:** ⚠️ `txns` — forward advance is bounded (reads `d−1`), but a late row's cascade over *every* later partition is unbounded ❌ (the CONDITIONAL).
- *New day `D`* (forward) — *top-right* RMW-region reading own prior partition. input clamp: `txns WHERE d = D` + stored `wallet_balance WHERE d = D−1`; output clamp: partition `d = D`.
  ```sql
  INSERT INTO wallet_balance
  SELECT t.account_id, t.d, COALESCE(bal.balance,0) + SUM(t.amount)
  FROM txns t LEFT JOIN wallet_balance bal ON bal.account_id = t.account_id AND bal.d = t.d - INTERVAL '1 day'
  WHERE t.d = D GROUP BY t.account_id, t.d, bal.balance;
  ```
- *Late row at day `p`* — *bottom-right* recompute partition `p` **plus an ordered cascade** `p+1 … n` (each depends on the prior) → ❌ forward footprint unbounded. Nothing schedules the cascade today (G-08); the knob `on_backfill: cascade|warn|forbid` is the fix.

### EX-22 — cumulative signups chart (unbounded lateness, no contract)
- grid: construct=window running total (`SUM() OVER (ORDER BY d)`) | sources=signups: append-only, **unbounded lateness** | output-shape=trajectory (value at every day) | technique(s)=none admissible
- expected: REFUSED(a late row has unbounded forward footprint — every later stored row is stale; no fold repairs a trajectory (§7), recompute-forward is unbounded, and the honest weaker contract (as-of-run) is deferred by decision OQ2 — refuse with a diagnostic naming the two escapes: bound the lateness (EX-23) or change grain to end-state (EX-24))
- probe-status: not-probe-worthy(the refusal is a classification outcome; the *trajectory staleness mechanism* is already witnessed by G-08)
- use-case: growth dashboard "cumulative signups over time" — beloved, and quietly wrong in most naive incremental implementations.

**Physical maintenance.** One source `signups`, unbounded lateness. **Partition-local:** ❌ — a late row's forward footprint is *every* later trajectory row, spanning all partitions. This ❌ **is** the refusal: no fold repairs a trajectory, recompute-forward is unbounded, and the honest weaker contract (as-of-run) is deferred (OQ2). No admissible op; the diagnostic names the two escapes — bound the lateness (EX-23) or change grain to end-state (EX-24).

### EX-23 — the same chart with a 3-day lateness clamp
- grid: as EX-22 but signups declares `source_lateness: '3 days'` | technique(s)=recompute-region over the rolling `[watermark−3d, now]` suffix each run; older trajectory rows settled
- expected: CONDITIONAL(bounded-lateness truncation: rows arriving later than 3 days are excluded by the horizon clamp — the settle bound becomes absolute, and the maintained suffix cost is bounded)
- probe-status: unprobed-candidate
- use-case: same dashboard, once the ingest team commits to a 3-day delivery SLA.

The declared lateness bound converts §7's irreducible case into a bounded rolling
recompute: the forward footprint of any admissible late row is capped at 3 days of
trajectory, so "recompute the last 3 days of the curve every run" is a *complete* plan.
This is the single clearest demonstration that a **source declaration buys a technique** —
the pivotal input to [`05-source-properties.md`](05-source-properties.md).

**Physical maintenance.** One source `signups`, `source_lateness: '3 days'`. **Partition-local:** ✅ **because of the declared bound** — the forward footprint of any admissible late row is capped at 3 days, so the maintained region is the rolling suffix `[wm−3d, now]`. The declaration is exactly what converts EX-22's ❌ into ✅.
- *Each run* — *bottom-right* recompute of the rolling suffix. input clamp: `signups WHERE d >= watermark − INTERVAL '3 days'`; output clamp: partitions `[wm−3d, now]`.
  ```sql
  DELETE FROM cumulative_signups WHERE d >= wm - INTERVAL '3 days';
  INSERT INTO cumulative_signups
  SELECT d, SUM(cnt) OVER (ORDER BY d) FROM signups_agg WHERE d >= wm - INTERVAL '3 days';   -- older trajectory settled
  ```
  Partition-aligned suffix — no amplification.

### EX-24 — customer lifetime spend (end-state grain)
- grid: construct=additive fold per key | sources=payments: append-only, any lateness | output-shape=keyed end-state | technique(s)=fold-delta into key state; lateness is harmless (order-independent additive fold)
- expected: HOLDS
- probe-status: unprobed-candidate (keyed-mode Link-C cells are the loop's next frontier; the abstract arm is P0-5)
- use-case: CRM "lifetime value" field — the EX-22 modeller's escape hatch when the trajectory wasn't actually the product.

```sql
---
refresh: keyed
---
SELECT user_id, SUM(amount) AS lifetime_spend, MAX(paid_at) AS last_payment_at
FROM smelt.sources.payments
GROUP BY user_id
```

The grain change that dissolves the §7 problem: no stored trajectory, no forward footprint.
Late data merely advances `S` — additive folds don't care about order. Paired with EX-22/23
this triple is the design-guidance story for every "running total" request.

**Physical maintenance.** One source `payments`, keyed end-state per `user_id` (no time partition, no trajectory). **Partition-local:** ✅ `payments` — targeted per key; late data just advances `S` (order-independent additive fold).
- *Payment Δ, any lateness* — *top-left* fold into key state. input clamp: the delta; output clamp: the affected `user_id` rows.
  ```sql
  MERGE INTO lifetime_spend m
  USING (SELECT user_id, SUM(amount) AS delta, MAX(paid_at) AS last FROM payments_delta GROUP BY user_id) s
  ON m.user_id = s.user_id
  WHEN MATCHED THEN UPDATE SET lifetime_spend = lifetime_spend + s.delta, last_payment_at = GREATEST(m.last_payment_at, s.last)
  WHEN NOT MATCHED THEN INSERT VALUES (s.user_id, s.delta, s.last);
  ```
  Merge key = grain (`user_id`); minimal amplification when clustered by it.

### EX-25 — day-over-day delta (LAG across the partition boundary)
- grid: construct=window LAG over the previous partition | sources=metrics: append-only, ≤1-day lateness | output-shape=time-partitioned | technique(s)=recompute-region with **derived scan widening `before=1 partition`**; a change to day D also dirties D+1's `dod_change` (footprint `after=1`)
- expected: HOLDS (the reach triple `(before=1d, after=0)` and its footprint reflection `(0, after=1d)` are exactly the paper's §5 dual-map machinery; the write region for a backfill of D must include D+1)
- probe-status: unprobed-candidate (whether smelt's write-window derivation actually widens the *write* to D+1 on a backfill of D is a sharp, falsifiable question — a strong next loop cell)
- use-case: metrics platform — every KPI table wants a day-over-day change column.

```sql
SELECT d, kpi,
       value - LAG(value) OVER (PARTITION BY kpi ORDER BY d) AS dod_change
FROM smelt.sources.metrics
```

**Physical maintenance.** One source `metrics`, partition = day, ≤1d lateness. **Partition-local:** ✅ `metrics` — reach `(before=1d, after=0)` and footprint `(0, after=1d)`: a change to day `D` reads `D−1` and dirties `D` *and* `D+1`. The write region must include `D+1`.
- *New day / backfill of `D`* — *bottom-right* recompute with scan widened `before=1` partition; write widened `after=1`. input clamp: `metrics WHERE d BETWEEN D−1 AND D+1`; output clamp: partitions `{D, D+1}`.
  ```sql
  DELETE FROM kpi_dod WHERE d IN (D, D+1);                            -- footprint after=1 → write must reach D+1
  INSERT INTO kpi_dod
  SELECT d, kpi, value - LAG(value) OVER (PARTITION BY kpi ORDER BY d) AS dod_change
  FROM metrics WHERE d BETWEEN D - INTERVAL '1 day' AND D + INTERVAL '1 day';
  ```
  Partition-aligned (`d`) — no amplification. (Whether smelt's write-window derivation actually widens to `D+1` is the sharp probe.)

### EX-26 — order current-status from a CDC feed (order-monotone overwrite)
- grid: construct=keyed `MAX_BY(status, updated_at)` | sources=order_updates: change feed (updates, no hard deletes), clocked | output-shape=keyed end-state | technique(s)=fold-delta (order-monotone overwrite family: latest-writer-wins, idempotent under re-delivery, frontier ledger)
- expected: HOLDS
- probe-status: unprobed-candidate
- use-case: operational mart — one row per order carrying its current status, fed by the order-service CDC topic.

```sql
---
refresh: keyed
---
SELECT order_id,
       MAX_BY(status, updated_at) AS current_status,
       MAX(updated_at)            AS status_as_of
FROM smelt.sources.order_updates
GROUP BY order_id
```

The keyed column-family catalogue's overwrite pattern, on the source property it was made
for. If the feed carried hard deletes, the row-*existence* question (does the key's row get
retracted?) moves into the skeleton and needs the change-feed delete semantics of
[`05-source-properties.md`](05-source-properties.md) — that variant belongs next to EX-28.

**Physical maintenance.** One source `order_updates` (change feed, updates only), keyed end-state per `order_id`. **Partition-local:** ✅ `order_updates` — targeted per key; latest-writer-wins is idempotent under re-delivery (frontier ledger).
- *Update Δ* — *top-left* fold, order-monotone overwrite. input clamp: the delta; output clamp: the affected `order_id` rows.
  ```sql
  MERGE INTO order_status m
  USING (SELECT order_id, MAX_BY(status, updated_at) AS status, MAX(updated_at) AS as_of FROM order_updates_delta GROUP BY order_id) s
  ON m.order_id = s.order_id
  WHEN MATCHED AND s.as_of > m.status_as_of THEN UPDATE SET current_status = s.status, status_as_of = s.as_of
  WHEN NOT MATCHED THEN INSERT VALUES (s.order_id, s.status, s.as_of);
  ```
  Merge key = grain (`order_id`) — minimal amplification when clustered by it.

---

## Family E — Dedup & SCD

Row-identity constructs: the skeleton *is* the product. Dedup collapses redelivered
physical rows to one logical row; SCD2 turns change history into validity intervals. Both
are acutely sensitive to what the upstream actually guarantees.

### EX-27 — event dedupe under bounded redelivery
- grid: construct=dedup-to-latest (keyed collapse; `MIN(ts)` + `MAX_BY(payload, ts)`) | sources=raw_events: append-only, at-least-once, `key_recurrence: {key: [event_id], window: '3 days'}` | output-shape=keyed, time-partitioned via key temporal locality | technique(s)=windowed keyed merge; the recurrence bound licenses pruning the merge scan to the locality slice
- expected: HOLDS (runtime-checked: a recurrence violation fails the run transactionally, never drops data)
- probe-status: unprobed-candidate (spec'd end-to-end in `keyed_models.md`; the locality gate is not yet built, so today this refuses)
- use-case: Kafka-sourced event ingestion with at-least-once delivery — the dedup layer everything else in the warehouse sits on.

```sql
---
refresh: keyed
timeseries: { event_time_column: first_seen_at, partition_column: first_seen_date, granularity: day }
---
SELECT event_id,
       MIN(event_ts)             AS first_seen_at,
       MIN(event_date)           AS first_seen_date,
       MAX_BY(payload, event_ts) AS payload
FROM smelt.sources.raw_events
GROUP BY event_id
```

The declared-and-checked pattern at its best: `key_recurrence` *narrows* a scan, so it is
never trusted — every consuming run verifies it transactionally. Upstream of EX-13/EX-20
this is what makes their append-only assumption true.

**Physical maintenance.** One source `raw_events` (at-least-once, `key_recurrence: {event_id, 3d}`), keyed collapse, partitioned by `first_seen_date` via key temporal locality. **Partition-local:** ✅ **under the recurrence locality gate** — the 3-day window prunes the merge scan to the locality slice (never trusted: a violation fails the run transactionally). Today refuses (gate unbuilt).
- *Event Δ* — *bottom-left* keyed windowed merge, pruned to the locality slice. input clamp: `raw_events` in the delta + stored rows `first_seen_date >= delta_date − INTERVAL '3 days'`; output clamp: those keys/partitions.
  ```sql
  MERGE INTO deduped m
  USING (SELECT event_id, MIN(event_ts) fs, MIN(event_date) fd, MAX_BY(payload, event_ts) pl FROM raw_events_delta GROUP BY event_id) s
  ON m.event_id = s.event_id AND m.first_seen_date >= s.fd - INTERVAL '3 days'   -- locality slice prunes the scan
  WHEN MATCHED THEN UPDATE SET first_seen_at = LEAST(m.first_seen_at, s.fs), payload = CASE WHEN s.fs < m.first_seen_at THEN m.payload ELSE s.pl END
  WHEN NOT MATCHED THEN INSERT VALUES (s.event_id, s.fs, s.fd, s.pl);
  ```
  The recurrence bound *narrows* the scan; it is verified, never trusted.

### EX-28 — customer dimension as SCD2 from CDC
- grid: construct=versioned intervals (close-old/open-new) | sources=customers_cdc: **change feed with before-images**, replayable, clocked by `updated_at` | output-shape=versioned (key × validity interval) | technique(s)=fold-delta (interval close-out combiner) — admissible because the input is a replayable change sequence, so `recompute ≡ fold` holds at the skeleton
- expected: UNSUPPORTED-TODAY(`refresh: versioned` is specified but does not parse — `versioned_models.md` §Known Divergences)
- probe-status: unprobed-candidate (once it parses, this is a priority Link-C family)
- use-case: the classic SCD2 customer dimension — "what was this customer's segment when the order shipped?" — fed by Debezium.

```sql
---
refresh: versioned
---
SELECT customer_id, segment, region, updated_at
FROM smelt.sources.customers_cdc
```

**Physical maintenance.** One source `customers_cdc` (change feed with before-images, replayable), versioned intervals per `customer_id`. **Partition-local:** ✅ `customers_cdc` — a replayable change sequence, so `recompute ≡ fold` at the skeleton; targeted per key. **UNSUPPORTED-TODAY** (`refresh: versioned` does not parse).
- *CDC delta* — *top-left* fold, interval close-out (close old version, open new). input clamp: the delta; output clamp: the affected `customer_id`'s open interval.
  ```sql
  MERGE INTO customer_scd2 m
  USING (SELECT customer_id, segment, region, updated_at FROM customers_cdc_delta) s
  ON m.customer_id = s.customer_id AND m.valid_to IS NULL
  WHEN MATCHED THEN UPDATE SET valid_to = s.updated_at;              -- close current version
  -- then INSERT the new open interval (valid_from = s.updated_at, valid_to = NULL)
  ```
  Merge key = `customer_id` — minimal amplification when clustered by it.

### EX-29 — SCD2 from nightly snapshots
- grid: as EX-28 but sources=customers_snap: **mutable snapshot**, observed nightly | technique(s)=snapshot-diff → interval rows
- expected: REFUSED(the interval *row set* is a function of the observation sequence, not of any replayable input — two observers with different snapshot cadences derive different skeletons, so `recompute ≡ fold` fails at the skeleton level; this inhabitant belongs to the deferred as-of-run contract (OQ2) and is refused until that contract exists, with the diagnostic naming EX-28's change-feed upgrade as the exact escape)
- probe-status: not-probe-worthy(classification refusal; no execution to falsify until OQ2 lands)
- use-case: the same dimension when the source team can only offer a nightly full dump — dbt-snapshot territory, and precisely the unlabeled looseness §6 exists to name.

**Physical maintenance.** One source `customers_snap` (mutable snapshot, observed nightly). **Partition-local:** n/a — REFUSED at the *skeleton*: the interval row set is a function of the observation sequence, not any replayable input, so `recompute ≡ fold` fails before any op is chosen. No admissible maintenance until the deferred as-of-run contract (OQ2) lands; the diagnostic names EX-28's change-feed upgrade as the escape.

### EX-30 — auto-generated surrogate key
- grid: construct=any | sources=any | output-shape=keyed on `uuid()` surrogate | technique(s)=n/a
- expected: REFUSED(OQ1 resolution: a non-deterministic column is barred from every skeleton position; a `unique_key` derived from `uuid()` is rejected outright — the diagnostic offers the stable escape: a hash of skeleton columns, EX-33)
- probe-status: not-probe-worthy(compile-time configuration error, already partially enforced by `batched_models.md`'s taint exclusions)
- use-case: the dbt habit of `dbt_utils.generate_surrogate_key(...)` done with `uuid()` instead of a content hash — works until the first backfill, then every downstream join silently churns.

**Physical maintenance.** n/a — REFUSED at compile time: a non-deterministic column (`uuid()`) is barred from every skeleton position, so no maintenance op is ever emitted. **Partition-local:** n/a (no op). The diagnostic offers the stable escape: a hash of skeleton columns (EX-33).

### EX-31 — audit stamp consumed downstream as a grouping key
- grid: construct=cross-model payload leak (two models) | M: emits `inserted_at = NOW()` declared in `nondeterministic_columns`; N: `GROUP BY date_trunc('day', inserted_at)` | output-shape=N is time-partitioned **on a payload column** | technique(s)=n/a
- expected: REFUSED(at the **consumer**: payload-ness propagates across the DAG; a payload column reaching a skeleton position in N fails loud, retro-tightening M's contract or forcing N onto the event-time column — §6's DAG-propagation rule, deliberately beyond today's intra-model taint)
- probe-status: unprobed-candidate (a two-model Link-C cell would pin today's actual behaviour — likely a silent acceptance, i.e. the gap the rule closes)
- use-case: "daily load report" built on the audit timestamp instead of event time — plausible-looking, backfill-hostile, and the canonical cross-model leak.

**Physical maintenance.** Two models: `M` (emits `inserted_at = NOW()`), `N` (partitions on `date_trunc('day', inserted_at)`). **Partition-local:** ❌ at `N` — partitioning on a *payload* column means a backfill of `M` can re-stamp rows into *any* `N` partition, so `N`'s footprint is unbounded (payload-partitioning destroys locality) — on top of the payload-in-skeleton refusal. REFUSED at the consumer: payload-ness propagates across the DAG (§6), retro-tightening `M`'s contract or forcing `N` onto the event-time column. No admissible op.

---

## Family F — Composition & delegation

Whole-DAG and whole-engine concerns: delegating maintenance to the engine, keeping
identities stable across models, and propagating per-column contracts downstream.

### EX-32 — near-real-time ops dashboard via engine IVM
- grid: construct=join + aggregation, arbitrary | sources=any the engine supports | output-shape=engine-defined | technique(s)=engine-maintained (delegate to native IVM; smelt runs no combiner, keeps no ledger — freshness owner is the engine)
- expected: HOLDS (with the distinct freshness contract: continuously maintained, no smelt run window; no silent fallback if the engine can't maintain the SQL)
- probe-status: not-probe-worthy(delegation correctness is the engine's; smelt's obligation — no-silent-fallback — is a compile-time check)
- use-case: Databricks-backed live ops dashboard where sub-minute freshness matters more than smelt-owned cost control.

```sql
---
refresh: materialized_view
---
SELECT region, COUNT(*) AS open_orders, SUM(amount) AS open_value
FROM smelt.orders_current WHERE status = 'open' GROUP BY region
```

The third arm of the trichotomy, present so the catalogue shows the ladder's top: when the
engine can maintain it natively, the whole plan matrix collapses to one delegated cell.

**Physical maintenance.** n/a — delegated to the engine's native IVM: smelt emits no `DELETE`/`INSERT`/`MERGE`, keeps no ledger, owns no clamp. **Partition-local:** n/a (the engine owns freshness and physical layout). smelt's only obligation is compile-time: **no silent fallback** if the engine cannot maintain the SQL.

### EX-33 — stable hash surrogate feeding downstream joins
- grid: construct=deterministic surrogate (`hash(user_id, event_ts, page)`) used as `unique_key` and joined downstream | sources=events: append-only | output-shape=time-partitioned, keyed | technique(s)=any — the surrogate is skeleton-derived, so every technique addresses the same rows across runs
- expected: HOLDS (the sanctioned alternative EX-30's refusal points at: identity as a pure function of skeleton columns is stable under recompute, fold, and backfill alike)
- probe-status: unprobed-candidate (a cheap Link-C confirmation: backfill a window, assert surrogate stability)
- use-case: conformed event key shared by a dozen downstream marts — the thing EX-30 was trying to build.

**Physical maintenance.** One source `events`, partition `event_date`; surrogate = `hash(user_id, event_ts, page)` (deterministic, skeleton-derived). **Partition-local:** ✅ `events` — identity is a pure function of skeleton columns, so it is stable under recompute, fold, and backfill alike; every technique addresses the same rows across runs. Ops follow the EX-02 shape (region-append + backfill recompute), keyed on the stable surrogate. No amplification (partition-aligned).

### EX-34 — medallion chain: settledness propagation
- grid: construct=three-model chain (EX-02 bronze → EX-01 silver → gold daily conversion-rate rollup) | sources=as upstream | output-shape=time-partitioned at each layer | technique(s)=per-model as above; the *new* content is the cross-model contract
- expected: CONDITIONAL(gold's `conversion_rate` inherits silver's `converted` settle bound — watermark-relative, ≥7d — and gold aggregates it, so gold's partition for day D is *plausible on read, settled only once the conversions watermark passes D+7d*; the per-column ledger must flow downstream or gold's consumers can't know)
- probe-status: unprobed-candidate (the loop currently probes single models; a chain cell is the natural extension)
- use-case: every medallion pipeline whose gold layer feeds a finance report — "when is Tuesday final?" is *the* operational question, and today no tool answers it per column.

```sql
-- gold/daily_conversion_rate.sql
---
refresh: batched
timeseries: { event_time_column: event_date, partition_column: event_date, granularity: day }
---
SELECT event_date,
       COUNT(*) FILTER (WHERE converted) * 1.0 / COUNT(*) AS conversion_rate
FROM smelt.silver_event_conversions
GROUP BY event_date
```

The catalogue's closing argument: per-column contracts (§6's two-dimensional ledger) are
only worth their bookkeeping if they **compose down the DAG** — settle bounds propagate
through aggregation, payload-ness fails loud in skeleton positions (EX-31), and freshness
becomes a queryable property of a column, not folklore about a pipeline.

**Physical maintenance.** Three models chained, each partition `event_date`. **Partition-local:** ✅ at every layer — bronze ✅ `events` (EX-02); silver ✅ `bronze`, ✅ `conversions` (EX-01); gold ✅ `silver` (aggregates a bounded `event_date` span). The *new* content is the cross-model trigger — a settled-partition signal, not a scan.
- *New bronze day `D`* (`events` Δ) → bronze region-append → silver region-append for `D` → gold recompute partition `event_date = D`. Each op is bounded to `D`.
- *Late conversion* (`conversions` Δ) → silver `MERGE` of `converted` (EX-01, ~8 partitions) → gold recompute of those `event_date` partitions.
  ```sql
  DELETE FROM daily_conversion_rate WHERE event_date IN (affected);   -- gold re-aggregates the touched partitions
  INSERT INTO daily_conversion_rate
  SELECT event_date, COUNT(*) FILTER (WHERE converted) * 1.0 / COUNT(*) FROM silver_event_conversions WHERE event_date IN (affected) GROUP BY event_date;
  ```
  Gold's `conversion_rate` inherits silver's watermark-relative (≥7d) settle bound; the per-column ledger must flow downstream or gold's consumers can't know when a day is final.

---

## Family G — Schema evolution (column addition)

Every family above is indexed by an **input-delta** trigger — a *source* changed. This
family's trigger is different in kind: the **model definition** changed, gaining one or
more output fields, while the sources stand still. It is the **definition-change trigger**
([`01-framework.md`](01-framework.md) §5, the third trigger beside *creation* and
*mutation*): a newly-added column-group's processed-input vector `S` starts **empty** over
every existing region, and the backfill advances it from `∅` to current (§8 ledger — the
new group's entries are instantiated at `S = ∅`; skeleton and sibling entries are
untouched).

The admissible plan is the 2×2's **left column** — a *targeted* write that touches **only
the new field(s)**, leaving the skeleton and every sibling column in place. It splits by
what the field must read:

- **top-left** (delta+state, empty input delta) — the field is a **pure function of
  already-stored columns**; no upstream read, just an in-place `UPDATE` of the stored
  region (EX-36). The cheapest tier.
- **bottom-left** (full-input) — the field must **re-derive from upstream** (an enrichment
  join, a subquery); a column-scoped `MERGE`, **keyed where the source is keyed** (EX-37).

Fields added together factor by **shared mutation-sensitivity** exactly as the base plan
does: co-sensitive fields share one op (EX-37's `{unit_price, margin}`), cross-group fields
get one op each (EX-38). A *foldable* added field's **backfill** is still full-input
(`∅ → current`) — there is no prior state of that column to fold onto; its ongoing fold is a
separate, later concern. The boundary: a field added to a **skeleton** position
(grouping / dedup / identity) is a **grain change**, not a column backfill (EX-39) — this
family applies to **payload** columns only (§6).

### EX-36 — add a derived pass-through field to the clickstream landing
- grid: construct=schema evolution (add one derived field, pure function of a stored column) | sources=events: append-only, clocked (**unchanged**) | output-shape=time-partitioned (day) | technique(s)=`{referrer_domain}` × column-added → in-place per-partition `UPDATE` from stored `referrer` (no upstream read)
- expected: UNSUPPORTED-TODAY(the column-scoped in-place backfill — no path detects that a model gained a field and populates only it; today the field is `NULL` on already-processed partitions until an explicit whole-partition recompute, which HOLDS but re-reads upstream and rewrites *every* column)
- probe-status: unprobed-candidate
- use-case: an analyst adds `referrer_domain` to the bronze clickstream (EX-02) months in; old partitions must gain the field without re-landing the raw events.

```sql
---
refresh: batched
timeseries: { event_time_column: event_ts, partition_column: event_date, granularity: day }
---
SELECT event_id, user_id, event_date, event_ts, page, referrer,
       regexp_extract(referrer, '://([^/]+)', 1) AS referrer_domain   -- newly added
FROM smelt.sources.events
```

The new field's mutation-sensitivity is `{}` — it depends only on the row's own stored
`referrer`, so the backfill needs **no upstream read at all**. This is the top-left corner
with an empty input delta: the "state" the read touches is the *stored output region*
itself (§3's read-scope definition). No key (a per-row function of own columns), skeleton
untouched (no identity change).

**Physical maintenance.** One source `events`, partition `event_date`; but the backfill
reads none of it. **Partition-local:** ✅ trivially (reads only stored output).
- *Column added* — *top-left* in-place `UPDATE` per stored partition, no upstream read.
  ```sql
  -- loop over existing partitions P:
  UPDATE clickstream SET referrer_domain = regexp_extract(referrer, '://([^/]+)', 1)
  WHERE event_date = P;                       -- reads only stored rows; skeleton + siblings untouched
  ```
- *Today's only path* — the whole-partition recompute (EX-02 backfill, *bottom-right*): re-reads `events`, rewrites every column. Correct but pays for the sibling columns it did not change.

### EX-37 — add re-pricing fields to existing order lines (keyed, per-partition)
- grid: construct=schema evolution (add a **co-sensitive field group** `{unit_price, margin}` from a keyed dimension) | sources=order_lines: append-only, clocked (**unchanged**); products: change feed, `unique_key: [product_id]` (**unchanged**) | output-shape=time-partitioned + column-scoped | technique(s)=`{unit_price, margin}` × column-added → *bottom-left* keyed column-scoped `MERGE` per partition
- expected: UNSUPPORTED-TODAY(the column-scoped backfill emitter — the same dormant `dimension_horizon_merge` as EX-08; adding the fields and running today either leaves them `NULL` on old partitions or forces a whole-history recompute)
- probe-status: unprobed-candidate
- use-case: EX-08's retailer adds `unit_price` and `margin` to an order-line table that previously stored only order fields; existing lines must gain **both** fields, keyed by product, per partition — not a full rebuild.

```sql
---
refresh: batched
timeseries: { event_time_column: order_ts, partition_column: order_date, granularity: day }
---
SELECT ol.order_id, ol.qty, ol.order_date, ol.product_id,
       p.unit_price, p.margin                 -- both newly added, sensitivity {products}
FROM smelt.sources.order_lines ol
JOIN smelt.sources.products p ON p.product_id = ol.product_id
```

The two added fields **share** mutation-sensitivity `{products}`, so they are **one column
group** and **one `MERGE`** populates both — the "one or more fields" case at its cleanest.
The field must read the dimension, so this is bottom-left (full-input), keyed on
`product_id` because the source is keyed.

**Physical maintenance.** Two sources: `order_lines` (partition `order_date`), `products`
(change feed, unclocked). **Partition-local:** ✅ `order_lines` — the backfill walks
partitions, scanning the join in bounds; `products` read per partition. The whole backfill
stays partition-confined, unlike a whole-table rebuild.
- *Fields `{unit_price, margin}` added* — *bottom-left* keyed column-scoped `MERGE`, per partition. input clamp: `order_lines WHERE order_date = P` ⋈ `products`; output clamp: `{unit_price, margin}` on partition `P`.
  ```sql
  -- loop over existing partitions P (or bounded by a declared backfill horizon):
  MERGE INTO order_line_margin m
  USING (SELECT ol.order_id, ol.order_date, p.unit_price, p.margin
         FROM order_lines ol JOIN products p ON p.product_id = ol.product_id
         WHERE ol.order_date = P) s
  ON m.order_id = s.order_id AND m.order_date = s.order_date        -- partition-pruned target
  WHEN MATCHED THEN UPDATE SET unit_price = s.unit_price, margin = s.margin;   -- one MERGE, both fields
  ```
  *Amplification (secondary):* `product_id` ⟂ `order_date` → within-partition scatter; deletion vectors / `OPTIMIZE` absorb it. The *partition* bound is what the plan guarantees.
- *Variant — no-locality dimension.* If the added field were enriched from an **unclocked mutable dim with no horizon** (EX-07's `customers`), its backfill footprint is every past row of that key across all partitions → ❌ scatters; the K8 guardrail refuses, exactly as EX-07. **A field-add backfill inherits its source's partition-locality verdict unchanged.**
- *Variant — foldable added field.* If instead `converted` (EX-01) were added, its *backfill* is still this full-input bottom-left shape (compute fresh over existing partitions — no prior `converted` state to fold); only its *ongoing* maintenance uses EX-01's fold cell.

### EX-38 — add two fields that span different groups (the backfill factors)
- grid: construct=schema evolution (add fields with **different** mutation-sensitivity) | sources=orders: append-only (**unchanged**); customers: mutable lookup (**unchanged**) | output-shape=time-partitioned | technique(s)=`{order_month}` (`{}`-sensitivity, in-place) + `{tier}` (`{customers}`, keyed `MERGE`) × column-added → **two distinct left-column ops**
- expected: UNSUPPORTED-TODAY(both column-scoped backfills; the point is the *factoring* — the definition-change trigger partitions by mutation-sensitivity exactly as an input-delta trigger does)
- probe-status: not-probe-worthy(the factoring is a classification fact, like EX-12; the *classifier producing distinct backfill ops per group* is the Link-B probe)
- use-case: in one edit an analyst adds both `order_month` (a pure date bucket of the stored `order_date`) and `tier` (joined from the customer dim) to the EX-07 orders table.

```sql
SELECT o.order_date, o.order_id, o.user_id, o.amount, c.tier,   -- tier: sensitivity {customers}
       date_trunc('month', o.order_date) AS order_month         -- order_month: sensitivity {}
FROM smelt.sources.orders o
JOIN smelt.sources.customers c ON c.user_id = o.user_id
```

Two added fields, two groups, two corners — the definition-change trigger factors by group
just like every other trigger.

**Physical maintenance.** **Partition-local:** `{order_month}` ✅; `{tier}` ❌ over unclocked `customers`.
- *`{order_month}` added* — sensitivity `{}` → *top-left* in-place `UPDATE`, no upstream read, no key.
  ```sql
  UPDATE orders_tiered SET order_month = date_trunc('month', order_date) WHERE order_date = P;
  ```
- *`{tier}` added* — sensitivity `{customers}` → *bottom-left* keyed `MERGE`; partition-local **only** if the dim is clocked/horizoned. Over EX-07's unclocked `customers` it is ❌ (full-table scatter), and the field-add inherits EX-07's refusal:
  ```sql
  MERGE INTO orders_tiered m USING (SELECT user_id, tier FROM customers) s
  ON m.user_id = s.user_id        -- ❌ no partition predicate (EX-07): full-table scatter unless customers is clocked
  WHEN MATCHED THEN UPDATE SET tier = s.tier;
  ```
`order_month` backfills in-place immediately; `tier` needs the EX-08 upgrade (clocked
change feed + horizon) before its backfill is partition-local. One edit, two independent
verdicts — the value is the diagnostic that separates them.

### EX-39 — the boundary: a skeleton-position field is a grain change, not a backfill
- grid: construct=schema evolution into a **skeleton** position (new `GROUP BY` / dedup key) | sources=payments: append-only (**unchanged**) | output-shape=**changes** (`pay_date` → `(pay_date, region)`) | technique(s)=none column-scoped — recompute-region
- expected: REFUSED-as-column-backfill(a field added to a membership / grouping / dedup / ordering position changes *which rows exist* — §6 skeleton — so it is a **grain change** (§10), not a payload field-add; no targeted column write is admissible, and the framework must **not** silently `UPDATE` it in place. The honest plan is a whole-region recompute — effectively a new model; the diagnostic names the grain change)
- probe-status: not-probe-worthy(compile-time classification: skeleton-role extraction decides it — the §2 machinery of [`09-spec-readiness.md`](09-spec-readiness.md))
- use-case: an analyst adds `region` to the `GROUP BY` of EX-13's daily-revenue rollup, expecting a cheap field-add — but this re-partitions every group; the row set itself changes.

```sql
-- was: SELECT pay_date, SUM(amount) AS revenue FROM … GROUP BY pay_date
SELECT pay_date, region, SUM(amount) AS revenue
FROM smelt.sources.payments
GROUP BY pay_date, region          -- `region` enters the skeleton (grouping key)
```

**Physical maintenance.** n/a **as a column backfill.** The added `region` is a skeleton
column (grouping key), so the output's row identity moves from `pay_date` to
`(pay_date, region)`: the stored rows are now the *wrong rows*, and no `UPDATE`/`MERGE` can
patch that in place. **Partition-local:** ✅ per partition *for the recompute* — but this is
a **recompute** (bottom-right), not a targeted field write.
- *Skeleton field added* — REFUSED as a column backfill; the admissible plan is recompute-region.
  ```sql
  DELETE FROM daily_revenue WHERE pay_date = P;
  INSERT INTO daily_revenue SELECT pay_date, region, SUM(amount) FROM payments WHERE pay_date = P GROUP BY pay_date, region;
  ```
The diagnostic names it a **grain change** (§10 anchor): adding a skeleton column is a
different model, not a payload field-add. This boundary is what keeps "single-field
backfill" honest — the whole family applies to *payload* columns (§6) and never to skeleton
positions.

---

## Candidate probe cells (lift-ready)

Unprobed-candidate entries above, in the loop catalog's row format
(`docs/research/property-discovery/catalog.md`):

| id | construct | source property | technique | layer | hypothesis (expected verdict) |
|---|---|---|---|---|---|
| EX-03 | pass-through, late rows | append-only + declared 48h lateness | read-modify-write region (append into stored partition, no upstream re-read) | linkC | HOLDS; bake-off vs recompute-region |
| EX-06 | UNION ALL, one mutable arm | live append-only + mutable history arm | recompute-region | linkC | CONDITIONAL(backfill-recovers, history arm only) |
| EX-11 | correlated scalar COUNT (30d window) | append-only, late | fold (+1 MERGE) with per-delta ledger | linkC | UNSUPPORTED-TODAY; recompute arm HOLDS (SC-1 analogue with additive combiner) |
| EX-14 | additive SUM over CDC with retractions | change_feed (delete images) | invertible fold-delta + ledger | linkC | UNSUPPORTED-TODAY; probe what `change_feed` does today (likely re-scan or refusal) |
| EX-18 | GROUP BY week over day partitions | append-only, late day mid-week | write-window coarsening to week boundary | linkC | HOLDS iff write window rounds up; sharp footprint-map probe |
| EX-19 | global scalar SUM (empty key) | append-only | fold into single-row keyed state | linkB/C | UNSUPPORTED-TODAY (surface refuses empty key); confirm today's behavior is full-only |
| EX-23 | running total + declared 3d lateness | append-only, bounded late | rolling-suffix recompute `[wm−3d, now]` | linkC | CONDITIONAL(bounded-lateness truncation) |
| EX-24 | keyed additive fold | append-only, unbounded late | fold-delta end-state | linkC | HOLDS (keyed-mode Link-C frontier) |
| EX-25 | LAG across partition boundary | append-only, ≤1d late | backfill of D must also rewrite D+1 | linkC | HOLDS iff write region includes footprint `after=1d`; REFUTED = real bug |
| EX-26 | MAX_BY latest-status | change_feed (updates only) | order-monotone overwrite fold | linkC | HOLDS |
| EX-27 | keyed collapse dedupe | at-least-once + key_recurrence 3d | locality-pruned windowed merge | linkC | HOLDS once locality gate lands; today refuses |
| EX-28 | SCD2 close-old/open-new | change_feed, replayable | interval fold | linkC | UNSUPPORTED-TODAY (`versioned` doesn't parse); confirm |
| EX-31 | cross-model payload→skeleton leak | append-only + NOW() payload | (classification) consumer-side fail-loud | linkB | probe today's behaviour — hypothesis: silently accepted (the gap) |
| EX-33 | hash-of-skeleton surrogate | append-only | surrogate stability under backfill | linkC | HOLDS |
| EX-34 | three-model chain settledness | append-only + unbounded-late conversions | cross-model watermark propagation | linkC | CONDITIONAL; no propagation surface today — probe what downstream sees |
| EX-35 | correlated MIN_BY first-value (7d window) | append-only, late | order-sensitive fold (stored-winner ledger) | linkC | UNSUPPORTED-TODAY; recompute arm HOLDS (EX-01 analogue, order-sensitive combiner) |
| EX-36 | add derived pass-through field (fn of stored column) | definition-change trigger (sources unchanged) | in-place per-partition UPDATE, no upstream read (top-left) | linkC | UNSUPPORTED-TODAY (column-scoped backfill); whole-partition recompute arm HOLDS |
| EX-37 | add co-sensitive field group from keyed dim | definition-change trigger + change_feed dim | keyed column-scoped MERGE per partition (bottom-left) | linkC | UNSUPPORTED-TODAY (same emitter as EX-08); confirm today leaves NULL or forces full rebuild |

---

**Summary**: 39 examples. Verdict distribution: **13 HOLDS** (EX-02, 05, 09, 13, 15, 17,
24, 25, 26, 27†, 32, 33, plus EX-01's probed recompute arm), **9 CONDITIONAL** (EX-03, 04,
06, 07, 12, 20, 21, 23, 34), **6 REFUSED** (EX-16, 22, 29, 30, 31, plus EX-39's
REFUSED-as-column-backfill grain-change boundary), **11 UNSUPPORTED-TODAY** (EX-01 fold
cell, 08, 10, 11, 14, 19, 28, 35, plus Family G's 36, 37, 38). († EX-27 HOLDS under the
spec'd-but-unbuilt locality gate.) Every construct row and source-property column of the
coverage matrix is inhabited by ≥2 examples except the deliberately-degenerate
multi-input-merge row (EX-12, definitional), engine-maintained (EX-32, delegation), and
correlated first-value pick (EX-35, added for the three-way ledger-grade contrast with
EX-01/EX-11). **Family G** (EX-36–39) adds the trigger-orthogonal *definition-change*
(column-addition) trigger: the single-field backfill as the 2×2's left column
([`01-framework.md`](01-framework.md) §5).

Every example now carries a `**Physical maintenance**` block (see §"Physical-maintenance
notation") giving the emitted `DELETE`+`INSERT` / `MERGE` per trigger, its input/output
clamps, and a per-source **partition-local** verdict — the physical realizability of the
plan, cross-referenced to [`01-framework.md`](01-framework.md) §5.
