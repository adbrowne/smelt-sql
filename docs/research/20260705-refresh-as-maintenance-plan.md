# Refresh is not a mode — it is a per-column maintenance plan

- **Date**: 2026-07-05
- **Status**: research (design exploration; predecessor to a possible spec change)
- **Author**: Andrew (with Claude)
- **Related specs**: `models.md`, `model_maintenance.md`, `batched_models.md`, `keyed_models.md`, `model_properties.md`, `model_transforms.md`
- **Related research**: `2026-05-20-incremental-gaps-from-web-analytics.md`, `20260521-incremental-as-planner-rule.md`, `20260705-keyed-collapse-application.md`, `20260705-keyed-time-superset.md`, `20260703-model-updates.md`, `20260705-property-discovery-loop.md` (the empirical engine that settles this paper's mechanical claims — §12's next-steps feedback)

---

## Summary

The refresh axis today names five peers — `full`, `batched`, `keyed`, `versioned`,
`materialized_view` (`models.md` §"Refresh axis"). This paper argues that, below the
top-level trichotomy **Full / Incremental / Engine-maintained**, the remaining
distinctions are not *modes* but *update strategies*, and that a single model needs
**different strategies for different inputs and different output columns**. The
`refresh:` value on a model is a lossy projection of a richer object:

> **A model's incremental maintenance is a plan indexed by `(output-column-group ×
> input-delta)`. Each cell chooses a point in a 2×2 of *read scope* × *write scope*;
> the two familiar techniques — recompute-a-region and fold-a-delta — are two of its
> four corners. Whether two techniques may serve the same cell interchangeably is
> governed by a single theorem: at a fixed processed-input set, they must produce
> identical state on the columns that decide *which rows exist*.**

Under this lens, the current "modes" are named projections of common plans; the
maintenance framework's admission matrices are *characterizations of where two
techniques disagree*; and the pragmatic relaxations a real project needs (audit
timestamps, plausible-not-identical payloads) fall out of a **skeleton / payload**
split of the equivalence invariant. The goal is a framework that *helps you be correct
about what you traded away* — not one that blocks pragmatic trade-offs in the name of
purity.

This does not discard the existing design. It **refines** three ideas already present
but under-developed — *output addressing* (`model_maintenance.md` §"One invariant, not
two"), *scope maps* (`model_maintenance.md` §"The composition contract"), and the
*algebraic ladder* (`model_maintenance.md` §"The algebraic maintenance ladder") — and
proposes **one genuinely normative change**: the *strategy content* of the refresh
enum becomes derived, while the *output shape/grain* stays a declared-and-checked
assertion (§9). Where the paper conflicts with a normative spec statement it says so
explicitly (§11).

---

## 1. Motivation: the modes are bleeding into each other

Two surface observations start the argument:

1. **`keyed` already supports partitioning.** A `refresh: keyed` model may
   time-partition its output where key temporal locality is established
   (`keyed_models.md` §"Key temporal locality"). So "keyed" is not "the unpartitioned
   one."
2. **`batched` already carries a key.** `batched.unique_key` exists for MERGE-capable
   backends (`batched_models.md` §"Strategy enum"). So "batched" is not "the keyless
   one."

The current design answers both correctly by insisting the load-bearing axis is
*output addressing* — partition-wholesale-rewrite vs key-merge — not "has a key" vs
"has a partition column" (`model_maintenance.md` §"One invariant, not two; addressing
is the real axis"). `batched.unique_key` is a *within-partition* dedup optimization; it
never reaches a stored row outside the input window, so it is not key-addressing in
the sense that matters. That rebuttal holds.

But it answers the wrong question. The deeper problem is not that two modes look alike
at the surface — it is that **one model is not one mode.** A model can be,
simultaneously and legitimately, window-forward-append with respect to one input, a
bounded key-merge with respect to a second, and a whole-region recompute when it is
backfilled. No single `refresh:` value describes that, because the value names a
property of *the model* when the real properties live at `(input, column)` cells.

---

## 2. The worked example

We use a canonical medallion shape. **The conversions/attribution framing is new to
this paper; the mechanical claims below are the *proposed* framework's, not today's
behavior** (§8 states what today's surface actually does with this SQL — plausibly
refuses it). The underlying bound-derivation and enrichment machinery is drawn from
`batched_models.md`, `keyed_models.md` §"Enrichment joins",
`2026-05-20-incremental-gaps-from-web-analytics.md` §3, and
`20260521-incremental-as-planner-rule.md`.

**Inputs.**

- `smelt.bronze.events` — a high-volume append-only timeseries source. Daily
  `partition_column` (`event_date`, derived from `event_ts_utc`). New data arrives as
  change-feed entries.
- `smelt.conversions` — a lower-volume table of conversion events, keyed by
  `user_id` with a `conversion_ts`. **Append-only with lateness**: a conversion, once
  it happens, is never retracted, but it can *arrive* days after the event it converts.
  (This is load-bearing — see §8. If conversions were genuinely mutable/retractable,
  the `converted` column below would be a non-invertible fold over a mutable sequence,
  i.e. the observer-semantics case the theorem in §4 refuses.)

**The model** `silver.event_conversions` — one row per bronze event, enriched with
whether that user converted within a 7-day attribution window after the event:

```sql
---
refresh: batched            -- today's single label; §9 argues it is lossy
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT
    e.event_id,
    e.user_id,
    e.event_date,
    e.event_ts_utc,
    EXISTS (
        SELECT 1
        FROM smelt.conversions c
        WHERE c.user_id = e.user_id
          AND c.conversion_ts BETWEEN e.event_ts_utc
                                  AND e.event_ts_utc + INTERVAL '7 days'
    ) AS converted
