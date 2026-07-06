# 10 — Dependency scope: what must run, and what must exist

- **Date**: 2026-07-07
- **Status**: research (part 10 of [`README.md`](README.md); day-granular tracer exists — see §10)
- **Author**: Andrew (with Claude)
- **Depends on**: [`01-framework.md`](01-framework.md) §5 (scan/footprint reflection,
  partition-local maintenance), [`07-example-catalogue.md`](07-example-catalogue.md) (EX-34
  cross-model settledness), [`08-code-placement.md`](08-code-placement.md) (crate placement)
- **Code**: `crates/smelt-logical/src/maintenance/propagate.rs` (pure v0),
  `crates/smelt-logical/tests/maintenance_tracer_propagation.rs`,
  `crates/smelt-cli/tests/property_discovery/tracer_propagation.rs`

The per-model maintenance plan (parts 1–9) answers *how* a model's partitions are
maintained. This part answers the two graph-level questions that sit above it:

1. **Forward — what must run.** Something landed upstream (a day of conversions, an hour
   of bronze arrivals). Which partitions of which downstream models are now stale, and
   which trigger cell repairs each?
2. **Backward — what must exist.** I want model `M` correct over period `[s, e)` — for a
   test build, a validation run, a dev sandbox, a backfill estimate. Which partitions of
   every ancestor must exist (or be built first), and in what order?

Both are compositions of the **same per-edge object** — the derived scan clamp — run in
opposite directions. Neither invents new information: everything below is the
`(partition_col, before, after)` reach triple of `01-framework.md` §5, lifted from one
model to the graph.

---

## 1. Why runs start from deltas, not from the clock

Today's mental model (dbt's, and most orchestrators') is *clock-driven*: a cron tick fires
the whole DAG for "today". But "today" is not what determines correctness — **what landed**
is. The cron tick is merely the poller that notices new data. Under the maintenance-plan
framing, the natural unit of scheduling is:

