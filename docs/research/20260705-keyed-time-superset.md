# Key- and time-addressed output — keyed as a superset of batched

**Date:** 2026-07-05
**Status:** research / brainstorm — proposes spec changes; nothing here is normative.
**Predecessors:** `20260703-model-updates.md` (Parts 19–20), `20260704-maintenance-fundamentals.md`,
`20260705-unified-keyed-refresh.md`, `20260705-model-refresh-review.md`,
`20260705-keyed-collapse-application.md`; specs `keyed_models.md`, `batched_models.md`,
`model_maintenance.md`, `timeseries.md`, `sources.md`.

---

## 0. The question, and the north star

The recurring ask (it has now surfaced in at least four prior documents, each time landing in a
different fragment):

> A keyed model should be able to carry a `timeseries:` block and merge time-partition by
> time-partition — not always, but as an option. A high-volume event table plausibly wants to
> dedupe on a short window (say 3 days) and process day by day, with enrichments merged in daily
> or hourly batches.

And the north star it should be judged against, which is the project's essential task:

> The user writes SQL models. From what we can **derive** about the model, **declare** about the
> sources, and **know** about the backend, smelt runs and maintains each model as efficiently as
> the backend permits — never trading correctness silently.

This note answers three questions:

1. Is there a principled reason `refresh: keyed` forbids a `timeseries:` block? (§1)
2. Given a unique key, how is keyed a superset of batched? (§3)
3. How does "different upstreams changing ⇒ different targeted queries" — the idea developed
   around accumulating_snapshot — come back as a first-class concept? (§5)

### 0.1 The trail of near-misses

The ask is not new. Each prior document brushed against it and resolved a *fragment*:

| Where | What was decided | What was left |
|---|---|---|
| `20260703` §19.5 (ETL grid) | dedup at ingest = "bounded-time dupes → `batched` + lookback; global dedup key → secretly `latest_value` on the event id" | the *bounded-time dupes with supersede semantics* case fits neither (§4.1) |
| `20260703` §19.6 (hybrid cells) | "running totals **with history** (one row per key per day)" — deferred | this is exactly a (key, time)-addressed output |
| `20260703` Part 20 (acc. snapshot) | windowed keyed merge with horizon `H`, settled-key eviction, `[start − H]` clamp | the whole key+time write design |
| `20260705-unified-keyed-refresh` §1.2 | horizon clamp demoted to a derived work-bound; `smelt.dedup_latest` proposed for the cross-partition dedup pattern | `smelt.dedup_latest` deferred; no output shape to hold it |
| `20260705-model-refresh-review` §6.5 | "at-least-once delivery lands duplicate events in *different* partitions; partition-local DELETE+INSERT can't dedup across them" — routed to keyed-by-event-id | open question verbatim: *"is per-window `merge_into` acceptable at event-grain cardinalities?"* |
| `20260705-model-refresh-review` §3.3/§6.4 | `timeseries:` forbid on `materialized_view`/`versioned` output blocks downstream partition pushdown | same clock-sink seam, other modes |
| Keyed collapse D3/D16 | `refresh: keyed` **forbids** `timeseries:`; consumer-facing `timeseries:` on keyed-family outputs deferred | the option the ask keeps requesting |

Seven sightings, one shape. Every fragment above is the same missing cell: **output addressed by
both a unique key and a time column, with writes that are key-addressed but provably
time-local.** The collapse (correctly) unified three modes whose outputs are keyed-only; in doing
so it inherited "no time axis on output" as if it were part of the contract rather than a
property of those three modes.

---

## 1. Audit: is the exclusion principled?

Four reasons are on the record. Audited in turn:

**R1 — Inherited output shape.** `keyed_models.md` §Surface: "the output has no partition
column." True *of the collapsed trio* (a customer's lifetime `SUM` lives at one keyed row,
updated forever). This is a **description that hardened into a prescription**
(`KeyedForbidsTimeseries`). Nothing in the equivalence invariant requires a keyed output to be
time-free; the invariant (`model_maintenance.md` §"The equivalence invariant") is
addressing-agnostic: `incremental_state(S) == full_refresh(source | input ∈ S)`.

