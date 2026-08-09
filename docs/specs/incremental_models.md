---
feature: incremental_models
status: experimental
last_reviewed: 2026-08-09
owners: [andrew]
---

# Incremental Models

> **What this is.** The normative spec for **maintained models** — everything declared `refresh: incremental` — covering the shared maintenance contract, the derived per-model **maintenance plan**, the dependency-**graph layer** built on it, and the declared shapes (partition grain, key grain). Out of scope, with their own homes: the provable properties of a model's SQL (`model_properties.md`); the physical transform mechanisms (`model_transforms.md`); the `refresh:` axis and declaration law (`models.md`); source world-facts (`sources.md`); the `timeseries:` declaration grammar (`timeseries.md`); engine-maintained views (`materialized_view.md`); backend capability flags (`multi_backend.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Overview

*This section is a non-normative primer: it introduces every concept the spec depends on, in dependency order, and names where each is specified. The normative statements live in §Surface, §Semantics, and §Constraints & Invariants — on any conflict, those win.*

### The one guarantee

An incremental model is an ordinary SQL model whose stored table smelt keeps current without re-running the SQL from scratch. The entire feature rests on one promise, the **equivalence invariant**:

> After any sequence of incremental runs, the stored table equals what a full refresh of the model's SQL would produce over the inputs those runs have processed so far.

Formally, writing `S` for the processed-input set:

```
incremental_state(S) == full_refresh(source | input ∈ S)
```

Everything else in this spec serves that equation. Properties of the model's SQL are proven so that a maintenance shortcut is *known* to preserve it; a shortcut that cannot be proven safe is **refused with a diagnostic** — never applied approximately, and never silently swapped for something slower but safer (§"Validator, not chooser"). Two consequences worth internalising before reading on:

- **Order doesn't matter.** The right-hand side depends only on the *set* `S`, so any two run histories that process the same inputs converge to the same table (§"The equivalence invariant").
- **Freshness is the only degree of freedom.** Anything smelt chooses — which technique runs, in what order — may change *when* the table reflects an input, never *what* the table says once it has (§"Per-cell admission").

### What you declare — two facts

A modeller declares `refresh: incremental` plus at most **two shape-defining facts** about the output, and nothing else:

- a **clock** (`timeseries:`) — the output has a time axis (`event_time_column`, `partition_column`, `granularity`) consumers can window over;
- an **identity** (`unique_key:`) — the output is addressable by key, one row per key.

Everything beyond those facts — which maintenance technique runs where, how writes locate stored rows, what each run scans, what bookkeeping exists — is **derived** from the model's SQL and the declared facts, and printed by `smelt explain`. The machinery **validates** the declaration and refuses when the SQL cannot uphold it; it never chooses a different shape for you.

### The four corners

The two facts vary independently, giving four inhabitable shapes. The friendly name for each corner is its **grain** — a derived label, writable in frontmatter only as a checked assertion:

| | **declares a clock** | **no clock** |
|---|---|---|
| **no identity** | complete time-partitioned table — derived `grain: partition` | — (no maintainable shape) |
| **declares identity** | time-partitioned keyed table — derived `grain: key` (partition ∉ key) or `grain: key_per_partition` (partition ∈ key) | keyed lookup, read in full — derived `grain: key` |

Three-line sketches, one per inhabited corner (the running example below fills them in):

```sql
-- clock, no identity → grain: partition: rewrite touched partitions
--- refresh: incremental
--- timeseries: { event_time_column: order_date, partition_column: order_date, granularity: day }
SELECT order_date, SUM(amount) AS revenue FROM smelt.orders GROUP BY order_date

-- identity, no clock → grain: key: fold deltas into keyed state
--- refresh: incremental
--- unique_key: [order_id]
SELECT order_id, SUM(item_count) AS total_items, MAX(shipped_ts) AS shipped_at
FROM smelt.order_events GROUP BY order_id

-- clock + identity → grain: key, time-partitioned: fold by key, pruned to a time slice
--- refresh: incremental
--- unique_key: [event_id]
--- timeseries: { event_time_column: first_seen_at, partition_column: first_seen_date, granularity: day }
SELECT event_id, MIN(event_ts) AS first_seen_at, CAST(MIN(event_ts) AS DATE) AS first_seen_date,
       MAX_BY(payload, event_ts) AS payload
FROM smelt.raw_events GROUP BY event_id
```

The corners are not modes to choose between — they are what the declared facts already imply, and they **compose**. Declaring `unique_key` on a clocked table doesn't bolt "dedup" onto it; it makes the output key-addressable, which unlocks keyed maintenance for dimension corrections while new-data arrival still rewrites partitions. Several capabilities exist *only* in the composed corner (§"What the composed shape enables").

### How smelt maintains it — the plan

For every maintained model smelt derives a **maintenance plan**: a set of **cells**, one per combination of

