---
feature: incremental_models
status: experimental
last_reviewed: 2026-08-16
owners: [andrew]
---

# Incremental Models

> **What this is.** The normative spec for **maintained models** — everything declared
> `refresh: incremental` — covering the correctness contract (the equivalence invariant and its
> declared relaxations), the delta algebra (delta signatures, the frontier), the derived
> per-model **maintenance plan**, and the dependency-**graph layer** built on it. Out of scope,
> with their own homes: the per-shape implementation chapters — partition grain and key grain —
> (`incremental_shapes.md`); migration across a change in the model's own SQL
> (`definition_deltas.md`); the provable properties of a model's SQL (`model_properties.md`);
> the physical transform mechanisms (`model_transforms.md`); the `refresh:` axis and declaration
> law (`models.md`); source world-facts (`sources.md`); the `timeseries:` declaration grammar
> (`timeseries.md`); engine-maintained views (`materialized_view.md`); backend capability flags
> (`multi_backend.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Overview

*This section is a non-normative primer: it introduces every concept the spec depends on, in
dependency order, and names where each is specified. The normative statements live in §Surface,
§Semantics, and §Constraints & Invariants — on any conflict, those win.*

### The one guarantee

An incremental model is an ordinary SQL model whose stored table smelt keeps current without
re-running the SQL from scratch. The entire feature rests on one promise, the **equivalence
invariant**:

> After any sequence of incremental runs, the stored table equals what a full refresh of the
> model's SQL would produce over the inputs those runs have processed so far.

Formally, writing `S` for the processed-input set:

```
incremental_state(S) == full_refresh(source | input ∈ S)
```

Everything else in this spec serves that equation. Properties of the model's SQL are proven so
that a maintenance shortcut is *known* to preserve it; a shortcut that cannot be proven safe is
**refused with a diagnostic** — never applied approximately, and never silently swapped for
something slower but safer (§"Validator, not chooser"). Two consequences worth internalising
before reading on:

- **Order doesn't matter.** The right-hand side depends only on the *set* `S`, so any two run
  histories that process the same inputs converge to the same table — up to the named
  carve-out and the determinism scope (§"The equivalence invariant").
- **Freshness is the only degree of freedom.** Anything smelt chooses — which technique runs, in
  what order — may change *when* the table reflects an input, never *what* the table says once
  it has (§"Per-cell admission").

### Deltas — the unit everything is typed in

Between one run and the next, something changed; the description of that change is a **delta**.
A delta comes in exactly two kinds:

- a **data delta** — rows changed in one of the model's inputs: new orders arrived, a customer
  row was corrected, an event was redelivered. This spec owns how data deltas are typed,
  planned, and folded.
- a **definition delta** — the model's own SQL changed. Its correctness statement is the same
  invariant with the model's function updated, its bookkeeping is the same frontier (below),
  but its workflow is different: a migration plan is presented and approved, never auto-applied.
  `definition_deltas.md` owns it; this spec only marks where it plugs in.

Not all data deltas are equally hard to absorb. smelt grades the *shape* of a change on a
three-point scale, from easiest to hardest:

1. **append-only within a window** — only new rows, and they land inside a bounded, recent time
   window. An event feed is the canonical case.
2. **keyed upsert** — rows are added or replaced, addressed by a key. The output of a keyed
   aggregation is the canonical case.
3. **general change** — anything: inserts, updates, deletes, anywhere. The least that can be
   assumed, and the most expensive to absorb.

The scale is ordered by what a consumer must be prepared to handle: a consumer that absorbs
`general` change absorbs anything; one that absorbs `keyed upsert` also absorbs an append of
fresh keys into a keyed relation; `append-only within a window` is the least a consumer must
handle. It is written `append-only within a window ⊑ keyed upsert ⊑ general` — read `⊑` as
"demands no more of a consumer than". (Element-wise, an append is a special case of an upsert
only for keyed relations whose appends carry fresh keys; over an unkeyed feed, append-only
relates to `general` directly — §"Delta signatures" states which shape × addressing pairs are
inhabited.)

### Delta signatures — what a relation emits

Every relation in the pipeline — a source or a model — has a **delta signature**: the typed
change it *emits* when it changes, stated per **column group** (a set of output columns that
always change together; what separates groups is which upstream changes each is sensitive to —
§"The plan matrix"). A signature has two parts:

- the **shape** of the change, on the three-point scale above;
- its **addressing** — how the changed rows are located: by time **window**, by **key set**, or
  only as **whole-table**.

A source's signature comes from its declared world-facts (`sources.md`: an `append_only` event
feed emits append-only-within-window deltas; a `mutable_snapshot` dimension emits general
change). A model's signature is **derived** from its SQL and its inputs' signatures — never
declared: a keyed `GROUP BY` over an append-only feed *emits* keyed upserts; a row-multiplying
join degrades what flows through it toward `general`. Signatures compose through the DAG, which
is what lets a chain of models be maintained incrementally end to end, and `smelt explain`
prints them per edge, naming the construct that degraded a type (§"Delta signatures").

This is the mental model to hold: **an incremental model is accumulated state, plus a function
from its inputs' delta signatures to its own, under a contract (below), with a frontier
recording what it has absorbed.** You declare what is true about the output; smelt types your
pipeline's changes end to end and shows you the plan.

### What you declare — two facts

A modeller declares `refresh: incremental` plus at most **two shape-defining facts** about the
output, and nothing else:

- a **clock** (`timeseries:`) — the output has a time axis (`event_time_column`,
  `partition_column`, `granularity`) consumers can window over;
- an **identity** (`unique_key:`) — the output is addressable by key, one row per key.

Everything beyond those facts — which maintenance technique runs where, how writes locate
stored rows, what each run scans, what bookkeeping exists — is **derived** from the model's SQL
and the declared facts, and printed by `smelt explain`. The machinery **validates** the
declaration and refuses when the SQL cannot uphold it; it never chooses a different shape for
you (§"Validator, not chooser").

The two facts are orthogonal and compose, giving three working stored shapes: a time-partitioned
table (clock only), a keyed lookup (identity only), and a time-partitioned keyed table (both).
The friendly name for a shape's addressing is its **grain** (`partition` / `key`) — a derived
label, not a mode. The per-shape implementation chapters — how each shape executes, what SQL is
admitted, and the diagnostics specific to each — live in `incremental_shapes.md`.

### The maintenance plan

For every maintained model smelt derives a **maintenance plan**: a set of **cells**, one per
combination of an output **column group**, a **trigger** (creation, mutation, definition
change, or backfill), and a **changed input** (which source or upstream model the trigger fired
for). Each cell records the **technique** that repairs it (rewrite a partition range; fold a
delta into keyed state; merge a single column; …), its **write addressing** — whether the write
locates stored rows by *region* or by *key* — and its **scan clamps**: the bounded window of
each input the cell reads.

Different cells of one model routinely derive different answers; that is the point. One model is
simultaneously append-driven, merge-driven, and recompute-driven at different cells, so no
single per-model "strategy" label could describe it. `smelt explain <model>` prints the plan;
none of it is declared.

### The frontier

The **frontier** is the bookkeeping record of which deltas each cell has already absorbed. For
columns where re-folding the same delta is harmless (`MAX`, `MIN`), the frontier is a simple
watermark; for columns where folding twice would double-count (`SUM`, `COUNT`), it records the
identity of every delta absorbed. It is what makes runs idempotent and resumable, and it is
shared by
both delta kinds: a definition delta is recorded as "this column group has processed nothing
yet, over every existing region", and migration catch-up advances that record under the same
rules as ordinary maintenance (§"The frontier"; `definition_deltas.md`).

### The graph layer

The **graph layer** lifts the plan to the DAG: given what landed upstream, which cells of which
downstream models must run over which regions (**forward propagation**) — and given a requested
output period, which upstream slices must exist first (**backward resolution**). Edges carry the
upstream's delta signature projected through the consumer, so a keyed stage no longer has to
kill the chain (§"The graph layer").

### Why cells differ — the three costs

The equivalence invariant fixes what the table must equal; it says nothing about how much work a
run does to get there. The plan exists because many physically different repairs reach the same
state, and they differ **only in cost**, which decomposes into three correlated but independent
dimensions:

- **Read cost** — how much input the run must scan: how cheaply the delta is discovered and how
  much input the repair needs (a fold consumes delta + stored state; a recompute re-reads the
  region). Neither always wins, which is why proven-interchangeable techniques are cost-modelled
  and measurable (`smelt bakeoff`), not fixed by shape.
- **Compute cost** — the engine work between read and write. smelt does not hand-compute minimal
  deltas; the engine evaluates the model's SQL, joins included, over a widened scan, keeping
  join optimisation where the optimiser lives. What smelt controls is the **unit of work**: a
  repair scoped region by region caps each statement's working set.
- **Write cost** — how the repair reaches stored rows: a **wholesale** write (`DELETE`+`INSERT`,
  swap) replaces a whole region, simple but rewriting unchanged rows; a **surgical** write
  (`UPDATE`, `MERGE`, column-scoped merge) touches only changed rows or columns, at the cost of
  needing row identity and change-comparability proofs (§"Per-cell write addressing").

The declared facts gate which write mechanisms exist at all; the proofs bound what must be read
and how small a unit of work may be; among mechanisms that survive admission, equivalence makes
the remaining choice a pure cost question — and freshness is the only thing at stake.

### Contracts — bounded, checked relaxations

The equivalence invariant is the **default contract**. A modeller may opt into a named,
parameterised **relaxation** that trades a bounded amount of equivalence for a capability the
default forbids — `frozen_horizon: H` (partitions older than `H` are never revisited; a late
arrival that would land there raises a diagnostic instead of silently folding),
`deferral: D` (the table may lag its inputs by up to `D`, licensing run skipping), and
`retain_departed` (keys the source no longer carries are kept — or tombstoned — instead of
deleted at reconcile). Each
relaxation is a triple — a declaration, a precise restatement of what the oracle becomes, and a
runtime probe that checks it — and `smelt explain` always prints what was relaxed (§"The
contract lattice"). Alongside the contract sit **probes** generally: declared facts about the
world (a source is append-only; a join key is unique) get cheap runtime tripwires that falsify
them, so "declared" means "checked in production", not "trusted forever" (`sources.md`).

### The running example

The spec's examples — shared with `incremental_shapes.md` and `definition_deltas.md` — draw
from one small warehouse:

- `sources.orders` — clocked order fact feed (`order_ts`; up to 2 days late), append-only;
- `sources.order_events` — clocked order-lifecycle event feed (`event_ts`), append-only;
- `sources.raw_events` — clocked event feed with redeliveries; any duplicate of an event
  arrives within 7 days of the first copy (declared `key_recurrence: '7 days'` — a source
  world-fact, `sources.md`);
- `sources.customers` — mutable dimension snapshot (`customer_id`, `tier`, `region`);
- `sources.customer_changes` — clocked update-events feed of customer attribute changes
  (`effective_ts`), one row per change, append-only.

| model | declares | shape |
|---|---|---|
| `daily_revenue` | clock | partition grain |
| `order_lifecycle` | identity | key grain (bare) |
| `order_facts` | clock + identity (joins `customers`) | composed — the per-cell-addressing example |
| `event_dedupe` | clock + identity | composed — the locality example |

One further model, `customer_history` (SCD2 over `customer_changes`), appears in §Limitations:
it is written as plain windowed SQL and is deliberately *not* a maintained shape.

### Reading guide

- *What can I write in frontmatter and on the CLI, and what errors can I get?* → §Surface here
  (shared surface); `incremental_shapes.md` §Surface (per-shape declarations and diagnostics).
- *What exactly does a run do, and why was my model refused?* → §Semantics here (the invariant,
  delta signatures, the contract lattice, the plan, windows and clamps, the frontier, the graph
  layer); `incremental_shapes.md` (per-shape execution and admission).
- *What happens on the first run, and how do I backfill?* → `incremental_shapes.md`
  §"First-run and backfill".
- *My model's SQL changed — what happens to the stored table?* → `definition_deltas.md`.
- *Why is it designed this way; was X considered?* → §Design (here and in each sibling).
- *What must never break?* → §Constraints & Invariants.
- *What does smelt deliberately not do?* → §Limitations.
- *Where does today's implementation fall short?* → §Known Divergences (each file carries its
  own).

## Surface

### The declared shape

The entire declared shape surface of an incremental model is the two shape-defining facts of
the Relation Contract (`models.md` §"The Relation Contract"):

```yaml
refresh: incremental        # the one refresh mode this spec covers
timeseries: { ... }         # the clock: event_time_column / partition_column / granularity (timeseries.md)
unique_key: [ ... ]         # the identity: makes the output key-addressable
grain: partition | key      # optional CHECK-ONLY assertion; drives nothing (key_per_partition is derived-only, see below)
```

The `refresh:` axis itself (including `full` and `materialized_view`) and the declaration law
are owned by `models.md` §"Refresh axis". The declarations name **shape-defining facts only**:
which technique realises which part of the output, and how each write physically addresses
rows, are per-cell derived properties (§"The plan matrix", §"Per-cell write addressing"), never
model-wide declarations, and the machinery validates the declared facts rather than choosing
them (§"Validator, not chooser").

**The two facts are orthogonal and compose.** Whether the output declares an identity and
whether it declares a clock vary independently. A model with both is a first-class shape, not an
edge case (`incremental_shapes.md` §"Key temporal locality (the time-partitioned output)").
Both axes are also orthogonal to **input consumption**: a bare keyed model over a clocked source
still consumes that source **window-forward** — stepping over the source's time windows in
order, rather than re-scanning it whole; the alternative, for a keyed model with no clocked
source, is **snapshot-reconcile** — re-scan the source whole and reconcile the keyed state
against it (both run shapes: `incremental_shapes.md` §"The two run shapes (derived, never
declared)"). A composed model's *output* clock is a property of its own stored shape, not of
its sources.

**Grain is a derived label.** `grain` is a classification computed from
`(clock?, identity?, partition_column ∈ key?)`, reported by `smelt explain`, and computed for
sources too (a source likewise has an effective grain: clocked-fact, keyed-dimension, …). A
modeller who wants the friendly name in frontmatter may write `grain: partition` or
`grain: key` only as a **check-only assertion**: it errors on mismatch with the derived facts
(`models.md` §"Constraint violations") and drives nothing. Declaring `grain: key_per_partition`
is a hard error at config parse — there is no writable spelling for the trajectory shape (the
key recurring across partitions); the error names the two facts that derive it (a `timeseries:`
clock and `partition_column ∈ unique_key`) and `grain: key` as the closest supported declared
shape. The declared *facts* stay one-per-node, so two declarations of one node can never
disagree. The single fact `partition_column ∈ unique_key` is what distinguishes the trajectory
from a keyed lookup whose key has a fixed home slice; the same fact reappears as
key-temporal-locality routes 1 and 2 (`incremental_shapes.md` §"Key temporal locality (the
time-partitioned output)").

The per-shape declaration walk-throughs — worked frontmatter for the partition grain and the
key grain, the column-family catalogue a keyed projection must classify into, and the
shape-specific configuration (`safety_overrides`, `columns.<c>.contract: plausible`) — are
`incremental_shapes.md` §Surface.

### Maintenance overrides (`maintenance:`)

Almost every model declares none of this. The `maintenance:` frontmatter block steers *choice
among proven-equivalent techniques* and states *expectations* the derived plan is checked
against — it never widens what admission allows:

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

- The override ladder is `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
  scope winning; `technique:` alone bypasses the cost model. Overrides select among
  **admissible** techniques only — an override can never select an inadmissible one (§"Per-cell
  admission"). (Cost-model and `prefer` consumption status: §Known Divergences.)
- `<source-address>` names the changed input the cell handles, as the model's SQL refers to
  it — a source (`sources.customers`) or an upstream model (`order_facts`). A worked pin from
  the running example — steer `order_facts`'s tier-correction cell to an unconditional write:

  ```yaml
  maintenance:
    cells:
      - columns: [customer_tier]
        on: sources.customers
        technique: unconditional
  ```
- `suppress`/`unconditional` are an orthogonal dimension from `fold`/`recompute`: they never
  change which technique family a cell resolves to, only whether a suppressible cell's matched
  arm writes conditionally (§"Windowed maintenance and the horizon", pruning category 2).
  `technique: suppress` on a cell whose write-suppression proof did not hold (no proven row
  identity, or a compared column not proven comparable across runs) is refused like any pin
  naming an unadmitted technique; `technique: unconditional` never refuses.
- `cells[].write` is a **hard per-cell addressing pin**: an open name resolved against the
  write-pattern registry, not a sealed keyword set (§"Per-cell write addressing"). Every pin is
  validated against the equivalence invariant for its cell — an addressing that cannot uphold
  equivalence is refused with `MaintenanceWriteAddressingRefused`, and an unrecognised name, or
  one the target backend cannot execute, is refused with `MaintenanceWritePatternUnavailable`.
  Never a silent downgrade.
- `cells[].columns` naming columns that span two derived groups is an error (it would silently
  re-partition the plan).
- `scan_bounds` is **check-only**: it never modifies a clamp; it only refuses (or warns) when
  the derived plan exceeds the stated expectation. A project-level default in `smelt.yml` sets
  the baseline; per-model blocks refine it.
- A sibling **top-level** frontmatter key, `horizon_ceiling: '<interval>'` (partition grain
  only), declares a ceiling on the derived horizon — a compile-time warning threshold, never a
  clamp modification (§"Windowed maintenance and the horizon").

### Contract relaxations (`contract:`)

`contract:` is a sibling top-level frontmatter block to `maintenance:`, opting a model into one
or more lattice points (§"The contract lattice"). Where `maintenance:` steers *choice among
proven-equivalent techniques* and never widens what admission allows, `contract:` does the
opposite: it names a bounded, checked relaxation of the equivalence invariant itself.

```yaml
contract:
  frozen_horizon: '90 days'      # partition grain only
  deferral: '6 hours'
  retain_departed: true          # keyed shape over a mutable snapshot only
  # retain_departed: {tombstone: <col>}   # alternative form: mark departure instead
  cells:                          # optional per-cell refinement, addressed like maintenance.cells
    - columns: [<col>, ...]
      on: <source-address> | backfill
      deferral: '1 day'
```

- `frozen_horizon: '<interval>'` is admitted **only on a partition-grain model**; declaring it
  on a key-grain model (which has no write-eligibility clamp, §"Windowed maintenance and the
  horizon") is a configuration error, `ContractFrozenHorizonInvalid`. An unparseable or negative
  interval is the same error.
- `deferral: '<interval>'` is admitted on either grain, model-level or per cell, but only where
  there is a clock to measure lag against: a model-level `deferral` requires the model to carry
  a `timeseries:` clock, and a `cells[]` entry's `deferral` requires its `on:` trigger to be a
  clocked, interval-representable source. `on: backfill`, an unclocked source, and a
  `mutable_snapshot` source each have no frontier to measure lag against and each raise
  `ContractDeferralInvalid`.
- `retain_departed: true` (or `retain_departed: {tombstone: <col>}` to mark departed keys in
  a column instead of merely keeping them) is admitted **only on a keyed shape consuming a
  mutable snapshot** — the one posture where departure is observable and deletion is the
  default (§"The equivalence invariant", key departure). Declaring it on any other shape or
  posture, or naming a tombstone column absent from the model's output, is a configuration
  error, `ContractRetainDepartedInvalid`.
- Model-level values are the default for every cell; a `cells[]` entry — addressed the same way
  as `maintenance.cells[].columns` / `.on` — refines one cell's `deferral`. `frozen_horizon`
  and `retain_departed` are
  model-level only (each governs the model's write behaviour as a whole, not a single cell's).
- An unparseable or negative `deferral`, or a `deferral` on a model or cell with no clock to
  measure lag against, is `ContractDeferralInvalid`.
- Absent `contract:` (the common case) is the default point: strict equivalence, no relaxation.
- The effective contract per cell — default or relaxed, with the relaxation's parameters — is
  always printed by `smelt explain`; a relaxation is never silent (§"CLI").

### CLI

- `smelt explain <model>` — prints the plan (a worked rendering is §"Per-cell write
  addressing"'s `order_facts` example). Its sections:
  - **Headline**: the model's **delta signature** — for a bare keyed model,
    `emits: keyed upsert over [order_id], key-addressed`; for a composed model, the same with
    its locality slice bound appended (the worked example below); for a partition-grain model,
    `emits: append-only within a window, window-addressed by order_date` — with the derived
    `grain` label alongside as the friendly name.
  - **Per cell**: the cells with their addressing, scan clamps, locality verdicts, the
    effective contract point, and the **per-column guarantee ledger** — the printed summary
    of what each output column is guaranteed (its equivalence contract and its **settle
    bound** — the derived interval after which a written slice provably receives no further
    changes, so consumers may treat it as final; a volatile column prints its determinism
    exemption in place of an equivalence contract, §"The equivalence invariant" determinism
    scope).
  - **Per inbound edge**: the derived **delta-signature shape** — `append-only within
    window`, `keyed upsert`, or `general` — the shape of change that edge's own upstream
    emits (§"Delta signatures"; a source edge is typed by its declared mutation profile, a
    model edge by the upstream's own derived verdict). A `general` edge names the construct
    or world-fact that degraded it (an unregistered operator, a row-multiplying join, a
    source with no declared `mutation_profile`); an edge with no derivable verdict prints no
    delta-type row at all rather than a fabricated one.
  - **Decomposed-state columns**: for every presented column that folds through decomposed
    state (`incremental_shapes.md` §"Decomposed state (rung 2) in keyed models"), the hidden
    state columns and the presentation map `π` that recomputes the presented value from them,
    labelled as internal state and explicitly not part of the model's public schema; a model
    with no decomposed-state columns prints no such section.
  - **Repair cells**: a per-group recompute cell (§"The repair family") additionally prints
    its **affected-key slice** (labelled a sound over-approximation), its **bounded per-group
    read slice**, and its **affected-key discovery mechanism** — the clamped current-source
    scan, the group-grain fingerprint-sidecar diff for a `mutable_snapshot` source (§"The
    repair family"), or, for a key-addressed model-edge cell (§"The graph layer"), the
    group-grain fingerprint-sidecar diff over the upstream's own output table — and, when a
    `write: diff_patch` pin matches the cell, the resolved **write mechanism** and its
    **delete-leg verdict**. Every other cell prints none of this.
  - **`--show-sql`**: additionally prints each cell's emitted maintenance statements — the
    same emitters' output a run executes (§"Statement emission (single owner)"; flag surface
    in `cli.md`).
- `smelt run --since-upstream --source <address> [--landed <start>..<end>]` (`--source`
  repeatable, `--landed` repeatable and optional per source) — **forward propagation**: the
  caller declares what landed for each source since it last propagated, or omits `--landed`
  for a source with a persisted watermark (`run_state.md` §"Per-source watermark") to have
  smelt treat `watermark → now` as that source's landed delta, refined live by the recorded
  observed-delta table where a record exists (model upstreams); the graph reflects
  those per-source deltas through the edges and runs exactly the propagated per-edge regions
  with their trigger cells (§"The graph layer"). `--source` accepts a declared source or an
  upstream maintained model (a model's landed delta is the output window a completed run
  wrote). A source named with neither a matching `--landed` nor a persisted watermark
  propagates nothing, and the refusal names the missing watermark — never a silent no-op and
  never an implicit full-table fallback. Opt-in; the intended default
  posture once trusted. Prints the **dirty set** — the per-model regions propagation says must
  run (§"The graph layer") — before acting.
- `smelt run --auto` — process only the intervals the run-state interval ledger
  (`run_state.md`) does not yet cover for the selected models; the keyed grain's staleness
  interaction is `incremental_shapes.md` §"Interaction with `--auto` / staleness".
- `smelt build <model> --period <start>..<end> --include-upstreams` — **backward resolution**:
  print the per-ancestor required slices and build order; optionally execute the bounded build
  (§"The graph layer").
- `smelt rebuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]` — a
  **ranged re-run**: rebuild a model (and, with selectors, its upstreams) over a time range
  using its ordinary maintenance plan, with batch-safety-aware chunking
  (`incremental_shapes.md` §"First-run and backfill"). A data-side verb, deliberately disjoint
  from definition-delta migration (`definition_deltas.md` §"`smelt rebuild`").
- `smelt migrate <model>` — definition-delta migration: derive, present, approve, and apply the
  plan for a change in the model's own SQL. Owned by `definition_deltas.md` §Surface.
- `smelt bakeoff <model> [--cells <col>@<source>,...] [--runs N] [--target <name>] [--keep] [--pin]`
  — measures every admissible technique for a set of cells against a representative window of
  real data and reports cost. `--cells` defaults to every cell with two or more admissible
  techniques. `--runs N` (default 3) splits the driving source's event-time extent into `N`
  sequential windows and replays them in order per technique; each replay is a real
  `execute_project` run against the project's actual data. Each measured technique runs against
  a scratch target: the chosen target is cloned in-memory under a synthetic name with schema
  `smelt_bakeoff_<model>_<technique>` (no runtime schema seam — schema already flows from
  `config.targets[target].schema`), dropped after measurement unless `--keep`. After each
  window the measured techniques' outputs are cross-checked against each other with
  `EXCEPT ALL` in both directions — the equivalence bakeoff exploits is verified, not assumed.
  `--target` selects which declared target to clone (default: the active target). `--pin` emits
  the winning `cells[]` entry (or a complete `maintenance:` block) as ready-to-paste YAML on
  stdout; it never rewrites the model's `.sql` file. An applied pin is an ordinary override,
  re-validated through admission on every compile.

`cells[].technique` pins and `prefer` preferences are honoured at execution: the same choice
ladder that governs `smelt bakeoff`'s measurement targets resolves the technique a live run
uses, and admission still binds.

**Run flags.** Every run is told its window — directly via the event-time flags, via
`--landed`, or via the interval ledger (`--auto`). `--landed` becomes optional per source once
a persisted watermark exists for it (`run_state.md` §"Per-source watermark"): `smelt run
--since-upstream` with no `--landed` for a source propagates `watermark → now` as its landed
delta, refined live by the recorded observed-delta table where a record exists; an explicit
`--landed` always overrides. Automatic
**snapshot diffing** of an external source with no watermark and no native delta feed remains
§Future Extensions. Which flags a model takes follows from its derived run shape:

```
smelt run     --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # partition grain; keyed window-forward
smelt rebuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # same, batch-safety-aware chunking
smelt run     [selectors]                                                             # keyed snapshot-reconcile
```

- Both flags are required for any direct partition-grain run; a forward-propagation run
  (`--since-upstream`) derives its regions from `--landed` instead. Format: ISO-8601
  (`2026-03-20`, `2026-03-20T00:00:00Z`). The end bound is **exclusive**.
- The supplied `[start, end)` range is the **run window**. It must be a positive integer
  multiple of `timeseries.granularity`, aligned to granularity boundaries (`timeseries.md`
  §"Granularity arithmetic"); run-window size may exceed partition granularity
  (`incremental_shapes.md` §"Run window vs partition granularity"). `smelt rebuild` uses the
  model's batch-safety class to expand or split the range (`incremental_shapes.md` §"First-run
  and backfill").
- For a **window-forward keyed** model, both flags are required and address the **driving
  source's** `partition_column`/`granularity` — never a column of the keyed output, even when
  an admitted output `timeseries:` block exists (run flags always address the source's clock).
- For a **snapshot-reconcile** keyed model (no clocked source), the flags are a **hard error** —
  *"model has no clocked driving source; run without event-time flags"*. Each run is a whole
  reconciliation.

### Diagnostics

All codes are catalogued in `diagnostics.md`. This spec owns the semantics of the shared plan
codes and the contract-lattice codes below; the partition-grain and key-grain codes are owned by
`incremental_shapes.md` §Surface, and the definition-delta code by `definition_deltas.md`
§Surface. Every rejection is fail-loud and fail-closed: nothing degrades to a silent fallback
(§"Validator, not chooser").

**Shared plan codes (`Maintenance*`).**

| Code | Fires when |
|---|---|
| `MaintenanceNoAdmissibleTechnique` | No technique survives a cell's admission; names the cell (§"Per-cell admission"). |
| `MaintenanceReachNotDerivable` | A required scan bound is neither derivable nor declared (§"Per-cell admission" obligation 4). |
| `MaintenanceScanUnbounded` | A scan/footprint cannot be partition-bounded (or exceeds a declared `max_lookback`) and no `allow_full_scan` acceptance exists (§"Partition-local maintenance (the K8 guardrail)"). |
| `MaintenanceUnboundedFootprint` | A targeted write was requested for a cell whose write footprint is unbounded, e.g. a stored trajectory under late data (§"Per-cell admission" obligation 5). |
| `MaintenanceGraphUnsupportedNode` | A cyclic edge set, an inadmissible self-referential model, or a keyed node whose delta signature degrades to `general` in the propagation graph (§"The graph layer"). |
| `MaintenanceWriteAddressingRefused` | A `cells[].write` pin names an addressing that cannot uphold the cell's equivalence invariant; names the cell and the refused pattern (§"Per-cell write addressing"). |
| `MaintenanceWritePatternUnavailable` | A `write:` pin names an unrecognised pattern, or one the target backend's capability registry does not provide; names the pattern and the backend (§"Per-cell write addressing"). |
| `MaintenanceRepairKeysNotDiscoverable` | The repair family's affected-key-discovery obligation fails: a changed input's delta cannot be resolved to a finite output key set; names the changed input and why the delta yields no key set (§"The repair family"). |
| `MaintenanceRepairSliceUnbounded` | The repair family's bounded-per-group-read-footprint obligation fails: the key→input-slice reach is neither derived nor declared-and-checked; names the source and the unbounded reach (§"The repair family"). |

`MaintenanceSkeletonColumnAdded` — a definition delta adding or changing a field in a skeleton
position — is owned by `definition_deltas.md` §Surface.

**Contract-lattice codes.**

| Code | Fires when |
|---|---|
| `ContractFrozenHorizonInvalid` | A `contract.frozen_horizon` is unparseable or negative, declared on a non-partition-grain model, or declared on a model whose driving source is not `append_only` (the probe's count comparison would be blind — §"The contract lattice"); names the failing condition (§"Contract relaxations (`contract:`)"). |
| `ContractLateArrivalOutsideHorizon` | Runtime probe, frozen-horizon point only: a frozen-band partition's baseline row count increased (or a new partition appeared in the frozen band); names the partition, the added row count, and `H` (§"The contract lattice"). |
| `ContractDeferralInvalid` | A `contract.deferral` (model- or cell-level) is unparseable or negative, or declared on a cell with no clock to measure lag against (§"Contract relaxations (`contract:`)"). |
| `ContractDeferralExceeded` | Runtime probe, deferral point only: the ledger-derived lag between a cell's maintained frontier and its input frontier exceeds the declared `D`; names the cell and the measured lag (§"The contract lattice"). |
| `ContractRetainDepartedInvalid` | A `contract.retain_departed` is declared on anything other than a keyed shape consuming a mutable snapshot, or names a tombstone column absent from the model's output; names the failing condition (§"Contract relaxations (`contract:`)"). |

## Semantics

The shared machinery, in dependency order: what a delta is and how it is typed (delta
signatures), the invariant every maintained model upholds and its declared relaxations (the
contract lattice), the plan that organises maintenance (cells, admission, addressing, repair),
the mechanics a chosen technique executes under, the frontier, and the graph layer. The
per-shape machinery — how a partition-grain or key-grain model actually executes — is
`incremental_shapes.md`.

### Delta signatures

Every relation — a declared source or a maintained model — has a **delta signature**: per
output column group, the typed change it emits, with two components:

- **shape**, on the three-point scale `append-only within a window ⊑ keyed upsert ⊑ general
  change`, ordered by how much a consumer must be prepared to handle (§Overview "Deltas");
  any operation that cannot preserve a point degrades toward `general` — never the other way.
- **addressing** — how the changed rows are located: by time **window**, by **key set**, or
  **whole-table**.

Shape and addressing are not free to combine arbitrarily; the inhabited pairs are:
`append-only within a window` is window-addressed (its bound *is* a window);
`keyed upsert` is key-set-addressed, refinable by a window slice where key temporal locality
holds (`incremental_shapes.md` §"Key temporal locality (the time-partitioned output)");
`general` pairs with any addressing, coarsest whole-table.

A **source's** signature is fixed by its declared world-facts (`sources.md`): `append_only`
plus a `timeseries:` clock emits `append-only within a window`, window-addressed; a
`mutable_snapshot` emits `general change`, whole-table-addressed (refinable to a key set where
a fingerprint sidecar can name the changed keys — §"The repair family"); a `change_feed`
declares its own delta shape.

A **model's** signature is **derived, never declared**: the output-delta shape proof
(`model_properties.md` §"Output-delta shape") types what the model's SQL emits given its
inputs' signatures — a keyed `GROUP BY` over an append-only feed emits `keyed upsert` over its
key columns; a windowed pass-through preserves `append-only within a window`; an unregistered
operator or a row-multiplying join degrades the verdict to `general`, and the proof records
*which construct* degraded it, so `smelt explain` can name it. Signatures are stated per column
group because different output columns of one model can carry different verdicts (a stable
dimension column vs an additive measure).

Signatures are consumed in three places, each specified in its own section: **admission** reads
the input side (what a cell may assume about a changed input — §"Per-cell admission");
**edges** carry the upstream's signature projected through the consumer (§"The graph layer");
and **`smelt explain`** prints them (§"CLI"). A signature never fabricates: where no verdict is
derivable, the relation has no printed signature and consumers assume `general`,
whole-table-addressed — widening, never narrowing.

### The equivalence invariant

Every maintained (non-`full`) model upholds **one** invariant, stated over an abstract
**processed-input set** `S`: an incremental run produces the result a full refresh would,
restricted to the inputs processed so far.

```
incremental_state(S) == full_refresh(source | input ∈ S)
```

`S` is a set of *source rows or partitions the runs have scanned* — not necessarily
clock-addressed. The **partition-set form** (`source | partition_col ∈ S`), used throughout
this spec, is the **clocked specialisation**, available whenever the driving source carries a
`timeseries:` clock; an unclocked (snapshot) source has no partition set to slice by, and its
specialisation is stated per shape profile (the key grain states it over "keys present in the
current snapshot" — `incremental_shapes.md` §"End-state equivalence: the SQL is the oracle").
The right-hand side depends only on the *set* `S`, so **order/set-determinacy is a corollary
for every shape** — trivially so for the partition grain's disjoint-union combiner, but present
nonetheless.

**Landed vs processed, and the evaluation point.** Write `L` for the **landed-input set**:
every input that has arrived in the model's sources, whether or not any run has processed it
(`S ⊆ L` always). The invariant constrains what the table says about the inputs *processed*;
it deliberately says nothing about freshness — at the default point, how far `S` lags `L` is
bounded only by run cadence, and bounding it by contract is the deferral point's job (§"The
contract lattice"). For **mutable** inputs, `S` records the input *version* each cell last
consumed, and the executable oracle substitutes the current version; the invariant is
therefore evaluated at **quiescence** — after any run sequence that has consumed every landed
delta, data and mutation alike, the stored table equals the full refresh over the current
inputs. Between a mutation landing and its consuming run, the table reflects the previously
consumed versions. Two mutation-interleaved histories accordingly agree at quiescence, not at
every intermediate step; the order/set-determinacy corollary is a statement about matching
processed sets, evaluated there.

The invariant covers both delta kinds. For data deltas, `S` grows as runs scan inputs. For a
**definition delta**, the model's function on the right-hand side is updated to the new
definition, and `S` is unchanged — the stored table must come to equal the *new* definition's
full refresh over the inputs already processed (`definition_deltas.md` §"The oracle").

**Strengthenings, not peer contracts.** Where an output slice depends only on its own bounded
input slice, the invariant is additionally checkable slice-by-slice: **per-partition
equivalence** (the partition grain, `incremental_shapes.md` §"Per-partition equivalence") and
**per-slice equivalence** (the keyed analogue, once key temporal locality is established,
`incremental_shapes.md` §"Key temporal locality (the time-partitioned output)") — both
strengthenings of the one invariant, never a second one. What actually distinguishes the shapes
is how their writes **address rows**, a per-cell fact (§"Per-cell write addressing");
key-addressed shapes discharge the *same* invariant because their writes reach stored rows by
key, wherever they live.

**The replayability split.** Full equivalence — an executable `full_refresh` oracle a test can
run — holds only for **replayable inputs**: a set `S` the model can re-evaluate its own SQL
over (a clocked source's processed partitions; a snapshot's currently-present keys), exactly
what per-column admission enforces (`incremental_shapes.md` §"Admission matrix (column family ×
source shape)"). Non-replayable combinations (a fold over scanned-and-since-mutated versions;
a fold needing unreplayable history) are not admitted **under the scanned-version oracle**;
snapshot-consuming cells are admitted against the current-snapshot oracle instead (above), and
the rest could one day get a weaker, never-smuggled-in **observer / prefix-consistency
contract** (§Future Extensions).

**Key departure follows the source posture.** Deletion is derived, never declared: the
default behaviour is whatever preserves the full-refresh equation for the posture actually
consumed. An append-only source never loses a key, so nothing departs and nothing is deleted.
A mutable snapshot's oracle is the current snapshot, so a key present in stored state but
absent from the incoming scan is **deleted** at reconcile — an anti-join over the scan the
model already performs (`incremental_shapes.md` §"The two run shapes (derived, never
declared)"). A windowed scan over a mutable source cannot observe departure at all — the
window carries no tombstone — so the affected region is recomputed, or the shape refused;
departure is never silently retained. A change feed's delete events are applied as
retractions. Keeping departed keys is available only as the declared **retention** relaxation
(§"The contract lattice"), never as a silent default.

**The determinism scope.** The promise extends exactly as far as determinism does: where two
full refreshes of the model would themselves disagree, no equivalence is promised. Volatile
expressions — `NOW()`, `CURRENT_DATE`/`CURRENT_TIMESTAMP`, random — run **as-is**, never
compile-time pinned and never rejected for volatility alone. The per-column determinism
verdict (`model_properties.md`) marks every column such an expression reaches; the
conformance oracle's comparison exempts those columns, and `smelt explain`'s per-column
guarantee ledger prints the exemption in place of an equivalence contract. The same verdict
gates every technique that relies on recompute-equality — change-suppression comparisons,
diff-then-patch probes — which exclude volatile columns or refuse, fail-closed, rather than
reading their inevitable drift as data change. Volatility in a **row-placement or identity
position** (partition placement, key derivation, a membership predicate) is different in
kind — writes could not address rows stably — so those positions still require deterministic
expressions (the placement taint rule, `incremental_shapes.md` §"Safety checks (per-cell
admission for recompute-a-region)").

**One named carve-out**, a consequence of the executable-oracle requirement rather than a gap
in it: **ordering-key ties** on an order-monotone overwrite column (equivalence holds up to
ties, since ordering-key uniqueness is not statically provable — `incremental_shapes.md`
§"Ordering ties (order-monotone overwrite)"). Every `model_properties.md` property and
`model_transforms.md` transform is proven/licensed in service of this invariant: the
smelt-driven shapes discharge it via the generative equivalence oracle (§References);
`refresh: materialized_view` discharges it via the **engine's** native IVM, with no smelt
combiner (`materialized_view.md`).

### The algebraic maintenance ladder

What a key-addressed model can maintain is fixed by the **algebra of its combiners**, not by
any backend feature. The ordering criterion is invertibility → maintainability: the raw
*discriminants* (is-monoid, needs-inverse, decomposable, value-vs-order-monotone) are
properties of the SQL owned by `model_properties.md`; the ordering and the
maintainable-vs-delegated cutoff are the maintenance consequence, owned here. The invariant
holds on every rung — only state representation and size change, never the fidelity of the user
value.

1. **Direct monoid.** The stored column *is* the answer; the combiner is a commutative monoid
   (associative, commutative, identity = the empty partition): `SUM`/`COUNT` (`+`), `MIN`/`MAX`,
   `BOOL_*`, `BIT_*`. The identity is realised in SQL as `NULL` with null-skipping combiners —
   `COALESCE`-guarded in the emitted merge, since a naive `state + delta` would null out a
   key's total.
2. **Decomposed monoid.** The user value is `π(state)` for a richer monoid element and a pure
   presentation map `π`: `AVG` = `(sum, count)` presented `sum/count`; variance = a moment-sum
   triple `(n, Σx, Σx²)`; approximate distinct = an HLL register vector. Where that state
   physically lives for the key grain, which column families it licenses, and how it stays
   invisible to consumers is `incremental_shapes.md` §"Decomposed state (rung 2) in keyed
   models".
3. **Group.** When inputs can change (corrections, reprocessing, deletes) the combiner must be
   **invertible** — a commutative group (`SUM`, `COUNT`, `BIT_XOR`) — for the change to fold
   in. Monoids that are not groups (`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR`) cannot un-see a
   contribution *by folding*; reprocessing them requires recomputation — cell-wide (full
   refresh) or, where affected-key discovery and slice completeness discharge, the repair
   family's bounded per-group recompute (§"The repair family").
4. **Opt-in bounded-domain multiset.** Holistic aggregates needing all rows (exact
   `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`, `DISTINCT`-modified
   aggregates) are maintained by storing the per-key value→count multiset. Retaining per-value
   counts is what lets even the non-invertible `MIN`/`MAX` be maintained under retraction —
   the new extremum is re-derived from the surviving domain, at `O(active domain)` per touched
   key — and the signed-count form (the "Z-set" of the IVM literature) additionally tolerates
   retractions folded ahead of their matching insertions. `π` is defined only over states with
   no negative count; a state left negative at presentation is a detected fault, not a value.
   **Opt-in and fail-loud**: state is `O(active domain)`, so an unbounded-state
   aggregate is refused by default (suggesting the approximate form or `refresh: full`) unless
   the modeller supplies a bounded-domain budget, and the runtime caps the multiset with a
   full-refresh fallback.

Rungs 1–4 are the boundary of what smelt maintains itself, via a `merge_into` loop (optionally
with a presentation view for decomposed state); general-operator retraction over joins and
unbounded non-additive state delegate to `refresh: materialized_view`'s engine-native IVM.

### Validator, not chooser

The machinery **validates** the declared shape — the `refresh:` value plus the shape-defining
facts, and any check-only `grain:` or `write:` assertion — against the derived properties, and
rejects fail-loud when the SQL cannot uphold the shape's contract. It **never chooses or
silently switches** the shape or the addressing. A full refresh is the honest fallback surfaced
as a diagnostic, never an automatic downgrade. Per-cell technique choice among
proven-interchangeable techniques (§"Per-cell admission") stays inside this rule: it may change
freshness, never observable bits at a fixed processed-input set.

### The contract lattice

The equivalence invariant stated above is the **default point** of a small declared lattice: a
modeller may opt a model or cell into a named **relaxation** that trades a bounded, checked
amount of equivalence for a capability the default point cannot offer (skipping stale-window
recompute, licensing a bounded write clamp). A relaxation is never ambient — it is declared,
validated, probe-checked at runtime, and always printed by `smelt explain`, never a silent
weaker default.

**A lattice point is admissible only as a complete triple, single-owned in `smelt-logical`:** a
declaration schema, a pure oracle transform (what
`incremental_state(S) == full_refresh(...)` becomes at this point, restated below per point),
and a probe emitter. The conformance gate (`maintenance_conformance`, root `CLAUDE.md`
§"Architectural invariants") consumes the oracle transform directly rather than encoding its
own comparator, and runtime probes emit from the same definition — mirroring the
statement-emission single-owner rule (§"Statement emission (single owner)"). Users pick and
parameterise a point; they never define one. The lattice defines three relaxations — frozen
horizon and deferral (chosen first for having the clearest oracles and probes), and
retention:

**Frozen horizon (`H`), partition grain only.** Declaring `frozen_horizon: H` clamps writes
**by contract** to output partitions within `H` of the current run — narrowing (never
widening) the derived horizon clamp (§"Windowed maintenance and the horizon"); a partition
older than `H` is never revisited even if the model's own reach would otherwise cover it. The
oracle is stated per **output partition**. Let `freeze(p)` be the first run at which partition
`p` fell outside `H`, and `S_before(p)` the inputs processed by runs preceding it. For every
unfrozen partition `p`:
`incremental_state.where(partition = p) == full_refresh(source | input ∈ S).where(partition = p)`;
for every frozen `p`, the same equation over `S_before(p)`. In words: unfrozen partitions are
exact as of now; **frozen partitions are exact as of the moment they froze** — a timely input
whose reach (lookback, window frame, skew) would rewrite a frozen partition has that part of
its effect deliberately dropped, which is precisely the relaxation bought. When the model's
derived reach never exceeds `H`, the two statements coincide and the point adds only the
probe.

The clamp is a declared contract rather than only a derived reach bound, so a genuinely late
arrival landing outside `H` is a checked condition — but the maintenance scan never reads a
row whose partition is already frozen, so the probe is **baseline-comparative**, not
scan-based: each run snapshots a per-partition row-count baseline over the **driving source's**
frozen band (partitions strictly before `end − H`; a read-only aggregate, outside the
maintenance scan the clamp bounds), and the next run compares it against the source's current
state. A frozen-band partition whose row count increased, or that is new since the baseline,
is a genuine late arrival, and the probe raises `ContractLateArrivalOutsideHorizon`, naming
the partition, the added row count, and `H` — closing the one accepted silent-data behaviour
of the default point for every model that opts in. An absent baseline — the first run, a
deleted `.smelt/`, or a posture that excludes it (`state.mode: stateless`) — degrades the
probe to baseline-establish-only, reported `ProbeBaselineUnavailable` (`state.md` §"Diagnostics");
`ContractLateArrivalOutsideHorizon` cannot fire without a baseline to compare against. This
split — `frozen_horizon` degrades where `contract.deferral` (below) refuses — follows directly
from `state.md` §"The optionality rule": the frozen-band baseline is an **observability**
structure (§"The state-structure inventory"), so its absence only narrows what the probe can
verify, never what the maintained table equals, whereas `contract.deferral`'s lag is measured
from the reconciliation ledger — a **correctness** structure whose absence no posture can
license working around (`DeclaredContractRequiresState`, `state.md` §"Diagnostics"). Count comparison is sound
only where the source is `append_only` (row counts non-decreasing); declaring `frozen_horizon`
on a model whose driving source has any other declared mutation profile is refused at
declaration time (`ContractFrozenHorizonInvalid`, naming the posture) rather than probed
blind.

**Deferral (`D`).** The oracle bounds the lag between the landed and processed sets (§"The
equivalence invariant", landed vs processed): at every scheduled evaluation, every input in
`L \ S` — landed but not yet processed — arrived within the last `D`, and equivalence over
the processed set stays strict: `incremental_state(S) == full_refresh(source | input ∈ S)`.
The maintained state may omit inputs no older than `D`, so long as it exactly reflects
everything it has processed and nothing older than `D` is left unprocessed. What this buys
over the default point is the **scheduling licence**: the default point never licenses
declining scheduled work, while `deferral: D` licenses two forms of it — **run skipping**,
when a run's entire pending input set is within `D` of arrival (nothing outside the window is
left unfolded, so skipping cannot violate the oracle); and **work subsumption**, when a pending
small run's input set is a subset of a larger run already scheduled within `D` (the ledger —
the reconciliation ledger, §"The frontier record (reconciliation ledger)" — proves the subset
relationship before the smaller run is dropped). The probe is ledger-derived
— the maintained frontier is the cell's own latest recorded interval end, the input frontier
the latest covered end across its clocked inputs' recorded landings — and raises
`ContractDeferralExceeded`, naming the cell and the measured lag, whenever
`lag = input_frontier − maintained_frontier` exceeds `D`.

The same two frontiers drive scheduling: `0 < lag ≤ D` licenses a skip, recorded on the run
manifest — the per-run record of what each cell did — as `skipped_deferral` (never silently
omitted), leaving the target table and interval
ledger untouched. `lag ≤ 0` or an unresolved frontier (the cell's first run) always falls
through to the normal path — skipping is a licensed relaxation, never the fallback, and never
available past `D`. A deferral skip propagates to every selected dependent, recorded
`skipped_deferral_upstream` — a dependent that ran while its upstream was deferred would record
coverage for a window its upstream never folded and never revisit it, the exact silent hole the
default point forbids. Work subsumption is proven from two ledger facts, never inferred from
range coverage alone: a prior manifest recorded `skipped_deferral` for this cell, **and** the
current run's write range covers that cell's pending window
(`(maintained_frontier, input_frontier]`); when both hold, the covering run's manifest records
the subsumed window alongside its normal `success` outcome.

**Retention (`retain_departed`), keyed shapes over a mutable snapshot.** At the default point
a key absent from the current snapshot is deleted at reconcile (§"The equivalence invariant",
key departure). Declaring `retain_departed` keeps such keys — history over currency, the
SCD-flavoured trade — optionally naming a **tombstone column** in which departure is marked.
The oracle is a **quotient that ignores departed keys**: for every key present in the current
snapshot, the strict per-key equation holds unchanged; a stored key absent from the current
snapshot is exempt from comparison (and, when a tombstone column is declared, must be marked
in it). The probe is the reconcile scan's own anti-join — the very computation that would
otherwise have deleted — recording the retained-departed key count on the run manifest, with
the retained set's tombstone marking checked where declared. The point is meaningful only
where departure is observable and deletion is the default, so it is admitted **only on a
keyed shape consuming a mutable snapshot**; declaring it anywhere else is a configuration
error (`ContractRetainDepartedInvalid`).

The points compose with `grain`, column families, and `maintenance:` overrides without a new
mode: a relaxed cell resolves technique via the same admission rule (§"Per-cell admission"),
checked against its restated oracle. A definition delta interacts with the frozen-horizon and
deferral points through the
plan-and-approve gate, never silently: a migration whose catch-up would enter a frozen band
surfaces the conflict on the presented plan, and a deferral skip never defers definition-delta
catch-up (`definition_deltas.md` §"Interaction with the contract lattice").

### The plan matrix

Every maintained model has a **maintenance plan**: pure data, derived once, consumed everywhere
(§Constraints & Invariants), cells keyed by `(output-column-group × trigger × changed-input)`.

**Column groups.** The plan factors the output columns into groups by shared
mutation-sensitivity (`model_properties.md` §"Per-column mutation-sensitivity / column
provenance" owns the proof and degenerate-collapse rule; this spec consumes the groups).
Creation is shared by every column — a new row's columns are computed together; mutation is
what partitions them. Sensitivity carries its kind into the cell. A **value-sensitive** group's
mutation cell may be repaired column-scoped (a `MERGE` rewriting the group's columns in place).
A **membership-sensitive** group — governed by a mutable source read in **row-admission
position**, i.e. in a join predicate or filter that decides whether an output row exists at
all (`model_properties.md`) — must be repaired by a row-creating/deleting technique: the
recompute family (delete+insert, change-suppressed where comparable), never a column-scoped
merge, which cannot fix which rows exist. A mutable join partner never read in any select item
still produces membership sensitivity — absence from every value-sensitivity set is not
admissibility for cheaper repair. The one admissible pruning is a proof: an enrichment join
proven unable to add or remove output rows (its **skeleton-source closure** proven `Closed`
over a provably outer join — `model_properties.md`) contributes none, since its deltas
provably cannot change which rows exist. A group whose sensitivity set spans **two or more
mutation-sensitive inputs** (a merged group — one output value drawing on multiple sources
that can each mutate) is repaired by **region recompute**, never a column-scoped merge: no
single input's delta determines the merged value, so the conservative, always-correct default
applies, the same posture every other blended-provenance rule in this spec takes.

**Triggers.** Four trigger classes index the plan:

- **creation** — new rows arrived in the driving source;
- **mutation** — a post-creation delta in a source some column group is mutation-sensitive to;
- **definition delta** — the model's own SQL changed (`definition_deltas.md` owns this
  trigger's classification, workflow, and plan policy; its cells sit in the same plan matrix
  and dispatch through the same machinery, behind the plan-and-approve gate);
- **backfill** — an explicit region recompute from replayable input.

Each trigger pairs with the **changed input** it fires for — the source, upstream model,
self-edge, or definition diff whose delta drives the cell — making "what runs when *this*
input changes" a first-class, per-input answer (the model's **scope maps**, surfaced by
`smelt explain`): a driving source's delta engages the windowed fold; a mutable dimension's
delta the delta-driven probe and horizon-bounded merge; a self-edge ordered execution; a
definition diff a targeted column catch-up (all `model_transforms.md`). The same column group
under the same trigger class can derive *different* write addressing for different changed
inputs (§"Per-cell write addressing").

**Each cell carries:** its **quadrant** of the read-scope × write-scope grid (below); its
**technique**, from the open write-pattern registry (§"The repair family"); its **write
mechanism** — the available-addressings rule, or a validated `write:` pin (§"Per-cell write
addressing"); its **derived scan clamps** per read source (§"Windowed maintenance and the
horizon"); its **partition-locality verdict** per source (§"Partition-local maintenance (the
K8 guardrail)"); and its **obligations** and **traded guarantees** (per-column: equivalence
contract × settle bound).

**The read/write grid.** Each cell occupies a quadrant of **read scope** (delta+state vs the
region's full upstream input) × **write scope** — the cell's physical write addressing
(targeted addresses vs region overwrite):

|              | write: targeted (keyed addressing) | write: region-overwrite (partition addressing) |
|---|---|---|
| **read: delta+state** | fold-a-delta | read-modify-write region |
| **read: full-input** | column-scoped re-derivation | recompute-a-region |

Recompute-a-region is contract-agnostic and unconditionally valid over replayable input; the
fold quadrant is contract-specific (needs a combiner algebra — §"The algebraic maintenance
ladder"). The repair family (§"The repair family") is recompute-a-region's targeted-write
refinement: the **column-scoped re-derivation** quadrant, scoped to a finite key slice,
inheriting recompute-a-region's correctness argument rather than needing its own. Where
interchangeability holds (§"Per-cell admission"), a region recompute **supersedes and resets**
what folds had written. "Unconditionally valid" is correctness, not admission or cost: it
holds even for a whole-table region, whose admission is gated separately by the
partition-locality guardrail (§"Partition-local maintenance (the K8 guardrail)").

The plan is **derived, never declared** — the model's shape-defining facts are validated
against it, an error on mismatch, never a silent flip. `smelt explain` prints every cell, its
addressing, clamps and locality verdicts, the per-column guarantee ledger, and the model's
inbound edges.

#### Per-cell admission

A technique enters a cell's plan space only when all of its obligations discharge (fail-closed;
an unrecognised construct refuses, never defaults). The obligations, each with its owner:

1. **Replayable input** (recompute family) — the source is re-readable at its current processed
   set; declared posture, `sources.md`.
2. **Faithful fold** (fold family) — source posture × combiner algebra both hold
   (`model_properties.md` §"Faithful-fold conditions"); either alone failing (e.g. retractions
   into a non-invertible combiner) refuses the fold family.
3. **Combiner algebra class** — derived, fail-closed (`model_properties.md` discriminants); a
   holistic or unrecognised combiner leaves only the recompute family.
4. **Bounded reach** — the scan bound `(clock_col, before, after)` per source is derived
   (`model_properties.md` §"Unified bound / reach derivation") or declared-and-checked; absent
   both, full-input only (`MaintenanceReachNotDerivable` when the trigger requires a bound).
5. **Bounded footprint** (targeted writes) — the write-scope reflection of the scan bound is
   bounded (`model_properties.md` §"Footprint reflection / bounded write footprint"); a
   trajectory column's unbounded forward footprint fails this
   (`MaintenanceUnboundedFootprint`).
6. **Well-defined groups** — the mutation-sensitivity partition is computable
   (`model_properties.md`); degenerate collapse is surfaced, never silent.
7. **Affected-key discovery** (repair family only) — a changed input's delta resolves to a
   finite, soundly over-approximated output key set (`model_properties.md` §"Affected-key
   discovery"); an unresolvable delta shape refuses the repair family by name
   (`MaintenanceRepairKeysNotDiscoverable`).

**Interchangeability and choice.** Two techniques serve one cell interchangeably iff, at a
fixed `S`, they produce identical state on the columns deciding which rows exist — the
`S`-indexed refinement of the equivalence invariant, `S` a **per-input vector** once the plan
factors: bit-preserving for faithful idempotent columns, state-preserving **modulo the ledger**
for additive ones (never fold a delta already reflected in state — fold-then-recompute is safe,
recompute-then-refold double-counts). Choice among interchangeable techniques belongs to the
cost model or operator (`prefer`/`technique`), governed throughout by §"Validator, not
chooser".

### Per-cell write addressing

Every cell derives its **physical write** — how it locates the stored rows it updates — from
the currently known write-pattern set, an **open registry**, not a closed enum:

```
{ region DELETE+INSERT, keyed MERGE, column-scoped MERGE, in-place UPDATE, full rebuild, diff_patch, … }
```

**The available-addressings rule.** A write mechanism is admitted for a cell iff
`available = (which contract facts the output declares) × (what the trigger/changed-input needs) × (the equivalence invariant) × (backend capability)`.
The first three factors are structural; the fourth is the target engine's capability registry
(`architecture.md`). Keyed `MERGE` / column-scoped `MERGE` / in-place `UPDATE` require a
declared `unique_key` (row identity); region `DELETE`+`INSERT` requires a declared partition
axis (`timeseries:`). A **bare lookup** (identity, no clock) admits only keyed merge or full
rebuild; a **bare partition table** (clock, no identity) admits only region rewrite or full
rebuild — gaining keyed dimension-change addressing requires declaring `unique_key`, making it
the composed clock-and-identity shape (`incremental_shapes.md` §"Key temporal locality (the
time-partitioned output)"); declaring identity is **load-bearing** (it admits keyed writes),
never a dedup footnote. A cell with no admissible write mechanism is
`MaintenanceNoAdmissibleTechnique`, naming the cell.

**Addressing is how a row is found, not how far the statement ranges.** Choosing keyed `MERGE`
picks row-location by identity; it does not make the statement table-wide. When the output also
declares a `timeseries:` axis, the write stays **bounded to the affected partitions**: the
changed-input delta resolves to the touched partitions first, and keyed `MERGE` is emitted per
partition (or with a partition predicate) against just those. A whole-table `MERGE` is reached
only when the cell **provably cannot** be bounded to a partition set — unboundedness is itself
a derived per-cell fact, fail-loud, never a default. Partition-scoping is orthogonal to
addressing (§"Partition-local maintenance (the K8 guardrail)"). **User pins**:
`maintenance.cells[].write` names the write mechanism per cell (§Surface), selecting among
*admissible* mechanisms without ever widening the admissible set — refused with
`MaintenanceWriteAddressingRefused` when the addressing cannot uphold the equivalence
invariant, and with `MaintenanceWritePatternUnavailable` when the name is unrecognised or the
backend cannot execute it.

**Worked example — the plan of a composed model.** `order_facts` (running example) declares
both `unique_key: [order_id]` and a `timeseries:` clock on `order_ts`/`order_date`, joining
mutable `customers` to project `c.tier AS customer_tier`; `smelt explain order_facts` prints
this plan (illustrative):

```
model order_facts  (emits: keyed upsert over [order_id], key-addressed,
                    slice-bounded by order_date under key temporal locality;
                    grain: key, time-partitioned — clock + identity declared)
cells:
  [all columns        × creation  × orders]     recompute-a-region   write: region DELETE+INSERT
      scan: orders(order_date, -0d, +0d); customers(full — lookup)
  [customer_tier      × mutation  × customers]  column-scoped merge  write: keyed column MERGE
      scan: customers(delta probe); target scoped to touched partitions
  [all columns        × backfill  × orders]     recompute-a-region   write: region DELETE+INSERT
```

One model, three cells, two addressings: new orders rewrite their partitions; a tier
correction merges one column by key into just the partitions the affected orders live in —
neither verdict declared, pinning either (`cells[].write`) validated, not trusted.

#### The write-pattern set is open (and partly backend-provided)

The patterns named above are the ones understood *today*. The set grows — partition/atomic
swap (Delta/Iceberg `REPLACE PARTITION`), copy-on-write vs merge-on-read, an
unmatched-by-source prune variant, staged-upsert, a predicate-targeted `UPDATE` locating rows
by something other than the row key, incremental MV refresh, engine-specific primitives — and
the durable contract is deliberately **not** the enumeration: the enumeration is data.

- **The invariant is the admission function, not the enum.** A new pattern is admitted by
  declaring which contract facts it requires (identity? a partition axis? ordered arrival?)
  and discharging the equivalence proof obligation for the cells it serves; grain stays
  derived, the cost model ranks whatever the rule admits, and a new mechanism can never be
  less correct than the ones it joins, since the equivalence gate is the price of entry.
  Concretely, a dimension-mutation cell could one day be served by an `UPDATE` locating rows
  through the **join key** (`customer_id`) rather than `unique_key`, by declaring a proven
  functional dependency from join key to the repaired columns and discharging equivalence —
  today's registry instead serves it with keyed column `MERGE` (worked example above).
- **The pattern set is backend-relative, and `write:` is an open, fail-loud vocabulary.**
  Engines differ sharply on atomic partition swap, true `UPDATE`, and merge-on-read, so
  admission carries backend capability as a fourth factor via the backend's capability
  registry (`architecture.md`), where backend-specific optimisations are *contributed* rather
  than special-cased in the planner, keeping a portable project from silently depending on a
  primitive only one engine has. `write:` pins resolve against this same registry, not a
  sealed enum: an unrecognised pin, or one naming a pattern the backend cannot provide, is
  refused with a diagnostic, never silently downgraded.

**`diff_patch` — compute, diff, write only the difference.** A pattern for reconciliation runs
and idempotent re-runs: the candidate rows for a slice are computed, diffed against the
slice's stored state, and only the difference is written — inserting absent rows, updating
rows whose compared columns differ, deleting stored rows absent from a *complete* candidate
set. It requires a declared `unique_key` for the insert/update legs and change comparability
(`model_properties.md` §"Change comparability") for the update leg; the delete leg
additionally requires **slice completeness** (the repair family's correctness premise, §"The
repair family") and without it degrades explicitly to insert+update, a reduced-capability
admission rather than a silently dropped delete leg. `diff_patch` is graded **Idempotent** (a
second run against unchanged input diffs to empty), making it the reconciliation and
drift-repair write; its slice is the *candidate's own* — affected-key set for a per-group
recompute, partition region for a windowed one — so it is not tied to a partition axis.
Backends **execute** registered patterns; they never **author** maintenance-statement text
(§"Statement emission (single owner)").

#### The repair family

A non-invertible combiner refuses reprocessing outright when a merged window's input changes
(`incremental_shapes.md` §"Reprocessing") — full refresh is the only universally correct
fallback. The repair family narrows that refusal for one common case: when the change is a
**retraction or mutation** whose affected output keys are provably finite
(`model_properties.md` §"Affected-key discovery"), the plan recomputes *only those groups*
from their bounded input slice instead of rebuilding the whole table or region — the
**targeted-write refinement of recompute-a-region**: the same full-input read, addressed by
key rather than region, landing in the **column-scoped re-derivation** quadrant of the
read/write grid (§"The plan matrix"), and like a region recompute it **supersedes and resets**
the ledger for the keys it rewrites (§"Per-cell admission", interchangeability).

**Why it is correct.** Recomputing key set `K` over an input slice that provably contains
*every* row contributing to any `k ∈ K` reproduces `full_refresh` restricted to `K`; keys
outside `K` stay bit-identical, so the equivalence invariant (§"The equivalence invariant")
holds cell-wide. The load-bearing premise, **slice completeness**, is not a new proof but a
reuse of **key temporal locality** (`incremental_shapes.md` §"Key temporal locality (the
time-partitioned output)"), whose purpose is establishing that a key's contributing rows lie
within a computable slice of the input. **Admission obligations**: a repair cell is admitted
only when §"Per-cell admission" obligations 4 (bounded reach — the key→input-slice reach,
derived via a key-temporal-locality route or declared-and-checked), 6 (well-defined groups —
the walk's grain names the groups recomputed), and 7 (affected-key discovery: the changed
input's delta names a finite key set; a sound over-approximation is admissible, an
under-approximation never is — a missed key leaves stale state for a touched group) all
discharge — fail-closed, any one unprovable refuses the repair family by name, never widening
to a whole-table repair, naming which obligation failed
(`MaintenanceRepairKeysNotDiscoverable` / `MaintenanceRepairSliceUnbounded`).

**Obligation 7 over a `mutable_snapshot` source.** This posture keeps no tombstone or change
history, so a key whose *entire* window contribution was deleted between runs leaves no row
for a current-source scan to select — an under-approximation obligation 7 forbids. Instead:

- **The discovery mechanism** is the **group-grain fingerprint sidecar diff** (`sources.md`
  §"The fingerprint sidecar" — "Partition grain", one stored digest row per output group
  key): a vanished group still surfaces on the "sidecar row with no matching source key" leg,
  with no source row left to name it.
- **The discovery read is unbounded by the cell's `ScanClamp`** — a clamped rescan against
  the sidecar's full digests would flag every out-of-clamp group as spuriously changed,
  degrading to a whole-table repair every run. The per-group *recompute* stays bounded by the
  discovered key set per obligation 4.
- **A missing or stale stored digest widens soundly.** Where the sidecar's stored digests are
  absent or stale-stamped (`sources.md` §"The fingerprint sidecar" — "Invalidation"), a
  vanished group cannot be told apart from one that never existed, so the affected set widens
  for that run to every currently-observed group *plus* every group already in stored output
  — a sound over-approximation (a runtime record being absent, distinct from admission
  refusing an unprovable obligation), degenerating to a whole-table repair for that run and
  self-healing once the sidecar refreshes.
- **An append-only source keeps the ordinary clamped scan** — no native deletion, so the
  sidecar is scoped to the one posture that needs it.

**Ledger grading and re-run safety**: per-group recompute is graded **Idempotent** for the
keys in its slice, exactly like a region recompute (`incremental_shapes.md` §"The
transactional frontier write (merge ledger)") — re-running reproduces the same state and
resets any additive ledger record rather than folding a second time on top of it.

**Repair over a decomposed combiner.** Its fold path (`incremental_shapes.md` §"Decomposed
state (rung 2) in keyed models") materialises hidden `__`-marked state columns alongside its
presented ones, so a repaired group's candidate must carry them too or the write's implicit
column list mismatches the physical table. The repair candidate is therefore the model's own
**state-augmented** projection — raw model SQL widened with the state columns' own
`per_partition_expr`s before compilation, identical to the widening
`execute_windowed_keyed`/`execute_snapshot_reconcile` apply for the ordinary fold — a no-op
for a stateless column family, and thus for every combiner admitted before decomposed state
existed. A `diff_patch` write over a decomposed repair extends its change-suppression
predicate over the hidden state columns too: a group whose presented value is unchanged but
whose state moved is still rewritten — strictly less suppression than presented-only
comparison, sound by construction.

### Maintenance mechanics

This group owns how a cell's chosen technique executes: scan/write windowing, the
partition-locality guardrail on the SQL windowing produces, and who authors that SQL. The one
trigger that plans differently from an ordinary delta — the model's own SQL changing — is
`definition_deltas.md`.

#### Windowed maintenance and the horizon

Maintenance runs over a **bounded input window by default** — a full scan is the surfaced
fallback, not the baseline. A run reasons about two windows, always with `scan ⊇ write`: the
**write window** (partitions or keys written this run) and the **scan window** (input rows
read to produce it correctly).

- **The scan window** is bounded **where the model carries a `timeseries:` clock** — only the
  new window plus a lookback is read, stored state standing in for history — and degrades to a
  full read without a clock (`models.md` §"Input-consumption axis"). Scan windowing is
  orthogonal to output addressing: a clocked *key-addressed* model still windows its **scan**
  even though its **write** reaches back by key outside that window. It never weakens the
  invariant: the engine evaluates the model, joins included, over the widened scan window, and
  the write is **clamped** to the exact write window (the two-layer widened-scan + exact-output-clamp transform, `model_transforms.md`), so join optimisation stays with the engine rather than smelt hand-computing
  minimal deltas.
- **The horizon (partition grain only)** is a **write-eligibility clamp** — a bound on which
  partitions a run may write to, the far edge of the maintained window past which inputs are
  no longer folded in. It is **derived**, never trusted from a declaration: the clamp comes
  from the model's own reach (lookback, window frames, join contribution —
  `model_properties.md`), since a declared horizon smaller than the true reach would drop rows
  that should have been rewritten. A modeller may declare a horizon *ceiling*
  (`horizon_ceiling: '30 days'`): smelt warns at compile time when the derived horizon would
  exceed it, and the clamp always uses the derived value.
- **Late arrivals at the default point.** Because the clamp *is* the model's SQL, a genuinely
  late arrival — landing after its natural partition passed the horizon — is **silently
  excluded** at the **default point**, not diagnosed: smelt cannot fail loud on a row it never
  scans. **Surfacing lateness is a model-author concern, not a maintenance guarantee**, unless
  the model opts into the **frozen horizon** contract-lattice point (§"The contract lattice"),
  turning this into a checked, diagnosed condition (`ContractLateArrivalOutsideHorizon`). The
  default-point pattern is to fold the late row into the current partition (re-stamping its
  partition time) carrying a lateness flag, so data still flows and a data-quality check can
  raise on flagged rows:

  ```sql
  SELECT
      CAST(ingested_at AS DATE)                    AS partition_date,   -- arrival time places the row
      order_date                                   AS original_date,
      CAST(ingested_at AS DATE) > order_date + 2   AS is_late,          -- flag for a data-quality check
      ...
  ```

  (Placement must stay deterministic — the taint rule bars `CURRENT_DATE` from any
  row-placement position — so the re-stamp uses the feed's own arrival timestamp,
  `incremental_shapes.md` §"Safety checks (per-cell admission for recompute-a-region)".)

**The key grain has no write-eligibility clamp.** A `grain: key` run merges **every** delta
row it scans, into whatever key it names, however old (`incremental_shapes.md` §"No
write-eligibility clamp"); a derived forward reach is still computed for observability but
never gates admission or bounds a write — deliberate: keyed write work is proportional to
delta size regardless of key age, so a clamp buys nothing for correctness and would silently
drop scanned inputs, the one thing the invariant forbids. What it would buy (settled-key GC, a
bounded working set) is deferred, shipping only with late-fact accounting if ever introduced.
**Three pruning categories, one principle:** *only proofs prune; a declared bound is admitted
only checked (fail-loud on violation); no unproven bound ever refuses a write.*

1. **Target-scan slice pruning** (read-side) — rows the write provably cannot touch are
   removed from the merge's *read* of stored state; licensed by the key-temporal-locality
   proofs or the transactionally-checked recurrence declaration (`incremental_shapes.md`
   §"Key temporal locality (the time-partitioned output)").
2. **No-op write elimination** (write-side) — a maintenance write is skipped **iff** the row's
   applied effect is proven the identity, per row *by evaluation*: an exact
   `IS DISTINCT FROM` comparison over every column that can differ under the cell's trigger
   (the mutation-sensitive group alone, since the rest are proven insensitive). Suppression
   never skips **evaluating** a scanned input — that is the separately-licensed
   delta-restricted enrichment compute (`model_transforms.md`, skeleton-source-closure proof).
   A compared column must be a pure function of the processed inputs; one varying run to run
   (`contract: plausible`, a volatile `NOW()` — the determinism verdict, §"The equivalence
   invariant" determinism scope) is incomparable and refuses the technique,
   fail-closed. At fixed `S` the suppressed and unconditional variants are interchangeable in
   the strongest sense of §"Per-cell admission" — a cost-model/`prefer`/`technique` choice.
   `model_transforms.md` owns the two realisations, both licensed by region row identity plus
   per-column change comparability: change-suppressed MERGE (a matched-arm
   `IS DISTINCT FROM` predicate, dialect-split on the unmatched-by-source side) and the
   staged-candidate conditional DELETE+INSERT (the merge-less realisation for a backend
   without `MERGE`).
3. **Write-eligibility clamps** — forbidden on the key grain; derived-only on the partition
   grain (the horizon above).

Categories 1–2 preserve the invariant bit-for-bit at fixed `S`; category 3 bounds which
inputs enter `S` at all, different in kind. A suppressed write is the write-side dual of slice
pruning (the proof is the per-row equality just evaluated), never a clamp. Two catalogued
transforms read a *derived* forward reach without being write clamps: the dimension-driven
horizon-bounded MERGE (a scan/recompute bound on the enrichment recompute, not the write) and
the horizon settled-delay/tail-rewrite mechanism (partition-grain forward-reach machinery),
both in `model_transforms.md`.

#### Partition-local maintenance (the K8 guardrail)

**K8** is this guardrail's short name, used across code comments and sibling specs; it names
the policy this section defines and nothing else. A cell's per-`(cell × source)` locality
verdict is the **partition-locality projection** (`model_properties.md` owns the proof,
including the cross-axis predicate requirement); this section owns the policy consuming it: emitted maintenance SQL must carry the partition
predicate on **both** the scan and the merge/overwrite target, since a bound stated only on a
non-partition column is one the storage layer cannot prune by. Under the default `scan_bounds`
(`require: partition_local`, `on_violation: error`), a non-local cell refuses
(`MaintenanceScanUnbounded`) unless the source carries `allow_full_scan: true`; `max_lookback`
additionally refuses a derived span wider than the operator's stated expectation. The
guardrail never modifies a clamp — it only refuses or warns (§Surface "Maintenance overrides
(`maintenance:`)").

#### Statement emission (single owner)

The physical statements a run executes for a cell — region `DELETE`+`INSERT`, keyed fold
`MERGE`, column-scoped `MERGE`, in-place `UPDATE`, first-run `CREATE TABLE … AS` — are
produced by pure emitter functions in the maintenance layer (`smelt-logical`): the
statement-level counterpart of "one derivation, many consumers". An emitter is a pure function
from plain data — target table, region literals, key columns, combiner-rendered set
expressions, the compiled/clamped SELECT body, a dialect tag — to an ordered statement group
plus its transactional requirement (a paired `DELETE`+`INSERT` is one transaction: a failed
`INSERT` rolls back its `DELETE`). Backends *execute* emitted statements and never author
maintenance-statement text; dialect differences (e.g. a full-row source projection versus an
explicit column-list `SET` for `MERGE ... UPDATE`) live in the emitters as dialect-keyed
variants. Three deliberate exclusions, all warehouse-resident bookkeeping owned per dialect by
`smelt-state`, interleaved transactionally with the write each describes but not itself a
maintenance statement: the reconciliation ledger's DDL/DML (§"The frontier record
(reconciliation ledger)"); the observed-output-delta record (§"The graph layer"); and the
fingerprint sidecar's own storage — DDL, digest-refresh upsert, GC delete (`sources.md` §"The
fingerprint sidecar") — excepting the sidecar's **diff query**, which IS emitter-authored
since which source keys count as "changed" is a derived maintenance-relevant comparison
(`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`). Non-maintenance SQL
(introspection, seed loading, schema-evolution DDL) is outside this rule; single ownership is
what makes maintenance SQL *observable* — the same emitters serve execution, the conformance
equivalence gates, and `smelt explain --show-sql`, so printed SQL cannot drift from executed
SQL. Definition-delta migration statements are inside this rule: they come from the same
emitter layer (`definition_deltas.md` §"Plan-and-approve").

### The frontier

A **frontier** is the record of which typed deltas a cell has absorbed — the plan's one ledger
concept, addressed by the cell's own delta signature (§"Delta signatures") and graded by
combiner algebra (§"Per-cell admission"). An **additive** grade records delta identities,
because a never-fold-twice check needs them; an **idempotent** grade records only a watermark,
because re-folding the same delta is harmless. Two operations, defined once here and
specialised by each realisation below: **fold** (for an additive grade, refuse if the delta is
already reflected in the frontier's recorded state; for an idempotent grade, an
already-reflected delta may re-fold — it converges; otherwise combine and extend the record)
and **recompute-reset** (a recompute resets the frontier for exactly the region or keys it
read, to the input it actually read — never fold ahead of a reset).

Both delta kinds live on the same frontier. A definition delta sets the affected column
groups' record to "nothing yet" over every existing region; migration catch-up then advances
it region by region under the same fold and never-fold-ahead rules, which is what makes
migration resumable and lets data deltas keep folding into unaffected groups mid-migration
(`definition_deltas.md` §"Frontier semantics").

The frontier has two named realisations, differing in addressing grain and storage, not in
these semantics: the **frontier record** (below), the `(output-region × column-group)`
bookkeeping every derived plan maintains; and the **transactional frontier write** — the
per-model backend table a window-forward keyed model's merge writes transactionally
(`incremental_shapes.md` §"The transactional frontier write (merge ledger)"). No divergence
entry may describe one realisation as lacking a concept the other tracks — per-cell addressing
is simply unbuilt for the frontier record's realisation (§Known Divergences), not a foreign
concept. Both realisations are **correctness structures** under `state.md`'s classification:
their normative residency is engine-resident, transactional with the write each describes
(`state.md` §"The residency rule"), and their availability on a given backend is what the
degradation contract resolves against (`state.md` §"The degradation contract"). This spec owns
the frontier's semantics; `state.md` owns where a realisation may live and what happens when
none is available.

#### The frontier record (reconciliation ledger)

Each entry records the processed-input vector `S_{i,g}` of one
`(output-region × column-group)` pair, graded per §"The frontier": additive groups record
delta identities, idempotent groups a watermark. Region↔window attribution is exact under key
temporal locality or explicit footprint tracking; a delta is attributed to the unique region
containing its footprint. A definition delta is a fold-family operation on this record: it
instantiates the affected groups' entries at `S = ∅` over every existing region
(`definition_deltas.md` §"Frontier semantics").

The record is engine-resident, graded by algebra into two backend tables: additive delta
identities in `_smelt_ledger`, idempotent frontier watermarks in `_smelt_frontier`. A region
recompute's reset (delete every intersecting `(region, group)` row, insert the input state the
recompute read) commits in the same backend transaction as the recompute's own write, on a
backend with a ledger builder (DuckDB today). The record is written per **recomputed batch
region** — a run's window is partitioned into batches, and each batch's reset writes its own
region's row inside that batch's own write transaction, rather than one row for the run's whole
window; the region-intersecting delete keeps these finer per-batch rows collapsible under a
later coarser reset. Never `.smelt/`-resident — that residency belongs to run-state
observability, not this correctness structure (`run_state.md` §"Relationship to the
reconciliation ledger"; `state.md` §"The residency rule").

### The graph layer

**Edges.** A dependency edge is `downstream reads upstream` under the downstream cell's
derived scan clamp, between two partition axes whose grain is the declared
`timeseries.granularity` of each node — never per-edge, never derived from the SQL (the
classifier only *checks* the declaration, e.g. against a `date_trunc` grouping). Clamp margins
ceil **outward** to whole partitions; each hop aligns its result outward to the receiving
axis's grain. Outward maps are monotone, so sufficiency composes; narrowing never does.
**Widen-never-narrow** is the graph layer's composition law.

**Typed edges.** An edge carries a **vector** of typed components, one per column group the
downstream consumer reads: `(delta shape × addressing × column set)` — the upstream's delta
signature (§"Delta signatures") projected through the consumer's own per-column
mutation-sensitivity. Today's day-interval dirt is exactly the `AppendOnlyWindow` component
under window addressing — forward propagation and backward resolution, below, are the
window-addressed case of this general vector, and the adjoint property
`forward(backward(P)) ⊇ P` continues to hold for that case unchanged. Widen-never-narrow
governs every addressing, not only the window-addressed one: a component whose type cannot be
projected through a consumer degrades to the coarsest component that consumer can act on
(whole-model dirt), never to nothing.

**Keyed dirt-sets and the narrowed refusal.** A keyed node without an admitted time axis is
not categorically refused. Where its delta signature is `keyed upsert` (the `KeyedUpsert`
output-delta verdict, `model_properties.md` §"Output-delta shape") over key set `k`, the edge
is key-addressed and propagates a **keyed dirt-set** carrying the **affected key values** —
not only the key columns and provenance — resolved by `model_properties.md` §"Affected-key
discovery", instead of an interval. Propagation stays a **pure function**: key values enter it
as *seed* input exactly as landed intervals do — the caller resolves them once (the group-grain
fingerprint-sidecar diff over the upstream's own output table, below, "Upstream model edges")
and passes them in; propagation composes them through edges by projecting the upstream's key
columns onto each consumer's own key scope.

**Composition rules.** A keyed component into a keyed consumer whose key scope the projection
resolves stays key-valued; one whose keys cannot be resolved through the consumer's grain
widens to whole-model dirt for that consumer — never nothing, never a silent key drop. The
`MaintenanceRepairKeysNotDiscoverable` refusal, below, continues to govern the *cell*; this
rule governs the *dirt*.

**Unresolved seeds.** A keyed edge whose values were not resolvable (a non-DuckDB target, or
no sidecar) propagates the **symbolic form** — key columns and provenance, no values — and
widens at dispatch (§"Dispatch — from propagated components to run units"): the honest
degradation, not an empty key set. Empty-and-resolved (nothing changed) and unresolved are
distinct, the same way an empty observed delta and an absent one are (§"Observed deltas on
model edges").

The `MaintenanceGraphUnsupportedNode` refusal below fires only where the node's delta signature
degrades all the way to `general` (the `General` verdict), and its message names the operator
that degraded the type.

**Upstream model edges.** A maintained model's ref to another maintained model in the same
project is a plan edge of the same standing as a `sources.*` ref: the upstream model's own
validated `timeseries:` declaration supplies the clock the downstream creation cell is clamped
by, and scan bounds compose through the chain exactly as the propagation graph composes them.
An upstream-model ref whose clock cannot be derived (the upstream declares no `timeseries:`
and none is inferable) is a recorded refusal on that cell (`MaintenanceReachNotDerivable`,
naming the edge) — never a silent drop. A ref to a `full`-mode or view upstream derives no
creation cell (there is no incremental delta to receive); it participates in
mutation/backfill triggers only. For forward propagation, `--source` accepts either a declared
source or an upstream maintained model; a model's landed delta is the output window a
completed run wrote for it.

An upstream maintained model whose own derived delta signature is `keyed upsert` contributes a
**key-addressed** edge instead: no `timeseries:` clock is required on the upstream at all, and
a keyed-grain downstream (one with no `output_partition_col`) receives such a cell too — the
partition axis the clock-based route clamps against does not exist on either side of a
key-addressed fold. The cell's read restriction is the affected key set the upstream's delta
names (`model_properties.md` §"Affected-key discovery"), recomputing and writing only those
key groups (`Technique::PerGroupRecompute`, §"The repair family" — folding an upstream upsert
delta is exactly "recompute and write only the affected key groups", the repair family's own
definition). `MaintenanceReachNotDerivable` narrows accordingly: it fires only for a clockless
upstream whose derived shape is *not* key-addressed (`append-only within window` or `general`)
— a clockless `keyed upsert` upstream is admitted via the key-addressed route rather than
refused. The fail-closed leg is explicit: when the downstream's own SQL does not carry the
upstream's key columns (they cannot be resolved through the downstream's own grain), the edge
is refused by name (`MaintenanceRepairKeysNotDiscoverable`) rather than falling back to a
silent whole-table cell.

A key-addressed cell's affected-key set is discovered from the **group-grain fingerprint
sidecar diff** over the upstream's own output table (§"The repair family" — "Obligation 7 over
a `mutable_snapshot` source"), keyed at the upstream's key columns: a clockless keyed upstream
is, from the consumer's own view, exactly a mutable snapshot with no clock to clamp a scan by,
so the sidecar's stored comparandum is what bounds the read instead — a clamp-less
`SELECT DISTINCT` over the upstream would degenerate to a full-table rescan, which the
per-group recompute technique this cell admits exists specifically to avoid. The
changed upstream keys are then projected through the upstream relation onto the downstream's
own key columns (the key columns this cell's key scope names, `KeyScope::keys`):
`SELECT DISTINCT <key_expr(key_scope.keys)> FROM <upstream_table> WHERE <upstream key expr> IN
(<changed keys>)`. The candidate recompute is the downstream's full (unwindowed) SQL
semi-joined to that key relation, and the write is the repair family's own targeted
`DELETE`+`INSERT` (or the `write: diff_patch` write leg, when pinned) — identical to any other
per-group-recompute cell's lowering once its affected-key relation and candidate
select exist; only how the key relation is discovered differs. The upstream sidecar partition
refreshes in the same backend transaction as the downstream's write, so a failed write never
leaves the sidecar advanced past a change it did not actually consume. The sidecar
comparandum (the stored digests being compared against) must behave **per consuming edge**:
each `(upstream, downstream)` pair's diff-then-refresh cycle is its own, so one consumer's
refresh can never make a sibling consumer's next diff report "no change" for a delta that
sibling never consumed — the classic shared-cursor bug of change-capture consumers, and the
"wrong-and-quiet" outcome this layer forbids. The same per-consumer requirement governs a
`mutable_snapshot` source feeding several models (§"The repair family"; storage layout owned
by `sources.md` §"The fingerprint sidecar", which upholds this requirement).
This discovery route is
DuckDB-only, matching the sidecar's existing posture elsewhere in this spec — a non-DuckDB
target dialect refuses by name before any backend call, never a silent widening to a
full-table read. A `key_scope` key the upstream relation does not carry is a fail-loud
refusal, never a widening to every key.

**Forward propagation — what must run.** Runs are driven by **what landed**, per source, as
partition intervals on that source's own axis; a cron tick is only the poller. Processing
nodes in topological order, each node's merged dirt reflects through each outgoing edge — an
upstream delta of `[a, b)` dirties downstream `[a − after, b + before)` — accumulating:

- **per-edge dirt** `(model, upstream) → intervals`: keys the trigger cell — the plan cell for
  that inbound source runs over exactly these regions (recompute for a driving-source delta,
  column-scoped merge for an enrichment delta);
- **per-model dirt** (the union across inbound edges): what that model's own consumers see as
  *their* upstream delta.

Running exactly the per-edge dirty regions with their cells must leave every model equal to a
full refresh (sufficiency); partitions outside the dirty set are never scheduled. A delta on a
source nothing reads, or an empty delta, propagates nothing. A delta on an **unclocked**
source dirties the **whole model** for every mutation-sensitive consumer — never a silent
no-op (the cell was only admitted under `allow_full_scan`, so the full-table run is a declared
cost).

**Backward resolution — what must exist.** Given a target model and period `[s, e)` (aligned
outward to the target's grain), walking the ancestor sub-DAG in reverse topological order and
applying each edge's clamp directly — `[s, e)` requires upstream `[s − before, e + after)` —
yields, for every ancestor, the partition intervals that must exist (a data prerequisite for a
raw source; a build region for a model) plus the build order. This is the bounded
test/validation build: stage exactly the resolved source slices, build bottom-up, and the
target period equals a build over complete history. The required slice of an unclocked source
is the whole table. The two directions are **one-sided inverses at best** ("adjoint, not
inverse"): `forward(backward(P)) ⊇ P` — resolving what a period needs and propagating it
forward may over-cover the period, never under-cover it.

**Observed deltas on model edges.** A model edge's propagated delta follows the same
landed-delta refinement as a source edge (`sources.md` §"Landed-delta (derived, recorded)"):
where a run recorded an **observed output delta** — the changed-row set a conditional write
(§"Windowed maintenance and the horizon", category 2) actually touched, restricted to
comparable columns — that set, projected onto the model's own partition axis, is the edge's
delta; absent a record the edge falls back to the run's written window, the coarser and
always-correct form (widen-never-narrow). The record is warehouse-resident, alongside the
reconciliation ledger, and written in the **same backend transaction as the write it records**
— a delta visible without its write, or a write without its delta, breaks propagation
soundness. **Trust boundary:** an observed delta is trusted because the state is smelt-owned,
written only by smelt's own conditional-write path — the general form of this trust argument
is `state.md` §"The residency rule"; there is no out-of-band-edit tripwire — an
external mutation to the target table between runs is not detected (an explicit Open
Question, §Known Divergences). Empty and absent are distinct: an empty recorded delta means
the run executed and changed nothing (a real, propagatable fact); an absent record means no
delta was recorded, and a consumer must not conflate the two. This composes with the derived
settle bound (`incremental_shapes.md` §"Key temporal locality (the time-partitioned
output)"): a stable upstream chain degenerates to empty-delta no-op propagation with a
provable horizon behind it.

**Refusals.** The graph refuses fail-loud (`MaintenanceGraphUnsupportedNode`) on: a cyclic
edge set; a **self-referential** model (a table-graph cycle that is a DAG only when
time-unrolled — admissible in principle iff its self-clamp is strictly time-backward, with
forward dirt running to the frontier and backward resolution reaching the model's
basis/checkpoint); and a **keyed node whose delta signature is `general`** (no partition axis
for interval dirt and no admitted key addressing either — treating it as day-axis would be
wrong-and-quiet). A keyed node proven `keyed upsert` is **not** refused on this ground — see
"Keyed dirt-sets and the narrowed refusal", above — and neither is a locality-admitted
time-partitioned keyed output: it is a clocked node whose edges use its declared granularity,
and whose outbound dirt is the key→partition projection of what its runs changed — exact under
locality routes 1–2, widened backward by `r` plus margins under route 3
(`incremental_shapes.md` §"Key temporal locality (the time-partitioned output)").

### Dispatch — from propagated components to run units

The run loop's currency is the **typed component vector** on each edge (§"The graph layer"
"Typed edges"), not day-intervals. A propagation result yields, per `(model, upstream)` edge,
a set of **run units**: one per component, each carrying the component's addressing (window /
keyed / whole-model) and its restriction (interval set, key set, or "everything").

**Dispatch is keyed by the component's addressing, never by the downstream model's grain.** A
`Keyed` component dispatches the derived key-addressed repair cell (`Technique::PerGroupRecompute`,
§"The graph layer" "Upstream model edges") whatever the downstream's `grain` is — a
`grain: partition` downstream of a clockless `keyed upsert` upstream is the named example: the
key-addressed edge dispatches its `PerGroupRecompute` cell exactly as it would into a
`grain: key` downstream. Routing a key-addressed component through the ordinary whole-model run
route is correct-but-not-incremental and is a defect against this paragraph, not an acceptable
fallback.

**Widen-never-narrow at dispatch.** A component whose cell cannot be derived degrades to the
coarsest run unit the consumer can act on and *says so* — an explain-visible downgrade,
never to nothing and never silently. Two distinct causes produce this degradation and both stay
visible: a state-availability shortfall (`state.md` §"The degradation contract",
`MaintenanceStateDowngraded`) and an unresolved or under-typed component (the unresolved-seed
case, above, or a delta signature that itself degraded to `general`,
§"Delta signatures").

A model receiving several components in one tick dispatches each; per-edge dirt keying
(§Design "Per-edge dirt keys trigger cells") is unchanged — components refine it, they do not
replace it.

**Restrictions compose by union.** A key-addressed run unit's read restriction is the **union**
of (a) the keyed component's propagated key values and (b) the values the cell's own affected-key
discovery resolves (§"Upstream model edges" — the group-grain fingerprint sidecar diff over the
upstream's own output table) — never an intersection. The sidecar refresh commits in the same
backend transaction as the cell's write, so narrowing the repaired set to only the values both
sides agree on would advance the comparandum past keys that were never actually consumed —
wrong-and-quiet. A propagated component whose values are unresolved (§"Unresolved seeds")
contributes no keys to the union and never narrows it; it widens at dispatch by the rule above.

### Shape profiles

A maintained model composes properties (`model_properties.md`), transforms
(`model_transforms.md`), world-facts (`sources.md`, `timeseries.md`), output shape (§Surface),
and scope maps (the per-input dispatch §"The plan matrix" names). The per-shape composition —
one profile chapter per stored shape, each opening with a composition table naming required
properties, consumed world-facts, default-plan transforms, and the upheld invariant
specialisation — is `incremental_shapes.md`; the engine-maintained profile is
`materialized_view.md`. A profile owns only what is meaningful inside its shape and never
re-specifies a capability a capability spec or a shared section here already owns (§Design
"Placement is definitional").

### Interactions

The invariant, signatures, ladder, horizon, and validator-not-chooser are owned above; the
plan's per-cell theorem is the `S`-vector refinement, and per-cell choice operates strictly
inside validator-not-chooser. Output shape/grain and the refresh trichotomy — and the
**declaration law and litmus rule** (`models.md` §Design) governing whether a fact is
declared, derived, or implied, and whether a combination earns a new peer shape — are owned by
`models.md`; the plan validates against and consumes them. **Input consumption** (`models.md`
§"Input-consumption axis") is a derived, cross-cutting axis that never changes the equivalence
contract, only what is scanned; default is windowed, full scan the surfaced fallback. Source
postures (`mutation_profile`, lateness, retention, delta identity, unique keys) are declared
in `sources.md` and consumed by admission. The technique primitives (`merge_into`,
DELETE+INSERT, column-scoped merge, targeted backfill) are catalogued in
`model_transforms.md`; the outer output clamp is the subquery wrap over the model's output
schema defined there. Definition-delta migration (`definition_deltas.md`) shares this spec's
frontier, emitters, and oracle form, and differs only in workflow (plan-and-approve).

## Design

Each paragraph records one load-bearing decision and what was rejected. Deeper derivations
live in `docs/research/` and are cited by full path. The partition-grain and key-grain design
paragraphs are `incremental_shapes.md` §Design; the definition-delta design paragraphs are
`definition_deltas.md` §Design.

**Delta signatures are the front door; the stored-shape grid is not.** The spec opens with
what a relation *emits*, not with a 2×2 of stored shapes, for three reasons. The grid was
never really four: one corner is uninhabitable (no clock, no identity) and one is derived-only
and refused at plan derivation (`key_per_partition`), so two orthogonal facts give three
working shapes and the grid promises a symmetry that isn't there. It types the wrong thing:
everything the machinery derives — typed deltas per column group, frontiers, contract points,
cells keyed by trigger and changed input — classifies *change*, while the grid classifies
stored output. And it is model-local: "what shape is your table" cannot pose the question the
substrate answers — what does this model emit, and what can its consumers do with that? The
shapes survive as implementation profiles (`incremental_shapes.md`) and `grain` as the
friendly label; nothing about the declared surface changes.
(`docs/research/20260811-delta-signatures-and-definition-deltas.md` §2.)

**One word, one grid.** The read-scope × write-scope grid inside the plan matrix is described
in quadrant vocabulary, and the stored-shape catalogue is described by grain name; no single
word names both. Letting one word ("corners") do double duty across two unrelated grids was
rejected as a standing source of confusion.
(`docs/research/20260811-delta-signatures-and-definition-deltas.md` §2.)

**Two delta kinds, one algebra, two workflows.** A definition change is modelled as a delta —
empty when eclipsed, region-complete otherwise — folded under the same frontier and the same
oracle form as a data delta, rather than as a separate migration subsystem; what differs is
workflow (data deltas fold automatically; definition deltas are planned and approved). The
full rationale and rejected alternatives are `definition_deltas.md` §Design.
(`docs/research/20260811-delta-signatures-and-definition-deltas.md` §3, §4.)

**Strategy content is derived; shape stays declared.** One model is not one mode — it is
simultaneously append-driven, merge-driven, and recompute-driven at different cells, so a
per-model strategy enum would be a lossy projection; strategy is derived per cell. Deriving
*shape* too was rejected: it reintroduces the silent contract swap the declaration law exists
to prevent. Shape-defining facts remain declared-and-checked.
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §10, §13.)

**One invariant; addressing is the real axis, and it is per-cell.** An earlier framing split
the contract into a per-partition equivalence and an end-state equivalence, one per shape —
miscast, since order/set-determinacy falls out of the single invariant for every shape and
per-partition equivalence is a *strengthening*, not a peer. Addressing is a property of *a
write*, not of *a model*: the declared facts fix which addressings are available, and each
cell derives its own — the composed shape's dimension-correction cell is keyed while its own
creation cell is region-addressed. A *declared model-wide* addressing token was rejected; the
per-cell plan already knows better.
(`docs/research/20260716-relation-contract-and-per-cell-addressing.md`.)

**The two write mechanisms stay binary per cell; locality is a refinement, not a third pole.**
Region-overwrite vs keyed-merge is the write-scope axis; which concrete pattern realises a
quadrant is drawn from the open registry, so the mechanism set grows without the quadrant
distinction changing. Key temporal locality adds a proof about *where* addressed rows can live
— licensing target pruning, a time-partitioned keyed output, and per-slice equivalence —
without changing how a keyed write is addressed. Promoting it to a third addressing pole was
rejected: it would suggest a different write primitive and identity requirement where there is
none. (`docs/research/20260705-keyed-time-superset.md`.)

**The axes compose.** The composed shape — both a clock and an identity declared — is
deliberately first-class, not an edge case (§Surface "The declared shape";
`incremental_shapes.md` §"Key temporal locality (the time-partitioned output)"): a DAG whose
clock dies at a keyed stage, a keyed node excluded from propagation categorically, or a
conditional-write cost sized to the whole key space are each a defect against those sections,
not an acceptable simplification.

**Scope maps name the per-input dispatch.** Without the name, the run shape reads as a
property of the *model*, hiding that different inputs changing engage different targeted
repairs. Naming the dispatch makes "what runs when this input changes" an explainable
per-input answer and gives future multi-clock driving-source work a stable home.
(`docs/research/20260705-keyed-time-superset.md` §5.)

**Sensitivity is factored and derived, both directions.** Two hazards bracket one derivation.
Over-attribution: a column that reads a second input's *immutable-at-creation* value must not
inherit that input's mutation-sensitivity, or the plan degenerates and the targeted cells are
lost — this is what makes the append-only declaration on a source load-bearing
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §5).
Under-attribution: a mutable source read only in row-admission position has empty *value*
sensitivity, and stopping there would leave its mutations entirely unmaintained — no cell, no
refusal, a quiet equivalence hole — so membership sensitivity is its own derived kind,
deciding admission from the join-predicate read actually performed and forcing repair into the
membership-capable recompute family. A degenerate whole-model collapse that happens to cover
the source is not an acceptable substitute for either: collapse-admitted cells assign repair
by accident and vanish the moment the collapse is fixed (established empirically — the
keyed-enriched shape's dimension cell was admitted solely by a collector misparse until
membership sensitivity was made a first-class derivation).

**Per-edge dirt keys trigger cells.** The trigger taxonomy is per-edge: a dirty set merged per
model would erase which repair runs where, and two sources landing in one tick genuinely
drive different techniques over different regions of the same table.
(`docs/research/20260705-refresh-as-maintenance-plan/10-dependency-propagation.md` §3.)

**Widen-never-narrow.** Every approximation in the plan and graph widens: partial-day clamps
ceil outward, coarse grains align outward, whole-partition dirt over-runs, an unclocked delta
dirties everything. Widening costs compute; narrowing costs correctness silently — the
declared guardrails (`scan_bounds`) make widenings *visible* costs, refused by default when
unbounded.

**Granularity is declared, not derived.** Deriving the propagation grain from a `date_trunc`
projection would let a refactor silently change downstream scheduling semantics; the
declaration is checked against the derived partition grid instead (`incremental_shapes.md`
§"Run window vs partition granularity").

**The clamp runs both directions.** Forward reflection and backward resolution are one edge
object run in opposite directions — the scan/footprint duality lifted to the graph. Keeping
them one object makes the test-build story (backward) automatically consistent with the
scheduling story (forward); the adjointness containment `forward(backward(P)) ⊇ P` is the
honest statement of their relationship.
(`docs/research/20260705-refresh-as-maintenance-plan/10-dependency-propagation.md` §2.)

**Offline cost measurement is first-class.** Because per-cell technique choice is
contract-preserving at fixed `S`, smelt may measure alternative physical plans over real data
offline and pin the cheapest (`smelt bakeoff`) — a capability per-query optimisers
structurally lack. The measurement is real: each candidate executes the project's actual
`execute_project` pipeline against the project's own data in a disposable scratch schema.
Pinning is deliberately a human act: `--pin` only emits YAML for review, and an applied pin
remains subject to admission like any override.
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §11.)

**Windowed by default; full scan is the surfaced fallback.** Treating full-table
recomputation as the baseline and windowing as a per-shape optimisation inverts the real
economics: a clocked model can always be maintained over a bounded scan window, and only the
absence of a clock forces a wider read. Windowing-by-default keeps the common case scalable
and pushes join optimisation to the engine over a safe widened scan, rather than smelt
hand-computing minimal deltas.

**The horizon is derived, not declared.** Trusting a declared horizon risks an under-estimate
that silently corrupts the clamp; deriving it keeps clamps correct by construction, and a
declaration is admitted only as a *ceiling* that warns. The consequence — a late arrival
beyond the derived reach is silently excluded rather than diagnosed — is accepted and
documented (§"Windowed maintenance and the horizon") as a model-author + data-quality
concern, softenable later, consistent with derive-else-declare (`models.md` §Design).

**Validator, never chooser.** Auto-selecting or silently downgrading the declared shape was
rejected: it reproduces dbt's `strategy:` footgun where the effective contract is invisible.
The declared shape is authoritative; the machinery only proves or refuses it.

**Placement is definitional, not consumer-counted.** A capability whose verdict is stateable
without naming a shape profile lives in a capability spec (`model_properties.md` /
`model_transforms.md`); a capability meaningful only inside a profile lives in that profile's
chapter (`incremental_shapes.md`, or `materialized_view.md`) — pushdown-depth is a SQL
property in `model_properties.md`, backfill chunking stays in partition-grain execution.
Every capability gets exactly one home, without a mechanical consumer-count rule; the
invariant, signatures, ladder, plan, and graph layer live here because every profile cites
them as its contract.

**Dispatch is typed by addressing, not by model grain.** Rejected: the per-grain run branch —
routing a key-addressed component only when the *downstream* model's own grain is `key` —
which is why the key-addressed route was unreachable from a `grain: partition` downstream even
though the component itself was already correctly typed `Keyed`. Addressing is a property of
the *component* (§"The graph layer" "Typed edges"), not of the model it lands on; dispatch
reads that property directly (§"Dispatch — from propagated components to run units").
(`docs/handoffs/2026-08-16-delta-signature-closure-programme.md`.)

**Key values seed propagation; propagation stays pure.** Rejected: resolving keys inside
propagation via backend I/O — a pure derivation function performing a database read breaks the
Salsa purity invariant (`architecture.md` §"Salsa purity rule (analysis)") and the graph
layer's own status as a pure composition. Rejected also: keeping dirt symbolic (key columns
only) and resolving values only at run time — the scheduler then cannot size, dedupe, or skip a
key-addressed run unit before dispatch, since it does not yet know which keys are affected. Key
values are resolved once, by the caller, from the group-grain fingerprint-sidecar diff
(§"The graph layer" "Upstream model edges"), and passed into propagation as a seed input exactly
as a landed interval already is.
(`docs/handoffs/2026-08-16-delta-signature-closure-programme.md`.)

**The watermark is a field on the landed-delta family, not a new one.** Rejected: a new
correctness-classified state family for the watermark — correctness structures must be
backend-resident and transactional with the write they describe (`state.md` §"The residency
rule"), and a watermark that *gated* forward propagation's inputs rather than merely recording
them would make forward propagation require state to run at all, contradicting the optionality
rule (`state.md` §"The optionality rule"): observability's absence must degrade, never block.
Filing it as a field on the existing landed-delta record keeps it observability-classified,
`state.mode: stateless`-optional, and subject to the same degrade-or-refuse contract every
other absent observability structure already has (`run_state.md` §"Per-source watermark") —
with the **refuse leg** chosen for absence: the coarser behaviour would be recomputing
everything downstream of the source, an unbounded cost the operator never asked for, so
absence withdraws only the convenience of omitting `--landed`, never substitutes a full
recompute. Granularity is **per source, not per `(source, consumer)`**: the watermark
advances only on a run that completed every consumer of the source, so a selective run stalls
it rather than silently dropping a span for the unselected consumers. Rejected for now: a
per-`(source, consumer)` watermark, which would un-stall selective runs at the cost of a
consumer-keyed record family and per-consumer read composition — deferred until
selective-run stalling is a demonstrated pain, not a hypothetical one.
(`docs/handoffs/2026-08-16-delta-signature-closure-programme.md`.)

**Rejected alternatives, briefly.** A `strategy:` sub-knob (the invisible-contract footgun); a
dedicated `smelt-maintenance` crate (the derivation needs the tightest coupling to the sibling
classifiers; the module boundary is kept extraction-mechanical instead); qualifying the output
clamp to a resolved inner alias (answers a question the output clamp must never ask); a third
addressing pole for locality (changes no write primitive); per-edge grain declarations (two
declarations can disagree — resolved by the derived label + check-only assertion); a closed
write-pattern enum baked into the surface (bakes today's engines in). Deeper rationale:
`docs/research/20260705-refresh-as-maintenance-plan/` parts 01–10, with the
decision-acceptance records in `09-spec-readiness.md` §1 and `10-dependency-propagation.md`
§11 of that directory.

## Constraints & Invariants

- The **equivalence invariant** holds for every non-`full` model and on every ladder rung; a
  transform that cannot preserve it for a given model is refused, never applied approximately.
  Order/set-determinacy is its corollary; per-partition equivalence is a strengthening, not a
  separate contract. For a definition delta the invariant's right-hand side uses the new
  definition (`definition_deltas.md`).
- **Delta signatures are derived, never declared.** A source's signature comes from its
  declared world-facts; a model's from the output-delta proof. Where no verdict is derivable,
  consumers assume `general`, whole-table-addressed — widen-never-narrow, never fabrication.
- **Write addressing is per-cell, not per-model**, derived by the available-addressings rule
  (`available = declared contract facts × trigger/changed-input needs × equivalence invariant × backend capability`)
  over the **open write-pattern registry**. A keyed write on a clocked output stays
  partition-scoped unless it provably cannot be; key temporal locality refines keyed
  addressing with a derived slice bound without changing the addressing quadrant.
- **The write-pattern set is an open registry, not a closed enum.** New patterns are admitted
  by declaring their required contract facts and discharging the equivalence proof obligation;
  the `write:` pin is an open, fail-loud name; a pattern the target backend cannot execute is
  not a candidate. The stable contract is the admission function + equivalence gate, never the
  enumeration.
- Maintenance is **windowed by default** where the model is clocked; a full scan is a surfaced
  fallback, never the silent baseline. Always `scan window ⊇ write window`.
- The **horizon is derived**; a declared `horizon_ceiling` is a warning threshold only and
  never relaxes the clamp. At the default point, late arrivals beyond the derived reach are
  silently excluded — a model-author + data-check concern — unless the model opts into the
  frozen-horizon contract-lattice point.
- **A contract-lattice point is admissible only as a complete triple, single-owned in
  `smelt-logical`**: a declaration schema, a pure oracle transform, and a probe emitter
  (§"The contract lattice"). The conformance gate consumes the oracle transform rather than
  encoding its own comparator; runtime probes emit from the same definition — a lattice point
  is never defined ad hoc by a caller.
- **One home per capability and per rule.** The invariant, signatures, ladder, plan, and
  graph layer are owned here; the shape profiles in `incremental_shapes.md`; definition-delta
  migration in `definition_deltas.md`; properties in `model_properties.md`; transforms in
  `model_transforms.md`; the declaration law and litmus rule in `models.md`. No spec
  re-specifies another's content.
- **Proofs are fail-closed**: an undecidable construct rejects; a declared escape hatch may
  only widen eligibility, never substitute for a proof's default reject.
- The declared **`refresh:` value plus the shape-defining facts are the only shape surface**;
  `grain` is a derived check-only assertion, write addressing is derived per cell (steerable
  only via the validated `write:` pin), input-consumption is derived from the source. No
  `strategy:` sub-knob — the machinery **validates, never chooses**.
- **The plan is pure data, derived by pure functions, in one place** (`smelt-logical`);
  consumers never re-derive it (also an invariant in `architecture.md`).
- **Maintenance statements have one author** (§"Statement emission (single owner)"); backends
  execute, never author. Printed, gate-verified, and executed SQL are the same emitters'
  output by construction. Definition-delta migration statements are inside this rule.
- **Never fold a delta already reflected in the state.** Every fold consults the ledger; every
  region recompute resets the entries it overwrote. No path may merge a window twice. The
  same rule governs definition-delta catch-up.
- **Write window = output window**, per cell: the DELETE/merge target and the output clamp
  range over the same output-axis column and window, by construction.
- **Only proofs prune.** A declared bound is admitted only checked; a guardrail
  (`scan_bounds`, `horizon_ceiling`) may refuse but never modifies a clamp; no unproven bound
  drops a scanned input.
- **Fail-loud, fail-closed.** Every admission failure, non-local scan, skeleton-position add,
  and unsupported graph node is a named diagnostic; the graph layer never silently under-runs
  — unrepresentable dirt widens to whole-model, never to nothing.
- **Widen-never-narrow** is the composition law of every interval operation (clamp ceiling,
  grain alignment, footprint reflection, backward widening).
- Out of scope, deliberately: content-aware delta pruning (an engine/CDF concern); file-level
  write-amplification minimisation (the engine's job — the plan guarantees the partition
  bound); cross-*project* propagation (project isolation, `architecture.md`).

## Limitations

Deliberate scope boundaries: things smelt does not do **by design** at this spec's current
cut. Unlike §Known Divergences (implementation lagging decided intent), nothing here is a gap
to be closed by a tracked plan — changing an entry requires its own spec diff. Each entry
states the boundary, the reason, and the sanctioned alternative.

### No smelt-maintained SCD2 — history-keeping is plain SQL

smelt has no declared or derived history-keeping shape: no frontmatter opts a keyed model
into retaining every version of a key. SCD2 is written as ordinary windowed SQL over a change
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

- **`refresh: full`** — rebuild from the change stream each run. Always correct; cost is a
  full rescan of the feed.
- **`refresh: materialized_view`** — the same SQL, engine-maintained, where the backend has
  native IVM (`materialized_view.md` §Design "No named pattern").

There is deliberately no `refresh: incremental` route: `LEAD` is inadmissible in every shape —
the key grain rejects window functions outright (`KeyedForbidsWindowFunctions`,
`incremental_shapes.md`), and under the partition grain a new event must rewrite a row in an
already-written earlier partition, outside every output clamp. Recognising the
LEAD-over-clock-within-key pattern as an admissible incrementally-maintained shape is sketched
in §Future Extensions.

### No SCD2 over mutable snapshots

smelt never manufactures a change stream by diffing successive scans of a mutable snapshot
source. A version history needs an event time per version boundary, and a snapshot diff can
stamp boundaries only with the scan time — the run clock — so the resulting history would
depend on when `smelt build` happened to run, breaking the replay-safety the equivalence
invariant demands (§"The equivalence invariant"). SCD2 therefore requires a source that
already carries change events with their event times (an update-events / CDC feed).
Maintaining the *current state* of keys from a snapshot (snapshot-reconcile, key grain) is
in-bounds; retaining the *history* of snapshot states is not — a snapshot-to-change-stream
facility, if ever wanted, is a source-layer concern (`sources.md`), not a model shape.

### Other deliberate boundaries

Boundaries stated normatively elsewhere, collected here for discoverability:

- **Late arrivals beyond the derived horizon are excluded**, silently; surfacing them is a
  model-author + data-check concern (§"Windowed maintenance and the horizon").
- **No continuous freshness.** smelt-owned maintenance is pull-based and per-run; the history
  is correct as of the last `smelt build`. Engine-continuous freshness is a different refresh
  mode (`materialized_view.md`).
- **Non-replayable observation contracts are refused.** Min-ever-observed,
  first-observed-value, and similar fold-the-observation-sequence columns have no executable
  full-refresh oracle and are rejected rather than approximated (§"The equivalence
  invariant"; a possible opt-in weaker contract is §Future Extensions).
- **No delete signal under window-forward consumption.** An append-only feed carries no
  delete events, so a key never departs and there is nothing to delete — retention over such
  a feed is simply correct, not a carve-out. Where the underlying source *is* mutable but
  only a window of it is scanned, departure is unobservable and the affected region is
  recomputed or the shape refused (§"The equivalence invariant", key departure).
- **No out-of-band-edit detection.** An external mutation to a target table between runs is
  not detected. A per-run digest tripwire was considered and rejected: it taxes every run to
  catch a rare, self-inflicted failure mode, and a full refresh is always the repair. This is
  a stated non-goal, not an open question.
- **Skeleton-position definition changes are never migrated in place**
  (`definition_deltas.md` §"Skeleton changes are a new relation").

## Known Divergences / Open Questions

Live gaps between this spec and the implementation, and questions where intent itself is
undecided, as of `last_reviewed`. Completed work is not recorded here — history lives in git
and §References → Plans. Shape-profile gaps are `incremental_shapes.md` §Known Divergences;
definition-delta gaps (including the unwired synthesis layer and the verb renames) are
`definition_deltas.md` §Known Divergences.

- **Posture-derived key departure is unimplemented; the runtime retains departed keys
  unconditionally.** A snapshot-reconcile run does not delete keys absent from the incoming
  scan (no anti-join delete leg exists), and the `retain_departed` retention point — the
  declared way to keep that behaviour — has no declaration parsing, oracle transform, probe
  emitter, or `ContractRetainDepartedInvalid` diagnostic. Today every keyed model behaves as
  if `retain_departed` were silently declared (decision record:
  `docs/research/20260816-open-questions-triage.md`).
- **The determinism scope is unimplemented.** The runtime still compile-time-pins
  `NOW()`/`CURRENT_*` in partition-grain models and rejects them in keyed models, instead of
  running them as-is; the conformance oracle's comparison and the recompute-equality
  technique gates do not yet consult the per-column determinism verdict, and `smelt explain`
  does not print the determinism exemption in the per-column guarantee ledger (decision
  record: `docs/research/20260816-open-questions-triage.md`).
- **The scheduler does not yet consume delta signatures end to end**, per the design now
  pinned in §"Dispatch — from propagated components to run units". Signatures shape admission
  and are printed, but the DAG scheduler's currency for "what needs re-running" is still whole
  day-intervals in most respects: the graph layer's keyed channel now carries resolved key
  *values*, not just key columns and provenance (§"Keyed dirt-sets and the narrowed refusal"),
  but only when a caller feeds them in as a seed — live resolution (reading the actually-changed
  key values off the backend, and consuming the recorded observed-delta table for
  `--since-upstream` rather than trusting the command line) is still the run-time mechanism's own
  job, not yet wired live into propagation; and cross-model runs require the operator to state
  what landed upstream on the command line, because no per-source watermark is yet persisted (the
  watermark's shape is pinned, `run_state.md` §"Per-source watermark", but no run yet writes or
  reads one). Key-addressed model edges now dispatch outside the `grain: key` branch, composed:
  a clockless `keyed upsert` upstream feeding a `grain: partition` downstream runs the repair
  family's `PerGroupRecompute` cell, not the ordinary route, and several key-addressed edges into
  one downstream compose — each dispatches in the same tick rather than only the first. The
  residue is an inbound input that is not itself key-addressed (a declared source, or a model
  edge that resolved no cell), which widens to the ordinary route with a reported
  `dispatch_widened` downgrade rather than risking a silently dropped component. Tracked:
  `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`;
  `docs/outcomes/20260809-output-delta-typing/outcome.md`;
  `docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 1.
- **`smelt explain` does not yet print the delta-signature headline** (§Surface "CLI" makes
  the signature the first line; today's output leads with grain), nor the per-column guarantee
  summary or derived run shape. Tracked:
  `docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 4.
- **Per-cell `deferral` is not yet scheduled** — it parses, validates, and prints as declared,
  but needs per-cell frontier addressing, which the frontier record tracks only per-region
  today (a state-shape change, not a lattice-point change). Tracked:
  `docs/outcomes/20260809-contract-lattice-v1/outcome.md`.
- **`diff_patch` over the region `DeleteInsert` default has no runtime lowering** — the
  resolver fails loud by name, but no caller today reaches it, so the pin is unenforced rather
  than refused for that case. Tracked: `docs/outcomes/20260809-repair-family/outcome.md`.
- **Frontmatter-time grain checking has one narrow gap**: a `grain: key` model deriving
  identity from its `GROUP BY` (no top-level `unique_key:`) is checked only at plan
  derivation, not frontmatter validation (cross-ref `models.md` §Known Divergences).
- **The write-pin equivalence factor is structural only** — the per-cell equivalence hook
  always accepts; threading column-comparability or a suppression-specific proof is later
  work. Tracked: `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **An inadmissible write-*variant* pin has no pre-execution gate** — forcing
  `technique: suppress` on a refusing cell silently falls back to full recompute instead of
  refusing; `smelt explain` also misses this case. Tracked:
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Observed-delta consumption is partial**: `--since-upstream` doesn't read the recorded
  delta table live; backward resolution consumes none; the keyed-fold and staged-candidate
  write families record nothing; the settle-bound × observed-delta composition has no live
  "delta empty" leg. Tracked:
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **No execution technique keys off a maintained-model creation cell** — the propagated
  region materializes via the ordinary run loop, not a per-cell technique. Tracked:
  `docs/plans/20260710-web-analytics-maintenance-demo.md`.
- **Plan-consumer gaps**: the horizon-clamped partition-local mutation quadrant is
  unreachable from any real workspace; dispatch cannot distinguish "a mutation genuinely
  happened" from re-derivation; the `prefer` soft-bias ladder and
  `scan_bounds.on_violation: warn` parse but are not consumed (every refusal is an Error);
  the cost model between two admissible techniques is unbuilt; `AppendOnly` sources get no
  `UpstreamMutation` cell. Refs: `docs/plans/20260707-maintenance-plan-impl.md`.
- **The frontier reset is not yet fused with every write path that recomputes a region.** The
  ordinary DuckDB `DeleteInsert` batch, on an already-existing target, fuses its frontier reset
  with its own write in one transaction. Three sibling write paths still write their frontier
  record after the model write completes, in a separate (non-fused) transaction: the first-run
  `CREATE TABLE AS` bootstrap materialization (no existing target to fuse against), the
  delta-restricted recompute, and the column-scoped-merge / in-place-update techniques. Tracked:
  `docs/outcomes/20260816-state-residency/outcome.md`.
- **Emission remainders**: the additive fold's MERGE-inside-ledger-transaction interior is
  not observable at the statement-group seam (its parity leg uses an idempotent fixture
  instead). Refs: `docs/plans/20260707-maintenance-plan-impl.md`.
- **Locality and diagnostic residues on the maintenance-plan proofs**: a keyed-grain output
  poses no partition-locality question, so a locality-admitted keyed model's clamps carry an
  assumed (underived) write-footprint mirror into propagation;
  `MaintenanceSkeletonColumnAdded` is reachable (unit coverage, and via `smelt-runtime`'s
  maintenance driver, the only caller with I/O access to derive a real `ColumnAdded` trigger)
  but not yet surfaced as an LSP/CLI diagnostic ahead of a run (`smelt-db`'s own
  diagnostics/`smelt explain` path always derives an empty trigger set);
  column-group-scoped dirt coarsens to whole-partition (safe, over-running); hour granularity
  is declared surface but propagation is day-ordinal; the built grain-alignment check
  validates only the declaration (widen-never-narrow, `MaintenanceGranularityMismatch`), and
  graph edges still take the declaration directly. Refs: `model_properties.md` §Known
  Divergences; `docs/plans/20260808-derived-maintenance-proofs.md`.
- **The ledger/frontier warehouse substrate is DuckDB-only** — an additive-graded cell (or a
  region recompute needing a frontier reset) on another backend downgrades to the recompute
  family with a recorded, explain-visible `MaintenanceStateDowngraded` (`state.md` §"The
  degradation contract") rather than realising the technique it would get on DuckDB. A
  Spark-dialect ledger/frontier builder is deliberately deferred until a real Spark-targeted
  incremental workload demands one — on a ledger-less backend the recorded downgrade is the
  intended behaviour, not a stopgap (decision record:
  `docs/research/20260816-open-questions-triage.md`).
- **Graph-layer gaps**: bare `grain: key` nodes with no admitted locality refuse
  (`MaintenanceGraphUnsupportedNode`); time-unrolled self-edges are designed but unbuilt; the
  `examples/web_analytics` workspace is not fully `--since-upstream`-compatible end to end (a
  self-referential model and a bare-keyed model with readers each refuse the whole-workspace
  graph); no `--select` scoping exists.
- **Delta detection for `--since-upstream` is explicit-only in v1** — the runner supplies
  landed deltas on the command line; the per-source watermark is pinned as surface
  (`run_state.md` §"Per-source watermark") but no run yet persists or reads one, so `--landed`
  is not yet optional in practice. Automatic **snapshot diffing** of an external source with
  no native delta feed remains genuinely future work (§Future Extensions).
- **Straddle attribution without locality is scoped out of the ledger's v1** (a per-key
  footprint chaining across history;
  `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §8).
- **The derived model-wide horizon is under construction**, as is the data-quality check for
  the model-author lateness-flag pattern. Tracked: `docs/plans/20260704-model-updates.md`.
- **Override-ladder reach (Open Question)**: the keyed-fold suppression consumer honours
  `Suppressed` unconditionally — the first-build-vs-steady-state rule doesn't reach it; no
  real fixture derives a column-scoped/keyed-fold cell under a first-build/backfill trigger,
  so that branch is proven only at resolver level; `smelt bakeoff` measures technique-family
  cost only, not the write-suppression dimension; whether a future cost model needs
  region-level change-ratio statistics from prior observed deltas is open.
- **docs-site coverage of the plan's CLI surface is partial** — a one-time close-out task:
  enumerate the undocumented residue once, then document or explicitly drop each item.
- **The merged-group region-recompute rule is unverified in the implementation** — a group
  whose sensitivity spans two or more mutation-sensitive inputs must take region recompute
  (§"The plan matrix"); whether today's derivation ever admits a column-scoped repair for
  such a group has not been audited, and no check or fixture pins the rule (decision record:
  `docs/research/20260816-open-questions-triage.md`).
- **`change_feed` sources do not yet get an `UpstreamMutation` cell** — every other
  mutation-sensitive posture receives one, and a change feed must too; today none is derived,
  and even where the posture is threaded through, only full-input re-derivation is admitted
  (live fold machinery for a change feed's delta shape is §Future Extensions, blocked on the
  retention point).
- **`INTERSECT`/`EXCEPT` are unclassified set operations**: they collapse to whole-model
  mutation-sensitivity, so every admitted cell is region recompute; a future distribution
  proof needs per-arm-cardinality reasoning. Cross-ref `model_properties.md` §Known
  Divergences.
- **Conditional-maintenance gaps**: `smelt explain --show-sql` renders the unconditional
  matched arm, never the suppressed form a live run executes; the region DELETE+INSERT
  family has no conditional variant; the whole-row (keyless) staged-candidate realisation
  does not exist; no `write:` pin selects between keyed MERGE and staged-candidate;
  delta-restriction admission doesn't yet consume an external `mutable_snapshot` source's
  fingerprint-sidecar delta; non-DuckDB targets keep the widened-scan recompute. Refs:
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

## Future Extensions

Ideas for widening the admission space that are **not decided**. Nothing here is surface;
none of it may be relied on or implemented against until it graduates into
§Surface/§Semantics via its own spec diff and plan.

- **Further contract-lattice points.** Candidates, in the priority order the delta framing
  suggests (`docs/research/20260811-delta-signatures-and-definition-deltas.md` §5):
  - **Reconciliation points** — equivalence promised at declared moments (say, end of day)
    rather than after every run, licensing cheap approximate folds in between; the
    diff-then-patch write is simultaneously the probe (it measures drift) and the remedy (it
    repairs it).
  - **Declared indifference** — equivalence modulo stated tie-breaks or tolerances, making
    the carve-out currently special-cased inside the invariant (ordering ties) an ordinary
    declared point; costs the conformance comparator a quotient by the
    declared relation.
  - **Per-column-group freshness** — not a separate point: blocked on the same per-cell
    frontier bookkeeping gap as per-cell deferral, and rides that work.
- **Live change-feed folds.** Consuming a change feed's insert/update/delete rows as an
  incremental fold — the delta shape applied directly instead of full-input re-derivation
  (the admitted route today, §Known Divergences). Delete events are retractions whose
  contract home is the posture-derived departure rule (§"The equivalence invariant", key
  departure), which must be implemented first; in delta terms this fills the **retraction**
  row of the delta-shape scale and is the prerequisite for maintaining aggregates under
  deletes.
- **Proofs as product.** A `smelt prove` report card, `must_hold:` assertions that fail
  compilation, and a proof-diff surface for CI — making the derivation visible and
  assertable, printed over the full lattice. Deliberately after the scheduler and
  definition-delta work, which change what the proofs say.
  (`docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 4.)
- **Smelt-maintained SCD2 via succession-pattern recognition.** The plain-SQL SCD2 shape
  (§Limitations) could gain a `refresh: incremental` route by *recognising* the pattern
  rather than declaring it: a walk-produced verdict that every window function in the
  projection is `LEAD(t)` (or an expression over it) partitioned by an entity key and ordered
  by the driving source's event-time column. The maintenance theorem: a new event touches
  only its own row and its immediate predecessor within the key — bounded footprint, late
  events included (a mid-history splice touches exactly the predecessor and reads its
  successor). The technique is a keyed `MERGE` plus a targeted predecessor patch, and the
  standard equivalence invariant applies directly (the SQL is its own oracle). The machinery
  generalises beyond SCD2 to any `LEAD`/`LAG`-over-clock-within-key model (next-event
  features, sessionisation gaps), which is what would justify building it. Open: the
  classifier grammar (expressions over `LEAD`, post-window delete filtering), the fail-loud
  diagnostics for near-misses, and the `model_properties.md` walk vocabulary for window
  functions. Full sketch: `docs/research/20260723-scd2-succession-pattern.md`.
- **Automatic, watermark-diffed `--since-upstream`.** Today the caller supplies each source's
  landed delta explicitly (§Surface "CLI"). A future extension persists a per-source "last
  propagated through" watermark in `smelt-state` and diffs it against the source's current
  `covered_intervals`, so a bare `--since-upstream` discovers its own delta. This still does
  not solve a never-modeled raw source's freshness (no `covered_intervals` exists for data
  smelt never landed) — live backend freshness querying stays out of scope. The explicit and
  automatic forms are not exclusive: the automatic form computes the same `--landed`
  intervals the explicit form takes directly, layering on top without changing the graph
  layer or CLI.
- **An observer / prefix-consistency contract for non-replayable combinations.** Per-column
  admission refuses folding state *observations* into fold-family columns because no
  executable full-refresh oracle exists (§"The equivalence invariant";
  `incremental_shapes.md` §"Admission matrix (column family × source shape)") — min-ever
  observed, first-observed-value, and similar are contracts over the *observation sequence*.
  A future opt-in could state that weaker equivalence explicitly (a property of the observed
  prefix, not a re-runnable refresh) and admit those cells under it, rather than smuggling
  them under the executable-oracle invariant. Open: the formal statement, the opt-in
  surface, and what a conformance oracle even is for a non-replayable history.

## References

- **Code**: `crates/smelt-logical/src/maintenance/{mod,derive,emit}.rs` (the per-cell
  derivation); `crates/smelt-logical/src/maintenance/propagate.rs` (the pure graph-layer
  composition math — `propagate`/`required_inputs`); `crates/smelt-runtime/src/propagation.rs`
  (the real per-workspace graph assembly, `smelt run --since-upstream` planning, and
  `smelt build --include-upstreams` planning — `build_forward_graph`, `plan_since_upstream`,
  `resolve_build_plan`, all consuming the same `Edge` list);
  `crates/smelt-logical/src/analysis/` (the classifiers admission consumes, including
  `output_delta.rs` — the delta-signature derivation);
  `crates/smelt-logical/src/contract/` (the lattice-point triples);
  `crates/smelt-runtime/src/{cumulative,maintenance_driver,dimension_horizon_merge,transformer,backfill}.rs`
  (today's technique executors and clamps); `crates/smelt-state/src/intervals.rs` (the
  degenerate ledger); `crates/smelt-backend/src/lib.rs` (technique primitives).
- **Tests**:
  - `crates/smelt-logical/tests/{maintenance_tracer,maintenance_tracer_evolution,maintenance_tracer_propagation}.rs` — pure derivation- and graph-composition-math regression floor (chains, fan-out, diamonds, granularity mapping)
  - `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs` — the `forward(backward(P)) ⊇ P` adjointness law
  - `crates/smelt-logical/tests/{output_delta_spec,typed_edge_spec,contract_lattice_spec}.rs` — delta-signature and lattice-point spec companions
  - `crates/smelt-runtime/tests/{tracer_maintenance,tracer_evolution,tracer_propagation,since_upstream_propagation}.rs` — DuckDB equivalence oracles and real-workspace propagation-graph assembly
  - `crates/smelt-cli/tests/since_upstream.rs` — CLI-wired forward propagation; sufficiency-vs-full-refresh equivalence
  - `crates/smelt-cli/tests/include_upstreams.rs` — CLI-wired backward resolution; resolved-slices-suffice-vs-full-refresh over a two-hop chain, plus an unclocked-ancestor case
  - `crates/smelt-maintenance-testkit` (dev-only, `publish = false`) — the in-process harness: real-run-pipeline driver, typed `ModelRecipe` generator (`recipe.rs`), schema-generic schedule generator (`schedule_gen.rs`), S-tracked equivalence oracle (`s_tracker.rs`, `oracle_modes.rs`), and multiset-equality oracle (`oracle.rs`); wired as a dev-dependency of `smelt-cli`
  - `cargo test -p smelt-cli --test maintenance_conformance` — the standing generative equivalence gate: a deterministic-seeded sample of typed `ModelRecipe` values (append-only partition-grain, fact+mutable-dimension, `grain: key`, generated 2–3 node DAGs) driven through `execute_project` against real DuckDB, asserted equal to a full-refresh oracle after every run step under adversarial append/lateness/mutation/redelivery/definition-change schedules (`SMELT_CONFORMANCE_CASES` scales sample depth)
  - the same gate's composed (`grain: key` + `timeseries:`) recipe family exercises all three key-temporal-locality routes, gated by its own admission-rate floor (`SMELT_CONFORMANCE_COMPOSED_CASES`); the key-determined route's slice-pruned target scan runs with the slice predicate omitted — DuckDB's `MERGE` binder refuses the real predicate shape (the `BindMerge` divergence, also worked around by `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`)
  - the same gate's generated model-edge enrichment recipe family (one closure-admissible `LEFT JOIN` shape plus two closure-failing siblings) drives the delta-restricted-vs-widened-scan choice both ways over a fixed processed-input set `S` — gated by its own derived skeleton-source-closure verdict, end states asserted bit-identical — and a fully-suppressed conditional write's cascade (zero rows written, a present-and-empty recorded delta, zero downstream regions scheduled), each recipe family gated by its own admission-rate floor
  - the gate's `pinned` module reproduces every construct × posture cell and named hazard schedule as deterministic, always-reproducible cases; its `registry` module tracks named divergences with a staleness report
  - `crates/smelt-cli/tests/property_discovery/` — disposable per-cell probes for constructs the typed recipe generator has no vocabulary for yet (self-referential models, `UNION ALL`, `LEFT JOIN`, correlated `EXISTS`, stacked window frames, cross-source column-name collision, a mutable source aggregated directly); gated by `.claude/scripts/property-experimental-gate.sh`
  - `crates/smelt-cli/tests/incremental/` — drives a backend's incremental strategy directly given a hand-supplied filter, independent of how that filter is derived
  - `cargo test -p smelt-logical --test maintenance_plan_conformance :: coverage_matrix_is_inhabited` — the standing inventory gate over the research example catalogue's coverage matrix (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md` §"Coverage matrix", plus one added `INTERSECT`/`EXCEPT` row); every inhabited `(construct × source-property)` cell of the ~100 inhabited cells is `CLAIMED` (9 catalogue ids today — EX-02, EX-08, EX-12, EX-14, EX-18, EX-24, EX-26, EX-27, EX-35, plus EX-41/EX-42) or named individually in `KNOWN_GAPS`; both lists are per-cell, never per-row, additive-only
  - adjacent standing gates this spec's machinery is graded against: `cargo test -p smelt-runtime --test statement_parity` (executed-vs-emitted maintenance-statement parity, §"Statement emission (single owner)"); `cargo test -p smelt-runtime --test execute_parity` (CLI↔UI compile+execute pipeline parity, `architecture.md` §"Run pipeline parity rule (CLI ↔ UI)"); `cargo test -p smelt-logical --test walk_coverage` (the property composition walk this spec's admission consumes, `architecture.md` §"Property composition walk rule")
- **User docs**: `docs-site/docs/index.md`,
  `docs-site/docs/guide/{incremental-models,sql-models,materializations}.md`,
  `docs-site/docs/concepts/how-it-works.md`,
  `docs-site/docs/reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md` describe the
  trichotomy + grain surface; `docs-site/docs/reference/cli.md` also documents
  `--since-upstream`, `--include-upstreams`, and `smelt explain`'s cell/clamp/ledger report
  with `--show-sql`; `docs-site/docs/reference/smelt-yml.md` documents the `maintenance:`
  block.
- **Plans (history)**: `docs/plans/20260322-incremental-model-support.md`;
  `docs/plans/20260325-materialization-types.md`; `docs/plans/20260523-cumulative-aggregate.md`;
  `docs/plans/20260704-model-updates.md`; `docs/plans/20260704-model-updates-fundamentals.md`
  (the L1+L2 substrate); `docs/plans/20260705-keyed-collapse.md`;
  `docs/plans/20260705-property-discovery-loop.md` (the empirical engine);
  `docs/plans/20260707-maintenance-plan-impl.md` (the target frontmatter surface and
  diagnostics); `docs/plans/20260809-keyed-frontier.md`.
- **Research**: `docs/research/20260521-incremental-as-planner-rule.md`;
  `docs/research/20260703-model-updates.md`;
  `docs/research/20260704-maintenance-fundamentals.md`;
  `docs/research/20260705-refresh-as-maintenance-plan/` (parts 01–10);
  `docs/research/20260705-keyed-time-superset.md`;
  `docs/research/20260705-unified-keyed-refresh.md`;
  `docs/research/20260705-keyed-collapse-application.md`;
  `docs/research/20260704-monotone-join-maintenance.md`;
  `docs/research/20260705-model-refresh-review.md`;
  `docs/research/20260715-conditional-maintenance-without-cdf.md` (change-suppressed writes,
  delta-restricted compute, derived change feeds — the source of the pruning taxonomy's no-op
  write-elimination category and the composed-shape composition points);
  `docs/research/20260716-relation-contract-and-per-cell-addressing.md` (the shared Relation
  Contract, grain-as-derived-label, per-cell write addressing, and the open write-pattern
  registry §"Per-cell write addressing" and §"The declared shape" encode);
  `docs/research/20260809-incremental-rethink.md`;
  `docs/research/20260811-delta-signatures-and-definition-deltas.md` (the delta-signature
  front door and the definition-delta unification this spec encodes).
- **Related specs**: `incremental_shapes.md` (the shape profiles); `definition_deltas.md`
  (definition-delta migration); `model_properties.md` (the derived proofs — monotonicity
  trace, bound/reach, partition alignment, determinism, discriminants, anchor resolution,
  once-write and join-contribution proofs, output-delta shape, affected-key discovery);
  `model_transforms.md` (the physical mechanisms — pushdown, DELETE+INSERT, the clamps,
  pinning, `merge_into`, the windowed-keyed-maintenance driver, dimension-horizon MERGE);
  `models.md` (the refresh axis, the declared shape facts + derived grain label, the Relation
  Contract, three-state declaration law, input-consumption axis, litmus rule);
  `timeseries.md` (declares `event_time_column`, `partition_column`, `granularity`);
  `sources.md` (host of `timeseries:` and source-lateness/mutation-profile/key-recurrence
  world-facts, and the fingerprint sidecar); `expansion.md` (function expansion; runs before
  every analysis stage here); `functions.md` (the pattern-function surface);
  `materialized_view.md` (the engine-owned shape profile — where beyond-the-ladder shapes and
  hand-written SCD2 go); `multi_backend.md` (backend capability flags a strategy checks);
  `state.md` (state-ownership doctrine: the frontier realisations' correctness
  classification, residency rule, and the availability-downgrade contract);
  `schema_evolution.md`, `run_state.md`, `virtual_environments.md`, `diagnostics.md`,
  `architecture.md`, `cli.md`.