**R2 — D6, the no-write-eligibility-clamp principle.** "A run merges every delta row it scans …
no scanned input is silently dropped." This is the strongest reason and it is *correct* — the
review §3.2 showed the declared-`H` clamp violates the invariant exactly this way. But D6 bans
**silent, under-declared truncation of writes**. It does not ban **proven no-op elimination of
reads**: skipping target rows that *provably cannot match* any scanned delta row is not a clamp,
it is pruning. D6's own carve-out concedes this: the dimension-driven horizon-bounded MERGE (F15)
survives *"for derived `H` only — a scan-side bound that cannot under-cover because it is
derived."* F15 **is already a key-addressed, time-bounded write**. The principle to preserve is
therefore narrower than the rule that encodes it: *no write may be refused by an unproven bound;
a violated declared bound must fail loud, never drop.*

**R3 — The binary addressing axis.** `model_maintenance.md` §Design: partition-addressed
(identity-free rewrite) **vs** key-addressed (identity-requiring merge), with SCD2 close-out
cited as proof that key writes escape time windows. SCD2 proves *some* key-addressed writes
escape *any* window. It does not prove all do. With a derived bound, many provably don't — and
the framework already computes exactly the needed facts (forward/backward reach, F1; functional
dependency key → column; join-contribution monotonicity, F6). The axis has a third value that was
never enumerated: **key-addressed with derived time-locality**.

**R4 — The peer litmus.** `models.md` §Design: "a contract/shape combination earns a new peer or
DAG-composition, not a hidden flag." This constrains **how** to add the cell (loudly, as declared
surface with its own gating facts), not **whether**. An optional `timeseries:` block is not a
hidden flag; it is the most visible output-shape declaration the framework has.

**Verdict.** The user is not missing a deep reason. There is exactly one honest technical core —
**unbounded key reach** — and it is a *derivable world-fact per model*, not a mode axiom. The
spec discarded the (key, time) output shape together with the declared-`H` clamp, when only the
clamp was unsound.

The sharpest symptom that the current taxonomy brackets-but-misses the workload:
`KeyedGroupByContainsPartitionColumn` steers `GROUP BY (event_id, day)` to `refresh: batched` —
the mode that, per review §6.5, **cannot dedup across partitions**. The diagnostic sends exactly
the dedupe-with-history shape to the one mode that cannot express its correctness requirement,
while `refresh: keyed` (which can) refuses the time axis that makes it affordable and keeps the
downstream clocked.

---

## 2. The missing cell: output addressed by (key, time)

**Definition.** A stored model whose output declares both a `unique_key` **and** a `timeseries:`
block (partition column + granularity), where:

- full refresh yields **one row per key** (the SQL is still the oracle — GROUP BY key, or a
  cardinality-proven unique key via the fan-out/cardinality proof F6), and
- the partition column is an ordinary projected column of that row.

**The gating world-fact: key temporal locality.** For the writes to be time-bounded, each key's
row must live in a provably bounded slice of the time axis relative to the deltas that can touch
it. Three sources, strongest first:

1. **Functional dependency, key → partition column** (the FD declaration already exists for
   once-write enrichment). The key's partition never moves: `partition = date(MIN(event_time))`
   under once-write is the canonical case. Pruning is exact.
2. **Derived recurrence/reach bound** (F1 forward reach, `source_lateness`, an explicit
   `BETWEEN … + INTERVAL` in the model SQL): a delta row in window `W` can only touch keys whose
   rows lie in `[W − r, W + r']` for derived `r, r'`. Pruning is derived ⇒ cannot under-cover
   (F15 precedent).