FROM smelt.bronze.events e
```

`converted` is `EXISTS(...)` = a `BOOL_OR` over conversions in the window — a **lattice
(idempotent, monotone) fold**: over an append-only conversions stream it can only flip
`false → true`, and it is not invertible (see §4 condition 3, and why append-only
matters).

**Proposed bounds (not derived by today's analyzer — §8):**

- `bronze.events → (event_date, before=0, after=0)` for the pass-through columns —
  each event's identity/attributes depend only on its own row.
- `conversions → (conversion_ts, before=0, after=7d)` — an event's `converted` value can
  be changed by a conversion arriving up to 7 days *later*. This reads a bound out of a
  correlated `EXISTS` subquery, which is **beyond** the Form-B source-filter derivation
  the cited docs implement (§8).

The plan factors by **column group** (§5), and within each group the maintenance
differs by *what changed*. Read the table as `(column-group × trigger) → (read scope,
write scope)`:

| Column group (provenance) | new bronze day | late conversion (append) | backfill of `[t₀,tₙ)` |
|---|---|---|---|
| `{event_id, user_id, event_date, event_ts_utc}` (`bronze`) | delta-read → region-append *(fold/append)* | *(untouched)* | full-input → region-overwrite *(recompute)* |
| `{converted}` (`conversions` + event key/time) | bounded-read of conversions `[D, D+7d]` → region-append *(off-diagonal)* | delta-read (the new conversion) → targeted `MERGE` of `converted` for that user's events in `[conv_ts−7d, conv_ts]` *(fold)* | full-input → region-overwrite *(recompute)* |

A fourth trigger completes the picture: **late bronze rows** (bronze is a change feed,
not only whole new days). A bronze row that arrives within the model's derived horizon
rewrites its partition (and reads conversions `[D, D+7d]` for the `converted` cell); one
arriving *beyond* the horizon is **silently excluded** — not by any window in the SQL,
but by window-forward delta discovery and the derived horizon clamp
(`model_maintenance.md` §"Windowed maintenance and the horizon"), whose lateness
accounting is a model-author + data-quality concern by design. The table omits this
column only to stay legible.

Note the point of finding-2's confusion and its resolution: you *cannot* classify "the
new-bronze-day step" into one technique, because it spans two column groups that land
in **different** 2×2 corners (the pass-through group folds/appends; the `converted`
group does a bounded conversions read). Only after factoring by column group does each
cell get a well-defined corner. And the genuinely valuable, currently-inexpressible cell
is the `converted` × late-conversion one — the **fold corner** (delta read, targeted
write): a delta-directed, column-scoped, key-and-window-bounded merge that today's
`refresh: batched` cannot express (§8).

---

## 3. The technique space is a 2×2, not a dichotomy

An earlier draft claimed "exactly two families." That is a false binary; the honest
structure is two **independent axes**, and the two familiar techniques are two of the
four corners.

- **Read scope** — what an update must read to be correct:
  - *delta + state*: only the input delta, plus `O(1)`–`O(bounded)` stored state — where
    "state" is either a folded aggregate *or the current contents of the output region
    being patched* (a read-modify-write of stored output, never a re-read of upstream).
  - *region's full input*: all *upstream* input feeding the affected region, re-derived.
- **Write scope** — what an update touches:
  - *targeted*: specific addresses (a key, a column, a bounded key×time window).
  - *region overwrite*: a whole partition/region replaced wholesale.

|              | write: targeted | write: region-overwrite |
|---|---|---|
| **read: delta+state** | **fold-a-delta** (pure IVM corner: `SUM`/`MAX` state merge; the `converted` late-conversion cell) | **read-modify-write a region** (read the delta + the region's *current stored contents* as state, rewrite the region — LSM-style compaction; the "read the current partition, then DELETE+INSERT" pattern) |
| **read: full-input** | **column-scoped region re-derivation** (re-derive one column-set over a region from its full upstream input, write only that column — the dimension-driven horizon-bounded MERGE, `keyed_models.md` §"Enrichment joins") | **recompute-a-region** (re-derive the region from full upstream input; DELETE+INSERT a partition) |

The two off-diagonal corners are real and already in the corpus, and are distinguished
by *what they read*, not merely by a trigger: the top-right reads the stored output
region (not upstream) and rewrites it — genuinely distinct from bottom-right, which
re-derives the region from upstream. The bottom-left is the enrichment MERGE the specs
define, and **delta-propagation through joins** (`ΔR ⋈ S`) is a top-left inhabitant
whose read touches `S` but is still delta-driven. (Window-forward batched sits in the
top-right *only in the degenerate pure-append case* where the new region has no prior
contents, so the read-modify-write reduces to an insert; the moment a within-horizon
late row rewrites an existing partition, the cell is bottom-right — a full upstream
re-derivation of that partition.) Exhaustiveness is defensible: *every* incremental
write picks a read scope and a write scope.

The two *named* techniques remain the load-bearing corners because they anchor the cost
model and the theorem:

- **recompute-a-region** (full-input, region-overwrite) — cost ∝ region size;
  **contract-agnostic and unconditionally valid whenever the region's input is
  replayable** (it is just `full_refresh` restricted to the region).
- **fold-a-delta** (delta+state, targeted) — cost ∝ delta size; **contract-specific**
  (needs an appropriate combiner algebra; `model_maintenance.md` §"The algebraic
  maintenance ladder").

The algebraic ladder is *the machinery that moves a cell leftward on the read axis* —
it converts an unbounded input dependency into a bounded state read. A column with no
foldable algebra has no delta+state read available and must be read full.

**Backfill, precisely (and correctly against the spec).** Recompute-a-region is the
universal *ground-truth reset*: a backfill of window `W` recomputes `W` from replayable
input, unconditionally correct. Where the incremental path had already written `W`, the
recompute *supersedes* it. This is **not** the same operation the current keyed ledger
performs — `keyed_models.md` refuses re-running a ledgered additive window
(`KeyedReprocessedWindow`) and offers whole-table `--full-refresh`, not a
region-scoped recompute-with-reset. The generalized reconciliation structure this paper
needs (§4) is a proposed extension of that ledger, not a behavior it grants today.

---

## 4. The interchangeability theorem

Two techniques may serve the same cell interchangeably **iff they produce the same
state at a fixed processed-input set.** The processed-input index `S` (`model_maintenance.md`
§"The equivalence invariant") is load-bearing and was missing from the earlier draft.

For a fixed processed-input set `S`, an output region `R`, and a column `c`:

```
recompute(R, c, S) = c evaluated by the model SQL over (input restricted to S), read over R
fold(R, c, S)      = the state after folding exactly the deltas that partition (input ∩ S)
                     and whose footprint touches R
