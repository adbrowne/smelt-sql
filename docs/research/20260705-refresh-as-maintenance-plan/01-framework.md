# Refresh is not a mode — it is a per-column maintenance plan

- **Date**: 2026-07-05 (reformatted as a finished doc 2026-07-06)
- **Status**: research (design exploration; predecessor to a spec change)
- **Author**: Andrew (with Claude)
- **Part**: 1 of the `20260705-refresh-as-maintenance-plan/` research directory — see
  [`README.md`](README.md) for the index. Empirical results live in
  [`02-loop-findings.md`](02-loop-findings.md); the spec-readiness gap list in
  [`09-spec-readiness.md`](09-spec-readiness.md).
- **Related specs**: `models.md`, `model_maintenance.md`, `batched_models.md`, `keyed_models.md`, `model_properties.md`, `model_transforms.md`
- **Related research**: `2026-05-20-incremental-gaps-from-web-analytics.md`, `20260521-incremental-as-planner-rule.md`, `20260705-keyed-collapse-application.md`, `20260705-keyed-time-superset.md`, `20260703-model-updates.md`, `20260705-property-discovery-loop.md` (the empirical engine that settles this paper's mechanical claims)

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
*algebraic ladder* (`model_maintenance.md` §"The algebraic maintenance ladder") —
proposes **one genuinely normative change**: the *strategy content* of the refresh
enum becomes derived, while the *output shape/grain* stays a declared-and-checked
assertion (§10) — and contributes a concrete design for the **generalized
reconciliation ledger** (§8) that the per-cell plan needs. Where the paper conflicts
with a normative spec statement it says so explicitly (§13).

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

We use a canonical medallion shape. **The conversions/attribution framing is this
paper's; the mechanical claims below are the *proposed* framework's, not today's
behavior** (§9 states what today's surface actually does with this SQL). The
underlying bound-derivation and enrichment machinery is drawn from
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
  (This is load-bearing — see §9. If conversions were genuinely mutable/retractable,
  the `converted` column below would be a non-invertible fold over a mutable sequence,
  i.e. the observer-semantics case the theorem in §4 refuses.)

**The model** `silver.event_conversions` — one row per bronze event, enriched with
whether that user converted within a 7-day attribution window after the event:

```sql
---
refresh: batched            -- today's single label; §10 argues it is lossy
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
`false → true`, and it is not invertible (see §4's faithful-fold condition, and why
append-only matters).

**The bounds the framework needs (see §9 for what today derives):**

- `bronze.events → (event_date, before=0, after=0)` for the pass-through columns —
  each event's identity/attributes depend only on its own row.
- `conversions → (conversion_ts, before=0, after=7d)` — an event's `converted` value can
  be changed by a conversion arriving up to 7 days *later*. This reads a bound out of a
  correlated `EXISTS` subquery projection, a shape beyond the source-filter Form-B
  derivation the cited docs describe (§9).

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

Note why a single technique label cannot classify even one trigger: the
"new-bronze-day step" spans two column groups that land in **different** 2×2 corners
(the pass-through group folds/appends; the `converted` group does a bounded
conversions read). Only after factoring by column group does each cell get a
well-defined corner. And the genuinely valuable, currently-inexpressible cell is the
`converted` × late-conversion one — the **fold corner** (delta read, targeted write):
a delta-directed, column-scoped, key-and-window-bounded merge that today's
`refresh: batched` cannot express (§9).

---

## 3. The technique space is a 2×2, not a dichotomy

The honest structure of the technique space is two **independent axes**, not "two
families"; the two familiar techniques are two of the four corners.

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

**The left column has a second occupant: schema-evolution backfill.** The targeted-write
column (both left corners) is also where a **single-field backfill** lands when a model
gains a column (§5's definition-change trigger,
[`07-example-catalogue.md`](07-example-catalogue.md) Family G): it writes only the new
field(s), leaving skeleton and siblings in place. It splits across *both* left corners by
what the field reads — **bottom-left** (full-input) when the field re-derives from upstream
(a column-scoped `MERGE`, keyed where the source is keyed), **top-left** (delta+state with an
*empty* input delta) when the field is a pure function of already-stored columns, so the
"state" read is the stored output region itself and the op is an in-place `UPDATE` with no
upstream read. The dimension-driven MERGE and the schema-evolution backfill are thus the
same corner reached by two different triggers.

**Physical caveat on the write axis (partition-locality — §5).** The left column's
cheapness is not automatic. A *targeted* write is only partition-bounded when the delta's
footprint projects onto a bounded set of the output's partitions; when it does not — a
per-key footprint with no temporal locality — a "targeted" write scatters across every
partition and can cost *more* than a bounded recompute-region. **Partition-locality** (§5)
is the derived property that decides this, and a guardrail declaration ([`04-knobs.md`](04-knobs.md)
§K8) turns it into a fail-loud check so a silent full-table scan is impossible. The 2×2
classifies the *logical* read/write scope; partition-locality is the orthogonal
*physical* question of whether that scope prunes to bounded partitions.

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
needs is a proposed extension of that ledger (§8), not a behavior it grants today.

---

## 4. The interchangeability theorem

Two techniques may serve the same cell interchangeably **iff they produce the same
state at a fixed processed-input set.** The processed-input index `S`
(`model_maintenance.md` §"The equivalence invariant") is load-bearing throughout.

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
exactly `[t₀, tₙ)` and reads enough upstream to get those rows right. (Note the
subtlety this guards against: it is tempting to demand that regions be quotiented by
"footprint closure" so that no delta's footprint straddles a region boundary. For a
*user*-scoped conversion footprint that demand is degenerate — transitive closure over
per-user footprints chains a frequently-converting user across all history — and it is
not needed for value-correctness, which is per-address. The straddle question genuinely
bites only for **region-granular bookkeeping** — the additive ledger of §8, which must
attribute a delta to exactly one region so it is neither double-counted nor lost.
There, and only there, a region is a union of whole footprints.)

**Interchangeability (idempotent columns).** For an idempotent column whose fold is
*faithful* (below), `recompute(R,c,S) = fold(R,c,S)` at every reachable `S`; either
technique may be used, and switching between them is free.

**Faithful fold** is defined precisely: the delta stream is a *partition* of the input
multiset (no overlaps, no retractions), and the combiner's fold over any sub-multiset
equals the batch aggregate over that sub-multiset. `BOOL_OR`/`MIN`/`MAX` over an
**append-only** source are faithful; the same combiners over a *mutable/retractable*
source are **not** (a removed row cannot be un-folded from a non-invertible combiner) —
this is the third admission condition, and it is why §2 makes conversions append-only.

**State-equivalence-modulo-ledger (additive columns).** For a non-idempotent column
(`SUM`/`COUNT`), recompute and fold *converge to the same state*, but the hazard is
**asymmetric**, not "never apply both": fold-then-recompute is safe (the recompute
overwrites the region from ground truth), while recompute-then-refold-the-same-deltas —
or folding any delta twice — double-counts. The ledger's real obligation is therefore
**"never fold a delta already reflected in the state,"** which a region recompute
satisfies by resetting the ledger for the region it overwrote. Interchangeability here
is thus *state-transition equivalence given the ledger*, strictly weaker than the
idempotent case's value-interchangeability — the two strengths must not be equivocated
under a single `≡`.

**Where the two disagree — the admission matrix, re-derived.** The theorem's failure
cases are exactly the existing refusals:

- **Observer semantics** — `MIN(price)` folded over successive *mutable snapshots*:
  `fold` = *min ever observed*, `recompute` = *min in the snapshot at S*. Unequal at
  almost every `S` — this is `KeyedSnapshotSourceUnsupportedColumn` (`keyed_models.md`
  §"Admission matrix"). Note the second condition (replayable input) and the third
  (faithful fold) are **independent**: a replayable change feed that carries
  *retractions*, folded into a non-invertible `MIN`, satisfies replayability but fails
  faithfulness.

**The `S`-index resolves the apparent collision with "validator, not chooser."** A
stored folded value reflects the `S` at which it was last folded; a fresh recompute
reflects the current `S`. When these differ it is because the fold is *stale*, not
because the technique changed the contract. Advancing from a folded `S′ ⊂ S` to a
recomputed `S` is a **freshness advance** (more input processed), never a contract
swap. So the invariant `state = full_refresh over processed S` holds under either
technique; technique choice may only change *which `S` is reflected* — the settle-bound
dimension (§6), surfaced, monotone-good. This is what licenses the cost model (§11) to
choose fold-vs-recompute freely: at a fixed `S` the choice is bit-preserving on
faithful/idempotent columns, and on additive columns it is state-preserving modulo the
ledger. A choice that changed observable bits *at a fixed `S`* would indeed be a
chooser and is forbidden.

**`S` is a per-input vector.** Once the plan factors by `(column-group × input)`, each
cell's processed set ranges over its own source, so the whole-model invariant is the
vectorized `state = full_refresh(each input i restricted to its own Sᵢ)` — well-defined
given clean provenance partitioning (§5), and a refinement of `model_maintenance.md`'s
single-`S` statement, not a contradiction of it. Two consequences: `recompute(R, c, S)`
at an arbitrary *past* `S` is counterfactual for a real source (only current content is
re-derivable), so the replayability condition means **replayable at the current `Sᵢ`**,
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
often mutation-sensitivity partitions the output non-trivially. (This makes the
append-only premise of §2 do double duty: it is what keeps bronze out of `converted`'s
mutation-sensitivity set — under a *mutable* bronze the two groups would merge and the
targeted update would be lost, exactly as the observer-semantics refusal of §4
predicts.)

**A third trigger: definition change (schema evolution).** Creation and mutation are both
*data* triggers — an input delta. A model **gaining one or more output fields** is a third
trigger of a different kind: the *definition* changed while the sources stood still. A
newly-added column-group has, over every already-materialized region, an empty
processed-input vector `S = ∅` (nothing of its inputs is yet reflected in a column that did
not exist), and its **backfill advances `S` from `∅` to current** — a one-time catch-up that
touches only the new group. It is graded by the *same* mutation-sensitivity machinery: the
added group's sensitivity decides its scan and its 2×2 corner (§3's targeted-write column),
exactly as for a dimension-driven update. Fields added together **factor by shared
mutation-sensitivity** just as the base plan does — co-sensitive fields share one backfill
op, cross-group fields get one each. Two consequences are worth stating. First, the backfill
of a newly-added group is **always full-input** (`∅ → current`), even for a column whose
*ongoing* algebra would fold, because there is no prior state of that column to fold onto;
its fold is a separate, later concern. Second, a field added to a **skeleton** position is
not a payload backfill at all but a **grain change** (§10): it changes which rows exist, so
it forces a recompute and must be refused as a column backfill rather than silently patched
in place. The worked inhabitants are Family G of
[`07-example-catalogue.md`](07-example-catalogue.md).

**How often does the plan actually factor?** Settled by decision rather than survey:
the cost gap between a per-cell targeted maintenance and a whole-model recompute is
large enough that users will structure their models so the plan factors (keeping
mixed-mutation projections apart). The payoff is therefore realised by *design
guidance*, not gated on a corpus-frequency measurement; the discovery loop
([`02-loop-findings.md`](02-loop-findings.md)) incidentally reports how often factoring
occurs, but nothing waits on it.

This is the *scope maps* idea (`model_maintenance.md` §"The composition contract")
promoted from a composition-contract footnote to the organizing principle. Two derivable
facts drive it: column **mutation-sensitivity** (column provenance from the SQL, refined
by each source's mutation profile — an immutable-at-creation reference drops out), and
per-input footprint/reach (the `(source_partition_col, before, after)` triple,
`20260521-incremental-as-planner-rule.md` — subject to §9's caveat about correlated
subqueries).

**Note the scan/footprint reflection.** One bound triple encodes two dual maps: the
*scan* bound (input read window per output window) and the *footprint* map (output
write window per input delta), which are reflections of each other. For conversions,
scan `(before=0, after=7d)` reflects to footprint `(before=7d, after=0)`: an event's
run window `[s,e)` reads conversions over `[s, e+7d)`; a conversion at `t` writes events
over `[t−7d, t]`. The numbers look symmetric here only by coincidence; an asymmetric
window would make the reflection visible, so it must be stated, not assumed.

**Partition-local maintenance (the physical realizability of the reflection).** The
scan/footprint reflection is a claim about *logical* windows on the event-time axis.
Whether it is *cheap* to execute is a second, derivable question: does each window
project onto a **bounded set of partitions** of the table it addresses? Define, for a
model partitioned on `P_out` fed by sources `i` partitioned on `P_i`:

> A model's maintenance is **partition-local in source `i`** when, for every trigger
> driven by a change to `i` (and for a bounded backfill), the **scan clamp** projects
> onto a bounded interval of every read source's `P` (no source read in full) **and** the
> **footprint** projects onto a bounded interval of `P_out` (no write touches an unbounded
> set of output partitions). When it holds, all maintenance triggered by `i` runs
> **partition-by-partition** — bounded scans, bounded joins, bounded transactions; no
> full-table scan, whole-table shuffle, or table-spanning commit.

This is **derived, not declared** — the `(partition_col, before, after)` triple projected
onto each source's and the output's partition column. The obligation is threefold:
**derive** it per source; **emit** the partition-pruning predicate into the maintenance
SQL (both the scan *and* the merge/overwrite target must carry the `P` predicate, or the
engine cannot prune — a footprint window stated only on a non-partition column is a
logical bound the storage layer cannot use); and **warn/refuse** when it fails. It fails
exactly at §4's degeneracy — a footprint with no temporal locality (a per-key correlation
chaining across all history, an unclocked mutable dimension) spans unbounded partitions,
so no predicate bounds it and a "targeted" write scatters across the whole table. That
refusal is the honest boundary; the alternative is a silent full-table operation. The
declared guardrail that makes this a fail-loud check (rather than a silent cost) is
[`04-knobs.md`](04-knobs.md) §K8 — an assertion on the derived scan span that never
modifies the clamp, only refuses when it is wider than the operator will tolerate.

**Partition-locality is not clustering-alignment.** A model can be partition-local while
its merge key is *orthogonal* to `P_out`: the conversions example is partition-local in
`conversions` (a late conversion touches a bounded `event_date` span) even though it
merges on `user_id`. The orthogonality costs only *within-partition* write amplification —
under copy-on-write, data files in the touched partitions holding a matched row are
rewritten whole. That cost is **secondary and mitigated**: deletion vectors /
merge-on-read avoid the rewrite, and `OPTIMIZE`/compaction over recent partitions reclaims
fragmentation. The property smelt computes and guarantees is the *partition bound* — that
maintenance stays confined to a bounded partition set — not file-level rewrite
minimization, which is the engine's job.

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

**Two dimensions, not one.** Equivalence *strength* and *settledness* are orthogonal.
`converted` is `S`-indexed **exact** (not a payload relaxation) but **unsettled** — and
its settle bound is **watermark-relative, not a fixed 7 days**: because conversions are
append-only *with unbounded arrival lateness* (§2), an event's `converted` is settled
only once the conversions watermark passes `event_ts + 7d`. Stating an *absolute*
settle time (e.g. "7 days after the event") would require a **declared source-lateness
bound** on conversions, which the example does not carry — so the honest ledger entry
is the watermark condition, and a fixed number is exactly the unlabeled looseness this
section's discipline forbids. The ledger of per-column guarantees is therefore
two-dimensional:

| column | equivalence contract | settle bound |
|---|---|---|
| `event_id` | skeleton, exact | settled immediately |
| `converted` | exact (idempotent monotone fold) | conversions watermark ≥ `event_ts + 7d` (absolute only with a declared conversions-lateness bound) |
| `inserted_at` *(if present)* | payload, plausible | n/a |
| a running-total trajectory *(if admitted, §7)* | as-of-run / prefix-consistency | never (per late data) |

The equivalence-contract column has (at least) three values — `exact`,
`plausible-payload`, and the deliberately-weaker `as-of-run` of §7 — so the split is
**not** binary; §7 depends on the third value existing.

**Determinism rule (settled).** Non-deterministic columns are **barred from every
skeleton position and every correctness-determining expression.** Concretely: a
`unique_key`, a partition/grain column, a `JOIN … ON` / `WHERE` / `GROUP BY` / dedup /
ordering key, and any expression that determines a window lookback/horizon MUST be
deterministic. Non-determinism is admissible only in payload columns.

**Payload-ness leaks across the DAG.** Skeleton is a *role*, and role is
model-relative. A column that is payload in `M` (plausible-only) but consumed in `N`'s
`JOIN … ON` / `WHERE` / `GROUP BY` is **skeleton for `N`**: non-determinism admitted in
`M` now decides which rows exist in `N`. `batched_models.md`'s taint analysis is
intra-model and does not catch this. The rule the invariant needs, and which this paper
adopts as settled: **payload-ness is a column-level property that propagates
downstream, and a payload/non-deterministic column reaching a skeleton position fails
loud at the consumer** — the error either retro-tightens the producer's contract or
forces a stable derivation (a hash of skeleton columns). The surrogate-key edge case
resolves the same way: a `unique_key` derived from a non-deterministic surrogate is
**rejected** unless it is a stable hash-of-skeleton-columns.

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

It is tempting to call this simply a mis-assignment that fold-a-delta repairs. The
truth splits on **what the output grain is**:

- **If the modeller wants the end-state** (one settled value per key), then yes — the
  column is mis-assigned: its algebra wants fold-a-delta into key-addressed state, and
  the honest fix is a **keyed model**. But note this is a **grain change** (`key` vs
  `key × partition`), not a mere cell re-routing — it changes what rows the table
  stores, so it is a different model, consistent with §10's grain anchor.

- **If the trajectory itself is the product** — the running value *at every partition*,
  one stored row per `(key, partition)` — then a late input row at time `t` has an
  **unbounded forward footprint**: it changes the stored value at `p` and *every
  partition ≥ t*. This breaks *A* (targetable output), not just *B* (see §12). Folding
  a late delta into key state repairs the cheap end-state projection but does nothing
  for the stored trajectory, whose every later row is now stale and must be rewritten —
  cost ∝ number of later partitions, i.e. exactly the unbounded write the fold was
  supposed to avoid. Fold moves *where the cost lands* (state vs history re-read); it
  does not make the trajectory's late-data footprint bounded.

So for a genuine trajectory output the forward footprint is **irreducibly unbounded
under late data**, and the honest options are (a) the **as-of-run / prefix-consistency**
contract below — explicitly labeled in the §6 ledger, not silently tolerated — or (b)
bounded-lateness truncation (drop or re-stamp arrivals past a horizon,
`model_maintenance.md` §"Windowed maintenance and the horizon"). PerPartitionOnly is a
*mis-assignment* only in the first bullet's case; when the trajectory is wanted it
is a *legitimate weaker contract that must be named*.

**The third contract: deferred, tracked, not driving the design.** The
`as-of-run` / prefix-consistency contract is accepted *in principle* as a future third
equivalence value — the likely home is a later, deliberately-naïve refresh type
analogous to dbt's microbatch (per-partition-only, correct-as-of-run, no
cross-partition guarantee), which would also give SCD2-over-snapshots a home (§10). It
is **not** built now: the two named corners (§10) plus the skeleton/payload split carry
the present design without it, and the framework is shaped so this can slot in later as
an added, honestly-labelled contract in the §6 ledger rather than a retrofit.

---

## 8. The generalized reconciliation ledger

The per-cell plan needs a reconciliation structure of which the current keyed
window-ledger and the batched no-ledger case are degenerate specializations: a
**`(input × column-group)` → `(output-region × column-group)`** ledger. This section
gives it a concrete design.

**State.** The ledger is a set of *reconciliation entries*, one per `(output-region r,
column-group g)`. Each entry records the **processed-input vector** `S_{i,g}` — for
every input source `i` feeding `g`, the set of that source's deltas already reflected
in the stored state of `(r, g)`. This is §4's vectorized invariant made concrete: the
stored value of `(r, g)` equals `full_refresh` over exactly `⋃_i S_{i,g}`.

**Storage is graded by the algebraic ladder (a real optimisation, not incidental).**

- *Additive / non-idempotent groups* (`SUM`, `COUNT`): the entry must record **which
  deltas** (by delta identity — partition key / change-feed offset) have been folded,
  because the obligation is "never fold a delta already reflected in state" (§4).
  Per-delta bookkeeping.
- *Idempotent groups* (`MIN`/`MAX`/`BOOL_OR`): re-folding is harmless, so only the
  **frontier** `S_{i,g}` (a watermark per input) need be stored — the ladder
  classification licenses dropping per-delta identity. This is where the generalization
  pays for itself.

**Region↔window correspondence.**

- *Under key temporal locality* (`keyed_models.md`): a region `r` is a time window, and
  a delta reconciles against the regions its **footprint** touches, via the
  scan/footprint reflection (§5) — a conversion at `t` writes events over `[t−7d, t]`,
  so it reconciles the ledger regions covering `[t−7d, t]`.
- *Without locality* (a per-user footprint chains across all history, §4): the
  correspondence is **not** a clean interval. The entry then keys on the *output
  addresses* the delta touches (a key-set, not a time interval) and footprint
  membership is tracked explicitly. More expensive; it is exactly the "region-granular
  bookkeeping" §4 isolates.

**Two region notions, to make straddle attribution well-defined (§4).** Separate the
**write region** (what a recompute physically overwrites — arbitrary boundary, can be
fine-grained) from the **ledger region** (drawn on *footprint-closure* boundaries — a
union of whole footprints). A delta is attributed to the **unique** ledger region that
contains its footprint, so it is neither double-counted nor lost. The write region may
be finer than the ledger region; the ledger region exists only for attribution.

**The two operations.**

- *Fold a delta into `(r, g)`*: refuse if the delta is already in the entry's processed
  set (checked against recorded identities for additive groups; a no-op check for
  idempotent groups). Otherwise combine and extend `S_{i,g}`.
- *Recompute write-region `W` for group `g`*: this establishes ground truth over `W`'s
  input, so it **resets** every ledger entry whose ledger-region intersects `W` — after
  the reset, the processed set for those `(r, g)` is exactly the input `S` the
  recompute read. This is what makes fold-then-recompute safe and
  recompute-then-refold a double-count (§4's asymmetric hazard).

**Schema evolution is a ledger operation, not a new mechanism.** Adding a column-group `g`
**instantiates** ledger entries `(r, g)` for every existing region `r`, each at
`S_{i,g} = ∅`; the field-backfill (§3's targeted-write column, §5's definition-change
trigger) is then just *fold/recompute into `(r, g)`* advancing those entries to current `S`,
while every skeleton and sibling `(r, g′)` entry is untouched. Sensitivity-sharing does not
short-circuit this: a field co-sensitive with an **existing** group still instantiates at
`∅` and forms its own catch-up group, merging with its sibling only once its processed-input
vector has caught up over every region — until then a delta that folds into the sibling has
nothing sound to do on the new group's unbackfilled regions, exactly the
never-fold-ahead-of-the-entry refusal above ([`07-example-catalogue.md`](07-example-catalogue.md)
EX-40). The ledger already carries exactly the state schema evolution needs — which is why a
single-field backfill is a first-class plan cell rather than a bespoke migration path.

**Degenerate specializations (why this is "the ledger, generalized" and not a new
thing).**

- The current keyed window-ledger = this structure with a single column-group (the
  whole additive payload), key-addressed regions, under temporal locality.
- The batched no-ledger case = every column-group is recompute-only (region overwrite),
  so the frontier is implicit in the partition watermark and no per-delta bookkeeping
  exists.

**Honest caveat (mirrors §13 point 2).** The region↔window correspondence is exact
only under locality or explicit footprint tracking; calling this "the shipped ledger,
generalized" is a design proposal, not a property of today's ledger. The discovery
loop ([`02-loop-findings.md`](02-loop-findings.md)) is where the additive / idempotent
/ straddle cases get empirically pinned before this is specced; the remaining proof
obligations are enumerated in [`06-proof-obligations.md`](06-proof-obligations.md).

---

## 9. What today's surface actually does with the example

Stated precisely against the shipped code and specs:

- Today's Form-B derivation reads offsets from **source-filter** `WHERE`/`JOIN`
  predicates (`batched_models.md` §"Observing the per-source clamp";
  `20260521-incremental-as-planner-rule.md`). A **correlated `EXISTS` projection
  subquery** over an *unclocked* table is not a shape any cited derivation *describes*
  reading.
- If `conversions` carries no `timeseries:`, it is a **lookup, read in full**
  (`batched_models.md` §"Execution model", step 3) — no bound, no targeted-column
  update; every run re-reads all conversions.
- No targeted per-column maintenance exists on any path: the `converted` ×
  late-conversion fold cell of §2 is inexpressible today.

**Empirical postscript (2026-07-06).** The property-discovery loop ran this exact
shape through the real planner and settled the guesses this section previously had to
hedge: the Form-B extractor **does** derive the `conversions → (0, 7d)` bound from the
correlated `EXISTS` — initially by a column-blind whole-text scan that also leaked the
same bound to `events` (fixed, production, as `FIX-1`: bound attribution is now
column-aware), and the model executes rather than being refused; but every batched
cell is served by unconditional recompute-region (DELETE+INSERT of the requested
window), so the targeted fold cell remains inexpressible, exactly as argued. See
[`02-loop-findings.md`](02-loop-findings.md) (cells `SC-1`, `SC-1b`, `FIX-1`,
`G-01`–`G-11`) for the full findings.

The lesson stands: the example is a *specification target* for the proposed framework,
with today's behavior now empirically mapped rather than guessed.

---

## 10. What stays singular — and what is declared vs derived

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

**Schema evolution respects the anchor.** Adding a *payload* field is a within-grain change
— a column backfill (§3's targeted-write column, §5's definition-change trigger). Adding a
field to a **skeleton** position (a `unique_key` / grain / grouping / dedup / ordering role)
changes *what a row is*, so it is a **grain change**, not a field-add: the stored rows become
the wrong rows and no targeted write repairs them. The framework must refuse it as a column
backfill and name the grain change — the honest plan is a recompute, effectively a new model
— never silently patch a skeleton column in place.
[`07-example-catalogue.md`](07-example-catalogue.md) EX-39 is the worked boundary.

**Output shape (partitioned vs keyed) — declared-and-checked, *not* silently derived.**
Deriving output shape from the plan was considered and rejected: it reintroduces
exactly the silent-contract-swap the declaration law exists to prevent (`models.md`
§"Refresh modes are peers", §"The declaration law"). Shape governs downstream
consumption (windowed source vs read-in-full lookup), the write primitive, and the
identity requirement, and deriving it means a refactor of a projection could flip it
with *no diagnostic* — the trigger merely moved from a YAML knob to the SQL, which is
worse for review. The "checked assertion ≠ selector" defense covers *bounds* (there
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
  `recompute ≡ fold` at the skeleton level; that inhabitant belongs to §7's third
  contract, not to the two named corners).

The user-facing declaration surface this implies — what is a knob, what is a checked
assertion, what is derived-and-reported — is worked out in
[`04-knobs.md`](04-knobs.md); the source-declaration surface it depends on in
[`05-source-properties.md`](05-source-properties.md).

---

## 11. Design principle: offline cost measurement

The `S`-indexed theorem (§4) proves that switching a cell between fold and recompute is
**contract-preserving at a fixed processed-input set** — it changes only *which `S` is
reflected* (freshness), never the skeleton bits. That safety property unlocks a
capability real-time query engines structurally cannot have:

> **Technique selection is a compile-time / offline decision, not a per-query one.**
> smelt is a compiler and orchestrator, so it may run the *alternative* physical plans
> (fold vs recompute vs column re-derivation) over the *same* real data for hours or
> days, measure actual cost, and pick the plan that minimises spend **over a year** —
> rather than the plan an optimiser can choose in milliseconds under a latency budget.

Concretely: (1) sensible per-cell defaults (delta size vs region size, backend merge
support) chosen at plan time, overridable in frontmatter (see
[`04-knobs.md`](04-knobs.md)); (2) an offline **plan-bake-off** mode that materialises
each admissible technique for a cell over a representative window and reports measured
cost, so the engineer commits the cheapest whole-workload choice. The run-schedule +
cost harness in `20260705-property-discovery-loop.md` is the substrate this bake-off
reuses. This is a first-class smelt advantage, not a tuning footnote.

---

## 12. Appendix: the A/B property space

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
  footprint (¬A), which the ladder does *not* repair (§7). "Cumulative keeps A" is true
  only for the keyed end-state grain, not the partitioned trajectory that
  `PerPartitionOnly` actually materializes.

Full refresh reads all input and writes all output; you beat it by bounding the read
(B, or the ladder's state) or the write (A), decisively with both. This is the theory
beneath §3's axes: the read axis is B(-or-state); the write axis is A.

---

## 13. Relationship to the current design

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
   column-group)` reconciliation structure** (§8), of which the current keyed
   window-ledger and the batched no-ledger case are degenerate specializations. The
   region↔window correspondence is exact only under key temporal locality or explicit
   footprint tracking; presenting it as "the ledger, generalized" is a design proposal,
   not a property of the shipped ledger.
3. The skeleton/payload split with **DAG propagation** of payload-ness (§6) extends the
   intra-model `nondeterministic_columns` taint to a cross-model property with a
   consumer-side check.

What it explicitly does **not** change: output shape/grain stays a declared,
checked-against-the-plan assertion (§10) — the anti-footgun property of the declaration
law is preserved.

Recommended resolutions for the design forks the empirical loop surfaced (outer-clamp
qualification, composite unique keys, dormant-classifier wiring) are in
[`03-design-forks.md`](03-design-forks.md); the crate-placement proposal for the whole
framework is in [`08-code-placement.md`](08-code-placement.md).

### Open-question ledger (historical labels)

The original single-file paper tracked five open questions, all since resolved and folded
into the body. Sibling documents (and the property-discovery loop's artifacts) still cite
them by label:

| Label | Resolution | Now lives in |
|---|---|---|
| **OQ1** | Non-deterministic/payload columns barred from every skeleton position and correctness-determining expression; consumer-side fail-loud with DAG propagation | §6 |
| **OQ2** | The `as-of-run` / prefix-consistency third contract: accepted in principle, deferred, tracked, not driving the design | §7 (closing note) |
| **OQ3** | Technique selection: sensible defaults + frontmatter override + offline bake-off measurement | §11 |
| **OQ4** | The generalized reconciliation ledger design | §8 |
| **OQ5** | Plan-factoring frequency: no corpus survey needed — the cost gap means users design around it; realized as design guidance | §5 (closing note) |

---

## 14. References

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
  (input-consumption axis), `20260705-property-discovery-loop.md` (the empirical
  engine; results synthesized in [`02-loop-findings.md`](02-loop-findings.md)).
- **Sibling documents in this directory**: [`README.md`](README.md) (index),
  [`02-loop-findings.md`](02-loop-findings.md), [`03-design-forks.md`](03-design-forks.md),
  [`04-knobs.md`](04-knobs.md), [`05-source-properties.md`](05-source-properties.md),
  [`06-proof-obligations.md`](06-proof-obligations.md),
  [`07-example-catalogue.md`](07-example-catalogue.md),
  [`08-code-placement.md`](08-code-placement.md),
  [`09-spec-readiness.md`](09-spec-readiness.md).
- **Real models the example is adapted from**: `examples/web_analytics/models/silver/`
  (`device_user_edges.sql`, `sessions`).