3. **Declared bound + runtime check + late-fact accounting.** The dedupe window ("dupes arrive
   within 3 days") is a fact about the *source's delivery contract*, generally not derivable from
   SQL. Declared (on the source, next to `mutation_profile`/`source_lateness`), it licenses
   pruning **only with a fail-loud runtime assertion**: the merge counts scanned delta rows whose
   key falls outside the pruned slice; any violation aborts the transaction (or lands in a
   late-fact ledger under an explicit policy — review §3.2's recommendation). **Never silently
   dropped.** This is the derive-don't-declare posture: declaration is admitted only where
   underivable, and only checked, never trusted.

**Invariant.** Unchanged — the single equivalence invariant, which is addressing-agnostic. What
the cell adds is a *strengthening* symmetric to batched's per-partition equivalence: **per-slice
equivalence** — for slice-local columns, the stored slice equals the full refresh of that slice.
And a pruning theorem, not a write rule: *affected addresses of delta Δ ⊆ keys(Δ) ×
slice(window(Δ) ± reach)*. Write eligibility remains total, honoring D6.

**Execution.** Nothing new is needed at the driver layer. The windowed-keyed-maintenance driver
(F11) already steps driving partitions in temporal order with per-partition pushdown and
create-or-merge; the widened-scan/exact-clamp split (F13) already separates read margin from
write scope. The cell adds one thing to the merge step: **target-side pruning** — the `merge_into`
target scan carries `WHERE target.partition ∈ slice` derived per step. Per-window processing
("merge day by day") is exactly the existing chunking machinery: batch-safety classes /
column-family postures already decide chunk shape and ordering for both modes; a chunk here is a
slice. The per-window merge ledger (D7/K4) extends to record `(window, slice)`.

**What this cell absorbs** (the deferred-items inventory): §19.6's "one row per key per day"
hybrid cell; §6.5's cross-partition dedup at event grain; `smelt.dedup_latest`; Part 20's
settled-key eviction (keys older than the locality bound are provably out of every future slice —
GC becomes a *consequence*, not a policy); bounded-gap sessionization (a bounded gap ⇒ bounded
recurrence); D16's consumer-facing `timeseries:` on keyed-family outputs (§2.1).

### 2.1 The clock-sink problem (why this matters beyond one model)

Keyed output today carries no `timeseries:`, so a keyed stage **strips the clock from the DAG**.
Downstream of the dedupe model:

- a downstream *keyed* model has no clocked driving source ⇒ forced into snapshot-reconcile
  (whole-table diff every run);
- a downstream *batched* model gets no source pushdown filter ⇒ full source scan every run.

So "dedupe (keyed) → enrich daily (batched)" is not just inefficient at the dedupe stage; it
poisons every consumer. The user's pipeline is a *chain*, and the chain is only incremental if
clocks propagate. A keyed output with a declared `timeseries:` block is a clocked source like any
other — window-forward consumption downstream falls out with zero new machinery. (The same fix
answers review §3.3/§6.4 for `materialized_view`/`versioned` outputs; D16 already defers exactly
this under "needs pushdown wiring + small design.")

---

## 3. The superset construction: batched = keyed at a degenerate point

Claim: **given a unique key, partition-addressed replace is a special case of key-addressed
merge.** Take a batched model with partition column `T`, write window `W`, and unique key `K`
(declared, or proven by F6; batched YAML already carries a vestigial `unique_key` "for MERGE
backends").

```
DELETE WHERE T ∈ W; INSERT (recompute over W)
  ≡
MERGE INTO target USING recompute(W) ON target.K = source.K
  WHEN MATCHED THEN UPDATE
  WHEN NOT MATCHED THEN INSERT
  WHEN NOT MATCHED BY SOURCE AND target.T ∈ W THEN DELETE
```

The two modes then differ in exactly three places, and each difference is a **derived degree of
freedom**, not a contract wall:

| Degree of freedom | batched today | keyed today | who decides in the unified frame |
|---|---|---|---|
| **Recompute scope** | all rows of the write window (whole-region) | delta-affected keys only (fold via the ladder) | delta-discovery quality (F9) + algebraic class (F4): fold when the ladder admits it, recompute the region when it doesn't |
| **Absence semantics** | rows missing from the recompute are deleted (replace) | departed keys retained (D8) — absence not inferable from a delta | deletion is licensed **iff the run's source is complete over a provable address region** — whole-partition recompute gives the region; a delta doesn't. One rule explains both modes' behaviour |
| **Write primitive** | `DeleteInsert` (also `Append`/`InsertOverwrite`) | `merge_into` | **backend capability + cost**, per the logical/physical split — not the user's mode choice. dbt makes users pick `incremental_strategy`; smelt should derive it |

Everything else is already shared: the F11 driver, the F13 scan/clamp split, run-window
alignment, posture/ordering derivation (batched's batch-safety classes and keyed's D5
column-family postures are the *same scheduler decision* fed by the same derived facts — F10's
self-edge ordering on one side, "any overwrite column ⇒ sequential temporal order" on the other),
the run ledger, and the equivalence invariant itself.

**What batched remains for (the honest residual).** The superset claim is conditional on a key,
exactly as posed. Batched stays the right peer for:

- **keyless / multiset outputs** — batched admits arbitrary SELECT bodies (exploders, flattens,
  duplicate-producing joins); keyed's oracle requires one-row-per-key. No key ⇒ no merge
  identity ⇒ partition replace is the *only* sound addressed write.
- **backends without a usable MERGE** — replace is the universal fallback; on merge-capable
  backends the planner may still *choose* replace when the delta is dense (rewriting a partition
  is cheaper than merging most of it).
- **the multiset ladder rung** — bounded-domain exact-holistic recompute is region-shaped.

So the end state is not "delete batched." It is: **one logical contract — addressed, maintained
output over `(K?, T?)` — with batched and keyed as named strategy families over it**, and the
strategy chosen from derived facts + backend capabilities. `refresh: batched` and
`refresh: keyed` survive as the user-facing names of the two poles; the (K, T) cell belongs to
keyed because key identity is the stronger primitive; the planner owns primitive selection.

---

## 4. The motivating pipelines, replayed

### 4.1 High-volume dedupe (3-day window, day-by-day)

Model: `GROUP BY event_id`, payload columns under `smelt.latest(payload, event_time)` (supersede)
or first-observed (suppress-late). Output declares `timeseries: partition = date(canonical_time)`.
Locality: declared 3-day redelivery window on the source, runtime-checked (§2, source 3).

Today's two workarounds, and why each fails half the requirement:

- **`batched` + 3-day lookback** (the §19.5 answer). Correct for *suppress-late* dedupe only:
  the widened scan sees the earlier copy and drops today's dupe. It **cannot** express
  *supersede* (latest-wins): the winner must replace a row in a partition outside the write
  window — batched writes cannot reach it (that is the definition of the mode). And it pays a
  full 3-day rewrite every run regardless of dupe rate.
- **`keyed` on event_id** (the §6.5 answer). Correct for both flavours, but the merge's target
  scan is unbounded over an event-grain table (a hash-shaped key defeats zone maps), the output
  loses its clock (§2.1), and §6.5's cardinality question stands open.

The (K, T) cell: window-forward step over day `D`; delta = day `D`'s source partition; merge
target pruned to slice `[D − 3d, D]`; supersede updates the canonical row wherever it lives *in
the slice* (including moving its partition column — an in-slice row move, see §7 open Q3);
violations of the 3-day promise fail loud. Per-run cost: `O(|Δ_D|)` scanned + `O(3-day slice)`
target read + `O(affected keys)` written — versus batched's 3-full-days *written* per run and
unpruned-keyed's full-table target read. **This is the answer to §6.5's open question:
per-window `merge_into` is acceptable at event-grain cardinality precisely when the target is
slice-pruned by a locality bound.**

### 4.2 Enrichment in daily/hourly batches

Model: enriched events = events ⨝ dimensions, `K = event_id`, `T = event_date`. Two upstreams,
two shapes of change, **two different targeted queries** — this is §5's subject:

- **events advance** (clocked, append-only): window-forward fold of the new slices. Touches new
  addresses only.
- **a dimension changes** (mutable or change-feed): F15's dimension-driven horizon-bounded MERGE
  — probe the changed dimension keys (delta-driven probe or idempotent window re-scan, selected
  by the *dimension's* `mutation_profile`), merge into the affected target slices
  `[ts − H, ts]`, never re-read the fact. Touches old addresses, bounded by derived `H`.

Both transforms are built (F9 `79a21f8e`, F15 `909c899d`). What is missing is (a) the output
shape to hold the result (the cell), and (b) the *name* for the dispatch (§5). Hourly stepping
is a driver-granularity work item (`day`/`week` only today — Known Divergences), orthogonal to
the mode question and unchanged by this proposal.

---

## 5. Per-upstream scope maps — reinstating targeted refresh

The idea the user keeps reaching for ("different sources changing ⇒ different queries, targeted
at just that upstream") was developed around accumulating_snapshot but — the archaeology is
clear — **never named as a first-class concept**: `20260703` Part 20 and the retired spec
express it only as a fact-vs-dimension split, and `keyed_models.md` contains no occurrence of
*upstream / per-input / targeted*. It was not deliberately dropped; it was never promoted. The
pieces, however, are all built. Promote it:

**Definition.** For each upstream `U` of a maintained model, the planner derives a **scope map**
`σ_U : Δ(U) → (affected output addresses, recompute strategy, bound)`. A maintenance run is: for
each upstream with a non-empty delta, apply its scope map; union the work; schedule under the
derived posture (ordering/parallelism); write through the selected primitive.

**Already-built instances** (this is a naming exercise, not a build):

| Upstream shape | Delta discovery (F9) | Scope map | Strategy |
|---|---|---|---|
| clocked driving fact | window-forward | new slices × their keys | fold (ladder) per slice |
| mutable dimension | snapshot-diff / delta-driven probe | keys joined to changed dim keys, within derived `H` (F6 licenses) | F15 horizon-bounded MERGE, no fact re-read |
| change-feed source | change-feed | keys in the feed | fold / overwrite per column family |
| the model itself (self-edge) | — | forward-dependent slices | F10 ordered sequential execution |
| **the model's own definition** | model diff | **column** addresses (additive-only diff, F8) | F14 targeted column backfill |

Note the last row: scope maps generalise cleanly over the *address space*, covering
column-targeted maintenance with the same vocabulary.

**What's missing / unwired:**

- declared `append_only` / `mutable` mutation profiles do not yet change the F9 verdict
  (Known Divergences — everything falls back to snapshot-diff except `change_feed`);
- `smelt explain` should print **one row per upstream**: `(upstream, discovery verdict, scope
  map, strategy, bound, checked-or-derived)` — making "what runs when X changes" a first-class,
  inspectable answer; today the run shape reads as a property of the *model*, hiding the
  per-upstream dispatch;
- union-of-same-clock driving sources (D12, deferred) is two window-forward scope maps sharing a
  clock — it slots in here rather than as a special case of "the" driving source;
- `model_maintenance.md`'s composition contract should name the scope map as the fifth element:
  Properties + World-facts + Transforms + Output shape + **Scope maps (one per upstream)**.

This section is deliberately mode-agnostic: batched, keyed, and the (K, T) cell all read their
run shape off the same table. That is the "essential task" framing made concrete — the mode
names compress *which scope maps and write primitives are admissible*, nothing more.

---

## 6. What this asks of backends

The logical/physical split does the work: the cell's *contract* never requires a capability —
the planner selects the best admissible realisation, falling back to coarser ones.

| Capability | Needed for | Fallback if absent |
|---|---|---|
| `MERGE` (three-arm) | keyed at all | full refresh (existing rule) |
| `MERGE … WHEN NOT MATCHED BY SOURCE` | replace-as-merge (absence semantics, §3) | keep DELETE+INSERT for replace |
| partition/zone pruning honoured on merge **target** scan | slice-pruned merge (§4.1) | unpruned merge (correct, slower) — or partition-replace when recompute is region-complete |
| `UPDATE` of a partition-column value (row movement) | in-slice supersede that moves the canonical row | rewrite the containing slices via replace |
| `INSERT OVERWRITE` / partition replace | dense-delta strategy point | DELETE+INSERT |

Capabilities are per-backend rows to verify (DuckDB ≥ 1.4 ships `MERGE INTO` — arm coverage to
be confirmed; Delta/Databricks has the full set including `NOT MATCHED BY SOURCE`; engines
differ on updating partitioning columns). None of this is speculative machinery: it is the same
capability-matrix discipline `multi_backend.md` already applies, extended by a few rows. The
cost *crossover* (merge vs rewrite as delta density grows) is a planner-stats question and can
start with a crude density threshold.

---

## 7. Proposed spec deltas, adoption options, and open questions

### Spec change list (if adopted)

1. **`keyed_models.md`** — admit an optional `timeseries:` block on keyed output. Gate:
   partition column projected and derived from the driving clock, **and** key temporal locality
   established per §2 (FD / derived bound / declared+checked). `KeyedForbidsTimeseries` narrows
   to "keyed output without establishable temporal locality". Add the fail-loud slice-violation
   diagnostic + late-fact accounting policy. State per-slice equivalence as the strengthening.
2. **`model_maintenance.md`** — enumerate the third addressing value (key-addressed with derived
   time-locality); restate D6's principle in its narrow true form (*no unproven bound may refuse
   a write; violated declared bounds fail loud*); add **scope maps** to the composition contract.
3. **`batched_models.md`** — document replace-as-merge equivalence (§3) and primitive selection
   as physical strategy; no user-surface change.
4. **`sources.md`** — a declared delivery/redelivery-window fact (sibling of `source_lateness`),
   consumed only under runtime check; wire `append_only`/`mutable` into F9 verdicts.
5. **`smelt explain`** — per-upstream scope-map rows; slice-pruning provenance
   (derived/declared-checked) surfaced the way `source_bounds` already is.

### Adoption options

- **A. Extend keyed with the (K, T) cell** (deltas 1+2 minimally). Smallest sound step; unblocks
  both motivating pipelines; batched untouched.
- **B. A + the unified-contract recast** (all five deltas): batched documented as the
  keyless/whole-region strategy family of one addressed-maintenance contract; primitive
  selection moves planner-side over time. No mode is removed; `refresh:` names keep meaning
  *contract*, the planner owns *strategy*.
- **C. New peer mode for (K, T)** (strict litmus reading). Rejected-by-recommendation: the cell
  shares keyed's invariant, oracle, driver, ledger, and families — a peer would duplicate a spec
  to house one extra world-fact, i.e. the same fragmentation the collapse just removed.

**Recommendation: B as direction, A as the first increment.** A is a spec-only amendment to a
mode whose implementation phases (K1–K6) are *in flight but early* — as of this writing the
autonomy loop is mid-K1 (spec retirements staged). Nothing in K1–K6 conflicts: the cell is
additive (K3's family derivation extends to the partition column; K4's ledger gains a slice
field; K5's snapshot-reconcile composes with slice pruning when locality is derived). Sequencing
the spec amendment before K3 lands avoids re-cutting diagnostics twice.

### Open questions

1. **Keyless batched as degenerate keyed?** Is multiset output "keyed on an opaque row identity
   with region-complete recompute", or genuinely outside the merge frame? (Affects how far B's
   recast goes; nothing user-facing hangs on it.)
2. **Late-fact policy surface.** Abort-transaction vs quarantine-ledger vs both; where the
   policy is declared (model vs project); how it reports ("you declared 3 days; 0.4% arrived
   later" — review §3.2's ask).
3. **In-slice row movement.** Supersede that changes the canonical partition value = UPDATE of a
   partition column. Semantics are clear (slice covers old+new); physical support varies; the
   once-write/FD case avoids it entirely. Does the family derivation (D5) force
   sequential-temporal for moving-`T` models the way overwrite columns already do? (Likely yes.)
4. **Snapshot-reconcile × locality.** When the driving source is unclocked, does a derived
   locality bound still license slice pruning of the diff-merge, or is pruning window-forward
   only?
5. **Deletion signal.** D8 retains departed keys; with `NOT MATCHED BY SOURCE` + region-complete
   recompute, deletion becomes expressible — should it be admitted for the (K, T) cell where the
   region is a slice (e.g. re-dropped dupes), or held for the change-feed-with-deletes design?
6. **Hourly granularity.** Driver step arithmetic (Known Divergences) — orthogonal, but the
   dedupe/enrichment pipelines make it more valuable; worth re-prioritising once the cell lands.

---

## 8. Summary

- There is no theorem behind `KeyedForbidsTimeseries` — only the collapsed trio's output shape
  plus D6's (correct) refusal of unproven write clamps, over-applied to proven read pruning.
  The framework already contains a key-addressed, time-bounded write (F15) and a mode-agnostic
  windowed-keyed driver (F11); the cell is the missing *output shape*, not missing machinery.
- Given a unique key, batched is keyed at a degenerate point: whole-region recompute +
  absence-deletion + replace primitive. Each difference is a derived planner choice, not a
  contract wall. Batched remains the honest peer for keyless/multiset outputs and
  merge-less backends.
- The (K, T) cell absorbs seven previously-deferred fragments, answers review §6.5's open
  cardinality question, and un-sinks the DAG clock so keyed stages can feed batched/keyed
  consumers window-forward.
- Per-upstream targeted refresh gets its name back as **scope maps** — already built as
  F9/F14/F15/F10, needing only promotion into the composition contract and the explain surface.