```

**Well-definedness is per output address, not per region.** For a cell-addressed
idempotent fold, `fold(R, c, S)` is well-defined *address by address* regardless of how
`R`'s boundary is drawn — each output address folds exactly its own footprint's deltas.
A recompute of a window `[t₀, tₙ)` therefore needs its **read** widened by the scan
bound (`[t₀, tₙ+7d)` for conversions), never its **write** region widened: it rewrites
exactly `[t₀, tₙ)` and reads enough upstream to get those rows right. (An earlier draft
demanded quotienting regions by "footprint closure"; that was both mis-analogized to
batched's write-window widening — a time-coarse expansion, `batched_models.md`
§"Execution model" step 1 — and, for a *user*-scoped conversion footprint, degenerate:
transitive closure over per-user footprints chains a frequently-converting user across
all history.) The straddle question genuinely bites only for **region-granular
bookkeeping** — the additive ledger below, which must attribute a delta to exactly one
region so it is neither double-counted nor lost. There, and only there, a region is a
union of whole footprints.

**Interchangeability (idempotent columns).** For an idempotent column whose fold is
*faithful* (below), `recompute(R,c,S) = fold(R,c,S)` at every reachable `S`; either
technique may be used, and switching between them is free.

**Faithful fold** is defined precisely: the delta stream is a *partition* of the input
multiset (no overlaps, no retractions), and the combiner's fold over any sub-multiset
equals the batch aggregate over that sub-multiset. `BOOL_OR`/`MIN`/`MAX` over an
**append-only** source are faithful; the same combiners over a *mutable/retractable*
source are **not** (a removed row cannot be un-folded from a non-invertible combiner) —
this is condition 3, and it is why §2 makes conversions append-only.

**State-equivalence-modulo-ledger (additive columns).** For a non-idempotent column
(`SUM`/`COUNT`), recompute and fold *converge to the same state*, but the hazard is
**asymmetric**, not "never apply both": fold-then-recompute is safe (the recompute
overwrites the region from ground truth), while recompute-then-refold-the-same-deltas —
or folding any delta twice — double-counts. The ledger's real obligation is therefore
**"never fold a delta already reflected in the state,"** which a region recompute
satisfies by resetting the ledger for the region it overwrote. Interchangeability here
is thus *state-transition equivalence given the ledger*, strictly weaker than the
idempotent case's value-interchangeability — the earlier draft's single `≡` equivocated
between the two.

**Where the two disagree — the admission matrix, re-derived.** The theorem's failure
cases are exactly the existing refusals:

- **Observer semantics** — `MIN(price)` folded over successive *mutable snapshots*:
  `fold` = *min ever observed*, `recompute` = *min in the snapshot at S*. Unequal at
  almost every `S` — this is `KeyedSnapshotSourceUnsupportedColumn` (`keyed_models.md`
  §"Admission matrix"). Note conditions 2 (replayable input) and 3 (faithful fold) are
  **independent**: a replayable change feed that carries *retractions*, folded into a
  non-invertible `MIN`, satisfies 2 but fails 3.

**The `S`-index resolves the apparent collision with "validator, not chooser."** A
stored folded value reflects the `S` at which it was last folded; a fresh recompute
reflects the current `S`. When these differ it is because the fold is *stale*, not
because the technique changed the contract. Advancing from a folded `S′ ⊂ S` to a
recomputed `S` is a **freshness advance** (more input processed), never a contract
swap. So the invariant `state = full_refresh over processed S` holds under either
technique; technique choice may only change *which `S` is reflected* — the settle-bound
dimension (§6), surfaced, monotone-good. This is what licenses the cost model (§9,
OQ3) to choose fold-vs-recompute freely: at a fixed `S` the choice is bit-preserving on
faithful/idempotent columns, and on additive columns it is state-preserving modulo the
ledger. A choice that changed observable bits *at a fixed `S`* would indeed be a
chooser and is forbidden.

**`S` is a per-input vector.** Once the plan factors by `(column-group × input)`, each
cell's processed set ranges over its own source, so the whole-model invariant is the
vectorized `state = full_refresh(each input i restricted to its own Sᵢ)` — well-defined
given clean provenance partitioning (§5), and a refinement of `model_maintenance.md`'s
single-`S` statement, not a contradiction of it. Two consequences: `recompute(R, c, S)`
at an arbitrary *past* `S` is counterfactual for a real source (only current content is
re-derivable), so condition 2's "replayable" means **replayable at the current `Sᵢ`**,
which is all the theorem's actual uses need; and the freshness a fold reflects is
per-input (a `converted` value can be current on bronze but stale on conversions).

---

## 5. Maintenance is a per-`(column-group × input)` plan

The example's plan factors by **mutation-sensitivity**, not syntactic provenance, and
the distinction turns on separating two things a column's inputs do:

- **Creation** — which input's deltas bring *new rows* into being. This is the driving
  (fact) source, `{bronze}` here; it is a property of the table's **grain**, shared by
  every column, and it is what a "new bronze day" trigger exercises (all columns of the
  new rows are computed together at creation).
- **Mutation** — once a row exists, which inputs' deltas can still change *this column's
  value*. This is what partitions the columns for *targeted* maintenance.

The right question is therefore the mutation one: a reference to the row's **own,
immutable skeleton** (`e.user_id`, `e.event_ts_utc` — fixed the moment the bronze event
is materialized) creates no mutation-sensitivity, because append-only bronze never
rewrites an existing event. So although `converted = EXISTS(… c.user_id = e.user_id AND
c.conversion_ts BETWEEN e.event_ts_utc …)` *reads* bronze columns at creation, its only
**mutation-sensitivity is `{conversions}`**, while the pass-through columns' is `{}`
(never mutated after creation). That gap — not a difference in *creation* source — is
exactly why a late conversion can update `converted` alone.

Column groups are the partition induced by **shared mutation-sensitivity**. A projection
mutation-sensitive to two inputs (`e.x + c.y` where `e` too is mutable, receiving
post-creation deltas) merges their groups; a projection that merely references a second
input's *immutable* value at creation does not. In the limit where one projection is
mutation-sensitive to every input, a single group covers the table and the plan
degenerates to today's per-model story — so the thesis's value is proportional to how
often mutation-sensitivity partitions the output non-trivially, an empirical question
(OQ5). (This makes the append-only premise of §2 do double duty: it is what keeps
bronze out of `converted`'s mutation-sensitivity set — under a *mutable* bronze the two
groups would merge and the targeted update would be lost, exactly as the observer-
semantics refusal of §4 predicts.)

This is the *scope maps* idea (`model_maintenance.md` §"The composition contract")
promoted from a composition-contract footnote to the organizing principle. Two derivable
facts drive it: column **mutation-sensitivity** (column provenance from the SQL, refined
by each source's mutation profile — an immutable-at-creation reference drops out), and
per-input footprint/reach (the `(source_partition_col, before, after)` triple,
`20260521-incremental-as-planner-rule.md` — subject to §8's caveat about correlated
subqueries).

**Note the scan/footprint reflection.** One bound triple encodes two dual maps: the
*scan* bound (input read window per output window) and the *footprint* map (output
write window per input delta), which are reflections of each other. For conversions,
scan `(before=0, after=7d)` reflects to footprint `(before=7d, after=0)`: an event's
run window `[s,e)` reads conversions over `[s, e+7d)`; a conversion at `t` writes events
over `[t−7d, t]`. The numbers look symmetric here only by coincidence; an asymmetric
window would make the reflection visible, so it must be stated, not assumed.

---

## 6. The skeleton / payload invariant (two-dimensional)

The pragmatic relaxations a real project needs — an `inserted_at = NOW()` audit column,
a tolerable surrogate, "two full refreshes aren't bit-identical anyway" — are not a
weakening of correctness. They are a statement about **which columns carry the
invariant**. The batched spec already implements this as `nondeterministic_columns` +
taint analysis (`batched_models.md` §"Non-determinism and the payload rule"); this paper
lifts it to the governing principle:

> **State the equivalence invariant over the *skeleton*, not the *payload*.**
> The **skeleton** = which rows exist, their identity, partition placement, and every
> membership / grouping / dedup / ordering role. The **payload** = everything else.
> Skeleton columns are held to strict (`S`-indexed) equivalence; payload columns may be
> non-deterministic, or merely *plausible*, provided non-determinism is *proven not to
> leak into the skeleton*.

**Two dimensions, not one.** The earlier draft's rail table conflated equivalence
*strength* with *settledness*. They are orthogonal. `converted` is `S`-indexed **exact**
(not a payload relaxation) but **unsettled** — and its settle bound is **watermark-
relative, not a fixed 7 days**: because conversions are append-only *with unbounded
arrival lateness* (§2), an event's `converted` is settled only once the conversions
watermark passes `event_ts + 7d`. Stating an *absolute* settle time (e.g. "7 days after
the event") would require a **declared source-lateness bound** on conversions, which the
example does not carry — so the honest ledger entry is the watermark condition, and a
fixed number is exactly the unlabeled looseness §6's discipline forbids. The ledger of
per-column guarantees is therefore two-dimensional:

| column | equivalence contract | settle bound |
|---|---|---|
| `event_id` | skeleton, exact | settled immediately |
| `converted` | exact (idempotent monotone fold) | conversions watermark ≥ `event_ts + 7d` (absolute only with a declared conversions-lateness bound) |
| `inserted_at` *(if present)* | payload, plausible | n/a |
| a running-total trajectory *(if admitted, §7)* | as-of-run / prefix-consistency | never (per late data) |

The equivalence-contract column has (at least) three values — `exact`,
`plausible-payload`, and the deliberately-weaker `as-of-run` of OQ2 — so the split is
**not** binary; §7 depends on the third value existing.

**Payload-ness leaks across the DAG.** Skeleton is a *role*, and role is
model-relative. A column that is payload in `M` (plausible-only) but consumed in `N`'s
`JOIN … ON` / `WHERE` / `GROUP BY` is **skeleton for `N`**: non-determinism admitted in
`M` now decides which rows exist in `N`. `batched_models.md`'s taint analysis is
intra-model and does not catch this. The rule the invariant needs: **payload-ness is a
column-level property that propagates downstream, and a payload column consumed in a
skeleton position fails loud at the consumer** (retro-tightening the producer's
contract, or forcing a stable derivation such as a hash of skeleton columns). This is
the sharper form of OQ1.

The discipline that keeps *pragmatic* from becoming *sloppy*: every relaxation **names
the guarantee it trades**, per column. Non-determinism trades bit-identity → plausibility
(payload only); as-of-run trades `S`-exactness for a prefix-consistency guarantee under
late data (§7) — a much larger trade. Both may be allowed; they must not wear the same
label. The sins guarded against are *unlabeled* looseness (dbt's hidden `strategy:`) and
looseness that *leaks into the skeleton* — not looseness itself.

---

## 7. PerPartitionOnly: mis-assignment *or* an honest weaker contract

`batched`'s `PerPartitionOnly` class (`batched_models.md` §"Batch safety
classification") triggers when a source is `Unbounded` — partition `p` depends on *all
history ≤ p*. batched then builds partitions one at a time, each re-reading all prior
history, and the result is "correct as-of-the-run, not equal to a full refresh."

The earlier draft called this simply a mis-assignment and claimed fold makes the wart
"disappear." That is wrong, and the distinction is **what the output grain is**:

- **If the modeller wants the end-state** (one settled value per key), then yes — the
  column is mis-assigned: its algebra wants fold-a-delta into key-addressed state, and
  the honest fix is a **keyed model**. But note this is a **grain change** (`key` vs
  `key × partition`), not a mere cell re-routing — it changes what rows the table
  stores, so it is a different model, consistent with §9's grain anchor.

- **If the trajectory itself is the product** — the running value *at every partition*,
  one stored row per `(key, partition)` — then a late input row at time `t` has an
  **unbounded forward footprint**: it changes the stored value at `p` and *every
  partition ≥ t*. This breaks *A* (targetable output), not just *B* (see §10). Folding
  a late delta into key state repairs the cheap end-state projection but does nothing
  for the stored trajectory, whose every later row is now stale and must be rewritten —
  cost ∝ number of later partitions, i.e. exactly the unbounded write the fold was
  supposed to avoid. Fold moves *where the cost lands* (state vs history re-read); it
  does not make the trajectory's late-data footprint bounded.

So for a genuine trajectory output the forward footprint is **irreducibly unbounded
under late data**, and the honest options are (a) the **as-of-run / prefix-consistency**
contract of OQ2 — explicitly labeled in the §6 ledger, not silently tolerated — or (b)
bounded-lateness truncation (drop or re-stamp arrivals past a horizon,
`model_maintenance.md` §"Windowed maintenance and the horizon"). PerPartitionOnly is a
*mis-assignment* only in case (a-of-the-first-bullet); when the trajectory is wanted it
is a *legitimate weaker contract that must be named*.

---

## 8. What today's surface actually does with the example

The earlier draft asserted the `conversions → (0, 7d)` bound as something "smelt derives"
and said the current surface "can express only the third technique." Both overstate.
Precisely:

- The bound is **proposed**, not derived today. Today's Form-B derivation reads offsets
  from **source-filter** `WHERE`/`JOIN` predicates (`batched_models.md` §"Observing the
  per-source clamp"; `20260521-incremental-as-planner-rule.md`). A **correlated `EXISTS`
  projection subquery** over an *unclocked* table is not a shape any cited derivation
  reads.
- `conversions` carries no `timeseries:`, so today it is a **lookup, read in full**
  (`batched_models.md` §"Execution model", step 3) — no bound, no targeted-column
  update; every run re-reads all conversions.
- More likely, the correlated subquery over a mutable lookup lands in **`NotDerivable`
  / refusal** territory (`BatchedNotSafe`), so today's surface may express **none** of
  the three techniques for this model, not just "only recompute." That is worth
  checking against the analyzer, and either way it *strengthens* the motivation: the
  targeted per-column maintenance is inexpressible today.

The lesson: the example is a *specification target* for the proposed framework, not a
demonstration of current behavior.

---

## 9. What stays singular — and what is declared vs derived

If technique is per-situation and contract is per-column-group, is anything
irreducibly per-model? Two things, and their status differs.

**Output grain / row identity — the true anchor.** "What is a row, and how is it
addressed" must be stable across runs and agreed by every technique writing the table,
or a targeted merge and a region recompute could disagree on which physical rows they
touch. Two facts, separated:

- **Whether a row identity is *needed* is derived — over *admissibility*, not the chosen
  plan.** A table needs an identity iff a targeted-write cell is **admissible and in its
  plan space** (i.e. fold / column-merge is a legal technique for some cell), *not* iff
  this run's cost model happened to pick it — otherwise identity would flip with delta
  size. This is deliberately the *admissible* reading, not "the only technique is fold":
  a cell where fold and recompute are interchangeable still needs an identity (fold is in
  its plan space), which is exactly what lets §4's cost model switch between them freely.
  Recompute-only tables (no cell admits a targeted write — pure batched) need no
  identity, which is why `batched.unique_key` is optional.
- **The identity itself, when needed, is declared and checked** — the `unique_key` /
  grain the modeller states, validated against the plan (an error if a cell's targeted
  write cannot be addressed by it), never silently inferred and then depended on.

**Output shape (partitioned vs keyed) — declared-and-checked, *not* silently derived.**
The earlier draft proposed deriving output shape from the plan. That reintroduces
exactly the silent-contract-swap the declaration law exists to prevent (`models.md`
§"Refresh modes are peers", §"The declaration law"): shape governs downstream
consumption (windowed source vs read-in-full lookup), the write primitive, and the
identity requirement, and deriving it means a refactor of a projection could flip it
with *no diagnostic* — the trigger merely moved from a YAML knob to the SQL, which is
worse for review. The §8 "checked assertion ≠ selector" defense covers *bounds* (there
is an SQL fact to validate against) but **not** shape (nothing to validate against).
The resolution: **shape stays a declared assertion, validated against the derived plan
— an error on mismatch, never a silent flip.** This keeps the paper's real claim (the
enum's *strategy content* is derived) while preserving the anti-footgun property.

So the honest "3 not 5":

- **Full / Incremental / Engine-maintained** is the real trichotomy and the right
  primary axis. Full recomputes; Engine (`materialized_view`) delegates to native IVM
  with a different *freshness owner*; Incremental is smelt-maintained.
- **`batched` / `keyed` / `versioned` lose their status as *strategy* peers** — their
  strategy content (which technique per cell) becomes derived. What remains declared is
  the **output shape/grain** each implies, now checked against the plan rather than
  selecting it. `batched` ≈ "grain = partition, all cells recompute the partition";
  `keyed` ≈ "grain = key, cells fold into key state"; `versioned` ≈ keyed with an
  interval close-out — **but only over replayable input** (over snapshot-reconcile
  input an SCD2 *row set* is itself a function of the observation sequence and fails
  `recompute ≡ fold` at the skeleton level; that inhabitant belongs to OQ2's third
  contract, not to the two named corners).

---

## 10. Appendix: the A/B property space

For an `(input, column)` cell:

- **A — targetable output:** from an input delta, a bounded set of output addresses to
  replace.
- **B — clampable input:** an output cell's final value is computable from a bounded
  input slice.

A and B are duals connected by the input→output dependency graph:

- **Local dependency ⟹ A ∧ B** — `batched FullyBatchSafe`; the ideal corner.
- **Broadcast breaks A, keeps B** — a dimension change touches every fact row; each row
  still depends on a bounded slice. The enrichment case, bounded only by dimension
  churn (`keyed_models.md` §"Enrichment joins").
- **Cumulative breaks B — and *also* breaks A for a trajectory output.** A running total
  depends on all history (¬B); the ladder rescues B **only for the end-state
  projection** (one row per key), by folding history into bounded state. For a *stored
  trajectory* (value at every partition) a late row also has an unbounded forward
  footprint (¬A), which the ladder does *not* repair (§7). The earlier draft's
  "cumulative keeps A" held only for the keyed end-state grain, not the partitioned
  trajectory that `PerPartitionOnly` actually materializes.

Full refresh reads all input and writes all output; you beat it by bounding the read
(B, or the ladder's state) or the write (A), decisively with both. This is the theory
beneath §3's axes: the read axis is B(-or-state); the write axis is A.

---

## 11. Relationship to the current design

A **refactor of emphasis with one normative change**, stated honestly:

Kept (refined): the equivalence invariant and its processed-input `S` framing; the
algebraic ladder (reframed as the read-axis mechanism); the addressing distinction
(reframed as a *derived* footprint property of a column group); "validator, not
chooser" (extended — the planner composes per-cell techniques whose `S`-indexed
equivalence it has proven, and never swaps a contract); derive-else-declare and
"declared bound, admitted only checked" (generalized across the plan matrix); fail-loud
refusal (now "refuse the cell where the chosen techniques disagree at fixed `S` and no
weaker contract is labeled").

**Normative conflicts, named as such (not undersold as refinements):**

1. The refresh enum's *strategy content* moves from **declared** (`models.md`
   §"Refresh modes are peers": the mode *is* the strategy selector) to **derived per
   cell**. This directly revises the peers argument for the batched/keyed/versioned
   trio (though not for the Full/Incremental/Engine trichotomy, which the paper keeps).
2. The ledger is proposed as a **general `(input × column-group)` → `(output-region ×
   column-group)` reconciliation structure**, of which the current keyed window-ledger
   and the batched no-ledger case are degenerate specializations. The
   region↔window correspondence is exact only under key temporal locality or explicit
   footprint tracking; presenting it as "the ledger, generalized" is a design proposal,
   not a property of the shipped ledger (§3, OQ4).
3. The skeleton/payload split with **DAG propagation** of payload-ness (§6) extends the
   intra-model `nondeterministic_columns` taint to a cross-model property with a
   consumer-side check.

What it explicitly does **not** change: output shape/grain stays a declared,
checked-against-the-plan assertion (§9) — the anti-footgun property of the declaration
law is preserved.

---

## 12. Open questions

1. **Skeleton/payload across the DAG (§6).** Precise rule for propagating payload-ness
   downstream and failing loud when a payload column reaches a skeleton position; and
   the intra-model surrogate-key edge case (a `unique_key` derived from a
   non-deterministic surrogate — reject, or require a stable hash-of-skeleton
   derivation?).
   FEEDBACK: If I understand the question - then non determinstic columns should not be allowed in skeleton positions or where correctness is calculated (unique key and things that determine window lookbacks etc should be deterministic).

   → RESOLUTION (2026-07-05): **non-deterministic columns are barred from every skeleton
   position and every correctness-determining expression.** Concretely: a `unique_key`, a
   partition/grain column, a `JOIN … ON` / `WHERE` / `GROUP BY` / dedup / ordering key, and any
   expression that determines a window lookback/horizon MUST be deterministic. Non-determinism is
   admissible only in payload columns (§6). Because payload-ness leaks across the DAG (a payload
   column of `M` consumed in a skeleton position of `N`), the check is a **consumer-side fail-loud**:
   a payload/non-deterministic column reaching a skeleton position downstream is an error that
   retro-tightens the producer's contract or forces a stable derivation (a hash of skeleton
   columns). The surrogate-key edge case resolves the same way: a `unique_key` derived from a
   non-deterministic surrogate is **rejected** unless it is a stable hash-of-skeleton-columns. This
   is now a settled rule, not an open question.
2. **The third contract.** Is `as-of-run` / prefix-consistency admitted as an
   honestly-labeled equivalence contract (making a trajectory `PerPartitionOnly` a
   legitimate choice, §7, and giving SCD2-over-snapshots a home, §9), or refused? This
   determines whether the framework has one invariant or a small labeled lattice.
   FEEDBACK: It feels like we need this even though we lose all of our guarantees. I don't want to spend time on this now. Let's keep tracking it but we'll proceed with the rest of the design and then fit this in later. I don't want this to drive the design. It could be that we later add a more nieve refresh type that is basically DBT microbatch which would actually do this one fine.

   → RESOLUTION (2026-07-05): **deferred, tracked, and explicitly not driving the design.** The
   `as-of-run` / prefix-consistency contract is accepted *in principle* as a future third
   equivalence value — the likely home is a later, deliberately-naïve refresh type analogous to
   dbt's microbatch (per-partition-only, correct-as-of-run, no cross-partition guarantee). It is
   **not** built now and the two named corners (§9) plus the skeleton/payload split carry the
   present design without it. The rest of the framework is designed so this can slot in later as an
   added, honestly-labelled contract (§6 ledger) rather than a retrofit. Kept as a tracking item;
   removed from the critical path.
3. **Cost model observability.** The `S`-indexed theorem (§4) makes fold↔recompute
   switching contract-preserving at fixed `S` (only advancing freshness). What cost
   signals drive it (delta vs region size, backend merge support), and is the resulting
   freshness variation stable/explainable for operators?
   FEEDBACK: I actually think we should probably just have some sensible defaults and then allow them to be overridden in the frontmatter. Likely step two would be allowing an engineer to do a bunch of test runs using the alternatvie models on the same data and measure. This is actually a huge win for smelt - unlike engines we don't need to choose quickly or per query - an engineer could do a multi hour or multi day test to ensure they spend less over the whole year rather than fitting into an optimizer designed to return in "real-time".

   → RESOLUTION (2026-07-05): **sensible defaults, frontmatter override, and — the real win —
   offline whole-workload measurement.** Step one: the cost model picks fold-vs-recompute per cell
   from sensible defaults (delta size vs region size, backend merge support), overridable in
   frontmatter. Step two, and the structural advantage worth emphasising: because smelt is a
   compiler/orchestrator, **not** a real-time query optimiser, technique selection need not be
   decided quickly or per-query. An engineer can run the *alternative* plans over the *same* data
   for hours or days and measure actual cost, choosing the plan that minimises spend **over a
   year**, not the one an optimiser can pick in milliseconds. This is a first-class capability, not
   a footnote: the `S`-indexed theorem (§4) makes fold↔recompute switching contract-preserving at
   fixed `S`, so the choice is safe to defer to measured cost. This offline-measurement idea is
   promoted to a design principle (see the new subsection below and
   `20260705-property-discovery-loop.md`, which builds the run-schedule + cost-measurement harness
   this needs).
4. **The generalized ledger (§3, §11).** Its key structure, the region↔window
   correspondence under and without locality, and footprint-straddling reconciliation
   (§4) need a concrete design, not an assertion of generality.
   FEEDBACK: Can you help propose a design?

   → PROPOSED DESIGN (2026-07-05): the generalized reconciliation ledger.

   **State.** The ledger is a set of *reconciliation entries*, one per `(output-region r,
   column-group g)`. Each entry records the **processed-input vector** `S_{i,g}` — for every input
   source `i` feeding `g`, the set of that source's deltas already reflected in the stored state of
   `(r, g)`. This is the §4 vectorized invariant made concrete: the stored value of `(r, g)` equals
   `full_refresh` over exactly `⋃_i S_{i,g}`.

   **Storage is graded by the algebraic ladder (a real optimisation, not incidental).**
   - *Additive / non-idempotent groups* (`SUM`, `COUNT`): the entry must record **which deltas**
     (by delta identity — partition key / change-feed offset) have been folded, because the
     obligation is "never fold a delta already reflected in state" (§4). Per-delta bookkeeping.
   - *Idempotent groups* (`MIN`/`MAX`/`BOOL_OR`): re-folding is harmless, so only the **frontier**
     `S_{i,g}` (a watermark per input) need be stored — the ladder classification licenses dropping
     per-delta identity. This is where the generalization pays for itself.

   **Region↔window correspondence.**
   - *Under key temporal locality* (`keyed_models.md`): a region `r` is a time window, and a
     delta reconciles against the regions its **footprint** touches, via the scan/footprint
     reflection (§5) — a conversion at `t` writes events over `[t−7d, t]`, so it reconciles the
     ledger regions covering `[t−7d, t]`.
   - *Without locality* (a per-user footprint chains across all history, §4): the correspondence is
     **not** a clean interval. The entry then keys on the *output addresses* the delta touches (a
     key-set, not a time interval) and footprint membership is tracked explicitly. More expensive;
     it is exactly the "region-granular bookkeeping" §4 isolates.

   **Two region notions, to make straddle attribution well-defined (§4).** Separate the **write
   region** (what a recompute physically overwrites — arbitrary boundary, can be fine-grained) from
   the **ledger region** (drawn on *footprint-closure* boundaries — a union of whole footprints). A
   delta is attributed to the **unique** ledger region that contains its footprint, so it is neither
   double-counted nor lost. The write region may be finer than the ledger region; the ledger region
   exists only for attribution.

   **The two operations.**
   - *Fold a delta into `(r, g)`*: refuse if the delta is already in the entry's processed set
     (checked against recorded identities for additive groups; a no-op check for idempotent groups).
     Otherwise combine and extend `S_{i,g}`.
   - *Recompute write-region `W` for group `g`*: this establishes ground truth over `W`'s input, so
     it **resets** every ledger entry whose ledger-region intersects `W` — after the reset, the
     processed set for those `(r, g)` is exactly the input `S` the recompute read. This is what
     makes fold-then-recompute safe and recompute-then-refold a double-count (§4's asymmetric
     hazard).

   **Degenerate specializations (why this is "the ledger, generalized" and not a new thing).**
   - The current keyed window-ledger = this structure with a single column-group (the whole additive
     payload), key-addressed regions, under temporal locality.
   - The batched no-ledger case = every column-group is recompute-only (region overwrite), so the
     frontier is implicit in the partition watermark and no per-delta bookkeeping exists.

   **Honest caveat (mirrors §11 point 2).** The region↔window correspondence is exact only under
   locality or explicit footprint tracking; calling this "the shipped ledger, generalized" is a
   design proposal, not a property of today's ledger. The `20260705-property-discovery-loop.md`
   engine is where the additive/idempotent/straddle cases get empirically pinned before this is
   specced.
5. **How often does the plan actually factor? (§5)** Column-provenance partitioning is
   non-trivial only when projections don't mix inputs. An empirical survey of the
   example corpus (and the GROUP-BY-partition-column ambiguity frequency) would size the
   thesis's real payoff.
   FEEDBACK: The cost difference is so great that I think in many cases users will design around this.

   → RESOLUTION (2026-07-05): **accepted; no empirical survey needed to proceed.** The cost gap
   between a per-cell targeted maintenance and a whole-model recompute is large enough that users
   will structure their models so the plan factors (keeping mixed-mutation projections apart). The
   thesis's payoff is therefore realised by *design guidance*, not gated on a corpus-frequency
   measurement. OQ5 is closed for the purpose of driving this design; the discovery loop
   (`20260705-property-discovery-loop.md`) will incidentally report how often factoring occurs, but
   nothing waits on it.


   FEEDBACK: Next steps. I feel our research/spec/implement loop isn't serverving us very well to nail down these decisions and design. I'd like to propose we take a different path at this point (not going forward just for this piece). Can we create a custom loop that will build a set of models combining differnet sql constructs and work out what properties we can prove and what models hold. Think about what update patterns we could do based on properties of each upstream. Can you start creating a plan and run script to flesh that out - I want to leave this running. This should use claude -p (a bit like the regular autonomy loop) and like the autonomy loop the script should try again in 10 minutes so it can run and start when we get credits again the next session. I expect this loop will actually extend smelt with extra logic and tests around propeties of various sql constructs. I expect you to consider the different axis we have discussed here as well as the full range of sql constructs smelt supports (windows, joins (different types), union) as well as upstream properties - unique keys, append only, cdf etc.

   → DESIGN + PLAN (2026-07-05): `docs/research/20260705-property-discovery-loop.md` (design,
   adversarially reviewed by a Fable subagent) and `docs/plans/20260705-property-discovery-loop.md`
   (phased plan). The loop is a headless `claude -p` research engine (10-minute retry wrapper so it
   self-starts when credits return) whose green gate **executes smelt's own emitted incremental
   maintenance over adversarial run schedules and diffs it against a full refresh** — the direct
   answer to "what properties we can prove and what models hold." It maps the full
   `(SQL-construct × upstream-property × technique)` grid — windows, join types, `UNION`, correlated
   `EXISTS`; append-only / mutable / change-feed / unique-key / clocked / lateness — into a ledger of
   verdicts plus a **negative catalogue** of unsupported combinations annotated with why. Two concrete
   candidate smelt bugs are already seeded (`source_bounds` `(0,0)` fallback on correlated `EXISTS`;
   `input_delta` clocked-`Mutable`→`WindowForward`). Per your note, it is a *research* engine: it
   reuses smelt and puts proof code into smelt's test surface, extending internals only as
   CI-gated, test-only, throwaway on this branch.

---

## 12a. Design principle (promoted from OQ3): offline cost measurement

The `S`-indexed theorem (§4) proves that switching a cell between fold and recompute is
**contract-preserving at a fixed processed-input set** — it changes only *which `S` is reflected*
(freshness), never the skeleton bits. That safety property unlocks a capability real-time query
engines structurally cannot have:

> **Technique selection is a compile-time / offline decision, not a per-query one.** smelt is a
> compiler and orchestrator, so it may run the *alternative* physical plans (fold vs recompute vs
> column re-derivation) over the *same* real data for hours or days, measure actual cost, and pick
> the plan that minimises spend **over a year** — rather than the plan an optimiser can choose in
> milliseconds under a latency budget.

Concretely: (1) sensible per-cell defaults (delta size vs region size, backend merge support) chosen
at plan time, overridable in frontmatter; (2) an offline **plan-bake-off** mode that materialises
each admissible technique for a cell over a representative window and reports measured cost, so the
engineer commits the cheapest whole-workload choice. The run-schedule + cost harness in
`20260705-property-discovery-loop.md` is the substrate this bake-off reuses. This is a first-class
smelt advantage, not a tuning footnote.

## 13. References

- **Specs**: `models.md` (refresh axis, declaration law, litmus rule, peers argument),
  `model_maintenance.md` (equivalence invariant + `S`, algebraic ladder, composition
  contract, scope maps, addressing), `batched_models.md` (batch-safety classes,
  per-partition equivalence, `nondeterministic_columns`, first-run/backfill),
  `keyed_models.md` (admission matrix, ledger, key temporal locality, enrichment
  joins), `model_properties.md` (bound/reach derivation), `model_transforms.md`
  (merge_into, DELETE+INSERT, dimension-horizon MERGE).
- **Research**: `2026-05-20-incremental-gaps-from-web-analytics.md` §3 (lookback
  derivation; the `silver/sessions` 1-day cross-midnight example),
  `20260521-incremental-as-planner-rule.md` (the `(col, before, after)` triple),
  `20260705-keyed-collapse-application.md` (ledger doctrine; keyed collapse),
  `20260705-keyed-time-superset.md` (scope maps §5), `20260703-model-updates.md`
  (input-consumption axis).
- **Real models the example is adapted from**: `examples/web_analytics/models/silver/`
  (`device_user_edges.sql`, `sessions`).