> per source, the set of partition intervals whose content changed since the last
> reconciliation (the delta on that source's own partition axis), pushed through the graph
> to the exact downstream partitions whose stored state no longer equals
> `full_refresh over processed S`.

This has three consequences the clock-driven model gets wrong:

- **Late data schedules the *right* past partitions**, not "yesterday + a guessed lookback
  re-run". A conversion landing on day `D` dirties event partitions `[D − 14d, D]` — the
  footprint, derived — regardless of what day it is.
- **Different edges drive different repairs.** A bronze arrival day triggers the
  recompute-region cell over its footprint; a conversions day triggers the column-scoped
  MERGE cell over a *different* footprint of the *same* table. The dirty set must be keyed
  per inbound edge, because the trigger taxonomy (part 1, §5) is per-edge.
- **No-op ticks are free.** A source that landed nothing propagates nothing; a source no
  model reads propagates nothing. Work is proportional to change.

The forward question is therefore not an orchestration nicety — it is the ledger's `S`
advance (part 1, §8) made schedulable: a dirty interval on `(model, edge)` is exactly the
set of `(region, group)` ledger entries whose processed-input vector fell behind.

---

## 2. The edge model

A dependency edge is `downstream reads upstream` under the **derived scan clamp**:

```
Edge { upstream, downstream, before, after }
```

meaning: maintaining downstream region `[s, e)` must read upstream partitions
`[s − before, e + after)`. The clamp comes off the maintenance-plan cell
(`PlanCell::scans`, derived by `source_bounds` from the explicit partition predicate in
the SQL — never hand-typed, and absent-by-refusal when the SQL carries no derivable link,
part "evolution tracer" v1/v4 findings). Worked values from the tracer's model family:

| edge | clamp `(before, after)` | why |
|---|---|---|
| bronze(arrival) → silver(event) | `(0, 2d)` | 48h late-arrival window: event day `d` reads arrivals `[d, d+3)` |
| sessions(start) → silver(event) | `(2d, 0)` | 48h max session length: event day `d` reads sessions `[d−2, d]` |
| conversions → silver(event) | `(0, 14d)` | first score within 14 days |
| silver(event) → rollup(event) | `(0, 0)` | same-axis aggregation |
| rollup → report | `(7d, 0)` | 7-day trailing window |

**Day-granular v0.** Every partition axis is whole days; clamp seconds are ceiled
*outward* (`36h → 2 days`) because a partial-day margin touches whole partitions —
widening is safe, narrowing never is. §7 places grain mapping in the same seam.

**Two directions, one object.**

- **Forward (footprint)** — the reflection: an upstream delta of days `[a, b)` dirties
  downstream `[a − after, b + before)`.
- **Backward (scan)** — the clamp directly: a downstream requirement of `[s, e)` requires
  upstream `[s − before, e + after)`.

The directions are adjoint, not inverse: `forward(backward([s,e))) ⊇ [s,e)` (building a
period's inputs and replaying them forward dirties at least that period, generally more).
The tracer asserts the containment rather than pretending equality.

---

## 3. Forward propagation — what must run

**Definition.** Given per-source deltas (intervals on each source's own partition axis)
and the edge set, process nodes in topological order; at each node, reflect its merged
dirt through each outgoing edge, accumulating downstream:

- `per_edge[(model, upstream)]` — merged dirty intervals attributable to that inbound
  edge. **This keys the trigger cell**: the plan cell for `Trigger::…{source: upstream}`
  runs over exactly these regions (recompute-region for a driving-source delta,
  column-scoped MERGE for an enrichment delta, per part 1's 2×2).
- `dirty[model]` — the union across inbound edges. **This is what the model's own
  consumers see as *their* upstream delta** — dirt composes transitively whether the
  upstream is a raw source or another model.

Topological order guarantees a model's dirt is complete before its consumers read it; a
cyclic edge set is refused (§6 for the self-referential case).

**Semantics established by the tracer (DuckDB, EXCEPT-ALL oracle):**

- **Sufficiency** — running exactly the per-edge dirty regions with their cells leaves
  every model multiset-equal to a full refresh. A pre-window partition is never scheduled
  and stays correct, which is the point: sufficiency *without* running everything.
- **Union vs separation** — two sources landing in one tick produce separate per-edge sets
  (different cells) and a merged per-model set (what flows downstream).
- **Shape-correctness** — chains compose hop by hop; fan-out reaches every consumer;
  a diamond merges at the join without double-counting.

**Minimality caveats (deliberate v0 over-approximations):**

- **Whole-partition dirt.** A dirty partition is dirty for every column group. A
  conversions delta in truth stales only `{conversion_score}` — downstream models that do
  not read that column need not run. Column-group-scoped dirt requires cross-model column
  provenance (part 9 §2's payload-propagation machinery) and is future work (§8).
- **Interval-shaped dirt.** Dirt is tracked as intervals on the partition axis, so a
  keyed-grain model in the chain coarsens to "the whole keyed table is dirty" (no
  partition axis to be finer against). Fine for v0; keyed dirt-sets (per-key) are a
  later refinement with real cost trade-offs.
- **Content-blind deltas.** "A day landed" dirties its footprint even if every landed row
  is a no-op duplicate. Content-aware pruning is an engine/CDF question, not a plan one.

---

## 4. Backward resolution — what must exist

**Definition.** Given a target model and a period `[s, e)` on its partition axis, process
the target's ancestor sub-DAG in *reverse* topological order (consumers before producers);
at each node, push its merged requirement up each inbound edge using the scan clamp
directly. The result:

- `required[node]` — for every ancestor (models *and* raw sources), the partition
  intervals that must exist for the target period to be computable. Includes the target
  itself.
- `build_order` — the ancestor **models** in dependency order ending at the target: the
  order a bounded build materializes them.

**The non-obvious use case this exists for: test and validation builds.** To validate a
model over June you do not want June's cron history replayed — you want *"build `rollup`
for June, including upstreams"*: a bounded vertical slice of the DAG. The date arithmetic
runs **backwards** relative to the day-to-day forward runs:

```
rollup [Jun 1, Jul 1)
  ← silver   [Jun 1, Jul 1)           (0, 0)   same-axis
  ← bronze   arrivals [Jun 1, Jul 3)  (0, 2d)  late-arrival window
  ← conversions       [Jun 1, Jul 15) (0, 14d) attribution window
  ← sessions starts   [May 30, Jul 1) (2d, 0)  max session length
```

Note both directions of widening appear: forward-widening on the sources whose relevance
*follows* the event (arrivals, conversions), backward-widening on the ones that *precede*
it (sessions). Getting either wrong produces a silently-wrong validation build — a NULL
`conversion_score` for an event whose conversion landed after the naive slice's edge is
indistinguishable from a real non-conversion. The tracer's sandbox test stages **exactly**
the computed slices into an empty database, builds bottom-up in `build_order`, and proves
the target period multiset-equal to the same period computed over the complete universe —
with a sentinel row whose correctness depends on the widened slice, so an under-widened
resolution fails the oracle rather than passing vacuously.

**Other consumers of the same computation:**

- **Backfill cost estimation** — before a cascade backfill (part 4 K5, EX-21's
  `backfill: cascade|local`), `required` over the backfill window prices the read side and
  forward propagation prices the write side; together they are the "what will this
  actually touch" preview the CLI should print before running.
- **Dev sandboxes / CI fixtures** — materialize the minimal upstream slice needed to
  exercise a model change over a representative period.
- **Ledger interpretation** — `required` at a region is precisely which slice of each
  input the region's processed-`S` vector ranges over; the backward map is the ledger's
  address book.

**Sources vs models.** A node with no inbound edges is a raw source: its `required`
intervals are a *data prerequisite* (the slice to stage/verify), not a build step. A node
with inbound edges is a model: it appears in `build_order`. The same node classification
falls out of the edge set; nothing is declared.

---

## 5. Scenario catalogue

The scenarios the graph layer must handle; status: ✅ = tracer-tested (unit and/or
DuckDB), 🔜 = designed here, not yet built.

| # | scenario | direction | status |
|---|---|---|---|
| S1 | single source lands a day → footprint partitions of one model run | forward | ✅ |
| S2 | two sources land in one tick → per-edge dirt keys different cells; per-model union flows on | forward | ✅ |
| S3 | chain / fan-out / diamond composition (incl. same-axis identity hop, trailing-window extension) | forward | ✅ |
| S4 | delta on a source no model reads, or empty delta → propagates nothing | forward | ✅ |
| S5 | cascade: an upstream **model's** region recompute treated as a delta → downstream dirty sets (the `backfill: cascade` write-side) | forward | ✅ |
| S6 | build a model for a specified period **including upstreams** — required slices per ancestor + build order (test/validation builds) | backward | ✅ |
| S7 | backward slice *sufficiency*: sandbox staged with exactly the computed slices reproduces the target period bit-for-bit (multiset), with a sentinel that fails under-widening | backward | ✅ |
| S8 | adjointness: `forward(backward(P)) ⊇ P` — replaying the resolved inputs dirties at least the requested period | both | ✅ |
| S9 | self-referential model (reads own output at `t−1`): table-graph cycle, time-DAG — refuse in v0, unroll later | both | 🔜 (§6) |
| S10 | granularity mapping: daily model feeding a monthly rollup — day/month built (grain *derivation* from the SQL, and an hourly axis, remain) | both | ✅ (§7) |
| S11 | column-group-scoped dirt: a conversions delta skips consumers that never read `conversion_score` | forward | 🔜 (§8) |
| S12 | keyed-grain hop: dirt entering a keyed end-state model (no partition axis) and re-emerging | forward | 🔜 (§3 caveat) |

---

## 6. Self-referential models and cycles

`propagate`/`required_inputs` refuse a cyclic edge set — over tables. But a
self-referential model (a rolling balance reading its own yesterday, part 1 §7,
`window_independence`'s `Ordered` verdict, loop cell G-08) is a cycle in the *table* graph
and a DAG in the *time-unrolled* graph: partition `d` reads partition `d − 1`, strictly
backward. The resolution sketch:

- A self-edge is admissible iff its clamp is **strictly time-backward** (`before ≥ 1 day`,
  `after = 0` on its own axis) — exactly the condition under which the day-unrolled graph
  is acyclic and the existing sequential-batch execution is sound.
- Forward: a delta at day `d` on a self-referential model's input dirties `[d, ∞)` in the
  worst case — the unbounded forward footprint of part 1 §7. The honest propagation
  answer is "dirty to the frontier", i.e. the interval `[d − after, frontier)`, which is
  the cascade-vs-`as-of-run` trade (G-08) surfacing at the graph layer: `backfill: local`
  truncates the dirty set and *names* the staleness; `cascade` runs it.
- Backward: `required` on a self-referential model reaches back to the model's **basis**
  (its first partition or last settled checkpoint) — which is why a bounded test build of
  a rolling model needs either a checkpointed opening balance or a widened period. This
  should be an explicit, surfaced choice, not an infinite loop.

v0 refuses rather than approximates; the unrolling lands with the trajectory work.

---

## 7. Granularity mapping (daily → monthly, hourly → daily)

Every interval above lives on some table's partition axis in *days*. A coarser or finer
downstream axis is an **outward alignment** applied per edge, in exactly the seam where
the day-ceiling already sits:

- **fine → coarse** (daily silver → monthly report): reflect on the day axis, then align
  outward to month boundaries — a dirty day dirties its whole containing month
  (`[2026-01-17] → [2026-01-01, 2026-02-01)`). Backward: a requested month requires its
  days, then each edge's clamp applies on days.
- **coarse → fine** (monthly dim → daily model): a changed month dirties every contained
  day; backward, a requested day requires its containing month.

Formally each axis gets a `grain` and each edge composes `align_outward(grain_downstream)
∘ reflect ∘ align_outward(grain_upstream)` — monotone outward maps, so sufficiency
composes.

**Status: built for Day/Month** (`PartitionGrain` on each edge endpoint; intervals stay
day-ordinal, anchored to the civil calendar so month boundaries are real; seeds align
outward to their node's grain, so a mid-month delta on a monthly dim is a whole-month
delta and a partial-month *request* is a whole-month request). Tracer-proven both
directions, including a trailing window crossing a month boundary (dirties both months)
and the DuckDB leg where a dirty January day rebuilds exactly the January report
partition while February is never scheduled. Remaining: **grain derivation** — the edge's
grains are declared by the caller today; deriving Month from a `date_trunc('month', …)`
grouping projection is classifier work — and an **hourly axis** (sub-day ordinals, same
algorithm).

---

## 8. Column-group-scoped dirt (the payload-propagation join point)

Whole-partition dirt (S11) is the v0 coarsening with the largest real-world cost: a
conversions day currently dirties *every* downstream of silver, including models that
project only `{event_id, user_id}`. The refinement keys dirt by
`(partition-interval × column-group)` and prunes edges by **column provenance**: an edge
propagates a group's dirt only if the downstream actually reads a column of that group.
This is the same cross-model provenance machinery part 9 §2 already lists for payload
taint (skeleton-position fail-loud) — one derivation, two consumers — and it is why both
belong in `smelt-db`'s workspace layer (project-scoped) rather than per-model analysis.

Until then the coarsening is *safe* (over-runs, never under-runs), consistent with every
other v0 widening choice.

---

## 9. Placement and surface

Consistent with [`08-code-placement.md`](08-code-placement.md):

- **Pure math** — `smelt-logical/src/maintenance/propagate.rs` (exists): intervals,
  edges, forward `propagate`, backward `required_inputs`. No I/O, no graph discovery.
- **Graph assembly** — a `smelt-db` workspace query builds the edge set from the model
  graph × each model's derived `MaintenancePlan` (every `PlanCell::scans` entry is an
  edge). Project-scoped per the project-isolation rule.
- **Consumption** — `smelt-runtime` scheduling: per-source delta detection (landing
  manifests / CDF / interval-store diff) feeds `propagate`; the runner executes per-edge
  regions with their trigger cells behind `execute_project` (run-pipeline parity). The
  ledger records the advance.
- **CLI surface (sketch, not committed)** — forward: `smelt run --since-upstream`
  (compute deltas from the ledger, run the propagated set; the default posture once
  trusted). Backward: `smelt build <model> --period <start>..<end> --include-upstreams`
  (resolve, print the per-ancestor slices + build order, optionally execute) — the
  test/validation build. Both should print their plan before acting; the dirty/required
  sets are exactly the explainable artifact.

---

## 10. Tracer status and open questions

**Proven:** S1–S8 — forward sufficiency and per-edge cell routing on DuckDB; backward
slice resolution with both widening directions, sandbox-build equivalence with an
under-widening sentinel; cascade-as-delta; adjointness containment; cycle refusal (incl.
self-edge) — and S10 day/month grain mapping (both directions, boundary-crossing windows,
month-partition rebuild on DuckDB).

**Open, in order of expected next need:**

1. Grain derivation + hourly axis (§7 status note) — the grains are edge-declared today;
   deriving Month from a `date_trunc` grouping is classifier work.
2. Self-referential unrolling (§6) — blocks rolling-balance models; interacts with G-08's
   cascade knob.
3. Column-group-scoped dirt (§8) — cost, not correctness; lands with cross-model
   provenance.
4. Keyed-grain hops (S12) — dirty *key-sets* as a second dirt shape beside intervals.
5. Delta detection itself — what "landed" means per source posture (append-only landing
   manifests vs CDF offsets vs snapshot diffing); part 5's `mutation_profile` decides the
   mechanism, the interval-store (`smelt-state`) records it.