- an **output column group** — columns that change together (a new row's columns are all computed at once; what *separates* groups is which upstream changes each is sensitive to),
- a **trigger** — what kind of thing happened: new rows arrived (**creation**), an already-processed row changed upstream (**mutation**), the model definition gained columns (**definition change**), or an explicit region recompute was requested (**backfill**),
- a **changed input** — *which* source or upstream model the trigger fired for.

Each cell records the **technique** that repairs it (rewrite a partition range; fold a delta into keyed state; merge a single column; …), the **write addressing** — whether the write locates stored rows by *region* or by *key* — and the **scan clamps**: the bounded window of each input the cell reads. Different cells of one model routinely derive different answers; that is the point. One model is simultaneously append-driven, merge-driven, and recompute-driven at different cells, so no single per-model "strategy" label could describe it. `smelt explain <model>` prints the plan; none of it is declared.

The **graph layer** lifts the plan to the DAG: given what landed upstream, which cells of which downstream models must run over which regions (**forward propagation**) — and given a requested output period, which upstream slices must exist first (**backward resolution**).

### Why cells differ — the three costs

The equivalence invariant fixes what the table must equal; it says nothing about how much work a run does to get there. The plan exists because many physically different repairs reach the same state, and they differ **only in cost**. That cost decomposes into three dimensions — read, compute, write. They correlate but do not track each other, and each has its own governing machinery.

**Read cost — how much input the run must scan.** Two questions, each with a cheapest-to-dearest ladder:

- *How is the delta discovered?* A source-provided **change feed** hands the delta over directly — the smallest possible read. A **clock** allows window-forward discovery: only the new window plus a derived lookback is scanned (the **scan clamps**, §"Windowed maintenance and the horizon"). **No clock** leaves only a snapshot diff — a full read, surfaced and guarded (`scan_bounds`), never silent.
- *How much input does the repair itself need?* A **fold** consumes delta + stored state — the smallest read, paid for with combiner-algebra obligations (§"The algebraic maintenance ladder"). A **recompute** re-reads the region's full input — a larger read that needs no algebra at all. Neither dominates: a small delta into cheap state favours the fold; a delta touching most of a region can make the recompute cheaper. That is why proven-interchangeable techniques are cost-modelled and measurable (`smelt bakeoff`), not fixed by shape.

**Compute cost — the engine work between read and write.** Scanned volume is only a proxy for what the repair costs to *evaluate*: the joins, aggregations, and sorts between scan and write can dominate, and on a distributed engine the **shuffle** they induce is often the dominant term. Compute correlates with read but does not track it — a bounded scan feeding a global aggregation or a wide join can shuffle far more than it reads. smelt's posture is two-fold. It does not hand-compute minimal deltas: the engine evaluates the model's SQL, joins included, over a widened scan, keeping join optimisation where the optimiser lives (§"Windowed maintenance and the horizon"). What smelt does control is the **unit of work**: a repair scoped region by region caps the working set and the shuffle of each statement, which is why a keyed model with proven temporal locality may still be maintained partition by partition even though one whole-table keyed `MERGE` would be equivalent — the locality proofs buy bounded compute, not just a smaller write (§"Key temporal locality"). Where the trade is real (one big statement vs. many bounded ones), it is a measurable cost question, not a correctness one.

**Write cost — how the repair reaches stored rows.** Engines offer a handful of verbs — `INSERT`, `DELETE`, `UPDATE`, `MERGE` — but the cost structure behind them is three orthogonal properties, derived per cell (§"Per-cell write addressing"):

- **Row location.** A write finds its rows by **region predicate** (partition range — needs a clock: `DELETE`+`INSERT`, partition swap), by **identity** (needs a `unique_key`: keyed `MERGE`, in-place `UPDATE`), or by any other predicate a registered pattern can prove sound — the pattern set is open (§"The write-pattern set is open (and partly backend-provided)"). Pure append (`INSERT` of new rows) is the degenerate cheapest case: nothing to locate.
- **Replacement granularity.** A **wholesale** write (`DELETE`+`INSERT`, swap) replaces the whole region — simple and contract-agnostic, but it rewrites unchanged rows. A **surgical** write (`UPDATE`, `MERGE`, column-scoped merge) touches only changed rows or columns — less written, but it needs row identity, change-comparability proofs, and engine support.
- **Locality.** Every verb is cheaper when the touched rows cluster in few partitions: the plan resolves the delta to touched partitions first and scopes the statement to them, whatever the addressing (§"Per-cell write addressing", "Addressing is how a row is found, not how far the statement ranges").

The division of labour: the **declared facts** gate which write mechanisms exist at all; the **proofs** bound what must be read, where writes may land, and how small a unit of work a repair may be broken into; and among the mechanisms that survive admission, equivalence makes the choice a pure cost question — the cost model, an operator `prefer`/`technique` pin, or an offline `smelt bakeoff` measurement decides, and freshness is the only thing at stake (§"Per-cell admission").

### The running example

The spec's examples draw from one small warehouse:

- `sources.orders` — clocked order fact feed (`order_ts`; up to 2 days late), append-only;
- `sources.order_events` — clocked order-lifecycle event feed (`event_ts`), append-only;
- `sources.raw_events` — clocked event feed with redeliveries; any duplicate of an event arrives within 7 days of the first copy (declared `key_recurrence: '7 days'`);
- `sources.customers` — mutable dimension snapshot (`customer_id`, `tier`, `region`);
- `sources.customer_changes` — clocked update-events feed of customer attribute changes (`effective_ts`), one row per change, append-only.

and these models:

| model | declares | corner |
|---|---|---|
| `daily_revenue` | clock | partition grain |
| `order_lifecycle` | identity | key grain (bare) |
| `order_facts` | clock + identity (joins `customers`) | composed — the per-cell-addressing example |
| `event_dedupe` | clock + identity | composed — the locality example |

One further model, `customer_history` (SCD2 over `customer_changes`), appears in §Limitations: it is written as plain windowed SQL and is deliberately *not* a maintained shape.

### Reading guide

- *What can I write in frontmatter and on the CLI, and what errors can I get?* → §Surface.
- *What exactly does a run do, and why was my model refused?* → §Semantics — shared machinery first (the invariant, the plan, windows and clamps, the graph layer), then one profile section per shape.
- *Why is it designed this way; was X considered?* → §Design.
- *What must never break?* → §Constraints & Invariants.
- *What does smelt deliberately not do?* → §Limitations.
- *Where does today's implementation fall short of this spec?* → §Known Divergences.

## Surface

### The declared shape

The entire declared shape surface of an incremental model is the two shape-defining facts of the Relation Contract (`models.md` §"The Relation Contract"):

```yaml
refresh: incremental        # the one refresh mode this spec covers
timeseries: { ... }         # the clock: event_time_column / partition_column / granularity (timeseries.md)
unique_key: [ ... ]         # the identity: makes the output key-addressable
grain: partition | key | key_per_partition   # optional CHECK-ONLY assertion; drives nothing
```

The `refresh:` axis itself (including `full` and `materialized_view`) and the declaration law are owned by `models.md` §"Refresh axis". The declarations name **shape-defining facts only**: which technique realizes which part of the output, and how each write physically addresses rows, are per-cell derived properties (§"The plan matrix", §"Per-cell write addressing"), never model-wide declarations, and the machinery validates the declared facts rather than choosing them (§"Validator, not chooser").

**The two facts are orthogonal and compose.** Whether the output declares an identity and whether it declares a clock vary independently (§Overview "The four corners" shows the inhabited combinations). A model with both is a first-class shape, not a corner case (§"What the composed shape enables"). Both axes are also orthogonal to **input consumption**: a bare keyed model over a clocked source still consumes that source window-forward; a composed model's *output* clock is a property of its own stored shape, not of its sources. Text anywhere in this corpus that treats "partitioned" and "keyed" as mutually exclusive alternatives is wrong and is corrected against this section.

**Grain is a derived label.** `grain` is a classification computed from `(clock?, identity?, partition_column ∈ key?)`, reported by `smelt explain`, and computed for sources too (a source likewise has an effective grain: clocked-fact, keyed-dimension, …). A modeller who wants the friendly name in frontmatter may write it only as a **check-only assertion**: it errors on mismatch with the derived facts (`models.md` §"Constraint violations") and drives nothing. The declared *facts* stay one-per-node, so two declarations of one node can never disagree. The single fact `partition_column ∈ unique_key` is what distinguishes the trajectory (`key_per_partition` — the key recurs across partitions) from a keyed lookup whose key has a fixed home slice (`key`, time-partitioned); the same fact reappears as key-temporal-locality routes 1 and 2 (§"Key temporal locality").

### Maintenance overrides (`maintenance:`)

Almost every model declares none of this. The `maintenance:` frontmatter block steers *choice among proven-equivalent techniques* and states *expectations* the derived plan is checked against — it never widens what admission allows:

```yaml
maintenance:
  defaults:
    prefer: recompute | fold | suppress | unconditional | auto   # per-model soft default (auto = cost model)
  cells:
    - columns: [<col>, ...]                # names any member of a derived column group
      on: <source-address> | backfill      # the trigger + changed-input this cell handles
      prefer: fold | recompute | suppress | unconditional   # soft per-cell bias (cost model still refines)
      technique: fold | recompute | rederive_columns | suppress | unconditional
                                           # hard per-cell pin (bypasses cost model)
      write: <pattern>                     # hard per-cell addressing pin; OPEN name resolved against
                                           #   the write-pattern registry (e.g. region | keyed | column
                                           #   | update); unknown or backend-unavailable → refused
  scan_bounds:
    require: partition_local | none        # default: partition_local
    on_violation: error | warn             # default: error
    per_source:
      <source-address>:
        max_lookback: '<interval>'         # ceiling on the derived scan span for this source
        allow_full_scan: true              # named acceptance of a full read of this source
```

- The override ladder is `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower scope winning; `technique:` alone bypasses the cost model. Overrides select among **admissible** techniques only — an override can never select an inadmissible one (§"Per-cell admission").
- `suppress`/`unconditional` are an orthogonal dimension from `fold`/`recompute`: they never change which technique family a cell resolves to, only whether a suppressible cell's matched arm writes conditionally (§"Windowed maintenance and the horizon", pruning category 2). `technique: suppress` on a cell whose write-suppression proof did not hold (no proven row identity, or a compared column not proven comparable across runs) is refused like any pin naming an unadmitted technique; `technique: unconditional` never refuses.
- `cells[].write` is a **hard per-cell addressing pin**: an open name resolved against the write-pattern registry, not a sealed keyword set (§"Per-cell write addressing"). Every pin is validated against the equivalence invariant for its cell — an addressing that cannot uphold equivalence is refused with `MaintenanceWriteAddressingRefused`, and an unrecognised name, or one the target backend cannot execute, is refused with `MaintenanceWritePatternUnavailable`. Never a silent downgrade.
- `cells[].columns` naming columns that span two derived groups is an error (it would silently re-partition the plan).
- `scan_bounds` is **check-only**: it never modifies a clamp; it only refuses (or warns) when the derived plan exceeds the stated expectation. A project-level default in `smelt.yml` sets the baseline; per-model blocks refine it.
- A sibling **top-level** frontmatter key, `horizon_ceiling: '<interval>'` (partition grain only), declares a ceiling on the derived horizon — a compile-time warning threshold, never a clamp modification (§"Windowed maintenance and the horizon").

### Partition-grain declaration (`grain: partition`)

Opt-in: `refresh: incremental` plus a `timeseries:` clock and **no declared identity**. The stored `table` is implied. `daily_revenue` from the running example:

```sql
---
refresh: incremental
grain: partition              # optional CHECK-ONLY assertion of what the facts already fix
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
safety_overrides:             # optional; bypasses specific safety checks (§"Safety checks")
  allow_window_functions: false
  allow_having: false
  allow_subqueries: false
columns:
  inserted_at:
    contract: plausible       # optional; exempts this column from the determinism requirement
---

SELECT order_date, customer_id, SUM(amount) AS revenue
FROM smelt.orders
GROUP BY order_date, customer_id
```

Rules:

- The `timeseries:` block (grammar: `timeseries.md`) is **required**: a model asserting `grain: partition` without one is a hard error, `TimeseriesRequiredForPartitionGrain` (`models.md` §"Constraint violations").
- The declared `partition_column` must be **monotone**, validated by the event-time monotonicity trace (`model_properties.md`). Monotone admits a timestamp *or* an ever-increasing integer (sequence id / offset / watermark): a constant shift over such a column (`batch_id + 5`) is recognised on the same footing as a constant `INTERVAL` shift, while a non-monotone transform (`batch_id % n`, `batch_id * n`) is rejected fail-closed, naming the construct.
- `safety_overrides` is a top-level frontmatter key admitted **only** on a partition-shaped output (`models.md` §"YAML frontmatter keys").
- `columns.<c>.contract: plausible` (key owned by `models.md` §"`columns:` — column metadata"; semantics owned here) exempts that output column from the determinism requirement — audit stamps and surrogates the modeller accepts may vary between a run and a full refresh. Listing `event_time_column`, `partition_column`, or a `unique_key` column as `plausible` is a configuration error: skeleton positions must be deterministic (§"Safety checks").
- Declaring a `unique_key` here does **not** add a "dedup aid": it declares identity, which reshapes the output to the composed clock-and-identity corner (where keyed dimension-change addressing lives — §"Per-cell write addressing"), and `safety_overrides` then becomes a hard error. A model that wants only whole-partition rewrites declares no identity.

The same declaration may live in `smelt.yml` instead; frontmatter wins over `smelt.yml` when both set the same field:

```yaml
models:
  daily_revenue:
    refresh: incremental
    grain: partition
    timeseries: { event_time_column: order_date, partition_column: order_date, granularity: day }
```

### Key-grain declaration (`grain: key`)

Opt-in: `refresh: incremental` plus a declared `unique_key` — with no clock, or with a clock admitted under key temporal locality (below). The stored `table` is implied. `order_lifecycle` from the running example:

```sql
---
refresh: incremental
unique_key: [order_id]
grain: key                    # optional CHECK-ONLY assertion
---

SELECT
    order_id,
    MIN(event_ts)                 AS placed_at,       -- extremal fold
    MAX_BY(status, event_ts)      AS current_status,  -- order-monotone overwrite
    SUM(item_count)               AS total_items,     -- additive fold
    MAX(shipped_ts)               AS shipped_at       -- extremal fold (milestone)
FROM smelt.order_events
GROUP BY order_id
```

Rules:

- The body **must** be an aggregated `GROUP BY` query (`KeyedRequiresGroupBy` otherwise): `unique_key` must restate the `GROUP BY` column list, and every non-key projection must classify into exactly one column family (below). The SQL must itself express the per-key semantics, so that a full refresh of the SQL is the profile's executable correctness oracle (§"End-state equivalence: the SQL is the oracle").
- One profile covers the running-aggregate, latest-value, and milestone patterns; what distinguishes them is the **column family** of each projection, derived from the SQL, never declared.
- No shape-specific config block exists, and `safety_overrides` is a hard error once identity makes the output key-addressed: every keyed rejection guards the equivalence invariant itself, and there is nothing safe to waive (§"Key-grain design").
- By default the output carries no partition column and downstream consumers read it in full, like any lookup. A `timeseries:` block on the model is admitted **iff key temporal locality is established** (§"Key temporal locality"), refused otherwise with `KeyedForbidsTimeseries` naming the three routes and the nearest missing fact. Output partitioning is independent of *consumption*: a keyed model over a clocked source consumes it window-forward regardless.
- `grain: key_per_partition` is a **different grain**, not a sub-declaration: it stores the per-partition trajectory (`partition_column ∈ unique_key`), not the end-state this profile maintains.

The time-partitioned form, on the shape it exists for — event dedupe over a bounded redelivery window (`event_dedupe` from the running example; the driving source declares `key_recurrence` — `sources.md`):

```sql
---
refresh: incremental
unique_key: [event_id]
grain: key                    # derived from unique_key + locality-admitted clock
timeseries:
  event_time_column: first_seen_at
  partition_column: first_seen_date
  granularity: day
---

SELECT
    event_id,
    MIN(event_ts)              AS first_seen_at,    -- extremal fold (the output clock)
    MIN(event_date)            AS first_seen_date,  -- extremal fold (the partition column)
    MAX_BY(payload, event_ts)  AS payload           -- order-monotone overwrite (latest copy wins)
FROM smelt.raw_events
GROUP BY event_id
```

And in `smelt.yml` (frontmatter wins on conflict; the same `timeseries:`-admission constraint applies):

```yaml
models:
  order_lifecycle:
    refresh: incremental
    grain: key
    unique_key: [order_id]
```

#### The column-family catalogue

The classifier assigns each non-key projection to exactly one **column family**. The family fixes the cross-window combiner — a lookup off the aggregator; authors never declare combiners — and every derived property:

| Family | Per-key aggregators | Cross-window combiner | Idempotent (re-run safe) | Order-independent | Invertible | Run shapes admitted | Extra licence |
|---|---|---|---|---|---|---|---|
| **additive fold** | `COUNT(...)`, `SUM(...)`, `BIT_XOR(...)` | `+` / `xor` | no | yes | yes | window-forward only | ledger-enforced re-run refusal (§"The transactional merge ledger") |
| **extremal / lattice fold** | `MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` | `LEAST`/`GREATEST`/`AND`/`OR`/`&`/`\|` | yes | yes | no | window-forward only | — |
| **order-monotone overwrite** | `MAX_BY(value, ordering)`, `MIN_BY(value, ordering)` | max/min-by-ordering over hidden `(v, o)` state (§"Decomposed state (rung 2) in keyed models", §"Ordering ties") | yes | up to ordering-key ties | no | window-forward only | — |
| **once-write** | `COALESCE`-first-non-null over the group | `COALESCE(target, delta)`, or the decomposed `(value, written)` state fold for the fallback/multi-candidate spellings (§"Decomposed state (rung 2) in keyed models") | yes | yes (given the proof) | no | window-forward only | once-write provenance proof (`model_properties.md`): key-derived, or a declared functional dependency over a NULL-preserving reduction |
| **decomposed fold** | `AVG(...)`, `STDDEV_*(...)`, `VAR_*(...)` | pairwise state combiner (§"Decomposed state (rung 2) in keyed models") | no | yes | per underlying combiner (additive state, invertible) | window-forward only | ledger-graded as additive (§"The transactional merge ledger") |
| **plain overwrite** | `ANY_VALUE(...)` | incoming row wins | yes | n/a — one row per key per scan | no | **snapshot-reconcile only** | — |

Any other aggregate, any non-aggregate non-key expression, and any composite expression over aggregates (`SUM(x) + 1`) is rejected (`KeyedUnknownCombiner`). Add columns for the underlying aggregates and derive downstream.

The order-monotone overwrite family needs no companion projection: the ordering expression's
value is carried as hidden state (§"Decomposed state (rung 2) in keyed models"), so the
cross-window combiner compares the *stored* state's ordering value against the delta's without
the modeller projecting it themselves. A `MAX_BY(x, x)` materialises the same uniform two-column
`(v, o)` state as any other call — value and ordering coincide, so the ordering state column
repeats the value expression rather than introducing a new one; there is no one-column special
case.

The once-write family admits four spellings, and no others:

- `COALESCE(<unique_key column>, …)` — key-derived, no declaration needed. Fallback arguments are permitted here: a key column is non-null within its own group by construction, so a fallback can never stand in for a value a later window would supply.
- `COALESCE(MAX(<col>))` / `COALESCE(MIN(<col>))` — a single-column reduction with **no further argument**, admitted only under a declared functional dependency naming `<col>` (the source payload, never the projection's alias) over a key the model's `unique_key` covers.
- `COALESCE(MAX(<col>), <fallback>)` / `COALESCE(MIN(<col>), <fallback>)` — the same reduction with a fallback argument, admitted under the same functional dependency, backed by the decomposed `(value, written)` state (§"Decomposed state (rung 2) in keyed models"): the raw reduction and the fallback are kept apart, so the fallback is applied fresh in `π` on every read rather than merged into the stored value.
- `COALESCE(MAX(<a>), MAX(<b>))` (and the `MIN` variants, and longer candidate lists) — a multi-candidate reduction, admitted under a declared functional dependency naming *every* candidate column, backed by one decomposed `(value, written)` state pair per candidate: `π` applies the arguments' declared preference order over the candidates whose state is `written`, so the order candidates happened to arrive in across windows never overrides the declared preference.

The family's **NULL-preservation obligation** follows directly from the equivalence invariant: the presented value must be NULL exactly when the key has no value yet under a full refresh. The bare key-derived and no-fallback single-reduction spellings discharge it directly, since their cross-window combiner *is* `COALESCE(target, delta)` — "the first non-null value any window produced wins." The fallback-bearing and multi-candidate spellings discharge it through the decomposed state instead: the state never stores a fallback-applied or preference-collapsed value, only the raw per-candidate reduction plus its `written` flag, and `π` — a pure function of one row's state — applies the fallback or preference order on every read. A declared functional dependency asserts that a candidate's payload is a per-key constant; it never asserts that the payload is non-null, and this family is literally "first non-null", so intra-key NULLs are anticipated for every candidate independently. Every spelling refuses `KeyedOnceWriteUnproven` absent its functional dependency (§Diagnostics).

A `COALESCE(...)` used as a null-safe composite `GROUP BY` key is a key column, not a once-write column, and needs no proof.

The pattern functions `smelt.latest(value, ordering)` (→ `MAX_BY`), `smelt.once(value)` (→ the once-write canonical spelling), and `smelt.current(value)` (→ `ANY_VALUE`) are intent-naming sugar for the overwrite, once-write, and plain-overwrite families; they are ordinary transparent functions (`functions.md`) whose expansions are admitted on exactly the same terms as hand-written calls.

### CLI

- `smelt explain <model>` — prints the plan: cells, addressing, clamps, locality verdicts, the per-column guarantee ledger, and the model's inbound edges. For every presented column that folds through decomposed state (§"Decomposed state (rung 2) in keyed models"), it additionally lists that column's hidden state columns and the presentation map `π` that recomputes the presented value from them, labelled as internal state and explicitly not part of the model's public schema; a model with no decomposed-state columns prints no such section. With `--show-sql`, additionally prints each cell's emitted maintenance statements — the same emitters' output a run executes (§"Statement emission (single owner)"; flag surface in `cli.md`).
- `smelt run --since-upstream --source <address> --landed <start>..<end>` (`--source`/`--landed` repeatable, one pair per source) — **forward propagation**: the caller declares what landed for each source since it last propagated; the graph reflects those per-source deltas through the edges and runs exactly the propagated per-edge regions with their trigger cells (§"The graph layer"). `--source` accepts a declared source or an upstream maintained model (a model's landed delta is the output window a completed run wrote). No per-invocation delta is computed automatically — a source named without a matching `--landed` propagates nothing. Opt-in; the intended default posture once trusted. Prints the dirty set before acting.
- `smelt build <model> --period <start>..<end> --include-upstreams` — **backward resolution**: print the per-ancestor required slices and build order; optionally execute the bounded build (§"The graph layer").
- `smelt bakeoff <model> [--cells <col>@<source>,...] [--runs N] [--target <name>] [--keep] [--pin]` — measures every admissible technique for a set of cells against a representative window of real data and reports cost. `--cells` defaults to every cell with two or more admissible techniques. `--runs N` (default 3) splits the driving source's event-time extent into `N` sequential windows and replays them in order per technique; each replay is a real `execute_project` run against the project's actual data. Each measured technique runs against a scratch target: the chosen target is cloned in-memory under a synthetic name with schema `smelt_bakeoff_<model>_<technique>` (no runtime schema seam — schema already flows from `config.targets[target].schema`), dropped after measurement unless `--keep`. After each window the measured techniques' outputs are cross-checked against each other with `EXCEPT ALL` in both directions — the equivalence bakeoff exploits is verified, not assumed. `--target` selects which declared target to clone (default: the active target). `--pin` emits the winning `cells[]` entry (or a complete `maintenance:` block) as ready-to-paste YAML on stdout; it never rewrites the model's `.sql` file. An applied pin is an ordinary override, re-validated through admission on every compile.

`cells[].technique` pins and `prefer` preferences are honoured at execution: the same choice ladder that governs `smelt bakeoff`'s measurement targets resolves the technique a live run uses, and admission still binds.

**Run flags.** Which flags a model takes follows from its derived run shape:

```
smelt run       --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # partition grain; keyed window-forward
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # same, batch-safety-aware chunking
smelt run       [selectors]                                                             # keyed snapshot-reconcile
```

- Both flags are required for any direct partition-grain run; a forward-propagation run (`--since-upstream`) derives its regions from `--landed` instead. Format: ISO-8601 (`2026-03-20`, `2026-03-20T00:00:00Z`). The end bound is **exclusive**.
- The supplied `[start, end)` range is the **run window**. It must be a positive integer multiple of `timeseries.granularity`, aligned to granularity boundaries (`timeseries.md` §"Granularity arithmetic"); run-window size may exceed partition granularity (§"Run window vs partition granularity"). `backbuild` uses the model's batch-safety class to expand or split the range (§"First-run and backfill").
- For a **window-forward keyed** model, both flags are required and address the **driving source's** `partition_column`/`granularity` — never a column of the keyed output, even when an admitted output `timeseries:` block exists (run flags always address the source's clock).
- For a **snapshot-reconcile** keyed model (no clocked source), the flags are a **hard error** — *"model has no clocked driving source; run without event-time flags"*. Each run is a whole reconciliation.

### Diagnostics

All codes are catalogued in `diagnostics.md`; this spec owns their semantics. Every rejection below is fail-loud and fail-closed: nothing degrades to a silent fallback (§"Validator, not chooser").

**Shared plan codes (`Maintenance*`).**

| Code | Fires when |
|---|---|
| `MaintenanceNoAdmissibleTechnique` | No technique survives a cell's admission; names the cell (§"Per-cell admission"). |
| `MaintenanceReachNotDerivable` | A required scan bound is neither derivable nor declared (§"Per-cell admission" obligation 4). |
| `MaintenanceScanUnbounded` | A scan/footprint cannot be partition-bounded (or exceeds a declared `max_lookback`) and no `allow_full_scan` acceptance exists (§"Partition-local maintenance"). |
| `MaintenanceUnboundedFootprint` | A targeted write was requested for a cell whose write footprint is unbounded, e.g. a stored trajectory under late data (§"Per-cell admission" obligation 5). |
| `MaintenanceSkeletonColumnAdded` | A field was added in a skeleton position: a grain change, refused as a column backfill (§"The definition-change trigger"). |
| `MaintenanceGraphUnsupportedNode` | A cyclic edge set, an inadmissible self-referential model, or a bare keyed node in the propagation graph (§"The graph layer"). |
| `MaintenanceWriteAddressingRefused` | A `cells[].write` pin names an addressing that cannot uphold the cell's equivalence invariant; names the cell and the refused pattern (§"Per-cell write addressing"). |
| `MaintenanceWritePatternUnavailable` | A `write:` pin names an unrecognised pattern, or one the target backend's capability registry does not provide; names the pattern and the backend (§"Per-cell write addressing"). |
| `MaintenanceRepairKeysNotDiscoverable` | The repair family's affected-key-discovery obligation fails: a changed input's delta cannot be resolved to a finite output key set; names the changed input and why the delta yields no key set (§"The repair family" obligation (c)). |
| `MaintenanceRepairSliceUnbounded` | The repair family's bounded-per-group-read-footprint obligation fails: the key→input-slice reach is neither derived nor declared-and-checked; names the source and the unbounded reach (§"The repair family" obligation (b)). |

**Partition-grain codes.**

| Code | Fires when |
|---|---|
| `TimeseriesRequiredForPartitionGrain` | `grain: partition` asserted with no `timeseries:` block (rule owned by `models.md` §"Constraint violations"). |
| `PartitionGrainNotSafe` | The batch-safety classifier rejects the model's SQL (§"Safety checks"). |
| `EventTimeColumnNotVisibleAtOuterSelect` | The outer output-clamp cannot bind: a set operation or subquery hides `event_time_column` at the outermost SELECT (§"Event-time outer-visibility"). |

**Key-grain codes.**

| Code | Fires when |
|---|---|
| `KeyedRequiresGroupBy` | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `KeyedForbidsTimeseries` | The model declares `timeseries:` but key temporal locality cannot be established — no route applies; names the three routes and the nearest missing fact (§"Key temporal locality"). |
| `KeyedUnknownCombiner` | A non-key projection is not a direct call to a catalogued aggregator; names the offending expression. For a bare column or `ANY_VALUE` under window-forward, names `MAX_BY(value, ordering)` as the fix. |
| `KeyedGroupByContainsPartitionColumn` | The `GROUP BY` contains the driving source's `partition_column` and the model declares no `timeseries:` block — ambiguous between the partition shape and the key-embedded time-partitioned shape; suggests both fixes: `grain: partition` + `timeseries:`, or declaring `timeseries:` on the model to stay `grain: key`. |
| `KeyedForbidsWindowFunctions` | The outer SELECT uses `OVER (...)`. The keyed state *is* the window. |
| `KeyedForbidsNondeterministic` | The SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions; cross-window merge requires deterministic per-window output. |
| `KeyedSqlNotParseable` | The model body cannot be parsed into the shape the classifier reads. |
| `KeyedMultipleDrivingSources` | More than one timeseries-tagged source in the FROM clause; lists the candidates. |
| `KeyedOnceWriteUnproven` | A once-write (`COALESCE`) column — bare key-derived, single-reduction, fallback-bearing, or multi-candidate — has no once-write provenance proof for one or more of its candidate columns; names the column, the unproven candidate(s), and the three fixes (key-derived form, declared functional dependency, remodelling). |
| `KeyedStateColumnCollision` | A decomposed-state column name (`<output>__<part>`, §"Decomposed state (rung 2) in keyed models") collides with a declared or projected user column; names both and the reserved suffix. |
| `KeyedRetractableContribution` | An enrichment join's per-key contribution is retractable — it feeds a decrementing aggregate or a value that must be un-seen — and the repair family cannot admit a per-group recompute for the retraction; names the failing repair obligation. Steers to `refresh: materialized_view` or DAG composition. Never fires on the join spelling alone (§"Enrichment joins", §"The repair family"). |
| `KeyedSnapshotSourceUnsupportedColumn` | A column family inadmissible under snapshot-reconcile appears in a model with no clocked driving source; names the column, the family, and why the current-snapshot oracle cannot hold (§"Admission matrix"). |
| `KeyedSnapshotPostureUnsupported` | No clocked driving source, and no single unambiguous source to reconcile against either (two or more unclocked candidates in the FROM clause) — neither run shape can be derived (§"The two run shapes"). |
| `KeyedReprocessedWindow` | A run window covers a ledgered window of a non-re-run-tolerant model, or `--auto` detects changed input under an already-merged window, and the repair family cannot admit a per-group recompute for the change; names the failing repair obligation and points at `--full-refresh` (§"Reprocessing", §"The repair family"). |
| `KeyedRecurrenceBoundViolated` | Runtime, declared-recurrence route only: a merged delta row matched (or would duplicate) a stored key outside the run's derived slice. The run's transaction rolls back; reports the violation count and sample keys (§"Key temporal locality"). |

## Semantics

The shared machinery comes first — the invariant every maintained model upholds, the plan that
organises its maintenance, and the graph layer built on the plan — then one profile section per
shape. A profile section owns only what is meaningful inside that shape; everything else it
composes by name (§"Shape profiles").

### The equivalence invariant

Every maintained (non-`full`) model upholds **one** invariant, stated over an abstract
**processed-input set** `S`: an incremental run produces the result a full refresh would,
restricted to the inputs processed so far.

```
incremental_state(S) == full_refresh(source | input ∈ S)
```

`S` is a set of *source rows or partitions the runs have scanned* — not necessarily
clock-addressed. The **partition-set form** (`source | partition_col ∈ S`), used throughout the
rest of this spec, is the **clocked specialisation**, available whenever the driving source
carries a `timeseries:` clock. An unclocked (snapshot) source has no partition set to slice by;
its specialisation is stated per shape profile (the key grain states it over "keys present in
the current snapshot" — §"End-state equivalence: the SQL is the oracle").

**Order/set-determinacy is a corollary, for every shape.** The right-hand side depends only on
the *set* `S`, never the order it was processed in, so every conforming profile is
order-independent. This is not special to the key-addressed shapes: a partition-grain model's
partitions are disjoint, so its combiner is disjoint union and the property is trivial — but it
is present.

**Strengthenings.** Where an output slice depends only on its own bounded input slice, the
invariant is additionally checkable slice-by-slice:

- **per-partition equivalence** — the partition grain's strengthening: each output partition
  equals the full refresh restricted to that partition (§"Per-partition equivalence");
- **per-slice equivalence** — the keyed analogue, available when key temporal locality is
  established (§"Key temporal locality").

These are strengthenings of the one invariant, not peer contracts. What actually distinguishes
the shapes is how their writes **address rows** — a per-cell fact (§"Per-cell write
addressing"), not a second invariant. The key-addressed shapes discharge the *same* invariant on
their end-state because their writes reach stored rows by key, wherever they live.

**The replayability split.** Full equivalence — an executable `full_refresh` oracle a test can
run — holds only for **replayable inputs**: a set `S` the model can re-evaluate its own SQL over
(a clocked source's processed partitions; a snapshot's currently-present keys). Only
combinations whose oracle is executable this way are admitted — this is exactly what the
admission matrix enforces per column (§"Admission matrix"). For the combinations that are not
admitted (a non-replayable input under a partitioned output; a fold family that would need
history it cannot replay), a different, weaker **observer / prefix-consistency contract** — a
property of the observation sequence, not a re-runnable refresh — could one day be stated and
opted into explicitly (§Future Extensions). It is never smuggled in under the executable-oracle
invariant stated here.

**Two named carve-outs.** Every admitted keyed model's executable oracle carries exactly two,
both consequences of the executable-oracle requirement rather than gaps in it:

- **Retained departed keys** under snapshot-reconcile: a key present in the stored state but
  absent from the current snapshot is retained, not deleted, so the stored table is *the
  oracle's rows plus retained departed keys* (§"The two run shapes").
- **Ordering-key ties** on an order-monotone overwrite column: equivalence holds up to ties on
  the ordering expression, because ordering-key uniqueness is not statically provable
  (§"Ordering ties").

Every property in `model_properties.md` is proven in service of this invariant; every transform
in `model_transforms.md` is licensed because it preserves it. For the smelt-driven shapes the
invariant is discharged by the generative equivalence oracle (§References — the family's
regression net); for `refresh: materialized_view` it is discharged by the **engine's** native
incremental view maintenance, and smelt runs no combiner (`materialized_view.md`).

### The algebraic maintenance ladder

What a key-addressed model can maintain is fixed by the **algebra of its combiners**, not by any
backend feature. The ladder's ordering criterion is invertibility → maintainability, which is
why it lives here with the invariant: the raw *discriminants* it reads (is-monoid, needs-inverse,
decomposable, value-vs-order-monotone) are properties of the SQL owned by `model_properties.md`;
the ladder — the ordering and the maintainable-vs-delegated cutoff — is the maintenance
consequence and is owned here. The equivalence invariant holds unconditionally on every rung;
only the state representation and its size change, never the fidelity of the user value.

1. **Direct monoid.** The stored column *is* the answer; the combiner is a commutative monoid
   (associative, commutative, identity = empty partition): `SUM`/`COUNT` (`+`, 0), `MIN`/`MAX`
   (±∞), `BOOL_*`, `BIT_*`.
2. **Decomposed monoid.** The user value is `π(state)` for a richer monoid element and a pure
   presentation map `π`: `AVG` = `(sum, count)` presented `sum/count`; variance = a Welford
   triple; approximate distinct = an HLL register vector. Kept in a state table, exposed through
   a presentation view.
3. **Group.** When inputs can change (corrections, reprocessing, deletes) the combiner must be
   **invertible** — a commutative group (`SUM`, `COUNT`, `BIT_XOR`). Monoids that are not groups
   (`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR`) cannot un-see a contribution and so cannot be
   reprocessed without a full refresh.
4. **Opt-in bounded-domain multiset.** Holistic aggregates needing all rows (exact
   `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`, `DISTINCT`-modified
   aggregates) are maintained by storing the per-key value→count multiset (a bounded-domain
   Z-set); its signed form makes retraction free even for the otherwise-irreversible `MIN`/`MAX`.
   **Opt-in and fail-loud**: state is `O(active domain)`, so an unbounded-state aggregate is
   refused by default (suggesting the approximate form or `refresh: full`) unless the modeller
   supplies a bounded-domain budget, and the runtime caps the multiset with a full-refresh
   fallback.

The ladder is the boundary: rungs 1–4 are what smelt maintains itself (a `merge_into` loop,
optionally with a presentation view). Beyond it — general-operator retraction over joins,
unbounded non-additive state — is delegated to the engine's native incremental view maintenance
via `refresh: materialized_view`.

### Decomposed state (rung 2) in keyed models

Ladder rung 2 (above) says the user value can be `π(state)` for a richer monoid element. This
section fixes where that state physically lives for the key grain, which column families it
licenses, and how it stays invisible to consumers — the missing piece every decomposed-state
admission in §"The column-family catalogue" cites by name.

**Physical layout.** State columns live in the *same* stored table as the presented columns,
named `<output>__<part>` (e.g. `total_spend__sum`, `total_spend__count`). The presented column
is materialised alongside them at merge time, computed by the presentation map `π` from that
row's own state. Rejected alternative: a separate `<model>__state` table plus a presentation
*view*. A second relation would make `ref()` sometimes resolve to a table and sometimes to a
view, add a second relation to every backend's DDL and atomic-swap path, and buy nothing — `π`
is a per-row pure function of the same row's state, so nothing about it needs a second query.

**Presentation projection.** State columns are excluded from the model's public schema:
`smelt.ref()` expansion, `SELECT *`, declared-schema checks, and downstream type inference see
only presented columns. A state column name colliding with a declared or projected user column
is a fail-loud refusal (`KeyedStateColumnCollision`, §Diagnostics), never a silent rename —
smelt does not guess which of the two a consumer meant.

Because state columns share the stored table, a wildcard in a consumer that reads a state-bearing
model is rewritten at compile time to that model's presented columns (sibling relations in the
same `FROM` keep their own `<rel>.*`); explicit column references are untouched, and a `__part`
name written by hand is an ordinary unresolved-column diagnostic, since it is not in the model's
public schema. If a wildcard's relations cannot be resolved while a state-bearing model is in
scope, the compile fails loud with the model and the unresolvable wildcard named — never a
pass-through that would leak state columns into the consumer's schema.

**The state-shape catalogue.** Each decomposable family has one fixed, hand-encoded state shape
and presentation map; there is no general decomposition procedure, matching rung 2's own
"kept in a state table, exposed through a presentation view" framing (above) with a concrete
per-family shape:

| Family | State columns (`__` suffix) | Combiner over state | Presentation map `π` |
|---|---|---|---|
| `AVG(x)` | `sum`, `count` | pairwise `+` on each column | `sum / count`, `NULL` when `count = 0` |
| `STDDEV_*(x)` / `VAR_*(x)` | `n`, `sx` (`Σx`), `sxx` (`Σx²`) | pairwise `+` on each column | per-family closed form over `(n, Σx, Σx²)` (population vs. sample divisor and `sqrt` per the specific function), `NULL` below the family's minimum `n` (`0` population, `1` sample) |
| `MAX_BY(v, o)` / `MIN_BY(v, o)` | `v`, `o` (the hidden ordering value) | keep the pair whose `o` is greater (`MAX_BY`) / lesser (`MIN_BY`); on equality the incumbent wins, matching §"Ordering ties" | `v` — `o` is never presented |
| once-write | `value`, `written` (boolean) | `written` is `OR`; `value` is `COALESCE(target.value, delta.value)` — the incumbent's value survives once written, the delta only ever fills a state row that was never written | family-specific, below |

`AVG`'s and `STDDEV_*`/`VAR_*`'s state combiners are commutative monoids over their state tuples
(component-wise `SUM`, itself a monoid), so the equivalence invariant and the order/set-determinacy
corollary (§"The equivalence invariant") hold over the state with no exception — they are graded
**additive** in the transactional merge ledger (§"The transactional merge ledger"), the same as
`SUM`/`COUNT`, since their state components are `SUM`-shaped. `MAX_BY`/`MIN_BY`'s state combiner
carries the same ordering-key-tie carve-out its rung-1 form already had (§"Two named carve-outs",
§"Ordering ties") — moving the ordering value into hidden state changes nothing about that
exception, only who tracks the ordering column. Once-write's state combiner is fully
order-independent given its provenance proof, exactly as its rung-1 `COALESCE(target, delta)`
form is: a per-key-constant value produces the same result regardless of which window's delta
supplies it first. `MAX_BY`/`MIN_BY` and once-write keep the idempotent grade their rung-1 form
already carries — replacing a hand-written companion column or a spelling restriction with
hidden state changes nothing about re-run safety.

**Once-write's `π` widens what the family admits**, because the state now separates the *raw*
per-key reduction from the presented value: the raw reduction is never fallback-tainted, so a
fallback or a preference order can be applied fresh on every read instead of being baked into
the merged value once and then re-merged incorrectly by a later window (§"The column-family
catalogue" states which spellings this newly admits). Concretely:

- **Fallback-bearing single reduction** (`COALESCE(MAX(<col>), <fallback>)`): one state pair
  `(value, written)` over the bare reduction `MAX(<col>)`/`MIN(<col>)`, with `written = (value IS
  NOT NULL)`. `π = value` if `written`, else `<fallback>`.
- **Multi-candidate reduction** (`COALESCE(MAX(a), MAX(b))`): one `(value, written)` pair per
  candidate, each folded independently exactly as the single-reduction case above. `π` applies
  the arguments' declared preference order over the candidates whose `written` is true — a pure
  function of that row's state, so the order the source's windows happened to merge in can no
  longer leak into which candidate wins.
- The bare key-derived spelling (`COALESCE(<unique_key column>, …)`) needs no decomposed state:
  a key column is already non-null by construction, so the plain `COALESCE(target, delta)`
  combiner (§"The column-family catalogue") already computes the presented value directly.

`smelt explain` renders state columns as internal state, distinct from the model's public
schema — see §Surface "CLI".

### Validator, not chooser

The machinery **validates** the declared shape — the `refresh:` value plus the shape-defining
facts, and any check-only `grain:` or `write:` assertion — against the derived properties, and
rejects fail-loud when the SQL cannot uphold the shape's contract. It **never chooses or
silently switches** the shape or the addressing. A full refresh is the honest fallback surfaced
as a diagnostic, never an automatic downgrade. Per-cell technique choice among
proven-interchangeable techniques (§"Per-cell admission") operates strictly inside this rule: it
may change freshness, never observable bits at a fixed processed-input set.

### The plan matrix

Every maintained model has a **maintenance plan**: pure data, derived once, consumed everywhere
(§Constraints). Its cells are keyed by `(output-column-group × trigger × changed-input)`.

**Column groups.** The plan factors the output columns into groups by shared
mutation-sensitivity (`model_properties.md` §"Per-column mutation-sensitivity / column
provenance" owns the proof and its degenerate-collapse rule; this spec consumes the groups).
Creation is shared by every column — all columns of a new row are computed together; mutation is
what partitions them.

Sensitivity carries its kind into the cell. A **value-sensitive** group's mutation cell may be
repaired column-scoped (a `MERGE` that rewrites the group's columns in place). A
**membership-sensitive** group — one governed by a mutable source read in row-admission
position (`model_properties.md` §"Per-column mutation-sensitivity / column provenance") — must
be repaired by a technique that can create and delete rows: the recompute family
(delete+insert, change-suppressed where the staged candidate is comparable), never a
column-scoped merge, which cannot fix which rows exist. A mutable join partner never read in
any select item still produces mutation cells through membership sensitivity; its absence from
every value-sensitivity set is not admissibility for cheaper repair. The one admissible
pruning is a proof, not a default: an enrichment join whose skeleton-source closure is proven
`Closed` over a provably outer join contributes no membership sensitivity — the closure
establishes its deltas cannot change which rows exist, so only its value sensitivity remains
(`model_properties.md` §"Per-column mutation-sensitivity / column provenance").

**Triggers.** Four trigger classes index the plan:

- **creation** — new rows arrived in the driving source;
- **mutation** — a post-creation delta in a source some column group is mutation-sensitive to;
- **definition change** — the model gained output fields while sources stood still;
- **backfill** — an explicit region recompute from replayable input.

Each trigger is paired with the **changed input** it fires for — the specific source, upstream
model, self-edge, or definition diff whose delta drives the cell. This third axis is what makes
"what runs when *this* input changes" a first-class, per-input answer (the model's **scope
maps**), surfaced by `smelt explain`: the driving source's delta engages the windowed fold; a
mutable dimension's delta engages the delta-driven probe and horizon-bounded merge; a self-edge
engages ordered execution; a definition diff engages the targeted column backfill (all
`model_transforms.md`). The same column group under the same trigger class can derive
*different* physical write addressing for different changed inputs (§"Per-cell write
addressing").

**Each cell carries:**

- its **corner** of the read-scope × write-scope 2×2 (below);
- the **technique** that realizes it, drawn from the open write-pattern registry (which includes
  the repair family, §"The repair family");
- the **write mechanism** admitted for it — derived by the available-addressings rule, or a
  validated user `write:` pin (§"Per-cell write addressing");
- the **derived scan clamps** — per read source, the `(partition_col, before, after)` window the
  cell reads, anchored to the output region (§"Windowed maintenance and the horizon");
- the **partition-locality verdict** per source (§"Partition-local maintenance");
- its **obligations** and any **traded guarantees** (per-column, two-dimensional: equivalence
  contract × settle bound).

**The 2×2.** Each cell occupies a corner of **read scope** (delta+state vs the region's full
upstream input) × **write scope** — the cell's physical write addressing (targeted addresses vs
region overwrite):

|              | write: targeted (keyed addressing) | write: region-overwrite (partition addressing) |
|---|---|---|
| **read: delta+state** | fold-a-delta | read-modify-write region |
| **read: full-input** | column-scoped re-derivation | recompute-a-region |

Recompute-a-region is contract-agnostic and unconditionally valid over replayable input; the
fold corner is contract-specific (it needs a combiner algebra — §"The algebraic maintenance
ladder"). The repair family (§"The repair family") is recompute-a-region's targeted-write
refinement: it lands in the **column-scoped re-derivation** corner — full-input read, targeted
write — scoped to a provably finite key slice rather than a whole region, and it inherits
recompute-a-region's contract-agnostic correctness argument rather than needing one of its own. Where the interchangeability conditions hold (§"Per-cell admission"), a recompute of a
region **supersedes and resets** what folds had written there. "Unconditionally valid" is a
correctness claim, not an admission or cost claim: it holds even when no partition bound exists and the
region is the whole table — whether that degenerate recompute is *admitted* is gated separately
by the partition-locality guardrail (§"Partition-local maintenance").

The plan is **derived, never declared**. What stays declared is the model's shape-defining
facts, validated against the plan — an error on mismatch, never a silent flip. `smelt explain`
prints the plan: every cell, its addressing, clamps and locality verdicts, the per-column
guarantee ledger, and the model's inbound edges.

### Per-cell admission

A technique enters a cell's plan space only when all of its obligations discharge (fail-closed;
an unrecognised construct refuses, never defaults). The obligations, each with its owner:

1. **Replayable input** (recompute family) — the source is re-readable at its current processed
   set; declared posture, `sources.md`.
2. **Faithful fold** (fold family) — the fold's two independent conditions (source posture ×
   combiner algebra) hold (`model_properties.md` §"Faithful-fold conditions"); a replayable feed
   carrying retractions into a non-invertible combiner passes the first condition and fails the
   second, and either failure alone refuses the fold family for this cell.
3. **Combiner algebra class** — derived, fail-closed (`model_properties.md` discriminants); a
   holistic or unrecognised combiner leaves only the recompute family.
4. **Bounded reach** — the cell's scan bound `(clock_col, before, after)` per source is derived
   (`model_properties.md` §"Unified bound / reach derivation") or declared-and-checked; absent
   both, full-input techniques only (`MaintenanceReachNotDerivable` when the trigger requires a
   bound).
5. **Bounded footprint** (targeted writes) — the write-scope reflection of the scan bound is
   bounded (`model_properties.md` §"Footprint reflection / bounded write footprint"); a
   trajectory column's unbounded forward footprint fails this (`MaintenanceUnboundedFootprint`).
6. **Well-defined groups** — the mutation-sensitivity partition is computable
   (`model_properties.md`); degenerate collapse is surfaced, never silent.
7. **Affected-key discovery** (repair family only) — a changed input's delta resolves to a finite
   output key set, a sound over-approximation admitted (`model_properties.md` §"Affected-key
   discovery"); an unresolvable delta shape refuses the repair family by name
   (`MaintenanceRepairKeysNotDiscoverable`, §"The repair family").

**Interchangeability and choice.** Two techniques may serve one cell interchangeably iff, at a
fixed processed-input set `S`, they produce identical state on the columns that decide which
rows exist — the `S`-indexed refinement of the equivalence invariant, where `S` is a
**per-input vector** once the plan factors. For faithful idempotent columns the choice is
bit-preserving; for additive columns it is state-preserving **modulo the ledger**, whose real
obligation is *never fold a delta already reflected in the state*: fold-then-recompute is safe
(the recompute resets the region's ledger), recompute-then-refold double-counts. Technique
choice among proven-interchangeable techniques belongs to the cost model or the operator
(`prefer`/`technique`); it may change only *which `S` is reflected* (freshness), never
observable bits at a fixed `S` — which is how per-cell choice stays inside
§"Validator, not chooser".

### Per-cell write addressing

Every cell derives its **physical write** — how it locates the stored rows it updates — from the
currently known write-pattern set, an **open registry**, not a closed enum:

```
{ region DELETE+INSERT, keyed MERGE, column-scoped MERGE, in-place UPDATE, full rebuild, diff_patch, … }
```

**The available-addressings rule.** A write mechanism is admitted for a cell iff:

> `available = (which contract facts the output declares) × (what the trigger/changed-input needs) × (the equivalence invariant) × (backend capability)`

The first three factors are structural; the fourth is the target engine's capability registry
(`architecture.md`). What each declared fact gates:

- keyed `MERGE` / column-scoped `MERGE` / in-place `UPDATE` require a declared `unique_key`
  (row identity);
- region `DELETE`+`INSERT` requires a declared partition axis (`timeseries:`) to delete by;
- a **bare lookup** (identity, no clock) has no region → only keyed merge or full rebuild;
- a **bare partition table** (clock, no identity) has no identity → only region rewrite or full
  rebuild. To gain keyed dimension-change addressing the output must declare a `unique_key`,
  which makes it the composed clock-and-identity shape (§"What the composed shape enables") —
  declaring identity is **load-bearing** (it admits keyed writes), never a dedup footnote.

A cell with no admissible write mechanism is `MaintenanceNoAdmissibleTechnique`, naming the cell.

**Addressing is how a row is found, not how far the statement ranges.** Choosing keyed `MERGE`
for a cell picks row-location by identity; it does not make the statement table-wide. When the
output also declares a `timeseries:` axis, the write stays **bounded to the affected
partitions**: the changed-input delta is resolved to the touched partitions first, and the keyed
`MERGE` is emitted per partition (or with a partition predicate) against just those. A genuinely
window-free keyed write — one whole-table `MERGE` — is reached only when the cell **provably
cannot** be bounded to a partition set; that unboundedness is itself a
derived per-cell fact, fail-loud, never a default. Partition-scoping is orthogonal to the
addressing corner: region and keyed writes alike ride the partition pruning the plan computes
(§"Partition-local maintenance").

**User pins.** `maintenance.cells[].write` names the write mechanism per cell (§Surface). A pin
is validated against the equivalence invariant for its cell — refused with
`MaintenanceWriteAddressingRefused` when the addressing cannot uphold it — and refused with
`MaintenanceWritePatternUnavailable` when the name is unrecognised or the target backend cannot
execute it. The pin selects among *admissible* mechanisms; it never widens the admissible set.

**Worked example — the plan of a composed model.** `order_facts` (running example) declares
both facts and joins the mutable `customers` dimension:

```sql
---
refresh: incremental
unique_key: [order_id]
timeseries: { event_time_column: order_ts, partition_column: order_date, granularity: day }
---
SELECT o.order_id, o.order_date, o.order_ts, o.amount, c.tier AS customer_tier
FROM smelt.orders o JOIN smelt.customers c ON o.customer_id = c.customer_id
```

`smelt explain order_facts` prints a plan of this shape (illustrative rendering):

```
model order_facts  (grain: key, time-partitioned — clock + identity declared)
cells:
  [all columns        × creation  × orders]     recompute-a-region   write: region DELETE+INSERT
      scan: orders(order_date, -0d, +0d); customers(full — lookup)
  [customer_tier      × mutation  × customers]  column-scoped merge  write: keyed column MERGE
      scan: customers(delta probe); target scoped to touched partitions
  [all columns        × backfill  × orders]     recompute-a-region   write: region DELETE+INSERT
```

One model, three cells, two addressings: new orders rewrite their partitions; a tier correction
merges one column by key into just the partitions the affected orders live in. Neither verdict
is declared, and pinning either (`cells[].write`) is validated, not trusted.

#### The write-pattern set is open (and partly backend-provided)

The patterns named above are the ones understood *today*. The set grows — partition/atomic swap
(Delta/Iceberg `REPLACE PARTITION`), copy-on-write vs merge-on-read variants, `MERGE … WHEN NOT
MATCHED BY SOURCE` prune, staged-upsert, a predicate-targeted `UPDATE` locating rows by
something other than the row key, incremental MV refresh, engine-specific primitives —
and the durable contract is deliberately **not** the enumeration; the enumeration is data.

- **The invariant is the admission function, not the enum.** A new pattern is admitted by
  declaring which contract facts it requires (identity? a partition axis? ordered arrival?) and
  discharging the equivalence proof obligation for the cells it serves. Nothing else moves:
  grain stays derived, the cost model ranks whatever the rule admits. A new mechanism can never
  be less correct than the ones it joins, because the equivalence gate is the price of entry.
  Concretely: a dimension-mutation cell could one day be served by an `UPDATE` that locates
  rows through the **join key** (`customer_id`) rather than the output's `unique_key`,
  partition by partition — admitted exactly like any other pattern, by declaring the facts it
  needs (a proven functional dependency from join key to the repaired columns) and discharging
  the equivalence obligation for that cell. Today's registry serves that cell with a keyed
  column `MERGE` (§"Per-cell write addressing", worked example).
- **The pattern set is backend-relative.** Engines differ sharply on atomic partition swap, true
  `UPDATE`, and merge-on-read, so admission carries backend capability as its fourth factor: the
  write layer queries the backend's capability registry (`architecture.md`), and a pattern the
  target cannot execute is simply not a candidate. The registry is where backend-specific
  optimisations are *contributed* rather than special-cased in the planner, and it keeps a
  portable project from silently depending on a primitive only one engine has.
- **The `write:` pin is an open, fail-loud vocabulary.** Pins name patterns and patterns are
  extensible, so `write:` is an open name resolved against the registry, not a sealed enum. An
  unrecognised pin, or one naming a pattern the target backend cannot provide, is refused with a
  diagnostic — never silently downgraded.

**`diff_patch` — compute, diff, write only the difference.** A pattern for reconciliation runs
and idempotent re-runs: the candidate rows for a slice are computed, diffed against the slice's
stored state, and only the difference is written — inserting rows absent from storage, updating
stored rows whose compared columns differ from the candidate, and deleting stored rows absent
from a *complete* candidate set. Contract facts it requires: a declared `unique_key` (row
identity for the diff join) for the insert/update legs, and change comparability
(`model_properties.md` §"Change comparability") over the written columns for the update leg. The
delete leg additionally requires **slice completeness** — the candidate set must provably contain
every row that should exist in the slice, the same premise the repair family's correctness
argument rests on (§"The repair family") — and is not admitted without it; lacking completeness,
the pattern degrades explicitly to insert+update, stated as a reduced-capability admission rather
than a silently dropped delete leg. `diff_patch` is graded **Idempotent** — a second run against
unchanged input diffs to empty — which is what makes it the reconciliation and drift-repair
write (§"The transactional merge ledger"). The slice a `diff_patch` write restricts to is the
*candidate's own* slice — the affected-key set for a per-group recompute (§"The repair family"),
a partition region for a windowed one — so the pattern is not tied to a partition axis.

Backends **execute** registered patterns; they never **author** maintenance-statement text
(§"Statement emission (single owner)").

### The repair family

A non-invertible combiner refuses reprocessing outright when a merged window's input changes
(§"Reprocessing") — full refresh is the only universally correct fallback for it. The repair
family narrows that refusal for one common case: when the change is a **retraction or mutation**
whose affected output keys are provably finite (`model_properties.md` §"Affected-key discovery"),
the plan recomputes *only those groups* from their bounded input slice instead of rebuilding the
whole table or region. It is the **targeted-write refinement of recompute-a-region**: the same
full-input read as a region recompute, addressed by key rather than by region — landing in the
**column-scoped re-derivation** corner of the 2×2 (§"The plan matrix") — and like a region
recompute it **supersedes and resets** the ledger for the keys it rewrites (§"Per-cell admission",
interchangeability).

**Why it is correct.** Recomputing a key set `K` over an input slice that provably contains
*every* row contributing to any `k ∈ K` reproduces `full_refresh` restricted to `K`; every key
outside `K` is untouched, and therefore stays bit-identical to its prior state. The equivalence
invariant (§"The equivalence invariant") holds cell-wide as a consequence: written keys equal the
full-refresh oracle restricted to `K`, unwritten keys equal it trivially. The load-bearing premise
is **slice completeness** — the input slice a per-group recompute reads must provably contain
every row that can contribute to a key in `K`. This is not a new proof: it reuses **key temporal
locality** (§"Key temporal locality"), whose whole purpose is establishing that a key's
contributing rows lie within a computable slice of the input.

**Admission obligations.** A repair cell is admitted only when three obligations discharge. Two
already exist in §"Per-cell admission"'s numbered list and are reused, not restated; the third is
new:

- **derivable group key** — obligation 6 ("well-defined groups"): the walk's grain names the
  groups a repair recomputes;
- **bounded per-group read footprint** — obligation 4 ("bounded reach"): the key→input-slice
  reach is derived (a key-temporal-locality route) or declared-and-checked;
- **affected-key discovery** — a new obligation 7, below: the changed input's delta names a
  finite key set (`model_properties.md` §"Affected-key discovery"). A sound over-approximation
  (a superset of the true affected keys) is admissible — it costs extra recomputation, never
  correctness; an under-approximation is never admissible, because a missed key would leave stale
  state for a group the retraction actually touched.

All three are fail-closed: any one unprovable refuses the repair family by name for that cell —
it never widens to a whole-table repair, and the refusal always names which obligation failed
(§Diagnostics, `MaintenanceRepairKeysNotDiscoverable` / `MaintenanceRepairSliceUnbounded`).

**Obligation 7 over a `mutable_snapshot` source.** A `mutable_snapshot` source keeps no tombstone
or change history, so a key whose *entire* window contribution was deleted between runs leaves no
row for a current-source scan to select — a scan-based affected-key read alone cannot witness that
key at all, which is exactly the under-approximation obligation 7 forbids. For this source posture
the affected-key relation is instead the **group-grain fingerprint sidecar diff**
(`sources.md` §"The fingerprint sidecar" — "Partition grain"): the sidecar keeps one row per output
group key, so a vanished group still has a stored comparandum and surfaces on the diff's "sidecar
row with no matching source key" leg even though no source row survives to name it. This discovery
read is **unbounded by the cell's `ScanClamp`** — a clamped rescan compared against the sidecar's
full stored digests would flag every group outside the clamp as spuriously changed, degrading to a
whole-table repair on every run rather than only when the comparandum is missing — while the
per-group *recompute* itself stays bounded by the discovered key set exactly as obligation 4
requires. An **absent or stale-stamped comparandum** (no prior sidecar partition, or one whose
identity stamp no longer matches — `sources.md` §"The fingerprint sidecar" — "Invalidation") cannot
distinguish a group that vanished from one that never existed, so for that run the affected set
widens further, to every currently-observed group *plus* every group already present in the stored
output — a sound over-approximation, distinct from the obligation's own "never widens to a
whole-table repair" rule above (that rule is about *admission* refusing an unprovable obligation;
this is a runtime comparandum being absent). It degenerates to a whole-table repair for that one
run and self-heals once the sidecar is refreshed. An append-only source (or any other posture with
no native deletion) keeps the ordinary clamped current-source scan — the group-grain sidecar is
scoped to the one posture that needs it.

**Ledger grading and re-run safety.** Per-group recompute is graded **Idempotent** for the keys in
its slice, exactly like a region recompute (§"The transactional merge ledger"): re-running it
reproduces the same state, and it resets any additive ledger record for those keys rather than
folding a second time on top of it.

### Windowed maintenance and the horizon

Maintenance runs over a **bounded input window by default** — a full scan is the surfaced
fallback, not the baseline. A run reasons about two windows, always with `scan ⊇ write`:

- the **write window** — the partitions or keys written this run;
- the **scan window** — the input rows read to produce that write window correctly.

The scan window is bounded **where the model carries a `timeseries:` clock**: input-delta
discovery is window-forward, so only the new window (plus a lookback) is read, and stored state
stands in for history. Without a clock the source can only be snapshot-diffed, so the scan
degrades to a full read (`models.md` §"Input-consumption axis"). Scan windowing is orthogonal to
output addressing: a clocked *key-addressed* model still windows its **scan** even though its
**write** reaches back by key outside that window. Bounding the scan never weakens the
invariant: the engine evaluates the model, joins included, over the widened scan window, and the
write is **clamped** to the exact write window (`model_transforms.md` §"widened scan + exact
clamp") — join optimisation stays with the engine rather than smelt hand-computing minimal
deltas.

**The horizon (partition grain only).** The horizon is a **write-eligibility clamp** — a bound
on which partitions a run may write to: the far edge of the maintained window, past which inputs
are no longer folded in. It is **derived**, never trusted from a declaration: the clamp bounds
are computed from the model's own reach (lookback, window frames, join contribution —
`model_properties.md`), because a declared horizon smaller than the true reach would drop rows
that should have been rewritten. A modeller may declare a horizon *ceiling*
(`horizon_ceiling: '30 days'`): smelt warns at compile time when the derived horizon would
exceed it, and the clamp always uses the derived value.

Because the derived clamp *is* the model's SQL, a genuinely late arrival — one landing after its
natural partition passed the horizon — is **silently excluded** from the maintenance run, not
diagnosed: smelt cannot fail loud on a row it never scans, and rows outside the scan window are
outside "inputs processed so far" by construction. **Surfacing lateness is a model-author
concern, not a maintenance guarantee.** The available pattern: fold the late row into the
current partition (re-stamping its partition time) carrying a lateness/validity flag, so the
data still flows, and let a data-quality check raise on the flagged rows.

**The key grain has no write-eligibility clamp.** A `grain: key` run merges **every** delta row
it scans, into whatever key it names, however old (§"No write-eligibility clamp"). A derived
forward reach is still computed and reported for observability, but it never gates admission and
never bounds a write. This is deliberate, not an oversight: keyed write work is proportional to
delta size regardless of key age, so a write clamp buys nothing for correctness — and it would
silently drop scanned inputs, the one thing the invariant forbids. What a clamp would buy
(settled-key GC, a bounded working set) is deferred optimisation that must ship together with
late-fact accounting if ever introduced.

**Three pruning categories, one principle.** *Only proofs prune; a declared bound is admitted
only checked (fail-loud on violation); no unproven bound ever refuses a write.* Exactly three
categories of narrowing exist:

1. **Target-scan slice pruning** (read-side) — rows the write provably cannot touch are removed
   from the merge's *read* of stored state; licensed by the key-temporal-locality proofs or the
   transactionally-checked recurrence declaration (§"Key temporal locality").
2. **No-op write elimination** (write-side) — a maintenance write is skipped **iff** the row's
   applied effect is proven to be the identity, per row *by evaluation*: an exact
   `IS DISTINCT FROM` comparison over every column that can differ under the cell's trigger
   (comparing only the mutation-sensitive group is sound *because* the other groups are proven
   insensitive). Suppression may never skip **evaluating** a scanned input — restricting what is
   *computed* is a separate concern with its own static licence (the delta-restricted
   enrichment compute, `model_transforms.md`, licensed by the skeleton-source-closure proof). A compared
   column must be a pure function of the processed inputs; a column that legitimately varies run
   to run (`contract: plausible`, run-pinned `NOW()`) is incomparable, and a cell containing one
   refuses the conditional technique, fail-closed. At a fixed `S` the suppressed and
   unconditional variants produce identical state — interchangeable in the strongest sense of
   §"Per-cell admission", so choosing between them is a cost-model/`prefer`/`technique` matter.
   `model_transforms.md` catalogues the two physical realisations: change-suppressed MERGE (a
   matched-arm `IS DISTINCT FROM` predicate, dialect-split on the unmatched-by-source side) and
   the staged-candidate conditional DELETE+INSERT (the merge-less realisation for a backend
   without `MERGE`), both licensed by region row identity plus per-column change comparability.
3. **Write-eligibility clamps** — forbidden on the key grain; derived-only on the partition
   grain (the horizon above).

Categories 1–2 preserve the invariant bit-for-bit at fixed `S`; category 3 is different in kind
— it bounds which inputs enter `S` at all. A suppressed write is the write-side dual of slice
pruning (the proof is the per-row equality just evaluated), never a clamp. Two catalogued
transforms read a *derived* forward reach without being write clamps: the dimension-driven
horizon-bounded MERGE (a scan/recompute bound on the enrichment recompute, not the write) and
the horizon settled-delay/tail-rewrite mechanism (partition-grain forward-reach machinery); both
in `model_transforms.md`.

### Partition-local maintenance (the K8 guardrail)

A cell's per-`(cell × source)` locality verdict is the **partition-locality projection**
(`model_properties.md` owns the proof, including the cross-axis predicate requirement). This
section owns the policy consuming the verdict: emitted maintenance SQL must carry the partition
predicate on **both** the scan and the merge/overwrite target — a bound stated only on a
non-partition column is one the storage layer cannot prune by. Under the default `scan_bounds`
(`require: partition_local`, `on_violation: error`), a non-local cell refuses
(`MaintenanceScanUnbounded`) unless the source carries `allow_full_scan: true`; `max_lookback`
additionally refuses a derived span wider than the operator's stated expectation. The guardrail
never modifies a clamp — it only refuses or warns (§Surface "Maintenance overrides").

### Statement emission (single owner)

The physical statements a run executes for a cell — the region `DELETE`+`INSERT` pair, the keyed
fold `MERGE`, the column-scoped `MERGE`, the in-place `UPDATE`, the first-run
`CREATE TABLE … AS` — are produced by pure emitter functions in the maintenance layer
(`smelt-logical`): the statement-level counterpart of "one derivation, many consumers". An
emitter is a pure function from plain data — target table, region literals, key columns,
combiner-rendered set expressions, the compiled/clamped SELECT body, a dialect tag — to an
ordered statement group plus its transactional requirement (a paired `DELETE`+`INSERT` is one
transaction: a failed `INSERT` rolls back its `DELETE`). Backends *execute* emitted statements
(connections, transactions, blocking dispatch) and never author maintenance-statement text;
dialect differences (e.g. `MERGE … UPDATE SET *` needing a full-row source projection versus an
explicit column-list `SET`) live in the emitters as dialect-keyed variants.

Three deliberate exclusions, all warehouse-resident bookkeeping owned per dialect by
`smelt-state`, each interleaved transactionally with the write it describes but not itself a
maintenance statement:

- the reconciliation ledger's DDL/DML (§"The reconciliation ledger");
- the observed-output-delta record (§"The graph layer");
- the fingerprint sidecar's own storage — table DDL, digest-refresh upsert, GC delete
  (`sources.md` §"The fingerprint sidecar"). The sidecar's **diff query** is the one exception
  within that feature: which source keys count as "changed" is a derived maintenance-relevant
  comparison, so it IS emitter-authored
  (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`).

Non-maintenance SQL (introspection, seed loading, schema-evolution DDL) is outside this rule.
Single ownership is what makes maintenance SQL *observable*: the same emitters serve execution,
the conformance equivalence gates, and `smelt explain --show-sql`, so printed SQL cannot drift
from executed SQL.

### The definition-change trigger

A model gaining output fields is a trigger of its own kind: the added group's processed-input
vector is `∅` over every existing region, and its backfill advances `∅ → current`, touching only
the new group. The classification of an added field — `SkeletonAdd` / `PureBackfill` /
`UpstreamRederive` — is the definition-change column classification proof
(`model_properties.md`); this section owns the plan-level policy each maps to:

- `SkeletonAdd` (identity / grouping / dedup / ordering) is a **grain change**, refused as a
  column backfill (`MaintenanceSkeletonColumnAdded`) — the honest plan is a recompute,
  effectively a new model.
- `PureBackfill` lands in the 2×2's targeted-write column as an in-place `UPDATE` (no upstream
  read); `UpstreamRederive` lands there as a column-scoped `MERGE`, keyed where the source is
  keyed, inheriting each read source's partition-locality verdict unchanged.
- Fields added together factor by shared mutation-sensitivity, one backfill op per group. The
  backfill of a newly-added group is **always full-input**, even for a column whose ongoing
  algebra folds — there is no prior state of that column to fold onto.
- **Group convergence:** a field co-sensitive with an *existing* group still instantiates at `∅`
  and forms its own catch-up group; mid-catch-up, a delta folds into the sibling group but is
  refused on the new group's unbackfilled regions (never fold ahead of the entry). The groups
  merge only once the new group's processed vector equals its sibling's over every region.
- **The backfill is atomic with the column's own migration.** A `PureBackfill` field's
  physical column and its backfilled values are created by the SAME statement group as the
  schema migration that adds the column — never a separately-dispatched write that could
  observe the column already added but not yet backfilled. Concretely: the backfill's
  `UPDATE` is folded into the migration's `ADD COLUMN` statement group before it executes,
  the same mechanism a declared `backfill:`/`default:` frontmatter directive already used.
  A group failure (a transactional-DDL backend) leaves neither the physical column nor the
  saved deployed-schema snapshot changed, so the next run's diff still sees the column
  missing and retries the whole migration+backfill together — there is no window in which
  the deployed-schema snapshot can outrun the column's real values (cross-ref §Known
  Divergences for the one case this does not cover).

### The reconciliation ledger

The plan's bookkeeping is a `(output-region × column-group)` ledger; each entry records the
processed-input vector `S_{i,g}` of that region-group. Storage is graded by algebra: additive
groups record **delta identities** (never-fold-twice needs them); idempotent groups record only
a **frontier** watermark (re-folding is harmless). Two operations: *fold* (refuse if the delta
is already in the entry's processed set; otherwise combine and extend) and *recompute-reset* (a
region recompute resets every intersecting entry to exactly the input it read). Region↔window
attribution is exact under key temporal locality or explicit footprint tracking; a delta is
attributed to the unique ledger region containing its footprint. Schema evolution is a ledger
operation: adding a group instantiates its entries at `S = ∅` (§"The definition-change
trigger").

### The graph layer

**Edges.** A dependency edge is `downstream reads upstream` under the downstream cell's derived
scan clamp, between two partition axes whose grain is the declared `timeseries.granularity` of
each node — never per-edge, never derived from the SQL (the classifier only *checks* the
declaration, e.g. against a `date_trunc` grouping). Clamp margins ceil **outward** to whole
partitions; each hop aligns its result outward to the receiving axis's grain. Outward maps are
monotone, so sufficiency composes; narrowing never does. **Widen-never-narrow** is the graph
layer's composition law.

**Upstream model edges.** A maintained model's ref to another maintained model in the same
project is a plan edge of the same standing as a `sources.*` ref: the upstream model's own
validated `timeseries:` declaration supplies the clock the downstream creation cell is clamped
by, and scan bounds compose through the chain exactly as the propagation graph composes them. An
upstream-model ref whose clock cannot be derived (the upstream declares no `timeseries:` and
none is inferable) is a recorded refusal on that cell (`MaintenanceReachNotDerivable`, naming
the edge) — never a silent drop. A ref to a `full`-mode or view upstream derives no creation
cell (there is no incremental delta to receive); it participates in mutation/backfill triggers
only. For forward propagation, `--source` accepts either a declared source or an upstream
maintained model; a model's landed delta is the output window a completed run wrote for it.

**Forward propagation — what must run.** Runs are driven by **what landed**, per source, as
partition intervals on that source's own axis; a cron tick is only the poller. Processing nodes
in topological order, each node's merged dirt reflects through each outgoing edge — an upstream
delta of `[a, b)` dirties downstream `[a − after, b + before)` — accumulating:

- **per-edge dirt** `(model, upstream) → intervals`: keys the trigger cell — the plan cell for
  that inbound source runs over exactly these regions (recompute for a driving-source delta,
  column-scoped merge for an enrichment delta);
- **per-model dirt** (the union across inbound edges): what that model's own consumers see as
  *their* upstream delta.

Running exactly the per-edge dirty regions with their cells must leave every model equal to a
full refresh (sufficiency); partitions outside the dirty set are never scheduled. A delta on a
source nothing reads, or an empty delta, propagates nothing. A delta on an **unclocked** source
dirties the **whole model** for every mutation-sensitive consumer — never a silent no-op (the
cell was only admitted under `allow_full_scan`, so the full-table run is a declared cost).

**Backward resolution — what must exist.** Given a target model and period `[s, e)` (aligned
outward to the target's grain), walking the ancestor sub-DAG in reverse topological order and
applying each edge's clamp directly — `[s, e)` requires upstream `[s − before, e + after)` —
yields, for every ancestor, the partition intervals that must exist (a data prerequisite for a
raw source; a build region for a model) plus the build order. This is the bounded
test/validation build: stage exactly the resolved source slices, build bottom-up, and the target
period equals a build over complete history. The required slice of an unclocked source is the
whole table. The two directions are **adjoint, not inverse**: `forward(backward(P)) ⊇ P`.

**Observed deltas on model edges.** A model edge's propagated delta follows the same
landed-delta refinement as a source edge (`sources.md` §"Landed-delta (derived, recorded)"):
where a run recorded an **observed output delta** — the changed-row set a conditional write
(§"Windowed maintenance and the horizon", category 2) actually touched, restricted to comparable
columns — that set, projected onto the model's own partition axis, is the edge's delta; absent a
record the edge falls back to the run's written window, the coarser and always-correct form
(widen-never-narrow). The record is warehouse-resident, alongside the reconciliation ledger, and
written in the **same backend transaction as the write it records** — a delta visible without
its write, or a write without its delta, breaks propagation soundness. **Trust boundary:** an
observed delta is trusted because the state is smelt-owned, written only by smelt's own
conditional-write path; there is no out-of-band-edit tripwire — an external mutation to the
target table between runs is not detected (an explicit Open Question, §Known Divergences).
Empty and absent are distinct: an empty recorded delta means the run executed and changed
nothing (a real, propagatable fact); an absent record means no delta was recorded, and a
consumer must not conflate the two. This composes with the derived settle bound
(§"What the composed shape enables"): a stable upstream chain degenerates to empty-delta no-op
propagation with a provable horizon behind it.

**Refusals.** The graph refuses fail-loud (`MaintenanceGraphUnsupportedNode`) on: a cyclic edge
set; a **self-referential** model (a table-graph cycle that is a DAG only when time-unrolled —
admissible in principle iff its self-clamp is strictly time-backward, with forward dirt running
to the frontier and backward resolution reaching the model's basis/checkpoint); a **keyed node
without an admitted time axis** (no partition axis for interval dirt — treating it as day-axis
would be wrong-and-quiet). A locality-admitted time-partitioned keyed output is **not** refused:
it is a clocked node whose edges use its declared granularity, and whose outbound dirt is the
key→partition projection of what its runs changed — exact under locality routes 1–2, widened
backward by `r` plus margins under route 3 (§"Key temporal locality").

### Shape profiles

A maintained model is a **composition** of things owned across the spec set:

- **Properties** — what its SQL can be proven (or declared) to be (`model_properties.md`);
- **Transforms** — the physical mechanisms a property licenses (`model_transforms.md`);
- **World-facts** — what its sources declare about the world (`sources.md`, `timeseries.md`);
- **Output shape** — the declared facts and their derived grain (§Surface);
- **Scope maps** — the per-input dispatch the plan's changed-input axis names
  (§"The plan matrix").

Each profile section below (and `materialized_view.md`, the one profile whose maintainer is the
engine rather than smelt) therefore opens with a **composition table** naming, for that shape:
the properties it requires, the world-facts it consumes, the transforms its default plan drives,
and the invariant specialisation it upholds. A profile's normative content is exactly that table
plus the profile's own **local** machinery, defined in full below it. A profile never
re-specifies a capability that a capability spec or a shared section of this spec owns — every
capability has one home (§Design "Placement is definitional").

### The partition grain (`grain: partition`)

The partition-addressed shape: a complete table with a monotone `partition_column`, kept current
by the recompute-a-region corner (partition DELETE+INSERT) as its default plan. Its declared
surface is §Surface "Partition-grain declaration".

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: partition` — a complete table with a monotone `partition_column`, addressed by partition, not by key | `models.md` §"Refresh axis" |
| **Properties (required)** | event-time monotonicity trace; column nullability gate; unified bound/reach derivation; frame-reach taxonomy; injection-point / pushdown-depth; partition alignment (scoped); driving-fact / anchor resolution; determinism (run vs row) + nondeterminism predicate + taint; body-structure classifier; set-operation distribution; static-seed detection; window-independence / ordered-execution | `model_properties.md` |
| **World-facts (consumed)** | the timeseries clock (`event_time_column`/`partition_column`/`granularity`); source mutation profile and lateness margin; the column-scoped equivalence contract (`columns.<c>.contract`) | `timeseries.md`, `sources.md`, `models.md` |
| **Default plan (recompute corner)** | source-filter pushdown; partition DELETE+INSERT; output-window derivation (partition-column skew inversion); outer output-clamp; two-layer widened-scan + exact output clamp; compile-time pinning | `model_transforms.md` |
| **Admission** | every check below is one instance of §"Per-cell admission" evaluated for the recompute-a-region corner over a partition-grain output (§"Safety checks") | this spec |
| **Invariant upheld** | per-partition equivalence — the partition-grain strengthening of the equivalence invariant and of the plan's `S`-vector refinement | §"The equivalence invariant" |

The machinery below is partition-grain-**local**.

#### Execution model (DuckDB, current)

For a run with run window `[start, end)`, the recompute corner drives four transforms from
`model_transforms.md`:

1. **Partition DELETE** from the output table where `partition_column` falls in the **derived
   output window** — the run window pushed through the model's declared partition-column
   relation (`model_transforms.md` §"The output window is derived, never assumed"): identity
   when `partition_column` tracks event time (output window = run window); skew-inverted when
   `partition_column` is derived and skews away from the driving date column (a Form B
   relation). For such a **write-rebasing model** — e.g. a session keyed by
   `session_start_date` gaining events the next day, `before = after = 1 day` — the output
   window for run `[D, D+1)` is `[D−1, D+2)`, so the DELETE covers **every** partition the
   INSERT will write, including the prior-day partition the new data reaches. Deleting only the
   run window would strand the skew-reached partition stale forever: no later run's window
   contains it.
2. **Outer output-clamp** — inject `WHERE partition_column >= out_start AND partition_column <
   out_end` at the outermost SELECT, constraining the model's *output* to the same derived
   output window the DELETE covers. This step is **dropped for the transparent slice** (exactly
   one timeseries source, zero-margin bound `Bounded(_, 0, 0)`, no partition-column skew): the
   per-source pushdown filter already *is* the output clamp. A genuine lookback margin, a
   partition-column skew, or a second timeseries source keeps the outer clamp — scan window and
   output window are then distinct, load-bearing windows. Each written partition's **scan** is
   sized from the derived output window's reach, never the run window's.
3. **Source-filter pushdown** — inject a per-source `partition_column` filter on each
   `smelt.<path>` reference, derived from the model's SQL. Sources without a `timeseries:`
   declaration are lookups: no bound, read in full.
4. **INSERT** the resulting query's output into the output table.

DELETE range and output clamp derive from **one** window, so the contract is idempotent for any
write-window width: re-running the same `[start, end)` under fixed input converges to the same
state. The derived output window is a range to be **covered**, not a mandate for one statement —
backfill chunking (§"First-run and backfill") splits it into sequential DELETE+INSERT pairs,
each chunk's scan sized from that chunk's own reach.

#### Strategy enum (backend-internal)

Strategy is not declared on the model — it is derived per cell. For the recompute corner,
backends pick a physical strategy from the model's config and their capabilities:

```rust
enum IncrementalStrategy {
    DeleteInsert,    // DELETE matching partitions + INSERT
    Append,          // insert-only; no dedup
    InsertOverwrite, // replace entire partitions atomically
}
```

DuckDB always uses `DeleteInsert`. A partition-shaped output's creation/backfill cells are
region-addressed; a pure partition grain (no declared identity) has no keyed addressing at all.
Keyed `MERGE` is the addressing a *dimension-change* cell derives on a composed
clock-and-identity output (§"Per-cell write addressing") — per-cell, driven by what changed, not
tied to a grain. A backend may select only a strategy that preserves the declared shape's
invariants; `Append` is unreachable until it is gated on ledger-verified unwritten windows
(§Known Divergences).

#### Run window vs partition granularity

The CLI `[--event-time-start, --event-time-end)` range declares a **run window**, not a
per-partition invocation. Within the alignment rules (§Surface "CLI"), run-window size and
partition granularity are independent: a daily-partitioned model run with a 30-day window is
**one** engine query and **one** partition-aligned DELETE over the 30 partitions followed by one
INSERT. Backfilling 60 days is one `smelt run`, not 60 invocations. Per-partition equivalence
holds regardless of run-window size.

The declared `timeseries.granularity` (`g_run`, governing run-window alignment) must be at least
as coarse as the granularity actually implied by the `partition_column` projection's
truncation/grid transform (`g_part`), derived independently from the SQL rather than trusted: a
model whose `partition_column` is `DATE_TRUNC('day', event_time)` has `g_part = day`, and
declaring `granularity: hour` on it is rejected — an hourly run window would misalign the
DELETE+INSERT contract. `g_run >= g_part` is checked under the closed enum's coarseness ordering
(`hour < day < week < month < quarter < year`, `timeseries.md`). When `g_part` cannot be derived
(an opaque projection), the comparison is skipped — undecided, not a positive disproof — and
only the declared-granularity alignment check applies. A sub-`g_part` run window is rejected
with a message naming the minimum window, never silently widened.

#### Batch safety classification

The optimizer rolls the per-source bound map (`BoundResult` per source, from the unified
bound/reach derivation) into a single class per model, meaningful only inside the
recompute-a-region execution shape:

| Class               | Meaning                                                                 | Execution                                                |
|---------------------|-------------------------------------------------------------------------|----------------------------------------------------------|
| `FullyBatchSafe`    | All timeseries sources `Bounded(_, 0, 0)`; no temporal dependencies     | Single query for any run window                          |
| `BoundedSafe(n)`    | All timeseries sources `Bounded`, with `n = max(before + after)` > 0    | Auto-sized chunks (3× context, clamped 7–90 partitions)  |
| `PerPartitionOnly`  | One or more timeseries sources `Unbounded` (cumulative-across-history)  | One partition at a time, sequential                      |

`n` is rendered in the source's partition-column unit and is the same value the source-filter
pushdown reads. A model with **any** `NotDerivable` source is **refused at planning time**, not
assigned a class (`MaintenanceReachNotDerivable`): the diagnostic names the offending construct
and its source-map points at the original SQL; the author rewrites into a derivable form, and
there is no silent downgrade to full refresh (§"Validator, not chooser").

**Wide single-batch builds.** When `FullyBatchSafe` yields a single batch spanning more than 30
partition periods, smelt warns and recommends `--per-partition` or `--batch-size <n>`; either
flag suppresses the warning (the user has opted into a safe batching shape).

#### First-run and backfill

A first run (no output table) and a backfill (re-run of a written range) follow the same
DELETE+INSERT contract — the DELETE is a no-op when the partition is absent. The planner picks a
chunking shape from the batch-safety class:

| Class                | Chunking                                                                                   |
|----------------------|--------------------------------------------------------------------------------------------|
| `FullyBatchSafe`     | A single DELETE+INSERT pair covers any `[start, end)`.                                     |
| `BoundedSafe(n)`     | Auto-sized sub-ranges (3× context, clamped 7–90 partitions), each one DELETE+INSERT pair, sequential in temporal order. |
| `PerPartitionOnly`   | One partition per iteration, sequential, temporal order.                                   |

- **Self-referential first-run bootstrap.** A non-self-referential model's first run creates its
  target directly with `CREATE TABLE … AS SELECT` over the first batch. A **self-referential**
  model (§"Window independence and self-referential models") cannot: its first batch reads the
  target via `smelt.<self>`, and no engine resolves a table to itself mid-creation. When the
  target does not exist, the runtime first materialises an **empty** target carrying the model's
  inferred output schema, then executes every batch — including the first — as ordinary
  partition DELETE+INSERT. The self-read over the empty table yields no prior state for the
  first partition, so the trajectory built from there is identical to seeding the table by hand.
  The bootstrap is keyed only on "does the target exist yet".
- **Calendar alignment.** When per-partition execution is forced (or `--per-partition`
  requested), `Month`/`Quarter`/`Year` batches advance by true calendar units, landing on
  month/quarter/year boundaries regardless of month length; `Day`/`Week` use fixed 1-day/7-day
  steps.
- **Output grain may be finer than partition grain.** A model whose `partition_column` holds
  monthly boundaries may emit daily/hourly rows within them; batch-splitting operates on the
  partition grain and reads/writes finer rows in their entirety within each batch.
- **Per-chunk transaction boundary.** Each chunk's DELETE+INSERT is one backend transaction:
  INSERT failure rolls back the chunk's DELETE; earlier committed chunks do not roll back —
  partial progress is intentional, since each chunk is idempotent.
- **Failure mode.** A run halts at the first failed chunk and exits non-zero; re-running the
  same `[start, end)` resumes correctly because every committed chunk is idempotent.
- **Late-arriving data (interim guidance).** smelt does not auto-re-run partitions when data
  arrives late. Interim mitigations: trail `--event-time-end` behind real time by the source's
  known latency, or run overlapping ranges (e.g. always re-process the last 7 days). A planned
  automated mechanism is per-column `data_latency:` (§Known Divergences). The contract-level
  statement is the derived horizon (§"Windowed maintenance and the horizon"): a late arrival
  past the derived clamp is silently excluded, and surfacing it is a model-author +
  data-quality concern; the mitigations only widen the window a late row can still land in.

#### Per-partition equivalence

For every partition `p` in the run window `[run_start, run_end)`:

```
partition_grain_run(model, [run_start, run_end)).where(partition_column = p)
  == full_refresh(model).where(partition_column = p)
```

This is the partition-grain strengthening of the equivalence invariant and of the plan's
`S`-vector refinement, independent of run-window size.

**Column-locality.** The equality holds for **local** columns — those whose value depends only
on source rows visible within the model's source-filter ranges. A column depending on history
outside those ranges (a cumulative aggregation, connected-components, backward-fill) is **not**
equivalent: its per-partition value reflects state at run time, not the final cumulative state.
Such a column forces its source to `Unbounded` and the model to `PerPartitionOnly`; the run is
correct as-of-the-run, just not equal to a full refresh with final input.

**Equivalence is up to full-refresh non-determinism.** The equality is bit-identical on
deterministic columns. A `contract: plausible` column need only be a *plausible full-refresh
value*. This never extends to a column that governs which rows exist, where they are
partitioned, or how they are deduplicated (§"Safety checks").

#### Safety checks (per-cell admission for the recompute corner)

The optimizer rejects a partition-grain model whose SQL uses constructs that break the
partition-DELETE-then-INSERT contract (`PartitionGrainNotSafe`). Each check applies a shared
`model_properties.md` proof to discharge one §"Per-cell admission" obligation for the
recompute-a-region corner over this output shape. Each is individually disabled via
`safety_overrides.allow_<check>: true` (opt-in, recorded).

| Check | Admitted when | Obligation instantiated |
|---|---|---|
| **Window functions** | `OVER (PARTITION BY <keys>)` where `<keys>` ⊇ `partition_column` (partition alignment, scoped over the window `OVER`) — every window evaluates within a single partition. Also admitted when `PARTITION BY` omits `partition_column` but the frame is a bounded `RANGE BETWEEN INTERVAL '…' PRECEDING` with no `UNBOUNDED` bound (frame-reach taxonomy — a derivable reach the source read widens to cover). `UNBOUNDED PRECEDING`, or an `OVER (...)` with no `PARTITION BY`, is never admitted this way. Escape hatch: `safety_overrides.allow_window_functions`. | Obligation 4, bounded reach |
| **`HAVING`** | the enclosing scope's own `GROUP BY` key ⊇ `partition_column` (partition alignment over `GROUP BY`) — every group is scoped to one partition value. | Obligation 4, bounded reach |
| **`DISTINCT`** | `partition_column` is projected in the same scope (partition alignment over the select list) — rows can only collide within a partition. | Obligation 4, bounded reach |
| **`LIMIT`** | never — a row-count cap never commutes with the partition filter: which rows survive depends on which other rows are present, and that differs between a run and a full refresh. | fails obligation 4 unconditionally |
| **Subqueries** (`SELECT … FROM (SELECT …)`) | rejected unless overridden. A `WITH`-clause CTE is *not* gated by this structural check — CTE bodies flow through bound derivation via the body-structure classifier; only a subquery nested in FROM/JOIN is. | Obligation 4, bounded reach |
| **Non-deterministic functions** | confined to a payload column with `contract: plausible` (below). | Obligation 6, well-defined groups |

All partition-alignment checks are evaluated **per scope**: a `UNION` branch's own
`HAVING`/`DISTINCT`/window is judged against that branch's own key set, never inheriting
alignment from a sibling or the outer query (set-operation distribution governs the branches).

**Non-determinism and the payload rule.** The profile consumes the determinism (run vs row) +
nondeterminism predicate + taint property (`model_properties.md`). A non-deterministic value is
admitted only when it flows **exclusively** into a column declared `columns.<c>.contract:
plausible` — a payload written once per window and never read back to place, filter, group, or
dedup a row. The taint check enforces three hard exclusions, rejecting regardless of the opt-in
and naming the offending position: the `event_time_column`/`partition_column` expression; any
`unique_key` column; any row-set-membership or grouping position (`WHERE`, `HAVING`,
`JOIN … ON`, `DISTINCT`, `GROUP BY`, a window's `PARTITION BY`/`ORDER BY`/frame). The
run-nondeterministic class (`NOW()`/`CURRENT_*`) is additionally admitted as a **direct**
SELECT-list projection even without `contract: plausible`, because compile-time pinning
(`model_transforms.md`) freezes it once per run — every row of a run sees one value. The
row-nondeterministic class (`RANDOM()`/`UUID()`) always requires the target column to be
declared `plausible`. Declaring an excluded column `plausible` is a configuration error. The
blunt `safety_overrides.allow_nondeterministic` drops the guardrail wholesale and is
discouraged.

#### Event-time outer-visibility

The outer output-clamp injects `WHERE event_time_column >= start AND event_time_column < end` at
the outermost SELECT, so `event_time_column` must be **accessible** there. A plain
`UNION`/`INTERSECT`/`EXCEPT`, or a `UNION ALL` whose branches cannot be proven traceable, would
bind the clamp to only the first branch; a subquery FROM that does not project
`event_time_column` references an inaccessible column. Either is rejected with
`EventTimeColumnNotVisibleAtOuterSelect` before execution.

A `UNION ALL` is **exempt** when every branch's projection of `event_time_column` traces
`Traceable` (event-time monotonicity trace, distributed by set-operation distribution) back to a
real source's own partition column: per-source pushdown then narrows each branch independently
and the outer clamp's placement is immaterial. A `StaticSeed` branch is named and rejected; a
`NotTraceable` branch conservatively keeps the whole-model outer clamp.

#### Observing the per-source clamp

Because lookback is derived from the model's SQL rather than declared (§"Partition-grain
design"), the author has no declaration to read back; the derived clamp — the window
`partition_col ∈ [run_start − before, run_end + after)` each `smelt.<path>` reference is read
under — is surfaced instead, so the author can confirm the analyzer read their SQL as intended.
Two surfaces expose it, both using the ISO-8601 duration rendering of the bound:

- **`smelt explain` (`--json`)** — the per-cell `source_bounds` map reports, per source, its
  `source_partition_col` and derived `(before, after)` offsets; with a concrete run window it
  additionally resolves the scan window.
- **Editor hover (LSP)** — hovering a `smelt.<path>` reference in a partition-grain model shows
  that reference's clamp alongside the schema/column readout.

| Outcome | Readout |
|---|---|
| `Bounded(c, 0, 0)` | read partition-by-partition; no lookback or lookforward |
| `Bounded(c, before, after)` | the window `c ∈ [run_start − before, run_end + after)`, with `before`/`after` shown |
| `Unbounded` | read across all history (cumulative); forces `PerPartitionOnly` |
| lookup (no `timeseries:`) | read in full; not a pushdown candidate |

A `NotDerivable` source surfaces the planning-time refusal diagnostic instead of a window.

#### Functions inside partition-grain bodies

Function expansion (`expansion.md`) runs **before** every analysis stage here — bound
derivation, source-filter pushdown, and the batch-safety sub-checks see the expanded CST, so a
`LAG()` inside a `smelt.define` body and one inlined at the call site are indistinguishable. The
outer output-clamp is injected at the outermost expanded query; pushdown reaches `smelt.<path>`
references that originated inside function bodies. **Opaque calls remain black boxes**: bound
derivation cannot read through `smelt.extern`/built-ins, so a model whose time-dependence hides
behind an opaque call is `NotDerivable` and refused unless a bound is provable from the
surrounding SQL (`planner_integration.md` §"Optimization boundary").

#### Window independence and self-referential models

Whether windows may be built **in parallel** or must build **sequentially in temporal order** is
the window-independence / ordered-execution property (`model_properties.md`), derived from the
dependency graph, never declared:

- **Window-independent (default).** Every window is a pure function of source rows in its own
  scan range; the lookback reaches into *sources*, never the model's own earlier partitions. A
  backfill may split into sub-ranges built in any order, including in parallel.
- **Window-dependent → ordered.** A **self-referential** model — one reading its own prior
  partitions via `smelt.<self>` (a running balance, a partition-by-partition state machine) — is
  in scope and still executes as partition DELETE+INSERT (it stays a partition-addressed table;
  it does not become key-grain), but its windows build **sequentially in strict temporal
  order**, and its backfill may not be parallelised or reordered. A self-edge the planner cannot
  prove converges partition-by-partition (a self-reference reading forward or across all
  history) is refused at planning time.

This is the same stateless/stateful spine that separates the partition grain from the key
grain: a self-referential partition-grain model is *stateful-ordered* in execution yet keeps the
partition-grain output shape — partitioned, and per-partition-equivalent within each window's
own input.

Ordered execution composes with the derived output window: a Form B skew relation — anchored on
a *non-self* source — rebases an `Ordered` model's write window exactly as it would a
window-independent model's, and ordering then applies over the *rebased* partitions, every one
building strictly sequentially. The self-edge itself is never a skew anchor: its own bounding
relation (the backward-bounded read proving the `Ordered` verdict) is a distinct convergence
mechanism, not a partition-column skew declaration, even when the self-referenced column shares
the model's `partition_column` name.

#### State ownership

smelt does not track watermarks, offsets, or run history for partition-grain models — the
backend owns computational state (DuckDB: table state + transactions; Delta/Spark: transaction
log + MERGE; Flink: checkpoints). Optional run-state tracking with gap detection is opt-in via
`state.mode: intervals` (`virtual_environments.md`); the on-disk layout is owned by
`run_state.md`. The one deliberate exception across the family is the key grain's transactional
merge ledger (§"The transactional merge ledger" and §"Key-grain design").

#### `partition_column` validation

Partition-column projection is owned by `timeseries.md` §"Constraints & Invariants" rule 1:
`partition_column` must appear in the model's output `SELECT` (and in the `GROUP BY` when
grouping is present), else `MalformedTimeseries`. This profile consumes that guarantee rather
than re-checking it.

### The key grain (`grain: key`)

The key-addressed shape: keyed state, one row per `unique_key`, kept current by the fold-a-delta
corner (keyed `merge_into`) as its default plan. Its declared surface is §Surface "Key-grain
declaration".

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: key` — the end-state per key, addressed by `unique_key`, not by partition | `models.md` §"Refresh axis" |
| **Properties (required)** | algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone) — they define the column families; driving-fact / anchor resolution; event-time monotonicity trace (the driving source's clock); once-write provenance; join-contribution monotonicity; input-delta discovery; key temporal locality for a time-partitioned output | `model_properties.md` |
| **World-facts (consumed)** | the timeseries clock of a clocked driving source; the source mutation profile; a declared key-recurrence bound where the recurrence route is declared rather than derived | `timeseries.md`, `sources.md` |
| **Default plan (fold corner)** | keyed `merge_into` (target-as-replica) sequenced by the windowed-keyed-maintenance driver, with source-filter pushdown on the driving source; the transactional merge ledger; for enrichment shapes, the dimension-driven horizon-bounded MERGE; the slice-pruned merge target under established key temporal locality | `model_transforms.md` |
| **Admission** | every check is one instance of §"Per-cell admission" evaluated for the fold-a-delta corner over a key-grain output (§"Admission matrix") | this spec |
| **Invariant upheld** | end-state equivalence — the end-state specialisation of the equivalence invariant; the oracle is the model's own SQL | §"The equivalence invariant" |

The machinery below is key-grain-**local**.

#### The two run shapes (derived, never declared)

The run shape is the keyed application of the input-consumption axis (`models.md`), derived from
the driving source:

- **Window-forward** — the FROM clause contains exactly one source whose resolved target
  declares `timeseries:` (the **driving source**, resolved by the shared driving-fact/anchor
  proof; zero clocked sources means snapshot-reconcile; two or more is
  `KeyedMultipleDrivingSources`). The run steps over the source partitions covered by
  `[run_start, run_end)` **in temporal order**: per partition, source-filter pushdown injects
  the window onto the driving source, the per-partition delta SELECT executes, and a
  `merge_into` folds the delta into the target with the per-column combiner map. Non-timeseries
  sources (lookups/dimensions) are read in full each step. If the target does not exist at the
  first step, it is created from that step's delta (`CREATE TABLE AS SELECT`).
- **Snapshot-reconcile** — no clocked source. The run re-scans the source whole, computes the
  per-key aggregation, and `merge_into`s the result: matched keys are overwritten, unmatched
  inserted. A key present in the store but **absent from the incoming scan is retained**
  unchanged; deletion requires an explicit mechanism (out of scope, §Known Divergences).

Out-of-order, parallel, or sliced-backfill window application is admitted **iff** the model is
order-independent (below); otherwise windows apply sequentially in temporal order.

#### Derived execution postures

Three model-level properties fold from the column families; each is derived, surfaced by
`smelt explain`, never declared:

1. **Re-run tolerance** — may an already-merged window be blindly re-merged over *unchanged*
   input? Holds iff every column is idempotent, i.e. no additive-fold column. A re-run-tolerant
   model's repeated window converges (`GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`); an
   additive model double-counts and must be refused (the ledger, below).
2. **Order-independence** — may windows apply out of order or in parallel? Holds iff every
   column's combiner is order-independent: the extremal/lattice, decomposed-fold, and proven
   once-write families qualify; the order-monotone overwrite family does not (its
   order-independence holds only up to ordering-key ties — §"Ordering ties"), so any model with
   an overwrite column executes windows sequentially in temporal order.
3. **Reprocessing refusal** — a window whose *input changed* since it was merged must not be
   re-merged for **any** family: an irreversible fold cannot un-see a removed contribution, and
   an overwrite cannot retract a superseded-by-nothing value (§"Reprocessing").

#### The transactional merge ledger

Every **window-forward** keyed model maintains a per-model **ledger** — a small backend table
recording each merged window — written **in the same backend transaction** as that window's
`merge_into`. By posture:

- **Additive-fold models** (not re-run tolerant): a run whose window is already ledgered is
  refused (`KeyedReprocessedWindow`) — exactly, not best-effort. Crash resume merges only
  unledgered windows; a run interrupted at window *k* of *n* resumes correctly by re-running the
  same range.
- **Re-run-tolerant models**: a ledgered window may be re-merged (a no-op on unchanged input);
  the ledger serves reprocessing detection and `--auto` bookkeeping, not refusal.

Snapshot-reconcile models keep no ledger — each run is a self-contained reconciliation.
The ledger is backend-resident and transactional with the write it describes; it is a
**correctness structure**, distinct from the opt-in run-state observability surface
(`run_state.md`). Rationale for why this does not violate the state-ownership doctrine:
§"Partition-grain design" ("smelt does not own state").

#### Admission matrix (column family × source shape)

The key-grain instance of §"Per-cell admission": each cell below is obligations 2 ("faithful
fold") and 3 ("combiner algebra class") discharged for one `(column family × run shape)` pair.
Fold families consume **events** (each row contributes exactly once — replayable,
retraction-free feed required); overwrite families consume **observations** (each row
supersedes — current-snapshot semantics required). Checked per column:

| Column family | window-forward (clocked source) | snapshot-reconcile (mutable snapshot) |
|---|---|---|
| additive fold | ✓ (ledger-enforced) | ✗ — re-folding state double-counts |
| extremal / lattice fold | ✓ | ✗ — observer semantics (below) |
| order-monotone overwrite | ✓ | ✗ — observer semantics (below) |
| once-write | ✓ (provenance proof) | ✗ — observer semantics (below) |
| decomposed fold | ✓ (ledger-enforced, graded additive) | ✗ — re-folding state double-counts, same as additive fold |
| plain overwrite | ✗ — order-dependent over events (`KeyedUnknownCombiner` names the `MAX_BY` fix) | ✓ (current-snapshot semantics) |

The three snapshot ✗ cells marked *observer semantics* are not double-count hazards — those
families re-merge safely — they are **equivalence failures**: `MIN(price)` folded over
successive snapshots computes *min ever observed* while a full refresh over the current snapshot
computes the *current* min; `MAX_BY(attr, updated_at)` retains a stale incumbent forever if a
mutation regresses the ordering value; `COALESCE`-once-write captures *first observed*,
unrecoverable from the current snapshot. Each is a different contract — a history *observation*,
not a recomputation — and is refused (`KeyedSnapshotSourceUnsupportedColumn`) rather than
admitted silently.

**Scope: fold-contributing sources, not every referenced source.** The replayable-feed
obligation binds each **fold-contributing source** — one whose columns feed an aggregate the
cumulative combiner folds — not every source the FROM clause names. A mutable source consumed
**only** through a covered enrichment cell (an `UpstreamMutation`-triggered column-scoped
`MERGE`) is admitted regardless of its own mutation profile: its post-creation mutations are
maintained by that separate cell, so the fold's obligation never reaches it. A source that is
**both** a fold input and a mutable enrichment stays refused (`MaintenanceNoAdmissibleTechnique`)
— its folded contribution really is un-retractable, and admission fails closed rather than
approximating which of a source's columns are "safe".

#### End-state equivalence: the SQL is the oracle

The key grain upholds the end-state specialisation of the equivalence invariant, and because the
body is required to be the aggregation itself (§Surface), the oracle is executable for every
admitted model — it is the model's **own SQL**:

- **Window-forward:** for any set `S` of processed driving-source partitions and any admitted
  ordering over `S`, the stored state equals the model SQL evaluated over
  `source.where(partition ∈ S)`. For overwrite columns the equality holds up to ordering-key
  ties (§"Ordering ties").
- **Snapshot-reconcile:** the stored row for every key **present in the current snapshot**
  equals the model SQL evaluated over that snapshot. Keys absent from the snapshot are retained
  — the stored table is the oracle's rows plus retained departed keys (the named carve-out,
  §"The equivalence invariant").

#### No write-eligibility clamp

A run merges **every** delta row it scans, into whatever key it names, however old that key is.
A derivable forward reach is computed and reported (`smelt explain`) but never gates admission
and never bounds which keys a run may touch — no scanned input is ever silently dropped. The
contract-level statement and its rationale live in §"Windowed maintenance and the horizon".

#### Key temporal locality (the time-partitioned output)

A keyed model may time-partition its output with a `timeseries:` block (grammar:
`timeseries.md`; the named columns must be projections of the model, and `event_time_column` may
name the partition column itself). Admission requires **key temporal locality** — a guarantee
that every stored row a run's deltas can touch lies within a computable **slice** of the
output's time axis. Locality is what lets the `merge_into` target scan be pruned to the slice,
and what lets downstream consumers window over the output.

Structural preconditions, checked before the routes:

- the run shape is **window-forward** — the partition values derive from the driving source's
  clock; snapshot-reconcile establishes no locality;
- `partition_column` names either a `unique_key` column or a non-key projection in the
  extremal-fold, order-monotone-overwrite, or once-write family, provably NOT NULL from a key's
  first stored row (`timeseries.md` validation rules);
- the block's `granularity` equals the driving source's granularity.

Any one of three **routes** establishes locality:

1. **Key-embedded** — `partition_column` is a `unique_key` column. A stored row's partition
   value is its key's own; a delta touches exactly its own partition values. Slice: the run's
   scan window, widened by the derived lateness/skew margins.
2. **Key-determined** — the partition projection is a per-key constant under the once-write
   provenance proof: a key-derived expression, or a declared functional dependency over a column
   present non-null on every input row. Every delta row carries its key's fixed partition value,
   so the slice is the delta's own partition values — exact regardless of key age (a years-old
   key prunes as tightly as a fresh one).
3. **Recurrence-bounded** — a **key-recurrence bound** `r` holds: every pair of input rows
   sharing a key lies within `r` of each other on the event-time axis. `r` is derived from the
   model's SQL where statically decidable; otherwise it is declared on the driving source
   (`sources.md`, `key_recurrence`). Slice: the scan window widened backward by `r`, plus the
   derived margins. A **declared** `r` is admitted only **checked**: the run verifies at merge
   time that no delta row matched (or would duplicate) a stored key outside the slice, and any
   violation fails the run transactionally (`KeyedRecurrenceBoundViolated`). A declaration can
   bound work; it can never silently drop data.

**Pruning is not a write clamp.** Slice pruning is no-op elimination on the merge's **target
scan**: rows outside the slice provably cannot match a delta key (routes 1–2) or are checked not
to (route 3). Every scanned delta row still merges — §"No write-eligibility clamp" is unchanged.
The governing principle is §"Windowed maintenance and the horizon": only proofs prune; a
declared bound is admitted only checked; no unproven bound ever refuses a write.

**Row movement.** Under routes 1–2 a key's partition value never changes. Under route 3 it may
move (an extremal or overwrite partition projection superseded by a late row); the merge updates
the stored row in place, partition value included, and both old and new values lie within the
slice by the bound. Movement does not change the derived postures — an overwrite column forces
sequential temporal order exactly as before.

**Per-slice equivalence.** With locality established, the invariant is additionally checkable
slice-by-slice: for any output slice, the stored rows equal the model SQL evaluated over the
source rows within the slice's derived reach — the keyed analogue of per-partition equivalence.

**The output as a clocked source.** An admitted block makes the output a clocked,
time-partitioned table: downstream partition-grain models receive source-filter pushdown against
it, and a downstream keyed model may take it as its clocked driving source — the clock
propagates through the DAG instead of stopping at the keyed stage. The output's **settle bound**
— how long a written slice may still change — is derived and surfaced by `smelt explain`: under
route 1 a slice settles with the source's lateness margin; under route 3 after `r` plus the
margins; under route 2 it never settles (a late delta may touch an arbitrarily old slice). A
re-written slice is *changed input* to downstream consumers, handled by the ordinary staleness
machinery (§"Interaction with `--auto` / staleness").

#### What the composed shape enables

The composed shape — key-addressed **and** time-partitioned — is not "keyed with an
optimisation"; several capabilities hold only in that form, which is why the two declared facts
must never be read as exclusive alternatives (§Surface "The declared shape"):

- **Propagation admissibility.** A bare keyed node refuses in the graph layer — it has no
  partition axis to carry interval dirt. A locality-admitted keyed output has one: it
  participates in forward propagation and backward resolution as a clocked node. The composed
  shape is the only way a keyed stage sits *inside* a propagation chain rather than terminating
  it.
- **Exact key→partition dirt projection.** Under routes 1–2 a stored row's partition value is a
  per-key constant, so a key-level change set projects to **exact** partition intervals — the
  keys' own partitions, no widening. Under route 3 the projection widens backward by `r` plus
  margins (widen-never-narrow). A composed node hands precise interval dirt downstream without
  any key-level dirt representation in the graph.
- **Slice-bounded no-op write elimination.** The conditional write (§"Windowed maintenance and
  the horizon", category 2) must read stored rows to compare against candidates. On a bare keyed
  output that read is the whole key space; on a composed output it is bounded by the pruned
  target slice — compare cost proportional to the slice, which is what makes suppression
  affordable at volume.
- **Settle-bound × observed-delta composition.** The settle bound (static: when a slice can no
  longer change) composes with the observed output delta (dynamic: which rows a run actually
  changed — §"The graph layer"): consumers skip settled slices unconditionally and skip
  unsettled slices whose observed delta is empty. A stable upstream chain degenerates to
  empty-delta no-ops with a provable horizon behind it.

The first two bullets bind at the graph layer, the third at statement emission, the fourth
across both; implementation status is recorded in §Known Divergences.

#### The maintenance boundary

On the algebraic ladder (§"The algebraic maintenance ladder") the keyed families sit on rungs 1
and 2: every catalogued combiner folds `(state, delta)` with no inverse and no history re-read.
The additive and decomposed-fold families sit on rung 1 and rung 2 respectively, and are
additionally **groups** (invertible) — what a future subtract-then-add reprocessing path would
exploit; the extremal/lattice, order-monotone-overwrite, and once-write families (the latter two
rung 2 for the state-widened spellings, §"Decomposed state (rung 2) in keyed models") are monoids
but not groups (a folded contribution cannot be un-seen), which is why reprocessing is refused
for them. Rungs 3–4 (group-rung retraction; the opt-in bounded-domain multiset) grow this shape
further without changing its contract; the transforms are catalogued in `model_transforms.md`
and the `bounded_domain:` budget declaration in `model_properties.md`. Beyond the ladder is
delegated to `refresh: materialized_view`.

#### Reprocessing

If a merged window's source data changes, re-running the ordinary reprocessing path does not
produce correct state for any family (posture 3). Before that refusal fires, the change **routes
to the repair family first** (§"The repair family"): a retraction or mutation whose affected keys
are discoverable and whose per-group slice is bounded recomputes just those groups, and no
reprocessing refusal is raised. This is a plan-level route, not a new mode or a user flag. The
rule refuses at planning time only when a repair obligation fails — the ledger says the window
was merged; `--auto` staleness says the input changed — with `KeyedReprocessedWindow` naming the
failing repair obligation and pointing at the two mitigations: `--full-refresh`
(truncate-and-rebuild), or a manual cascade rebuild. Subtract-then-add for all-invertible models
is a future path (§Known Divergences).

#### Ordering ties (order-monotone overwrite)

The pairwise combiner for `MAX_BY(value, ordering)`: the delta wins iff
`delta.ordering > target.ordering` (strict); **on equality the incumbent wins**. This is
deterministic given the processing history but not order-independent when ties occur across
windows — which is why overwrite columns force sequential execution (posture 2). Recommended
modelling practice: a composite, provably tie-free ordering expression (e.g.
`(updated_at, source_seq)`); the classifier cannot verify uniqueness and does not claim to.

#### Enrichment joins

A fact-to-dimension join that brings an enriching event in as a separately-arriving relation is
admitted when its per-key contribution is **provably monotone** — the join-contribution
monotonicity proof (`model_properties.md`): the contribution feeds only extremal, order-monotone,
or once-write columns and does not fan into a decrementing aggregate. The maintainability line
is monotone-vs-retractable **semantics, not join-vs-union spelling** — the join form is
normalised to the same keyed-monoid merge as the union form; only a genuinely retractable
contribution is refused (`KeyedRetractableContribution`). A re-scanned existence flag
additionally requires the dimension source to be declared `append_only` (`sources.md`); extremal
milestones are safe regardless. Where a dimension batch's forward reach `H` is **derivable from
the model's SQL**, the dimension-driven horizon-bounded MERGE (`model_transforms.md`) may clamp
the enrichment *recompute* to `[event_ts, event_ts + H]` — a scan-side bound that cannot
under-cover because it is derived; where `H` is not derivable, the transform is not licensed and
the enrichment evaluates through the ordinary widened scan. No declared value ever truncates a
recompute or a write.

#### Key-grain output shape

One row per `unique_key`; column names are the projection's `AS` aliases (or source column
names). By default there is no `partition_column`, no `event_time_column`, and no `timeseries:`
on the model; downstream consumers see the output as a lookup table read in full each run,
identical to any non-timeseries source. With an admitted `timeseries:` block
(§"Key temporal locality") the output is instead a clocked, time-partitioned keyed table —
still one row per key — that downstream consumers window over like any clocked source.

#### Functions inside keyed bodies

Function expansion (`expansion.md`) runs **before** the classifier: projection reading, GROUP-BY
inspection, FROM-clause walking, family classification, and pushdown operate on the expanded
CST. A `smelt.define`-resolved call is admitted iff its expanded body produces a catalogued
aggregator at the outermost expression position — the pattern functions (§Surface) are admitted
exactly this way, with no privileged treatment. Opaque calls (`smelt.extern`, non-inlinable
built-ins) in the projection list are rejected via `KeyedUnknownCombiner`.

#### Interaction with `--auto` / staleness

- **Window-forward:** stale driving-source windows are re-processed subject to posture —
  re-run-tolerant models re-step exactly the stale windows (safe by idempotence); additive
  models refuse re-processing of ledgered windows (`KeyedReprocessedWindow`) and steer to
  `--full-refresh`.
- **Snapshot-reconcile:** the model is treated as always-stale; every `--auto` run reconciles.

### Interactions

- The invariant, ladder, horizon, and validator-not-chooser are owned above; the plan's
  per-cell theorem is the `S`-vector refinement of the invariant, and per-cell choice operates
  strictly inside validator-not-chooser.
- Output shape/grain declaration and the refresh trichotomy are owned by `models.md`; the plan
  validates against them. The **declaration law and litmus rule** (`models.md` §Design) —
  whether a fact is declared, derived, or implied, and whether a proposed combination earns a
  new peer shape — are likewise owned there; this spec consumes them.
- **Input consumption** (`models.md` §"Input-consumption axis"): which input rows are new is a
  derived, cross-cutting axis (mutation-profile world-fact → input-delta-discovery proof →
  re-scan/probe transform). Moving along it never changes the equivalence contract, only what is
  scanned. The default is windowed; full scan is the surfaced fallback (§"Windowed maintenance
  and the horizon").
- Source postures (`mutation_profile`, lateness, retention, delta identity, unique keys) are
  declared in `sources.md` and consumed by admission; their runtime tripwires live there.
- The technique primitives (`merge_into`, DELETE+INSERT, column-scoped merge, targeted
  backfill) are catalogued in `model_transforms.md`; the outer output clamp is the subquery wrap
  over the model's output schema defined there.

## Design

Each paragraph records one load-bearing decision and what was rejected. Deeper derivations live
in `docs/research/` and are cited by full path.

**Strategy content is derived; shape stays declared.** One model is not one mode — it is
simultaneously append-driven, merge-driven, and recompute-driven at different cells, so any
per-model strategy enum is a lossy projection; strategy is derived per cell. Deriving *shape*
too was rejected: it reintroduces the silent contract swap the declaration law exists to prevent
(a projection refactor could flip downstream consumption semantics with no diagnostic).
Shape-defining facts remain declared-and-checked.
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §10, §13.)

**One invariant; addressing is the real axis, and it is per-cell.** An earlier framing split the
contract into a per-partition equivalence and an end-state equivalence, one per shape. That was
miscast: order/set-determinacy falls out of the single invariant for every shape, and
per-partition equivalence is a *strengthening*, not a peer. What actually drives the physical
transform is how a write addresses rows — and addressing is a property of *a write*, not of *a
model*: the declared facts fix which addressings are available, and each cell derives its own.
The composed shape is the proof that addressing is intrinsic to the write: a late dimension
correction is keyed — it targets specific rows across many partitions — while the same model's
creation cell is region-addressed. A *declared model-wide* addressing token was rejected — the
per-cell plan already knows better.
(`docs/research/20260716-relation-contract-and-per-cell-addressing.md`.)

**The two write mechanisms stay binary per cell; locality is a refinement, not a third pole.**
Region-overwrite vs keyed-merge is the write-scope corner; which concrete pattern realizes a
corner is drawn from the open registry, so the mechanism set grows without the corner
distinction changing. Key temporal locality does not change how a keyed write is addressed — it
adds a proof about *where* addressed rows can live, licensing target pruning, a time-partitioned
keyed output, and per-slice equivalence. Promoting it to a third addressing pole was rejected:
it would suggest a different write primitive and identity requirement where there is none, and
it would misplace a per-model derived/declared fact as a shape property.
(`docs/research/20260705-keyed-time-superset.md`.)

**The axes compose; exclusivity is the recurring error.** Treating "partitioned" and "keyed" as
rival modes has repeatedly produced designs that forget the composed shape: DAGs whose clock
dies at a keyed stage, keyed nodes excluded from propagation categorically, conditional-write
costs sized to whole key spaces. The composed shape is deliberately first-class
(§Surface "The declared shape", §"What the composed shape enables"), and reviewers should treat
one-or-the-other phrasing anywhere in the corpus as a defect against those sections.

**Scope maps name the per-input dispatch.** Without the name, the run shape reads as a property
of the *model*, hiding that different inputs changing engage different targeted repairs (a fact
delta folds forward; a dimension delta probes and horizon-merges; a definition diff backfills
columns; a self-edge forces ordering). Naming the dispatch makes "what runs when this input
changes" an explainable per-input answer and gives future multi-clock driving-source work a
stable home. (`docs/research/20260705-keyed-time-superset.md` §5.)

**Factoring by mutation-sensitivity, not syntactic provenance.** A column that reads a second
input's *immutable-at-creation* value must not inherit that input's mutation-sensitivity —
otherwise the plan degenerates and the targeted cells are lost. This is what makes the
append-only declaration on a source load-bearing.
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §5.)

**Membership sensitivity is derived, never inferred from collapse.** The complementary hazard
to over-attribution is silent under-attribution: a mutable source read only in row-admission
position has empty value sensitivity, and a derivation that stopped there would leave its
mutations entirely unmaintained — no cell, no refusal, a quiet equivalence hole. Membership
sensitivity exists as its own derived kind so that admission for such a source is decided by
the join-predicate read it actually performs, and so that its repair is forced into the
membership-capable recompute family. A degenerate whole-model collapse that happens to cover
the source is not an acceptable substitute: collapse-admitted cells assign repair by accident
(a column-scoped merge for what is really a membership change) and vanish the moment the
collapse is fixed. (Established empirically: the keyed-enriched shape's dimension cell was
admitted solely by a collector misparse until membership sensitivity was made a first-class
derivation.)

**Per-edge dirt keys trigger cells.** The trigger taxonomy is per-edge: a dirty set merged per
model would erase which repair runs where, and two sources landing in one tick genuinely drive
different techniques over different regions of the same table.
(`docs/research/20260705-refresh-as-maintenance-plan/10-dependency-propagation.md` §3.)

**Widen-never-narrow.** Every approximation in the plan and graph widens: partial-day clamps
ceil outward, coarse grains align outward, whole-partition dirt over-runs, an unclocked delta
dirties everything. Widening costs compute; narrowing costs correctness silently. The declared
guardrails (`scan_bounds`) exist so the widenings are *visible* costs, refused by default when
unbounded.

**Granularity is declared, not derived.** The propagation grain governs downstream scheduling,
so deriving it from a `date_trunc` projection would let a refactor silently change scheduling
semantics; the declaration is checked against the derived partition grid instead
(§"Run window vs partition granularity").

**The clamp runs both directions.** Forward reflection and backward resolution are one edge
object run in opposite directions — the scan/footprint duality lifted to the graph. Keeping them
one object makes the test-build story (backward) automatically consistent with the scheduling
story (forward); the adjointness containment `forward(backward(P)) ⊇ P` is the honest statement
of their relationship.
(`docs/research/20260705-refresh-as-maintenance-plan/10-dependency-propagation.md` §2.)

**Offline cost measurement is first-class.** Because per-cell technique choice is
contract-preserving at fixed `S`, smelt may measure alternative physical plans over real data
offline and pin the cheapest (`smelt bakeoff`) — a capability per-query optimisers structurally
lack. The measurement is real, not simulated: each candidate executes the project's actual
`execute_project` pipeline against the project's own data in a disposable scratch schema.
Pinning is deliberately a human act: `--pin` only emits YAML for review, and an applied pin
remains subject to admission like any override.
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §11.)

**Windowed by default; full scan is the surfaced fallback.** Treating full-table recomputation
as the baseline and windowing as a per-shape optimisation inverts the real economics: a clocked
model can always be maintained over a bounded scan window, and only the absence of a clock
forces a wider read. Making windowing the default keeps the common case scalable and pushes join
optimisation to the engine over a safe widened scan, rather than smelt hand-computing minimal
deltas.

**The horizon is derived, not declared.** Trusting a declared horizon risks an under-estimate
that silently corrupts the clamp — dropping rows still within the model's reach. Deriving it
keeps clamps correct by construction; a declaration is admitted only as a *ceiling* that warns.
The consequence — a late arrival beyond the derived reach is silently excluded rather than
diagnosed — is accepted and documented (§"Windowed maintenance and the horizon"); surfacing
lateness is a model-author + data-quality concern. This can be softened later if a legitimate
need to widen beyond the derived reach appears; the safe default is derive-for-correctness,
consistent with derive-else-declare (`models.md` §Design).

**Validator, never chooser.** Auto-selecting or silently downgrading the declared shape was
rejected: it reproduces dbt's `strategy:` footgun where the effective contract is invisible. The
declared shape is authoritative; the machinery only proves or refuses it.

**Placement is definitional, not consumer-counted.** A capability whose verdict is stateable
without naming a shape profile lives in a capability spec (`model_properties.md` /
`model_transforms.md`); a capability meaningful only inside a profile lives in that profile's
section here (or `materialized_view.md`). Pushdown-depth is a SQL property and lives in
`model_properties.md`; backfill chunking is meaningless outside partition-grain execution and
stays in that profile. Every capability gets exactly one home — what lets `smelt:validate`
catch drift — without a mechanical consumer-count rule. The invariant and ladder live in the
shared sections because every profile cites them as its contract.

**Rejected alternatives, briefly.** A `strategy:` sub-knob (the invisible-contract footgun); a
dedicated `smelt-maintenance` crate (the derivation needs the tightest coupling to the sibling
classifiers; the module boundary is kept extraction-mechanical instead); qualifying the output
clamp to a resolved inner alias (answers a question the output clamp must never ask); a third
addressing pole for locality (changes no write primitive); per-edge grain declarations (two
declarations can disagree — resolved by the derived label + check-only assertion); a closed
write-pattern enum baked into the surface (bakes today's engines in). Deeper rationale:
`docs/research/20260705-refresh-as-maintenance-plan/` parts 01–10, with the decision-acceptance
records in `09-spec-readiness.md` §1 and `10-dependency-propagation.md` §11 of that directory.

### Partition-grain design

**Logical SQL is pure; the framework injects the time filter.** A model body never contains
`is_incremental()` or conditional full-vs-incremental branching — the same SQL is both
descriptions; the framework injects the outer clamp and drives pushdown. Jinja-style
`is_incremental()` branching was rejected because it splits one model into two implicit ones
that drift. The trade-off — accepting the framework's per-model filter shape — is policed by the
batch-safety analysis.

**DELETE+INSERT over partition columns, not MERGE, as the default.** MERGE was rejected as the
default because it requires a `unique_key` (not every model has one) and carries cross-engine
subtleties; it stays in the strategy enum for backends that opt in. DELETE+INSERT is idempotent
under fixed input and aligns with the partition-column safety analysis.

**Three-class batch-safety taxonomy.** A binary safe/unsafe flag was rejected — too many real
workloads are bounded-safe and need auto-chunking. A continuous safety score was rejected — the
user-facing decision is qualitative and maps directly to three backend execution shapes.

**Derive lookback from the model's SQL, not from frontmatter.** A `lookback_days:` annotation
would let declaration and logic drift. The trade-off — a model with implicit time logic refuses
eligibility and must be rewritten into a derivable form — is the right outcome. Deriving removes
the artifact the author would read to confirm behaviour, so the derived clamp is made
observable (§"Observing the per-source clamp") as the deliberate counterpart.
(`docs/research/20260521-incremental-as-planner-rule.md`.)

**smelt does not own state — scoped to the partition grain.** Owning a watermark store was
rejected: it duplicates engine state and opens a sync-correctness window. The key grain's
transactional merge ledger is the one deliberate exception across the family, and it avoids both
defects: backend-resident, written in the same transaction as the merge it describes, so it
cannot drift from the state it records. A consequence: a backend may only select a physical
strategy that preserves the declared shape's invariants — the partition-grain `Append` strategy
is unreachable until gated on ledger-verified unwritten windows.
(`docs/research/20260705-keyed-collapse-application.md` D7.)

**Non-determinism is opted in per column, and confined by proof.** Whether a column is
acceptable-to-vary is a value judgement only the author holds, so it is declared
(`columns.<c>.contract: plausible`) — the one place derive-don't-declare correctly yields. A
whole-model `allow_nondeterministic` boolean as the primary mechanism was rejected: it drops the
guardrail keeping non-determinism out of the skeleton roles. The per-column opt-in keeps the
guardrail and still proves, via taint flow, that the tolerance did not leak.
(`docs/research/20260703-model-updates.md` §9.2.)

### Key-grain design

**One shape; the column family is the pattern.** The running-aggregate, latest-value, and
milestone patterns share the output shape, invariant, transform, and key derivation — they
differ only in per-column combiner algebra, and every consequence (re-run tolerance, ordering,
ledger, reprocessing) is derivable from the SQL. By the litmus rule (`models.md` §Design), facts
that change only execution posture under an unchanged contract are derived, never declared — so
they must not multiply the refresh enum. Splitting them into peer modes was also rejected for a
decisive second reason: combiner intent is **per column, not per model** — one table can mix an
additive fold, an overwrite, and two extremal milestones, a shape no per-pattern mode can
express without materialising the same keyed state several times.
(`docs/research/20260705-unified-keyed-refresh.md`;
`docs/research/20260705-keyed-collapse-application.md`.)

**The SQL is the oracle.** The body must be the aggregation itself so that
`full_refresh(model SQL)` is an executable correctness oracle for every admitted model. A
bare-projection surface with mode-imposed dedup was rejected: its full refresh is not one row
per key, so the invariant would have no executable oracle and the mode would add semantics the
SQL does not carry. The plain-overwrite family (`ANY_VALUE`) exists to give the snapshot posture
an honest aggregated spelling under this rule.
(`docs/research/20260705-model-refresh-review.md` §1.1.)

**Derive `unique_key` and combiners from the SQL, not frontmatter.** The `GROUP BY` names the
key; each projection names its aggregator; the combiner is a fixed lookup. A config block
restating them re-introduces metadata-vs-SQL drift. If it is in the SQL, it is not also in YAML.
(`docs/research/20260521-incremental-as-planner-rule.md`.)

**No write-eligibility clamp.** A horizon-clamped merge (only keys newer than `run_start − H`
eligible) was rejected: it silently drops *scanned* inputs — the one silent-data-loss point in
the maintained family — and it is not needed for correctness, since merge work is proportional
to delta size. What a clamp would buy (settled-key GC, a work bound) is deferred optimisation
that must arrive as a package with late-fact accounting. Slice pruning under key temporal
locality is not such a clamp: it removes provably-unmatchable rows from the merge's *read* side,
or checks the declared bound transactionally, while every scanned delta row still merges.
(`docs/research/20260705-keyed-collapse-application.md` D6.)

**The time-partitioned keyed output is locality-gated, not a new mode.** The composed (key,
time) output absorbs the shapes that previously fell between the corners — event-grain dedupe
over a bounded redelivery window, per-(key, period) aggregates, and the clock-sink problem where
a keyed stage strips the timeseries property from the DAG. A peer mode was rejected: the form
shares the key grain's invariant, oracle, driver, ledger, and column families, differing by one
derived/declared world-fact — by the litmus rule that earns a gate, not a peer. The gate exists
because without locality the merge target is the whole key space and an output clock would
promise a partition structure the writes do not respect; the declared route is runtime-checked
because an over-optimistic recurrence bound would otherwise re-import exactly the silent
truncation the no-clamp rule prevents.
(`docs/research/20260705-keyed-time-superset.md`;
`docs/research/20260705-model-refresh-review.md` §3.2.)

**Observer semantics are refused, not smuggled.** Folding state observations (a mutable
snapshot) into `MIN`/`MAX`/once-write columns yields min-ever / first-observed values no full
refresh can reproduce — a genuinely different contract. Admitting it silently would put two
contracts behind one mode. The refused cells name the observer contract as the future opt-in
path (§Future Extensions).

**Ties: honest boundary, not fake proof.** Incumbent-wins plus mandatory sequential execution
makes overwrite columns deterministic-given-history without claiming an order-independence no
static analysis can prove. A last-processed combiner (no ordering column, order-dependent for
all rows) was rejected outright; the snapshot posture's plain-overwrite family serves that need
where it is well-defined.

**No `safety_overrides:`.** The partition grain offers per-check overrides because some of its
rejections guard partial-correctness properties a modeller may knowingly waive. Every keyed
rejection guards the equivalence invariant itself — a bypass would produce silently
order-dependent or double-counted state. The escape from a rejection is to remodel, or to move
to `refresh: materialized_view`.

**One windowed executor, shared.** The window-forward step loop is the
windowed-keyed-maintenance driver (`model_transforms.md`), parameterised by
`(classifier, merge-SQL builder)`. Per-pattern copies of the loop were rejected as four-way
drift risk; a consequence is that every consumer inherits the driver's granularity support
(§Known Divergences).

## Constraints & Invariants

### The contract, plan, and graph layer

- The **equivalence invariant** holds for every non-`full` model and on every ladder rung; a
  transform that cannot preserve it for a given model is refused, never applied approximately.
  Order/set-determinacy is its corollary for every shape; per-partition equivalence is a
  strengthening, not a separate contract.
- **Write addressing is per-cell, not per-model**, derived by the available-addressings rule
  (`available = declared contract facts × trigger/changed-input needs × equivalence invariant ×
  backend capability`) over the **open write-pattern registry**. The declared facts fix which
  addressings are available. A keyed write on a clocked output stays partition-scoped unless it
  provably cannot be. Key temporal locality refines keyed addressing with a derived slice bound
  without changing the addressing corner.
- **The write-pattern set is an open registry, not a closed enum.** New patterns are admitted by
  declaring their required contract facts and discharging the equivalence proof obligation; the
  `write:` pin is an open, fail-loud name; a pattern the target backend cannot execute is not a
  candidate. The stable contract is the admission function + equivalence gate, never the
  enumeration.
- Maintenance is **windowed by default** where the model is clocked; a full scan is a surfaced
  fallback, never the silent baseline. Always `scan window ⊇ write window`.
- The **horizon is derived**; a declared `horizon_ceiling` is a warning threshold only and never
  relaxes the clamp. Late arrivals beyond the derived reach are silently excluded — surfacing
  them is a model-author + data-check concern.
- **One home per capability and per rule.** The invariant, ladder, plan, and graph layer are
  owned here; properties in `model_properties.md`, transforms in `model_transforms.md`, the
  declaration law and litmus rule in `models.md`. No spec re-specifies another's content.
- **Proofs are fail-closed**: an undecidable construct rejects; a declared escape hatch may only
  widen eligibility, never substitute for a proof's default reject.
- The declared **`refresh:` value plus the shape-defining facts are the only shape surface**;
  `grain` is a derived check-only assertion, write addressing is derived per cell (steerable
  only via the validated `write:` pin), input-consumption is derived from the source. No
  `strategy:` sub-knob. The machinery **validates, never chooses**.
- **The plan is pure data, derived by pure functions, in one place** (`smelt-logical`);
  consumers never re-derive it. (Also an invariant in `architecture.md`.)
- **Maintenance statements have one author** (§"Statement emission (single owner)"); backends
  execute, never author. Printed, gate-verified, and executed SQL are the same emitters' output
  by construction.
- **Never fold a delta already reflected in the state.** Every fold consults the ledger; every
  region recompute resets the entries it overwrote. No path may merge a window twice.
- **Write window = output window**, per cell: the DELETE/merge target and the output clamp range
  over the same output-axis column and the same window, by construction.
- **Only proofs prune.** A declared bound is admitted only checked; a guardrail (`scan_bounds`,
  `horizon_ceiling`) may refuse but never modifies a clamp; no unproven bound drops a scanned
  input.
- **Fail-loud, fail-closed.** Every admission failure, non-local scan, skeleton-position add,
  and unsupported graph node is a named diagnostic; nothing degrades silently. The graph layer
  never silently under-runs: unrepresentable dirt widens to whole-model, never to nothing.
- **Widen-never-narrow** is the composition law of every interval operation (clamp ceiling,
  grain alignment, footprint reflection, backward widening).
- Out of scope, deliberately: content-aware delta pruning (an engine/CDF concern); file-level
  write-amplification minimisation (the engine's job — the plan guarantees the partition bound);
  cross-*project* propagation (project isolation, `architecture.md`).

### Partition-grain constraints

1. **The logical model is pure SQL.** No `is_incremental()`, no macros, no conditional
   branches; the framework injects the time filter.
2. **`timeseries:` is required for `grain: partition`** — a hard error at workspace load
   otherwise (`models.md` §"Constraint violations").
3. **Strategy is not on the model.** The backend chooses the physical strategy for the
   recompute corner's execution.
4. **smelt does not manage computational state** (partition-grain-scoped doctrine); watermarks,
   offsets, and run history live in the backend. The key grain's transactional merge ledger is
   the one deliberate exception (§"Key-grain design"). A backend may select only a physical
   strategy that preserves the declared shape's invariants; `Append` is unreachable until gated
   on ledger-verified unwritten windows.
5. **Output-filter injection is per-model; source-filter pushdown is per-reference.**
6. **Per-partition equivalence with full refresh holds** on all local, deterministic columns; a
   `contract: plausible` column need only be a plausible full-refresh value; globally-dependent
   columns are not equivalent (§"Per-partition equivalence").
7. **Idempotence under fixed input:** re-running the same run window on unchanged sources
   converges to the same output state.
8. **Granularity is closed under partition arithmetic.** Run windows align to whole granularity
   units; the declared granularity must be at least as coarse as the derived partition grid
   (`g_run >= g_part`); violations reject, never silently widen (§"Run window vs partition
   granularity").
9. **Safety-check overrides are explicit.** A `safety_overrides` entry names the specific check
   it bypasses; there is no global disable.
10. **No silent downgrade to full refresh.** A rejected or `NotDerivable` model is refused at
    planning time with a diagnostic (§"Validator, not chooser").
11. **`event_time_column` must be accessible at the outermost SELECT**, unless every `UNION ALL`
    branch traces `Traceable`; otherwise `EventTimeColumnNotVisibleAtOuterSelect`
    (§"Event-time outer-visibility").
12. **Non-determinism stays in the payload.** Admitted only into `contract: plausible` columns
    (plus run-nondeterministic direct projections under compile-time pinning); never in
    `event_time_column`, `partition_column`, a `unique_key` column, or any membership/grouping
    position. Declaring an excluded column `plausible` is a configuration error.

### Key-grain constraints

1. **Opt-in is `refresh: incremental` + declared identity** (storage implied `table`);
   `unique_key` is required and must restate the `GROUP BY`. No config block;
   `safety_overrides:` is a hard error.
2. **A `timeseries:` block is admitted iff key temporal locality is established**; otherwise
   refused (`KeyedForbidsTimeseries`).
3. **The body is an aggregated `GROUP BY` query; every non-key projection classifies into
   exactly one column family.** The combiner is a fixed lookup; authors never declare combiners.
4. **The catalogue is closed and the classifier fail-closed.** Unrecognised aggregators,
   composite expressions, unproven once-write columns, and retractable contributions are
   refused — never approximated.
5. **End-state equivalence holds with the model's own SQL as the oracle**, with exactly two
   named carve-outs: retained departed keys under snapshot-reconcile, and ordering-key ties on
   overwrite columns.
6. **No write-eligibility clamp.** A run merges every delta row it scans; no scanned input is
   silently dropped. Slice pruning under locality is no-op elimination (or a
   transactionally-checked declared bound), never a write clamp. Any future clamp or settled-key
   GC must ship together with late-fact accounting.
7. **The run shape is derived from the driving source** (clocked ⇒ window-forward; unclocked ⇒
   snapshot-reconcile), surfaced by `smelt explain`, never declared.
8. **The admission matrix is enforced per column.** Fold and once-write families require a
   clocked (replayable) driving source; the plain-overwrite family requires the snapshot
   posture.
9. **Window-forward models maintain the transactional merge ledger**, written atomically with
   each window's merge. Additive-fold models refuse a ledgered window's re-run;
   re-run-tolerant models may re-merge. Snapshot-reconcile models keep no ledger.
10. **Ordering and parallelism follow the derived postures.** Out-of-order/parallel/sliced
    backfill only for order-independent models; overwrite columns force sequential temporal
    order.
11. **Reprocessing changed input is refused for every family** when detected; the mitigation is
    `--full-refresh` or a manual cascade rebuild.
12. **Exactly one clocked driving source under window-forward.** Zero selects
    snapshot-reconcile; two or more is refused.
13. **Without an admitted `timeseries:` block the output has no `partition_column`** and is
    consumed as a lookup; with one, it is a clocked, time-partitioned keyed table.
14. **The windowed step loop is the shared driver**, never a per-pattern copy
    (`model_transforms.md`).
15. **Key temporal locality is established only by the three named routes.** Derived routes
    prune by proof; the declared route prunes only under the transactional runtime check
    (`KeyedRecurrenceBoundViolated`). A violated declaration fails the run; it never silently
    drops.

## Limitations

Deliberate scope boundaries: things smelt does not do **by design** at this spec's current cut.
Unlike §Known Divergences (implementation lagging decided intent), nothing here is a gap to be
closed by a tracked plan — changing an entry requires its own spec diff. Each entry states the
boundary, the reason, and the sanctioned alternative.

### No smelt-maintained SCD2 — history-keeping is plain SQL

smelt has no declared or derived history-keeping shape: no frontmatter opts a keyed model into
retaining every version of a key. SCD2 is written as ordinary windowed SQL over a change
stream. `customer_history` over the running example's `customer_changes` feed:

```sql
---
refresh: full
---

SELECT
    customer_id,
    tier,
    region,
    effective_ts AS valid_from,
    LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) AS valid_to,
    LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) IS NULL AS is_current
FROM smelt.customer_changes
```

Every version of a key is a row; each version's validity interval closes at the next change's
event time; the newest version per key is open. If the feed carries no-op change events (full
row images where no tracked attribute changed), dedupe first — a `LAG`-comparison filter over
the tracked columns before the `LEAD` — so spurious versions never open.

The two sanctioned routes for keeping such a model current:

- **`refresh: full`** — rebuild from the change stream each run. Always correct; cost is a full
  rescan of the feed.
- **`refresh: materialized_view`** — the same SQL, engine-maintained, where the backend has
  native IVM (`materialized_view.md` §Design "No named pattern").

There is deliberately no `refresh: incremental` route: `LEAD` is inadmissible in every corner —
the key grain rejects window functions outright (`KeyedForbidsWindowFunctions`), and under the
partition grain a new event must rewrite a row in an already-written earlier partition, outside
every output clamp. Recognising the LEAD-over-clock-within-key pattern as an admissible
incrementally-maintained shape is sketched in §Future Extensions.

### No SCD2 over mutable snapshots

smelt never manufactures a change stream by diffing successive scans of a mutable snapshot
source. A version history needs an event time per version boundary, and a snapshot diff can
stamp boundaries only with the scan time — the run clock — so the resulting history would
depend on when `smelt build` happened to run, breaking the replay-safety the equivalence
invariant demands (§"The equivalence invariant"). SCD2 therefore requires a source that already
carries change events with their event times (an update-events / CDC feed). Maintaining the
*current state* of keys from a snapshot (snapshot-reconcile, key grain) is in-bounds; retaining
the *history* of snapshot states is not. If a snapshot-to-change-stream facility is ever wanted,
it is a source-layer concern (`sources.md`), not a model shape.

### Other deliberate boundaries

Boundaries stated normatively elsewhere in this spec, collected here for discoverability:

- **Late arrivals beyond the derived horizon are excluded**, silently; surfacing them is a
  model-author + data-check concern (§"Windowed maintenance and the horizon").
- **No continuous freshness.** smelt-owned maintenance is pull-based and per-run; the history is
  correct as of the last `smelt build`. Engine-continuous freshness is a different refresh mode
  (`materialized_view.md`).
- **Non-replayable observation contracts are refused.** Min-ever-observed, first-observed-value,
  and similar fold-the-observation-sequence columns have no executable full-refresh oracle and
  are rejected rather than approximated (§"The equivalence invariant"; a possible opt-in weaker
  contract is §Future Extensions).
- **No delete signal under window-forward consumption.** An append-only feed without delete
  events cannot express key deletion; departed keys persist (retention). Deletion semantics
  beyond retention are an open question (§Known Divergences "The key grain").

## Known Divergences / Open Questions

Live gaps between this spec and the implementation, and questions where intent itself is
undecided, as of `last_reviewed`. Completed work is not recorded here — history lives in git and
§References → Plans.

### The contract, plan, and graph layer

- **The `diff_patch` write pattern only routes over a per-group recompute.** A `write:` pin that
  resolves to `diff_patch` over a live `PerGroupRecompute` repair cell (§"The repair family")
  executes via its emitter, with the executed-vs-emitted `statement_parity` leg proven for that
  case. A `diff_patch` pin whose underlying recompute is the region `DeleteInsert` default has no
  runtime lowering yet — the runtime resolver that would route it fails loud by name rather than
  falling through to a plain write, but no caller today reaches that resolver for the
  `DeleteInsert` recompute, so the pin is presently unenforced for that case rather than refused.
  Tracked: `docs/outcomes/20260809-repair-family/outcome.md`.
- **The repair family's affected-key recompute ignores a decomposed combiner's hidden state.**
  `repair_candidate_select` wraps the model's plain PRESENTED projection with no widening for a
  decomposed combiner's hidden state (e.g. the order-monotone family's `(v, o)` pair, §"The
  column-family catalogue"); the physical table the fold's own create path built carries the
  extra hidden state columns, so a live `PerGroupRecompute` `INSERT` for such a combiner supplies
  fewer columns than the table has and the run errors rather than repairing. Only a combiner
  needing no hidden state (e.g. a plain `MAX`) repairs correctly today. Discovered by the
  conformance gate's repair-family recipe pool
  (`crates/smelt-cli/tests/maintenance_conformance/repair.rs`). Tracked:
  `docs/outcomes/20260809-repair-family/outcome.md`.
- **Frontmatter-time grain checking has one narrow gap.** A `grain: key` model with no top-level
  `unique_key:` (identity derived from the body `GROUP BY`) is checked against the derived key
  only at plan derivation, not at frontmatter validation; a bare `grain: key` model with neither
  declaration is unchecked (cross-ref `models.md` §Known Divergences).
- **The write-pin equivalence factor is structural only.** The available-addressings rule's
  equivalence factor is checked today as the pattern's declared required contract facts; the
  per-cell equivalence hook (`resolve_write_pin`'s `cell_can_uphold_equivalence`) always
  accepts. Deepening it (e.g. threading column-comparability or a suppression-specific proof) is
  later work.
- **An inadmissible write-*variant* pin has no pre-execution gate.** Forcing
  `technique: suppress` on a cell whose suppression proof refuses is not checked up front — the
  resolver silently falls back to full region recompute instead of refusing the run; `smelt
  explain` also misses the comparability-only-inadmissible case. Extending the pre-execution
  gate to this pin dimension: `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Observed-delta consumption is partial.** Recording and key→partition projection exist for
  the change-suppressed column-scoped MERGE family, but: `smelt run --since-upstream` does not
  yet read the recorded delta table live; backward resolution consumes no recorded delta (every
  ancestor requirement is the full clamp-derived slice); the keyed-fold and staged-candidate
  write families record nothing; and the settle-bound × observed-delta composition
  (§"What the composed shape enables") has no live "is this slice's recorded delta empty" leg.
  Tracked: `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **No execution technique keys off a maintained-model creation cell.** The cell drives forward
  propagation and `smelt explain`, but the propagated region is materialized by the ordinary
  incremental run loop rather than a per-cell technique. Tracked:
  `docs/plans/20260710-web-analytics-maintenance-demo.md`.
- **The definition-change backfill's atomicity is conditional on the schema-evolution gate
  actually running this run.** The fold described in §"The definition-change trigger" only
  happens inside `schema_evolution`'s migration call; a model whose `schema_evolution:
  strategy: full_refresh` frontmatter skips that gate entirely (columns instead arrive via a
  full-table rebuild, where there is nothing to backfill in place) falls back to a
  standalone, independently-dispatched `UPDATE` for `PureBackfill` fields — the same
  non-atomic two-step the general mechanism now avoids. This standalone path is also the
  only one exercised on a non-transactional-DDL backend (`BackendCapabilities::
  supports_transactional_ddl == false`): the migration's own statement group is no longer
  all-or-nothing there either, so a mid-group failure can still leave an added-but-unbackfilled
  column with an already-advanced schema snapshot. Neither case has a repair path today.
  Tracked: `docs/plans/20260809-sensitivity-precision.md` Phase 6.
- **Plan-consumer gaps.** The horizon-clamped partition-local mutation corner is not reachable
  from any real workspace (trigger construction emits `UpstreamMutation` only for unclocked
  sources; clocked mutable-source scan-bound derivation is deferred); dispatch cannot
  distinguish "a mutation genuinely happened" from re-derivation (change-aware triggering is
  `--since-upstream`'s job); the `prefer` soft-bias ladder and `scan_bounds.on_violation: warn`
  parse but are not consumed (every refusal is an Error); the cost model between two admissible
  techniques is unbuilt; `AppendOnly` sources get no `UpstreamMutation` cell. Refs:
  `docs/plans/20260707-maintenance-plan-impl.md`.
- **Emission remainders.** The additive fold's MERGE-inside-ledger-transaction interior is
  not observable at the statement-group seam, so its parity leg uses an idempotent fixture;
  `Backend::delete_partitions` / `insert_overwrite` still hand-author SQL for the
  production-unreachable `InsertOverwrite` strategy (dead code, allowlisted in the structural
  no-authoring gate).
- **Proof-layer residues.** All seven maintenance-plan proofs are derived
  (`model_properties.md` §Surface), with these gaps surviving: a keyed-grain output poses no
  partition-locality question, so a locality-admitted keyed model's clamps carry an assumed
  (underived) write-footprint mirror into propagation. `smelt-runtime`'s maintenance driver is
  the one production caller that derives a real `ColumnAdded` trigger (it alone has I/O access
  to the deployed-schema snapshot the trigger diffs against, read once per run before the
  schema-evolution gate); `smelt-db`'s own diagnostics/`smelt explain` path has no such access
  and always derives an empty trigger set, so `MaintenanceSkeletonColumnAdded` is reachable
  (`derive_model_maintenance_plan`'s own unit coverage) but not yet surfaced as an LSP/CLI
  diagnostic ahead of a run, nor does a skeleton-position add block the run itself today — an
  unadmitted `ColumnAdded` cell (skeleton add, or `UpstreamRederive` with no source to scan)
  simply leaves the ordinary region-recompute technique as the run's only dispatch, same as
  before this trigger existed. Column-group-scoped dirt
  coarsens to whole-partition (safe, over-running); hour granularity is declared surface but
  propagation is day-ordinal. The built grain-alignment check validates only the declaration
  (widen-never-narrow, `MaintenanceGranularityMismatch`); graph edges still take the
  declaration directly. Refs: `model_properties.md` §Known Divergences;
  `docs/plans/20260808-derived-maintenance-proofs.md`.
- **The ledger's warehouse substrate is DuckDB-only.** An additive-graded cell on another
  backend fails loudly (`UnsupportedFeature`); a Spark-dialect ledger builder is unbuilt.
- **Graph-layer gaps.** Bare `grain: key` nodes with no admitted locality refuse
  (`MaintenanceGraphUnsupportedNode`); time-unrolled self-edges are designed but unbuilt; no
  key-level dirt representation exists — intervals are the graph's only currency; the
  `examples/web_analytics` workspace is not fully `--since-upstream`-compatible end to end (a
  self-referential model and a bare-keyed model with readers each refuse the whole-workspace
  graph; no `--select` scoping exists).
- **Delta detection for `--since-upstream` is explicit-only in v1.** The runner supplies landed
  deltas on the command line; no persisted per-source watermark or automatic diffing is consumed
  by the graph layer (§Future Extensions).
- **Straddle attribution without locality is scoped out of the ledger's v1** (a per-key
  footprint chaining across history;
  `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §8).
- **No out-of-band-edit tripwire (Open Question).** An observed output delta is trusted because
  the recording state is smelt-owned and written only by smelt's own conditional-write path; an
  external mutation to a target table between runs is not detected. Whether a tripwire (e.g. a
  cheap table digest checked at run start) is worth its cost is open (§"The graph layer").
- **A proposed `on_column_add: backfill | leave_null | recompute` policy knob** is noted but not
  surface.
- **The derived model-wide horizon is under construction**, as is the data-quality check for the
  model-author lateness-flag pattern. Tracked: `docs/plans/20260704-model-updates.md`.
- **Locality machinery gaps.** The per-input scope-map explain surface is specified but
  unbuilt. Route 2's declared-FD sub-route is unreachable for an arbitrary non-clock-derived
  dimension column (the NOT-NULL derivation recognises only driving-clock-derived shapes), so a
  runnable end-to-end route-2 fixture is still missing — the once-write column family it needs
  now exists, but that NOT-NULL derivation gap remains
  (`docs/plans/20260705-keyed-collapse.md`). Route 2's `IN (SELECT DISTINCT …)` slice predicate
  is unexercised against a real backend due to a DuckDB MERGE binder limitation (confirmed
  v1.4.4/v1.5.4) — merges run unpruned; lifting needs a rewrite or a fixed DuckDB
  (`docs/plans/20260715-composed-axes-conditional-maintenance.md`). Plan derivation admits
  routes only where it can determine the driving source's granularity (runtime always can).
  Declared-vs-derived recurrence precedence (derived first) and order-independent key-set
  comparison are implementation choices the spec text underdetermines.
- **`grain: key_per_partition` derives no plan.** The value parses and validates but refuses at
  plan derivation (`MaintenanceUnsupportedGrain`); trajectory support (locality, emitted plan,
  graph admission) tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Conditional-maintenance gaps.** `smelt explain --show-sql` renders the unconditional
  matched arm, never the suppressed form a live run executes; the region DELETE+INSERT family
  has no conditional variant; the whole-row (keyless) staged-candidate realisation does not
  exist; no `write:` pin selects between keyed MERGE and staged-candidate; delta-restriction
  admission does not yet consume an external `mutable_snapshot` source's fingerprint-sidecar
  delta as a driving-source delta; non-DuckDB targets keep the widened-scan recompute. Refs:
  `docs/research/20260715-conditional-maintenance-without-cdf.md`;
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Override-ladder reach.** The keyed-fold suppression consumer honours `Suppressed`
  unconditionally — the first-build-vs-steady-state rule doesn't reach it; no real fixture
  derives a column-scoped/keyed-fold cell under a first-build/backfill trigger, so that branch
  is proven only at resolver level; `smelt bakeoff` measures technique-family cost only, not
  the write-suppression dimension. Open: whether a future cost model needs region-level
  change-ratio statistics from prior observed deltas.
- **docs-site coverage of the plan's CLI surface is partial**; the residue is not enumerated —
  audit and either document or drop this entry.
- **A group merged across two mutable inputs has no group-merge-provenance policy.** A
  partition-aligned multi-input mutable merge is admitted like a single-input case; whether
  provenance spanning multiple mutation-sensitive inputs should force region recompute is
  undecided.
- **`change_feed` sources never get an `UpstreamMutation` cell** (the trigger builder checks the
  literal `mutable_snapshot` declaration), and even where the posture is threaded through, only
  full-input re-derivation is admitted — no live fold machinery consumes a change feed's delta
  shape.
- **`INTERSECT`/`EXCEPT` are unclassified set operations**: they collapse to whole-model
  mutation-sensitivity, so every admitted cell is region recompute. A future distribution proof
  needs per-arm-cardinality reasoning. Cross-ref `model_properties.md` §Known Divergences.

### The partition grain

- **A row-shaped model's MERGE-dedup key has no `.sql` frontmatter home.** Declaring top-level
  `unique_key:` makes an output key-shaped, which a row-shaped body cannot occupy
  (`KeyedRequiresGroupBy`); the per-row identity for a column-scoped MERGE is declared today
  only via the `smelt.yml` override `models.<name>.batched.unique_key`. Whether the concept
  deserves its own frontmatter spelling is open:
  `docs/plans/20260719-prod-w8-composed-axes-followups.md`.
- **Two spellings of the plausible-contract mechanism coexist.** `.sql` frontmatter uses
  `columns.<c>.contract` only; the `smelt.yml` override's `batched.nondeterministic_columns`
  still parses as a separate spelling (`smelt-yml.md` §"Layer split").
- **One classification call site reads the outer SQL body**: the bound-`NotDerivable` refusal
  gate classifies on the outer `model.sql`, so a lookback living only inside a function body
  with no outer filter would diverge (no such case exists in the repo). Tracked:
  `docs/plans/20260530-thread-fn-registry-classification.md`.
- **The window-function batch-safety check runs on unexpanded outer SQL** — an `OVER` inside a
  `smelt.define` body is invisible to it. Same tracking plan.
- **Per-source clamp observability is partly emitted.** `smelt explain --json` reports the
  offsets but does not resolve the run-relative scan window when a run window is supplied; the
  editor-hover readout is unimplemented. Specified ahead of a plan.
- **Per-column `data_latency` is unimplemented**; the two interim mitigations
  (§"First-run and backfill") are the only options.
- **Non-deterministic row-set-membership or grouping is out of scope** — always rejected
  regardless of column contract; admitting it needs a frozen-per-window-membership design
  (`docs/research/20260703-model-updates.md` §9.1a).
- **CTE-only `event_time_column` references are not yet detected**: a CTE alias that fails to
  project it escapes the outer-visibility check and fails at execution. Tracked:
  `docs/plans/20260616-smelt-feedback-fixes.md`.
- **Schema evolution is unspecified** — a `partition_column` rename or output schema change has
  no defined handling.
- **The `smelt.metric()` interaction is unspecified** — metric expansion × time-filter injection
  for partition-grain models consuming metrics.
- **Per-`ModelDef` overrides for generator-emitted models are not part of the closed field set
  in v1.** Tracked: `docs/plans/20260509-meta-language-overall.md`.
- **`g_run >= g_part` auto-coarsening is not implemented** — sub-`g_part` run windows
  hard-reject (fail-closed chosen first); auto-coarsening or reject-with-suggestion is
  deferred.
- **Monotone-integer `partition_column` has no end-to-end run.** The trace and bound derivation
  admit it, but run windows, backfill chunking, scan-filter injection, and the explain clamp
  rendering are date-typed throughout. Tracked:
  `docs/plans/20260704-model-updates-l4-batched.md`.

### The key grain

- **A window-forward keyed run with no event-time window silently full-refreshes instead of
  refusing.** §Surface CLI requires both `--event-time-start` and `--event-time-end` for a
  window-forward keyed model; the runtime's no-window arm
  (`crates/smelt-runtime/src/execute.rs`, the keyed branch's fallback case) instead drops the
  target and recreates it from the whole-source SELECT. The end state is the full-refresh
  oracle, so nothing is silently *wrong*, but a run the spec says must refuse instead rebuilds
  the table — including the case where only one of the two flags is supplied. No test asserts
  the refusal, and the user documentation currently describes the fallback rather than the
  required-flags rule.
- **The once-write classifier has no nullability route around the fallback case.** The
  fallback-bearing and multi-candidate spellings admit onto decomposed `(value, written)` state
  (§"Decomposed state (rung 2) in keyed models"), but the only route to that state is the
  FD-backed proof; the NOT-NULL derivation
  (`crates/smelt-logical/src/analysis/not_null.rs`) proves not-null only for a partition /
  driving-clock-derived column, so establishing that a fallback can never fire needs a declared
  functional dependency, not a static not-null proof. The key-derived route still requires a bare
  reference to a `unique_key` column, not an arbitrary key-derived *expression*. The admission
  also reads whole-scope fan-out/set-operation facts rather than a per-column join trace, so any
  fan-out or undiscriminated set operation anywhere in the model's scope refuses every candidate,
  not only the one actually reached through it. Decision record:
  `docs/research/20260705-keyed-collapse-application.md`; tracking:
  `docs/outcomes/20260809-rung2-state-shapes/outcome.md`,
  `docs/plans/20260705-keyed-collapse.md`, `docs/plans/20260809-keyed-frontier.md`.
- **A re-run-tolerant keyed model keeps no ledger at all.** §"The transactional merge ledger"
  gives every window-forward model a ledger, refusal-bearing for additive folds and
  detection/bookkeeping-bearing for the idempotent families; the runtime only ever creates the
  ledger table for an additive-graded model (`Grade::Additive`), so a fully idempotent model has
  no record of which windows it merged — reprocessing detection and `--auto` bookkeeping have no
  substrate there. Nothing is unsound (re-merging those families converges), but the `--auto`
  staleness path cannot consult a ledger that was never written.
- **Snapshot-reconcile admits at most one source of any posture in the FROM clause** when zero
  are clocked — a join of two or more unclocked candidates refuses
  `KeyedSnapshotPostureUnsupported` rather than picking one. Widening this to a proven
  multi-source snapshot scan is unbuilt.
- **`KeyedRetractableContribution` has no implementation.** The code is specified (§Surface
  Diagnostics, §"Enrichment joins") but no classifier, diagnostic variant, or test produces it,
  so a retractable enrichment contribution is not refused on those grounds today.
- **`safety_overrides:` on a key-addressed model is not a hard error.** §Surface "Key-grain
  declaration" makes it one; frontmatter validation only checks the double-declaration case and
  never conditions on the derived grain, so the block parses on a keyed model and is ignored.
- **The reconciliation ledger's fold is transactional on DuckDB only.** §"The transactional
  merge ledger" requires the ledger write to share the merge's backend transaction; the default
  `Backend::fold_ledger_delta` is an explicitly best-effort check-then-act across separate
  statements, and only the DuckDB backend overrides it with a real transaction (the same
  DuckDB-only substrate the ledger DDL divergence above names).
- **`smelt explain` prints neither the per-column guarantee ledger (§Surface CLI) nor the
  derivable forward reach (§"No write-eligibility clamp")** — the cell/addressing/clamp/locality
  and edge sections are the whole of the rendered plan today.
- **Key temporal locality route 2 admits only a declared functional dependency.** §"Key temporal
  locality" gives route 2 a key-derived-expression sub-route alongside the declared FD; the
  locality derivation deliberately never consults the derivation's no-declaration branch, so a
  provably key-derived partition projection still refuses without the declaration.
- **The derived execution postures are internal, and one of the three is not derived at all.**
  Re-run tolerance reaches a run only as the reconciliation ledger's grade (`Grade::Idempotent`
  for every idempotent family, `Grade::Additive` for a model carrying an additive-fold column),
  and order-independence is not derived as a named verdict anywhere: every window-forward run
  applies its windows sequentially in temporal order regardless of family, which is safe but
  forgoes the parallel / out-of-order application §"Derived execution postures" admits. Neither
  the derived run shape nor any of the three postures is printed by `smelt explain`, which
  §"Derived execution postures" states as their surface.
- **The generative conformance pool cannot stage NULL payloads.** The generated row type's
  payload field (`GenRow::val`, `crates/smelt-maintenance-testkit/src/schedule_gen.rs`) is a
  non-nullable `i64` threaded through the schedule generators, the oracle materializer, the feed
  replay, and the Spark twin's readers, so the once-write family's NULL direction — a key whose
  first window carries only a NULL payload and whose real value arrives later — is covered by a
  targeted case in `crates/smelt-cli/tests/maintenance_conformance/gate.rs` rather than by the
  generated pool that proves every other keyed family.
- **Locality open questions**: whether a derived recurrence bound can license slice pruning
  under snapshot-reconcile (v1: window-forward only); relaxing the granularity-equality
  precondition (a daily driver with weekly output partitions); slice-scoped deletion
  (`NOT MATCHED BY SOURCE` over a provably complete slice) — interacts with the key-deletion
  question below.
- **The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped.** Each
  family is reachable only through its hand-written SQL spelling (`MAX_BY`/`MIN_BY`,
  `COALESCE`, `ANY_VALUE`), which is what the pattern functions would expand to. Whether they
  ship as built-ins or as a shipped template file of `smelt.define`s is an open decision, and
  the canonical once-write spelling is fixed alongside it. Tracked:
  `docs/plans/20260705-keyed-collapse.md`.
- **Driver granularity is `day`/`week` only** — inherited by every consumer of the shared
  driver; widening is driver work.
- **`--auto` staleness fidelity for all-invertible models is conservative in v1**; "exactly the
  changed windows" needs the group rung's delta-history mechanism.
- **Self-referential keyed models are rejected** (`state += delta − decay`); admitting them
  needs an explicit input/state distinction design.
- **Run-pinning alignment is deferred**: `NOW()`/`CURRENT_*` are rejected outright in keyed
  models rather than compile-time-pinned as the partition grain does.
- **Key deletion is unresolved beyond retention.** Snapshot-reconcile retains a key present in
  the target but absent from the incoming scan, unchanged and forever (§"The two run shapes"),
  and no explicit mechanism deletes a departed key; window-forward has no delete signal short of
  a change feed with delete events. Tombstones, opt-in hard delete, and the observer contract
  for the refused matrix cells are deferred
  (`docs/research/20260705-keyed-collapse-application.md` §5).
- **Ladder rungs 3–4 remain specified ahead of this profile's use of them.** Group-rung
  retraction (rung 3) and the bounded-domain multiset (rung 4) are out of scope for the
  rung-2 work above; rung 3 additionally depends on the change-feed consumption design — no
  live fold machinery consumes a change feed's delta shape today. Deferred by
  `docs/plans/20260809-keyed-frontier.md` §Scope,
  `docs/outcomes/20260809-rung2-state-shapes/outcome.md` §"Out of scope".

## Future Extensions

Ideas for widening the admission space that are **not decided**. Nothing here is surface; none
of it may be relied on or implemented against until it graduates into §Surface/§Semantics via
its own spec diff and plan.

- **Smelt-maintained SCD2 via succession-pattern recognition.** The plain-SQL SCD2 shape
  (§Limitations) could gain a `refresh: incremental` route by *recognising* the pattern rather
  than declaring it: a walk-produced verdict that every window function in the projection is
  `LEAD(t)` (or an expression over it) partitioned by an entity key and ordered by the driving
  source's event-time column. The maintenance theorem: a new event touches only its own row and
  its immediate predecessor within the key — bounded footprint, late events included (a
  mid-history splice touches exactly the predecessor and reads its successor). The technique is
  a keyed `MERGE` plus a targeted predecessor patch, and the standard equivalence invariant
  applies directly (the SQL is its own oracle). The machinery generalises beyond SCD2 to any
  `LEAD`/`LAG`-over-clock-within-key model (next-event features, sessionisation gaps), which is
  what would justify building it. Open: the classifier grammar (expressions over `LEAD`,
  post-window delete filtering), the fail-loud diagnostics for near-misses, and the
  `model_properties.md` walk vocabulary for window functions. Full sketch:
  `docs/research/20260723-scd2-succession-pattern.md`.
- **Row-local derivation for a *changed* column.** When a column is **added** and its expression
  is a pure function of other columns already stored in the same row, the `PureBackfill` verdict
  already covers it (§"The definition-change trigger"): an in-place `UPDATE`, no upstream read.
  The open extension is the **changed**-column case: redefining an existing column's expression
  has no plan-level treatment today (it falls to a full recompute even when the new expression
  reads only unchanged stored columns). Applying the same per-column-provenance test to a
  changed column's new expression could admit a targeted in-place `UPDATE` — but it needs its
  own trigger (distinct from the additive-only definition-change trigger), its own diagnostic
  for the fail-closed case, and a decision on ledger composition (a redefinition invalidates the
  group's provenance identity even though no upstream delta occurred).
- **Automatic, watermark-diffed `--since-upstream`.** Today the caller supplies each source's
  landed delta explicitly (§Surface "CLI"). A future extension persists a per-source "last
  propagated through" watermark in `smelt-state` and diffs it against the source's current
  `covered_intervals`, so a bare `--since-upstream` discovers its own delta. This still does not
  solve a never-modeled raw source's freshness (no `covered_intervals` exists for data smelt
  never landed) — live backend freshness querying stays out of scope. The explicit and automatic
  forms are not exclusive: the automatic form computes the same `--landed` intervals the
  explicit form takes directly, layering on top without changing the graph layer or CLI.
- **An observer / prefix-consistency contract for non-replayable combinations.** The admission
  matrix refuses folding state *observations* into fold-family columns because no executable
  full-refresh oracle exists (§"The equivalence invariant", §"Admission matrix") — min-ever
  observed, first-observed-value, and similar are contracts over the *observation sequence*.
  A future opt-in could state that weaker equivalence explicitly (a property of the observed
  prefix, not a re-runnable refresh) and admit those cells under it, rather than smuggling them
  under the executable-oracle invariant. Open: the formal statement, the opt-in surface, and
  what a conformance oracle even is for a non-replayable history.

## References

### The contract, plan, and graph layer

- **Code**: `crates/smelt-logical/src/maintenance/{mod,derive,emit}.rs` (the per-cell derivation);
  `crates/smelt-logical/src/maintenance/propagate.rs` (the pure graph-layer composition math —
  `propagate`/`required_inputs`); `crates/smelt-runtime/src/propagation.rs` (the real per-workspace
  graph assembly, `smelt run --since-upstream` planning, and `smelt build --include-upstreams`
  planning — `build_forward_graph`, `plan_since_upstream`, `resolve_build_plan`, all consuming the
  same `Edge` list);
  `crates/smelt-logical/src/analysis/` (the classifiers admission consumes);
  `crates/smelt-runtime/src/{cumulative,maintenance_driver,dimension_horizon_merge,transformer,backfill}.rs`
  (today's technique executors and clamps); `crates/smelt-state/src/intervals.rs` (the degenerate
  ledger); `crates/smelt-backend/src/lib.rs` (technique primitives).
- **Tests**: `crates/smelt-logical/tests/{maintenance_tracer,maintenance_tracer_evolution,maintenance_tracer_propagation,maintenance_propagation_adjoint}.rs`
  (pure derivation-side and graph-composition-math assertions — the regression floor for chains,
  fan-out, diamonds, granularity mapping, and adjointness — `maintenance_propagation_adjoint.rs`
  is the dedicated home for the `forward(backward(P)) ⊇ P` law); `crates/smelt-runtime/tests/
  {tracer_maintenance,tracer_evolution,tracer_propagation,since_upstream_propagation}.rs` (the
  DuckDB equivalence oracles, and the real-workspace propagation-graph assembly tests);
  `crates/smelt-cli/tests/since_upstream.rs` (the CLI-wired forward-propagation suite, including
  the sufficiency-vs-full-refresh equivalence check); `crates/smelt-cli/tests/include_upstreams.rs`
  (the CLI-wired backward-resolution suite: resolved-slices-suffice-vs-full-refresh equivalence
  over a two-hop chain, and an unclocked-ancestor-resolves-to-whole-table case);
  `crates/smelt-maintenance-testkit` (dev-only, `publish = false`; the Link-C in-process harness —
  the real-run-pipeline driver, the typed `ModelRecipe` generator (`recipe.rs`), the schema-generic
  schedule generator (`schedule_gen.rs`), the S-tracked equivalence oracle (`s_tracker.rs`,
  `oracle_modes.rs`), and the multiset-equality oracle (`oracle.rs`) — wired as a dev-dependency of
  `smelt-cli`); `cargo test -p smelt-cli --test maintenance_conformance` is the standing generative
  equivalence gate: on every `cargo test`, a deterministic-seeded sample of typed `ModelRecipe`
  values (append-only partition-grain, fact+mutable-dimension, `grain: key`, and generated 2-3 node
  DAGs) is staged, classified through the real maintenance derivation, and driven through
  `execute_project` against a real DuckDB backend, asserting emitted maintenance output equals a
  full-refresh oracle after every run step under adversarial append/lateness/mutation/redelivery/
  definition-change schedules (`SMELT_CONFORMANCE_CASES` scales the sample depth for a deeper local
  or nightly soak run). A composed (`grain: key` + `timeseries:`) recipe family exercises all three
  key-temporal-locality routes — key-embedded (driven through `execute_project`), key-determined,
  and declared-recurrence-bounded with in-bound redeliveries (both driven directly against a real
  DuckDB backend, the workaround `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`
  also uses) — asserting whole-table and per-slice equivalence after every step, gated by its own
  admission-rate floor (`SMELT_CONFORMANCE_COMPOSED_CASES` scales its sample depth). A generated
  model-edge enrichment recipe family (one closure-admissible `LEFT JOIN` shape and its two
  closure-failing siblings — a bare inner join, and a membership predicate over an enrichment
  column) drives the delta-restricted-vs-widened-scan choice both ways over the same fixed
  processed-input set `S` — its own P1 skeleton-source-closure verdict, derived through the real
  per-cell derivation rather than asserted, gates which cases the equivalence check runs, and the
  end states must be bit-identical; a second case drives a fully-suppressed conditional write
  through its own real observed-delta recording and asserts the cascade this composition exists
  to unlock — zero rows written, a present-and-empty recorded delta, zero regions scheduled across
  every downstream consumer, and an end state still equal to a from-scratch full-refresh oracle —
  both gated by their own admission-rate floor. The
  key-determined route's merge mechanics (write-once partition, additive fold) are exercised this
  way against real DuckDB, but its slice-pruned target scan is not — the driver runs every
  key-determined step with the slice predicate omitted because DuckDB's `MERGE` binder refuses the
  real predicate shape (§"Key temporal locality" above, the `BindMerge` divergence). A `pinned`
  module reproduces every construct × posture cell and named hazard
  schedule the gate subsumes as deterministic, always-reproducible cases (never proptest-drawn
  alone), and a `registry` module tracks named divergences with a staleness report (entries that
  never fire over the deterministic sample are reported, never failed — the same governance pattern
  `crates/smelt-db/tests/prop_helpers/known_unknowns.rs` uses). The per-cell probe modules under
  `crates/smelt-cli/tests/property_discovery/` cover constructs the typed recipe generator has no
  vocabulary for yet (self-referential models, `UNION ALL`, `LEFT JOIN`, correlated `EXISTS`,
  stacked window frames, cross-source column-name collision, a mutable source aggregated directly)
  and remain disposable research probes (see `.claude/scripts/property-experimental-gate.sh`).
  `crates/smelt-cli/tests/incremental/` is narrower still: it drives a backend's incremental
  strategy directly given a hand-supplied filter, proving the strategy executes correctly once
  handed one, independent of how that filter is derived. `cargo test -p smelt-logical --test
  maintenance_plan_conformance :: coverage_matrix_is_inhabited` is the standing inventory gate over
  the research example catalogue's coverage matrix (`docs/research/20260705-refresh-as-maintenance-plan/
  07-example-catalogue.md` §"Coverage matrix", plus one `INTERSECT`/`EXCEPT` row this gate adds):
  it encodes the matrix as data and asserts every inhabited `(construct × source-property)` cell
  is accounted for by exactly one of two explicit, disjoint lists — `CLAIMED` (a grounded,
  executable test proves the cell's HOLDS-or-refuses verdict; see
  `crates/smelt-logical/tests/maintenance_coverage_matrix.rs` and
  `crates/smelt-cli/tests/property_discovery/coverage_matrix_gaps.rs` for the cells this phase
  lifted) or `KNOWN_GAPS` (named, not silently omitted). Adding a matrix cell without a matching
  `CLAIMED`/`KNOWN_GAPS` entry fails the test, by construction (additive-only). `CLAIMED` currently
  lifts 9 catalogue ids (EX-02, EX-08, EX-12, EX-14, EX-18, EX-24, EX-26, EX-27, EX-35, plus the
  added EX-41/EX-42 row); the remainder of the matrix's ~100 inhabited cells are named individually
  in `KNOWN_GAPS` (most as "plausibly covered by an existing `maintenance_conformance::pinned`
  hazard case or `G-*`/`SC-*` property-discovery probe, not re-verified against this exact catalogue
  id" — cross-referencing those cases to catalogue ids by name is itself unbuilt; a few, like
  EX-25's LAG/LEAD footprint reflection and EX-29's as-of-run-contract gating, need production
  investigation not yet done). Both lists are per-cell, never per-row, so a future change can lift
  one cell at a time without re-deriving the whole inventory.
- **User docs**: `docs-site/docs/index.md`, `docs-site/docs/guide/{incremental-models,sql-models,materializations}.md`,
  `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md`
  describe the trichotomy + grain surface; `docs-site/docs/reference/cli.md` also documents
  `--since-upstream`, `--include-upstreams`, and `smelt explain`'s cell/clamp/ledger report with
  `--show-sql`; `docs-site/docs/reference/smelt-yml.md` documents the `maintenance:` block.
- **Plans (history)**: `docs/plans/20260704-model-updates.md`,
  `docs/plans/20260704-model-updates-fundamentals.md` (the L1+L2 substrate),
  `docs/plans/20260705-property-discovery-loop.md` (the empirical engine).
- **Research**: `docs/research/20260715-conditional-maintenance-without-cdf.md` (change-suppressed
  writes, delta-restricted compute, derived change feeds — the source of the pruning taxonomy's
  no-op write-elimination category and the composed-shape composition points);
  `docs/research/20260716-relation-contract-and-per-cell-addressing.md` (the shared Relation
  Contract, grain-as-derived-label, per-cell write addressing, and the open write-pattern registry
  this spec's §"Per-cell write addressing" and §"The declared shape" encode).
- **Related specs** (one list for the whole spec): `model_properties.md` (the derived proofs —
  monotonicity trace, bound/reach, partition alignment, determinism, discriminants, anchor
  resolution, once-write and join-contribution proofs, `bounded_domain:`); `model_transforms.md`
  (the physical mechanisms — pushdown, DELETE+INSERT, the clamps, pinning, `merge_into`, the
  windowed-keyed-maintenance driver, dimension-horizon MERGE); `models.md` (the refresh axis,
  the declared shape facts + derived grain label, the Relation Contract, three-state declaration
  law, input-consumption axis, litmus rule);
  `timeseries.md` (declares `event_time_column`, `partition_column`, `granularity`); `sources.md`
  (host of `timeseries:` and source-lateness/mutation-profile/key-recurrence world-facts);
  `expansion.md` (function expansion; runs before every analysis stage here); `functions.md` (the
  pattern-function surface); `materialized_view.md` (the engine-owned shape profile — where
  beyond-the-ladder shapes and hand-written SCD2 go); `multi_backend.md` (backend capability
  flags a strategy checks); `schema_evolution.md`, `run_state.md`, `virtual_environments.md`,
  `diagnostics.md`, `architecture.md`, `cli.md`.

### The partition grain

- **Code**:
  - `crates/smelt-core/src/config.rs` — `PartitionGrainConfig`, `Granularity`, `Weekday`
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, `ModelMetadata`
  - `crates/smelt-logical/src/rules/incremental.rs` — partition-grain detection + safety checks (in `smelt-logical`; `smelt-planner` re-exports)
  - `crates/smelt-logical/src/types.rs` — safety-override types
  - `crates/smelt-runtime/src/transformer.rs` — `inject_time_filter`, `inject_source_filters`, `is_transparent_single_source`
  - `crates/smelt-backend/src/lib.rs` — `Backend::delete_partitions`, `Backend::insert_into_from_query`, `Backend::delete_and_insert_transactional` (per-chunk transaction boundary)
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `DeleteInsert` impl
  - `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities::supports_merge`
- **Tests**: batched safety unit tests in `crates/smelt-logical/src/rules/incremental.rs`; CLI integration tests in `crates/smelt-cli/tests/incremental_*.rs`; the per-partition full-refresh-equivalence harness
- **User docs**: [`docs-site/docs/guide/incremental-models.md`](../../docs-site/docs/guide/incremental-models.md), [`docs-site/docs/guide/materializations.md`](../../docs-site/docs/guide/materializations.md)
- **Plans (history)**:
  - [`docs/plans/20260322-incremental-model-support.md`](../plans/20260322-incremental-model-support.md) — comprehensive plan; many phases still open
  - [`docs/plans/20260325-materialization-types.md`](../plans/20260325-materialization-types.md)
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — the mode-vertical master this spec re-cuts as a composition
  - [`docs/plans/20260707-maintenance-plan-impl.md`](../plans/20260707-maintenance-plan-impl.md) — lands the target frontmatter surface and diagnostics
- **Research**:
  - [`docs/research/20260521-incremental-as-planner-rule.md`](../research/20260521-incremental-as-planner-rule.md) — design direction this spec absorbs
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — batched eligibility audit; §9.2 non-determinism derivation
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance-framework design
  - [`docs/research/20260705-refresh-as-maintenance-plan/`](../research/20260705-refresh-as-maintenance-plan/) — the shape-profile demotion and per-cell admission this spec composes
- **Legacy reference**: `docs/DESIGN.md` §"Incremental Table Builds" — superseded for current behavior; useful for design rationale

### The key grain

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy`); `crates/smelt-logical/src/rules/cumulative.rs` (the built classifier seed — combiner lookup, GROUP-BY key derivation, driving-source resolution); `crates/smelt-runtime/src/maintenance_driver.rs` (the windowed-keyed-maintenance driver, `WindowedKeyedRule`); `crates/smelt-runtime/src/cumulative.rs` (per-window merge execution); `crates/smelt-backend/src/lib.rs` (`merge_into`), impls in `crates/smelt-backend-duckdb`/`-spark`.
- **Tests**: the cumulative classifier unit tests (`smelt-logical/src/rules/cumulative.rs`); the keyed end-state-equivalence harness; `smelt-backend-duckdb` `merge_into` tests.
- **User docs**: `docs-site/docs/reference/cumulative-aggregate.md` (the key-grain reference page — column families, the once-write proof, the two run shapes, the diagnostic codes); `docs-site/docs/guide/materializations.md` (author-facing walkthrough); `docs-site/docs/guide/incremental-models.md` §"The composed shape (key + time)" documents the composed (key-addressed *and* time-partitioned) form and its three locality routes; `docs-site/docs/examples/web-analytics/deduplication.md` is the worked tutorial — a redelivery-prone feed deduplicated by a keyed extremal fold under a declared recurrence bound, contrasted against the partition-grain `QUALIFY`-window workaround the preceding tutorial page builds.
- **Plans (history)**: `docs/plans/20260523-cumulative-aggregate.md` (the built seed); `docs/plans/20260704-model-updates.md` (the mode-vertical master this spec re-cuts as a composition); `docs/plans/20260705-keyed-collapse.md` (the keyed-collapse sub-plan); `docs/plans/20260707-maintenance-plan-impl.md` (lands the target frontmatter surface and diagnostics); `docs/plans/20260809-keyed-frontier.md` (the column-family union, the named ledger reprocessing refusal, and the snapshot-reconcile run shape).
- **Research**: `docs/research/20260705-keyed-time-superset.md` (key temporal locality, the time-partitioned output, per-input scope maps); `docs/research/20260705-model-refresh-review.md`; `docs/research/20260705-unified-keyed-refresh.md`; `docs/research/20260705-keyed-collapse-application.md` (the decision record this spec encodes); `docs/research/20260704-monotone-join-maintenance.md` (the monotone-vs-retractable boundary); `docs/research/20260703-model-updates.md`; `docs/research/20260705-refresh-as-maintenance-plan/` (the shape-profile demotion and per-cell admission this spec composes).
