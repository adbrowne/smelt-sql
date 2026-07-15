# Conditional maintenance without a change feed

**Date:** 2026-07-15
**Status:** research — predecessor to a possible spec diff against `maintenance_plan.md`,
`model_properties.md`, `model_transforms.md`, `sources.md`
**Question:** when an upstream input changes and there is **no** change data feed (CDF), can a
maintained model avoid recomputing and rewriting output that did not actually change — and, for
enrichment models whose expensive joins contribute only payload columns, avoid running those
joins at all except for the rows that are new or changed?

---

## 1. Problem statement

Take the shipped fixture `examples/timeseries/models/daily_events_enriched.sql`: a clocked fact
(`raw.events`) enriched by an unclocked `mutation_profile: mutable_snapshot` dimension
(`raw.users`), joined 1:1 on `user_id`, contributing exactly one payload column (`user_name`).
This is the fact+dimension enrichment shape the plan already derives an `UpstreamMutation` cell
for, served by the live column-scoped `MERGE`.

Today's behaviour, per `maintenance_plan.md` §Known Divergences:

> nothing yet distinguishes "an upstream mutation genuinely happened since the last run" from
> "this run happens to re-derive the same values" — the dispatch fires on every run
> unconditionally once its preconditions hold

So on every run, for the accepted-full-scan corner, smelt: (a) **recomputes** the enrichment join
over the whole batch window, (b) **rewrites** every matched row via `MERGE … UPDATE SET *`, even
when zero users were renamed, and (c) at the graph layer an unclocked dimension's delta "dirties
the whole model for every mutation-sensitive consumer" (`maintenance_plan.md` §"The graph
layer") — the coarsest possible downstream signal.

The cost of a maintenance run over a region decomposes into three parts, and change-awareness can
attack each independently:

- **C1 — candidate compute.** Evaluating the model's SQL (the expensive joins) over the scan
  window to produce the candidate rows.
- **C2 — write.** Physically writing the write window: `DELETE`+`INSERT` of the region, or
  `MERGE` updates of every matched row. On copy-on-write storage (Parquet/Delta/Iceberg) an
  unconditional region write rewrites every touched file even when bytes are identical, and
  pollutes any downstream CDC with no-op updates.
- **C3 — downstream dirt.** What the run's consumers are told changed. Today an unclocked
  mutation propagates as whole-model dirt; a region recompute propagates the whole reflected
  window, whether or not any output row differs.

The models where this hurts most are exactly the ones the question names: **enrichment models**
whose join is expensive (a large dimension, a multi-way lookup cascade) but whose *skeleton* —
which output rows exist, keyed how — is determined by the driving fact alone. For such a model,
re-running the joins over an entire horizon because *one* dimension row changed (or because
nothing changed but the dispatch can't tell) is almost pure waste.

The second motivating shape is the **model-edge chain** in `examples/web_analytics`:
`silver.events_enriched` and `gold.eventstream_with_identity` rejoin every event row to
`silver.sessions` (and, for the gold model, to three identity models) on every touched
partition. Sessionisation output is highly stable — a late-arriving or redelivered event
re-touches a partition of `events_parsed`, the sessions model recomputes that region and
reproduces **mostly identical session rows**, and today the downstream models re-join *every*
event in the reflected partitions to sessions anyway, because the propagation edge carries "this
window was written", not "these rows changed". When sessions hasn't changed, none of that
re-joining was necessary. This case matters doubly because the changed upstream is a
*maintained model*, not an external source — which, as §2/M3 shows, makes its delta available
for free.

## 2. The idea, factored into three composable mechanisms

The proposal decomposes into three mechanisms that compose but are separately adoptable. Naming
them separately matters because they have different licensing obligations and different
cost/benefit shapes.

### M1 — Change-suppressed writes (the conditional merge itself)

Compute the candidate output for the cell's region exactly as today, but make every write
conditional on the row actually differing from stored state:

- **Merge-capable backends:** add a change predicate to the matched arm —
  `WHEN MATCHED AND (t.c1 IS DISTINCT FROM s.c1 OR t.c2 IS DISTINCT FROM s.c2 …) THEN UPDATE`.
  Unmatched candidate rows insert as before; stored region rows absent from the candidate are
  deleted (`WHEN NOT MATCHED BY SOURCE` scoped to the region, or a separate scoped `DELETE`).
- **Merge-less backends** (the point that this is *not* a MERGE feature): stage the candidate
  once, compute the changed/new/departed row sets by diff joins against the stored region, then
  `DELETE` only `changed ∪ departed` and `INSERT` only `changed ∪ new` — a *conditional*
  `DELETE`+`INSERT` that is byte-equivalent to today's region overwrite but touches only rows
  that differ.

M1 attacks **C2** directly and produces, as a byproduct, the exact set of rows the run changed —
which is what makes M3's output half possible. It does **not** reduce C1: the candidate is still
fully computed.

### M2 — Delta-restricted enrichment compute (skeleton/enrichment factoring)

When the model's row skeleton is provably owned by the driving source alone — the enrichment
join is 1:1, row-preserving, and contributes only payload columns — the model factors as
`skeleton(driving) ⟕ enrichment(dimensions)`. Then the expensive joins need only run over the
**skeleton delta**: rows that are new (from the landed window) or whose enrichment inputs
changed (from a dimension delta). Everything else in the region is provably bit-identical to
stored state and need not be recomputed at all.

This is the standard delta-rule algebra of incremental view maintenance. For `E ⋈ U` with an
unchanged fact `E` and dimension delta `ΔU`, the affected output is `E ⋉ ΔU ⋈ U'` — join only
the fact rows whose key appears in the changed-dimension set. The general rule
`Δ(R ⋈ S) = ΔR ⋈ S ∪ R ⋈ ΔS ∪ ΔR ⋈ ΔS` needs both deltas; the payload-only 1:1 restriction is
what collapses it to something cheap and provably safe.

M2 attacks **C1** — the actual join compute — which is where the big win the question describes
lives. Its obligation is a *delta for the changed input*, which a CDF would supply; without one,
M3 supplies it.

### M3 — Derived change feeds (snapshot-diff made real, on both boundaries)

The spec corpus already names the missing piece without building it. `models.md`'s
input-consumption axis has exactly three values — window-forward / **snapshot-diff** /
change-feed — and defines snapshot-diff as "re-scan the source whole and compare against stored
state"; `sources.md`'s licensing table lists "snapshot-diff consumption" as what a
`mutable_snapshot` source licenses; and `sources.md` §Known Divergences confirms "snapshot
diffing [is] not yet built". M3 is that construction, applied at **two boundaries**:

- **Input boundary (fingerprint sidecar).** For a `mutable_snapshot` input, maintain a compact
  warehouse-resident relation `(key, content_digest)` — a fingerprint of the columns the
  consuming cell actually reads. Each run re-scans the (cheap, narrow) dimension projection,
  diffs digests, and derives the exact changed/new/departed key set: a **synthesized change
  feed** for a source that offers none. This is the classical snapshot-differential technique
  (Labio & Garcia-Molina; Data Vault's HASHDIFF — §9), and it is precisely a poor-man's CDF
  built from state smelt already controls.
- **Output boundary (observed output delta).** The model's own stored output *is* a replayable
  snapshot of its prior evaluation. M1's suppressed write computes, for free, the exact output
  delta — which rows this run actually changed. Recording that delta turns every maintained
  model into a `change_feed`-postured upstream for its own consumers, replacing "whole reflected
  window" dirt with exact dirt. A `mutable_snapshot` source's landed delta — today defined as
  "the whole table" (`sources.md` §"Landed-delta intervals") — refines to a key- or
  partition-level delta derived from what actually changed.

So M2's required delta has **two suppliers, by input class**: an upstream *maintained model*
gets it for free as the byproduct of that model's own conditional write (the web-analytics
sessions chain — no sidecar, no extra scan, exact by construction); an *external* mutable
snapshot needs the fingerprint sidecar. This is another instance of the scope-map idea — the
per-input dispatch already distinguishes what runs per input class; the delta's provenance
follows the same per-input split. It also means the mechanism compounds along a DAG: once
`sessions` writes conditionally, every consumer of sessions inherits an exact delta, and *their*
conditional writes hand exact deltas further downstream.

M3 attacks **C3** and feeds M2. The unifying observation: *the input-consumption axis's
snapshot-diff value, applied uniformly, at the input boundary derives the delta a CDF would have
given us, and at the output boundary derives the delta we hand downstream.* Nothing about either
requires backend CDF support; both require only state smelt already owns or can cheaply keep.

### Why the engine cannot do this for us

`maintenance_plan.md` §Design commits to "widened scan + exact clamp … leaving join optimisation
to the engine rather than smelt hand-computing a minimal delta". That stance is right for
*within-one-statement* optimisation and this proposal does not disturb it. But the optimisation
here is **cross-run**: it exploits the fact that the stored output equals the model's prior
evaluation over a known processed-input set `S`. A per-query optimiser sees one statement at a
time; it has no warrant to assume the target table's current contents relate to the source
expression at all. Only the maintenance layer holds that invariant — the equivalence invariant
is exactly the licence. This is the same structural argument the spec already makes for offline
cost measurement ("a capability per-query optimisers structurally lack", §Design): the ledgered
knowledge of *what the stored state is* is smelt's asset, and conditional maintenance is the
second consumer of it.

## 3. Worked example

The `user_name` cell of `daily_events_enriched` under a `raw.users` mutation trigger.

**Today** (accepted-full-scan corner, simplified from the emitters):

```sql
MERGE INTO daily_events_enriched t
USING (
  SELECT e.event_id, date_trunc('day', e.event_timestamp) AS event_date,
         e.user_id, e.event_type, u.user_name
  FROM raw.events e JOIN raw.users u ON e.user_id = u.user_id
  WHERE e.event_timestamp >= {batch_start} AND e.event_timestamp < {batch_end}
) s
ON t.event_id = s.event_id
WHEN MATCHED THEN UPDATE SET user_name = s.user_name
WHEN NOT MATCHED THEN INSERT *
```

Every matched row is written every run; the join runs over the whole window.

**With M1** (change-suppressed write — one predicate added, everything else identical):

```sql
WHEN MATCHED AND (t.user_name IS DISTINCT FROM s.user_name) THEN UPDATE SET user_name = s.user_name
```

Zero renamed users ⇒ zero rows written. The join still runs (C1 unchanged), but C2 collapses to
the true change volume, and the write set — recorded — is an exact downstream delta (M3-output).

**With M3-input + M2** (fingerprint sidecar + delta-restricted compute):

```sql
-- sidecar maintenance (cheap: narrow projection of the dimension)
-- _smelt_fp_raw_users(user_id PRIMARY KEY, digest)
-- Δusers = keys whose digest differs, is new, or departed, computed by a full outer
-- join of the fresh projection against the sidecar; sidecar upserted in the same txn.

MERGE INTO daily_events_enriched t
USING (
  SELECT e.event_id, …, u.user_name
  FROM raw.events e
  JOIN raw.users u ON e.user_id = u.user_id
  WHERE e.user_id IN (SELECT user_id FROM Δusers)          -- ← the delta restriction
    AND e.event_timestamp >= {horizon_start} AND e.event_timestamp < {batch_end}
) s
ON t.event_id = s.event_id
WHEN MATCHED AND (t.user_name IS DISTINCT FROM s.user_name) THEN UPDATE …
WHEN NOT MATCHED THEN INSERT *
```

The expensive part — the fact-side scan feeding the join — now touches only fact rows whose
dimension key changed. If 10 of 10 million users were renamed, the enrichment recompute is
~10⁻⁶ of today's. The sidecar diff costs one narrow dimension scan per run — which the
snapshot-reconcile posture already pays today just to *consume* the dimension, so the marginal
cost is the digest computation and the sidecar upsert.

Note what did **not** change: the scan/write windows, the clamps, the horizon derivation, the
ledger semantics. The delta restriction is a *further* narrowing inside an already-derived
bound, never a substitute for one.

**The model-edge variant** (web analytics): a redelivered event re-touches `events_parsed`
partition `[D)`; the `sessions` run over the reflected window writes conditionally and observes,
say, `Δsessions = {2 session keys changed}`. `eventstream_with_identity`'s creation cell for
the sessions edge then joins only events whose `session_id ∈ Δsessions` (plus the genuinely new
events from its own driving edge) instead of re-joining every event in the reflected
partitions — and its own conditional write hands an equally exact delta to the marts layer. No
sidecar exists anywhere in this chain; every delta is the recorded byproduct of the upstream's
own write. When nothing changed, the whole downstream cascade degenerates to a chain of
empty-delta no-ops.

## 4. Properties required

Following `model_properties.md`'s placement criterion (verdicts stateable without naming a
refresh mode), the machinery decomposes into rows the catalogue mostly already has, plus a small
number of genuinely new proofs.

### Existing proofs consumed as-is

| Property (existing row) | Role here |
|---|---|
| Skeleton-role extraction | identifies the identity/grouping/dedup/ordering columns whose provenance decides skeleton ownership |
| Per-column mutation-sensitivity / column provenance | scopes the change predicate: under trigger `t`, only the sensitive group's columns can differ, so only they are compared |
| Fan-out / cardinality (`OneToOne`) | the enrichment join must not multiply rows — already proven from the declared unique key vs the `ON` equality |
| Join-contribution monotonicity | already licenses the dimension-driven horizon MERGE this composes with |
| Determinism (run vs row) predicate | feeds the new change-comparability verdict (P3) |
| Whole-model property vector (grain key, FDs) | supplies a proven-unique region row identity where no `unique_key` is declared (P2) |
| Input-delta discovery | snapshot-diff is one of its three values; M3 is its implementation, not a new axis |

### New proofs needed

**P1 — Skeleton-source closure (the "payload-only join" verdict).**
`SkeletonClosure = Closed{driving} | Open{column, input, reason}`: every skeleton-role column
derives solely from the driving source, and no non-driving input can change **row membership**.
Composed from skeleton-role extraction + column provenance + `OneToOne` cardinality, plus two
sub-obligations that are new:

- *Row preservation.* An `INNER JOIN` to a dimension filters the skeleton when a key is absent —
  membership becomes dimension-sensitive and the closure fails. A `LEFT OUTER` 1:1 join is
  row-preserving by construction (provable). An inner join is admissible only under a **declared
  referential-integrity world-fact on the source** (every fact key resolves), which per
  `sources.md`'s trust rule is a narrowing declaration and must ship **paired with a
  verification mechanism** — a per-run count-preservation tripwire (candidate row count equals
  driving-side row count over the region) is cheap and exact. Notably, `daily_events_enriched`
  as shipped uses a bare `JOIN`, so the fixture itself would need the declaration or a `LEFT
  JOIN` to earn the closure — a good sign the proof is discriminating, not rubber-stamping.
- *No membership predicates on enrichment columns.* A `WHERE u.status = 'active'` makes
  membership dimension-sensitive; the closure fails (M1 remains available; M2 does not).

Fail-closed like every proof: any construct the walk cannot classify yields `Open`, and the cell
keeps today's unconditional techniques.

**P2 — Region row identity.**
`RowIdentity = Key{cols} | WholeRow | None`. Conditional writes must *address* the rows they
update, including under the partition grain where no row identity is otherwise required. The
verdict is: a declared `unique_key` (as `daily_events_enriched` declares `event_id`), else a
proven grain key (a `GROUP BY` output is unique per group by construction — the property
vector's grain fact), else `WholeRow` — the identity-free fallback where the diff is a
**multiset** diff (`EXCEPT ALL` in both directions) and the conditional write degenerates to
delete-the-disappeared / insert-the-appeared with counts. `WholeRow` still suppresses writes for
unchanged rows; it just cannot express an "update", only delete+insert pairs. `None` (a shape
the walk cannot normalise) refuses.

**P3 — Change comparability per column.**
`ChangeComparable = Comparable | Incomparable{reason}` per output column: the column's value is
a pure function of the processed inputs — re-evaluating at fixed `S` reproduces the bits. Both
row-nondeterministic functions (`RANDOM`, `UUID`) **and** run-deterministic pinnables
(`NOW()` pinned per run — pinned *differently* per run) are `Incomparable`: comparing them
produces spurious diffs that would rewrite the whole region every run and, worse, pollute the
observed output delta. This is a lattice fold the composition walk already carries for the
determinism taint; the new verdict is per-column rather than per-model. Columns under
`columns.<c>.contract: plausible` are definitionally `Incomparable` (§6 discusses the write
policy for them).

**P4 — Fingerprint-projection derivation** (for M3-input).
For a `(cell × source)` pair, the narrow projection of the source that the cell actually reads
(the columns feeding the sensitive group, plus the join key) — this is what the sidecar digests.
Derivable from column provenance; listed separately because the sidecar's soundness depends on
the projection being *complete* (missing a read column ⇒ a change that matters goes undetected —
an equivalence violation, not a performance bug). Fail-closed: an unprojectable shape (e.g. the
source consumed through `SELECT *` into an opaque construct) digests the full row.

### What is deliberately **not** a property

The change *predicate* itself needs no proof of soundness when it is the exact form —
`IS DISTINCT FROM` over the compared columns is definitionally sound (§6). A **digest**-based
predicate (compare stored hash vs candidate hash) is a performance refinement carrying collision
risk; per the fail-closed culture it should be the opt-in, not the default (§6, §10).

## 5. Transforms required

Following `model_transforms.md`'s catalogue discipline (mechanism named for what it does; a
property licenses, never chooses):

**T1 — Change-suppressed MERGE** (variant of keyed `merge_into` and of the column-scoped MERGE).
The existing emitters (`emit_keyed_fold`, `emit_column_scoped_merge`) gain a change-predicate
clause on the matched arm: `AND (t.c IS DISTINCT FROM s.c OR …)` over the cell's comparable
columns. Licensed by P2 (`Key`) + P3 on the compared set. Dialect-keyed like today's variants;
`WHEN NOT MATCHED BY SOURCE` (region-scoped) where the dialect has it, else a separate scoped
`DELETE` in the same statement group. This is a small, contained emitter change — the single
cheapest piece of the whole proposal.

**T2 — Staged-candidate conditional DELETE+INSERT** (variant of partition DELETE+INSERT; the
merge-less lowering). One transaction: (1) stage the candidate region once (temp relation);
(2) derive `changed`, `new`, `departed` sets by diff joins (keyed) or `EXCEPT ALL` both ways
(`WholeRow`); (3) `DELETE` region rows in `changed ∪ departed`; (4) `INSERT` `changed ∪ new`.
Byte-equivalent to today's region overwrite at fixed `S`. Requires one genuinely new emitter
capability: **statement groups with a staged temporary relation** (today's groups are
DELETE+INSERT pairs over the target only). This answers the "even if the backend didn't support
merge" clause — and fills a real documented gap: Spark-over-Parquet has `supports_merge = ✗`
and today no keyed lowering path at all (`multi_backend.md` names only the partition-range
DELETE+INSERT emulation).

**T3 — Delta-restricted enrichment recompute** (variant of the widened-scan candidate build).
Rewrites the cell's candidate SQL to semi-join the driving side against the changed-key set
(`WHERE e.user_id IN (SELECT key FROM Δ)`), inside the existing derived clamps. Licensed by
P1 (`Closed`) + an **exact** input delta (M3-input or a real CDF). This is the transform that
cuts join compute; it is also the one that most needs the licensing discipline, because a wrong
delta here silently under-maintains (§6).

**T4 — Snapshot-diff delta derivation** (the fingerprint sidecar; implements the
named-but-unbuilt "snapshot-diff consumption"). A warehouse-resident
`_smelt_fp_<source>(key, digest)` relation per consuming project; per run: scan the P4
projection, full-outer-join against the sidecar, emit `Δ = {changed, new, departed}` keys,
upsert the sidecar **in the same transaction** as the consuming write (the same
transactional-with-the-write argument that put the additive ledger in the warehouse rather than
the JSON store). Precedent for the primitive already exists in `sources.md`: the frontier
checksum ("a sampled per-partition fingerprint over skeleton columns") — same mechanism,
promoted from tripwire to delta source.

**T5 — Observed output delta recording** (M3-output). The conditional write's changed-row set
(keys, or their partition projection) is recorded as the model's landed delta, replacing the
whole-reflected-window entry. Consumers: the forward-propagation graph (exact per-edge dirt) and
`smelt run --since-upstream` (a model's landed delta becomes precise for free). Partition-grain
consumers can take the partition projection of the key set (widen to whole partitions —
widen-never-narrow is preserved); key-grain propagation ties into the designed-but-unbuilt keyed
dirt-sets (P7/P8), so v1 can record key-level and *propagate* partition-level.

Every one of these stays inside the statement-emission single-owner rule: new emitters or
emitter variants in `smelt-logical`'s maintenance layer, executed-never-authored by backends,
printable via `smelt explain --show-sql`, and gated by the same statement-parity and
conformance-oracle machinery.

## 6. Correctness

### The equivalence invariant is preserved bit-for-bit

At a fixed processed-input set `S`, let `C` be the candidate relation for the region and `T` the
stored region. Unconditional maintenance writes `T ← C`. Conditional maintenance writes only
rows of `C △ T` (symmetric difference under the row identity). The post-states are identical
**iff** every skipped row was byte-equal — which the exact predicate (`IS DISTINCT FROM` per
compared column) guarantees definitionally, given:

- **P3 (comparability)** — the skipped row's stored bits are what re-evaluation would produce.
  Without it, "unchanged" is not well-defined (a `NOW()` column differs every run; suppressing
  its write yields state neither today's technique nor a full refresh would produce).
- **Completeness of the compared set** — every column that *can* differ under this trigger is
  compared. This is exactly what mutation-sensitivity grouping asserts; comparing only the
  sensitive group is sound *because* the other groups are proven insensitive to this trigger.

Under the interchangeability clause (`maintenance_plan.md` §"Per-cell admission"), the
conditional variant and its unconditional parent are **proven interchangeable in the strongest
sense** — identical state at fixed `S`, not merely modulo the ledger — so per-cell choice
between them is squarely a cost-model/`prefer`/`technique` matter, inside validator-not-chooser.
The ledger is untouched: recompute-reset records *the input the recompute read*, which is
identical whether or not unchanged rows were physically rewritten.

### "Only proofs prune" — and why a suppressed write is not a clamp

`keyed_models.md` rejects write-eligibility clamps because they **silently drop scanned
inputs**. A suppressed write drops nothing: the scanned input's effect on the output is
computed, and that effect is *provably the identity* on the skipped row — proven not statically
but by evaluation, row by row, the strongest proof available. The right taxonomy entry is the
write-side dual of the already-blessed target-scan slice pruning: **no-op write elimination**.
The spec's own principle covers it verbatim — only proofs prune — the proof here being the
per-row equality the predicate just evaluated. A spec diff should name this third category
explicitly (beside slice pruning and the forbidden eligibility clamps) so the boundary stays
sharp: suppression may only ever skip a write whose applied effect is the identity; it may never
skip *evaluating* an input (that is M2's job, licensed separately and statically).

M2's pruning, by contrast, **is** static: it skips evaluation of fact rows whose dimension key
is not in `Δ`. Its licence is P1 + delta exactness: for a `Closed` skeleton, a fact row with an
unchanged enrichment input provably maps to a bit-identical output row (its skeleton columns
read only the — unchanged — driving row at creation; its payload reads only unchanged dimension
content). The load-bearing obligation is that `Δ` is **exact by construction**: T4's sidecar
diff compares full content digests of the complete P4 projection, so a changed-but-undetected
key is impossible up to digest collision (below). This is materially stronger than trusting a
declared bound — it is the same epistemic standing as a CDF, which is also a runtime artifact,
not a static proof. The precedent for runtime-checked (rather than statically-proven) narrowing
already exists in the admitted key-recurrence route ("always runtime-checked, never trusted").

### Digest collisions

The exact predicate has no collision problem; only digest-based comparison (T4's sidecar, or an
optional stored per-row digest to avoid re-reading wide stored rows) does. Options, in the
repo's own idiom: (a) follow `output_fingerprint.md`'s stance — SHA-256-class digests treated as
sound with the soundness invariant stated explicitly (`digest(a) = digest(b) ⇒ a = b`) and a
DuckDB-oracle property-test gate behind it; (b) treat any digest narrower than that (e.g. a
64-bit hash for cheapness) as a **named acceptance** in the `allow_full_scan` mould — declared,
surfaced in `smelt explain`, never a default. Recommendation: (a) for sidecars, exact
`IS DISTINCT FROM` for write suppression (where both sides are already in hand and hashing buys
little).

### `contract: plausible` and other incomparable columns

A `plausible` column admits run-to-run variance, so equality on it is meaningless in both
directions. Three policies for a group containing one:

1. **Refuse** the conditional technique for that cell (fail-closed default — consistent with the
   culture, costs nothing relative to today).
2. **Compare the comparable, always rewrite the incomparable when the row is otherwise
   written** — sound, but a row whose comparable columns are all unchanged is skipped entirely,
   leaving the plausible column's *old* value in place. That is admissible precisely when the
   contract says any plausible value is acceptable — and it has a pleasant side effect:
   suppression *stabilises* plausible columns (fewer spurious downstream deltas). But "old value
   under new inputs" may fall outside what the modeller means by plausible (e.g. a sampled-value
   column whose population changed).
3. **Exclude from predicate, always write the row** — loses most of the benefit.

Recommendation: policy 1 as the shipped default, policy 2 behind the per-column contract once
the contract's semantics are sharpened to say whether "stale but previously-correct" is
plausible (open question §10). The observed output delta (T5) must in any case be computed over
**comparable columns only** — a plausible column's flutter must never dirty downstream.

### Aggregates and other non-enrichment shapes

M1 is shape-agnostic: it compares candidate rows to stored rows and cares only that both sides
are well-defined (P2, P3) — an aggregating region recompute suppresses identically. M2's
factoring, however, must not reorder a join past an aggregation unsoundly; v1 should restrict
`SkeletonClosure` to non-aggregating enrichment scopes (the body-structure classifier already
distinguishes these) and treat join-below-aggregation as `Open`, admitting it later only with an
FD-based reordering proof. `INTERSECT`/`EXCEPT` compositions already collapse to whole-model
sensitivity and stay outside all of this, unchanged.

### Schema evolution and sidecar invalidation

A definition change or schema evolution invalidates stored digests (the P4 projection changed).
Sidecar entries must carry the projection's fingerprint; on mismatch the diff degrades to
"everything changed" — widen-never-narrow, and the definition-change trigger's own machinery
(backfill of the new group) proceeds as today.

### A free win worth naming: redelivery

An `at_least_once` source's redelivered, byte-identical rows currently produce real writes
(idempotent-family cells re-scan and rewrite). Under M1 a redelivery storm becomes a zero-write
no-op with an empty observed delta — turning redelivery from a write-amplification hazard into
noise the pipeline absorbs silently, with no ledger interaction at all (idempotent grades never
needed dedup identities anyway).

## 7. What the current specs would need to change

Collected tensions, each with the amendment it implies:

1. **`maintenance_plan.md` §Constraints, "content-aware delta pruning (an engine/CDF concern)"
   is out of scope.** The exclusion conflates two things. File-level pruning and CDF *transport*
   stay the engine's. But content-aware **write suppression** and **derived deltas** are
   maintenance-layer concerns by the spec's own logic — they consume the equivalence invariant
   itself as their licence. The item should be re-scoped to name what stays out (file layout,
   engine-native CDC transport) and what comes in (proof-licensed suppression + snapshot-diff
   delta derivation).
2. **§Design "widened scan + exact clamp … rather than smelt hand-computing a minimal delta."**
   Amend the rationale, not the mechanism: the widened scan remains the baseline; M2 is an
   additional, licensed narrowing available only under P1 + an exact delta — justified because
   the engine structurally cannot exploit cross-run state (§2).
3. **`keyed_models.md`'s pruning taxonomy** gains the third category: no-op write elimination
   (write-side dual of slice pruning; never skips evaluating an input).
4. **`sources.md` landed-delta representation**: `mutable_snapshot`'s "delta = the whole table"
   refines to the sidecar-derived key set (projectable to partitions); a maintained model's own
   landed delta refines from "the window a run wrote" to "the rows a run changed" (T5).
5. **`output_fingerprint.md`'s "ephemeral, never persisted" principle** does not transfer: the
   row-content fingerprint is a *different artifact class* that exists only cross-run. It needs
   its own spec (naming, storage home, transactionality, invalidation) and a disambiguation row
   alongside the existing two fingerprint concepts.
6. **`multi_backend.md`**: new capability flags (`supports_merge_not_matched_by_source`, or a
   lowering rule via scoped DELETE), a documented keyed conditional DELETE+INSERT lowering for
   merge-less backends — and, independently, the pre-existing drift that
   `supports_column_scoped_merge` is consulted by two specs but absent from the capability
   matrix should be fixed.
7. **Statement emission**: statement groups gain the staged-candidate form (one temp relation +
   dependent statements, one transaction); emitters stay the single author; statement-parity and
   the conformance oracle extend to the new families. The conformance harness is well-placed for
   this: the equivalence gate already drives real runs against a full-refresh oracle, and a
   suppressed-write bug is exactly the class it exists to catch.
8. **`models.md` litmus rule** — applied, it lands cleanly: conditional variants change *which
   technique serves a cell* (derived, `smelt explain`-reported, steerable via
   `maintenance:` prefer/pin), so **no new declared model surface**. The only new declarations
   anywhere are on *sources* (referential integrity for inner-join closure, per the existing
   trust rule with a paired tripwire) and possibly a digest-acceptance knob (§6).

## 8. Cost model — when each mechanism wins and loses

- **T1/T2 (suppression)** trade one extra read+compare of the stored region against the saved
  writes. Loses when the change ratio approaches 1 (first build, definition-change backfill,
  genuinely hot regions) — precisely the cases the cost model or `prefer: recompute` should keep
  on the unconditional path. Wins grow with storage write cost (copy-on-write formats, wide
  rows) and with downstream CDC sensitivity. The MERGE-predicate form is nearly free to *try*:
  the compare rides the merge's existing matched-row read.
- **T4 (sidecar)** costs a narrow dimension scan + digest per run plus sidecar storage
  (`O(keys)` × ~40 bytes). Wins when the dimension is large and slow-changing — the common case
  for enrichment dimensions. For a dimension the run already full-scans (today's
  snapshot-reconcile posture), the marginal read is zero.
- **T3 (delta-restricted compute)** wins ∝ join cost × (1 − change ratio); it is the mechanism
  that turns "one renamed user" from a horizon-wide join into a point lookup.
- The designed home for the decision is already specified: per-cell technique choice among
  proven-interchangeable techniques, `smelt bakeoff` to measure, `prefer`/`technique` to steer.
  Conditional variants slot into that machinery without any new decision surface.

## 9. Prior art

The survey splits cleanly along the M1/M2/M3 factoring: **write suppression (M1) is
well-trodden**; **delta-restricted join compute (M2) exists only inside engines or with real
change logs**; **deriving the delta without any log (M3) has one classic academic treatment and
essentially no modern uptake**. No found system combines all three — derive the delta by
snapshot diffing, restrict the join algebraically to it, and merge conditionally — in a
CDF-less batch framework. That combination appears to be genuinely open ground.

### Write suppression (M1) — abundant precedent, always post-compute

- **Data Vault 2.0 HASHDIFF** is the industry-canonical form: a satellite row is inserted only
  when the incoming payload hash differs from the latest stored hash for the key. Notable
  operational hazards documented there transfer directly: hashing is column-order/format
  sensitive, and adding a payload column falsifies every stored hash (the sidecar-invalidation
  problem of §6). [automate-dv.readthedocs.io/en/latest/best_practises/hashing]
- **Kimball SCD change detection** (CRC/hash compare against the stored dimension row) is the
  same pattern, decades old. **dbt snapshots' `check` strategy** (MD5 over `check_cols`) ships
  it — but only for the snapshot/SCD2 feature; dbt *incremental models* always fully recompute
  the candidate and merge unconditionally (`merge_update_columns` scopes which columns are
  written, never whether the row is). [docs.getdbt.com/docs/build/snapshots]
- **Hash-guarded lakehouse MERGE** (`WHEN MATCHED AND t.hash <> s.hash THEN UPDATE`) is a
  widely-documented community pattern on Delta Lake, motivated exactly by C2: an unguarded
  MERGE rewrites every Parquet file containing any matched row, so suppressing no-op matches
  directly cuts copy-on-write file rewrites and CDC noise. Engine-side analogues (deletion
  vectors, Databricks low-shuffle merge) attack the same waste physically. Notably this is
  *pattern*, not product: users hand-author the guard per model — which is precisely the gap a
  derivation-based framework can close (derive the compared set from mutation-sensitivity + the
  contract, rather than trusting a hand-listed hash). [community.databricks.com "Why your Delta
  Lake merge takes forever"]
- **SQLMesh** ships no built-in content guard either (`INCREMENTAL_BY_UNIQUE_KEY` overwrites
  matched rows unconditionally; `when_matched` is a hand-rolled escape hatch;
  `SCD_TYPE_2_BY_COLUMN` compares columns but only in the snapshot-history feature — the same
  split as dbt).

All of these decide write-vs-skip *after* the candidate row is fully assembled: none reduce
join compute.

### Delta-restricted compute (M2) — engines and logs only

- **Classical IVM** supplies the algebra: the delta-join rule
  `Δ(R⋈S) = ΔR⋈S ∪ R⋈ΔS ∪ ΔR⋈ΔS` and its correctness under bag semantics (Gupta, Mumick &
  Subrahmanian, SIGMOD 1993 — the counting algorithm; Griffin & Libkin, SIGMOD 1995 — bag
  semantics done right). The payload-only 1:1 restriction (P1) is what collapses the three-term
  union to the single cheap term. **Self-maintainability** (Quass, Gupta, Mumick & Widom, PDIS
  1996) is the direct theory for the enrichment case: it characterises exactly which auxiliary
  state (a projection of the dimension onto key + read columns — our P4 projection!) suffices
  to maintain a join view from one side's delta without re-querying the other side, and where
  it breaks (referential integrity — our P1 row-preservation obligation, rediscovered).
- **Modern IVM** — DBToaster's higher-order deltas (VLDB 2012), F-IVM's factorised rings
  (SIGMOD 2018), DBSP/Feldera's Z-set circuits (VLDB 2023 best paper), differential
  dataflow/Materialize, Noria's partially-stateful dataflow — is deep and fast, and F-IVM's
  key/payload ring split maps almost exactly onto the skeleton/payload distinction. But every
  one of these **assumes the input delta is given** (a stream, a CDC feed, a transaction log).
  DBSP's Z-sets are still worth borrowing as *vocabulary*: the `WholeRow` multiset diff of P2
  is a Z-set delta, and a conditional merge is literally adding a signed Z-set to stored state.
- **Shipped engine IVM**: Snowflake **Dynamic Tables** is the strongest product precedent — its
  incremental refresh genuinely joins only each side's changes against the other side,
  recomputes only affected grouping keys, and (in `ADAPTIVE` mode) reasons explicitly about
  expensive-operator recompute cost vs full refresh — but it is proprietary planner internals,
  fed by engine-internal change tracking, with silent full-refresh fallback (the exact
  "invisible contract" posture validator-not-chooser exists to avoid). SQL Server indexed views
  and Oracle MV fast refresh (via MV logs) are the classical engine forms; Oracle's answer to
  "no CDF" was to *make* one (trigger-maintained logs on every base table — continuous write
  overhead a batch framework deliberately avoids). BigQuery MVs do a shallow version
  (append-only, left-side-of-join only).
- **Netflix's Incremental Processing (IPS, Maestro + Iceberg)** is the closest large-scale
  pipeline precedent, and its "Pattern 2" is almost exactly M2: use the captured change set "as
  a row-level filtering mechanism" — join the source against the changed-key set to scope
  re-aggregation — with ~90% compute reduction reported. The delta, however, comes from Iceberg
  snapshot lineage (real storage-level change capture), not from content diffing; IPS is
  M2-with-a-CDF. [netflixtechblog.com "Incremental Processing using Netflix Maestro and Apache
  Iceberg"] Uber's Hudi re-architecture (180-day rewrite window → incremental, 20h→4h) is
  further evidence of the cost problem, solved with log/index machinery rather than diffing.

### Deriving the delta without a log (M3) — the thin part of the literature

- **Labio & Garcia-Molina, "Efficient Snapshot Differential Algorithms for Data Warehousing"
  (VLDB 1996)** is *the* academic treatment: compute an insert/update/delete delta from two
  snapshots via compressed (hashed) record signatures and a windowed comparison, performing
  best exactly when snapshots are similar — the regime conditional maintenance targets. T4's
  fingerprint sidecar is this algorithm, persisted, applied per (source × projection); M3's
  output half is the same idea applied at the model-output boundary.
- The ETL hash-diff practice above is this paper's operational descendant, but at
  row-assembly time only. Nectar (OSDI 2010) content-addresses derived datasets for
  reuse — cache-level, whole-dataset granularity, not row-level. Kassaie & Tompa (2020) offer a
  useful classification vocabulary (irrelevant / autonomously-computable / pseudo-irrelevant
  updates) for when a view delta is derivable without full re-evaluation.
- On the correctness side of *conditional merge as reconciliation*: the idempotent-merge
  argument is formalised in the CRDT literature (Shapiro et al. 2011 — state-based merge is
  idempotent, so re-merging unchanged rows is a no-op, which is why over-approximate deltas are
  safe) and recently bridged to IVM's group/ring deltas (Power et al., "Wrapping Rings in
  Lattices", PaPoC 2024). No classical bag-semantics IVM paper states the
  "write-only-what-differs preserves the maintained state" lemma directly — §6's argument fills
  that with the equivalence invariant, which is arguably cleaner than either tradition alone
  because smelt *owns* the invariant that stored state equals prior evaluation.
- **Databricks Enzyme** (SIGMOD Companion 2026), the IVM engine behind Spark Declarative
  Pipelines, is the nearest contemporary system to smelt's setting (batch data-engineering
  pipelines, not streaming) and worth a close read during spec work — it appears to lean on
  Delta CDF availability, which would leave the CDF-less side open.

### Positioning

The two synthesis observations worth stating explicitly in any eventual spec Design section:

1. **Everything shipped either suppresses writes after full recompute (dbt/DV2/hash-guarded
   MERGE) or restricts compute using a real change log (Snowflake DT, IPS, Oracle, SQL
   Server).** The bridge — synthesise the log by diffing state you already own, then apply the
   30-year-old delta algebra to it — is unclaimed in the batch-ELT space.
2. **smelt is unusually well-positioned to build the bridge**, because the pieces the
   literature says you need are already normative machinery: the equivalence invariant is the
   licence to trust stored state as "prior evaluation" (what the engine lacks); the property
   walk supplies the static side (P1's closure via skeleton-role, provenance, cardinality);
   sources.md already *names* snapshot-diff consumption; and validator-not-chooser gives the
   honest version of what Snowflake does silently (a derived, explainable, refusable technique
   choice instead of an invisible fallback).

## 10. Open questions

1. **`plausible` semantics under suppression** — does "previously computed, inputs since
   changed" still count as plausible? The contract's wording decides between policies 1 and 2
   (§6).
2. **Digest acceptance surface** — is SHA-256-collision-soundness a stated global assumption
   (as `output_fingerprint.md` implicitly takes) or a per-source named acceptance?
3. **Key-level dirt in the graph layer** — T5 records key-level deltas, but propagation is
   interval-based; the partition-projection widening is sound, but full value needs the
   designed-but-unbuilt keyed dirt-sets (P7/P8). Does T5 wait for them or ship
   partition-projected?
4. **Referential-integrity declaration shape** — a `sources.md` world-fact
   (`foreign_keys:`? per-consumer?) with the count-preservation tripwire; exact surface TBD.
5. **Sidecar lifecycle** — namespace (`_smelt_fp_*` beside the ledger tables?), GC on source
   removal, behaviour across `smelt build --full-refresh`, and multi-consumer sharing (one
   sidecar per (source × projection), or per consuming project?).
6. **Observed-delta trust boundary** — T5's delta is exact for *this* model's writes, but a
   manual out-of-band edit to the target table breaks it (as it breaks the equivalence
   invariant generally). Is the existing "state is smelt-owned" assumption stated strongly
   enough to lean on, or does T5 need a tripwire?
7. **First-run and backfill interplay** — suppression is pointless on first build
   (everything is new); the plan should admit-but-not-prefer it there. Does the cost model need
   region-level change-ratio statistics (from prior observed deltas) to choose well?

## 11. Suggested increments

Ordered so each step is independently shippable, spec-first per the workflow:

1. **T1 on the column-scoped MERGE** — one predicate in one emitter, licensed by P2/P3 verdicts
   that mostly exist (declared `unique_key`; determinism predicate per-column). Directly
   mitigates the recorded "fires every run unconditionally" divergence's cost. Conformance
   legs: suppressed-vs-unconditional bit-equality at fixed `S`.
2. **T1 on the keyed fold / T2 conditional DELETE+INSERT** — the staged-candidate statement
   group; gives merge-less backends (Spark/Parquet) their first keyed path.
3. **T5 observed output deltas** — record only (no propagation consumer yet); surfaces in
   `smelt explain` and immediately improves `--since-upstream`'s `--landed` precision for model
   edges.
4. **P1 skeleton-source closure + referential-integrity declaration** — the proof layer for M2,
   valuable on its own for `smelt explain` narrative.
5. **T3 delta-restricted compute over model edges** — consume T5's recorded deltas on the
   maintained-model edge first (`examples/web_analytics`'s events→sessions chain is the natural
   demo and already exercises upstream-model edges); no sidecar machinery needed yet.
6. **T4 input sidecars** (external mutable snapshots) — last, because it carries the most new
   state (sidecar tables, P4 projections, invalidation), and by this point T3 is proven on the
   free-delta case.

## References

- `docs/specs/maintenance_plan.md` — the plan, per-cell admission, interchangeability,
  "fires every run" divergence, out-of-scope item (§Constraints).
- `docs/specs/model_properties.md` — the property catalogue and composition walk this extends.
- `docs/specs/model_transforms.md` — the transform catalogue (column-scoped merge,
  dimension-driven horizon MERGE, idempotent re-scan vs delta probe row).
- `docs/specs/sources.md` — mutation profiles, the licensing table's "snapshot-diff consumption",
  the trust rule, the frontier checksum, landed-delta intervals.
- `docs/specs/keyed_models.md` — snapshot-reconcile, no-write-eligibility-clamp, slice pruning.
- `docs/specs/models.md` — input-consumption axis, declaration law / litmus rule,
  `columns.<c>.contract`.
- `docs/specs/output_fingerprint.md` — soundness-invariant framing; the ephemeral-fingerprint
  design principle this proposal must diverge from.
- `docs/specs/multi_backend.md` — capability flags, lower-don't-reject.
- `examples/timeseries/models/daily_events_enriched.sql` — the worked fixture (external
  mutable-snapshot dimension).
- `examples/web_analytics/models/{silver/events_enriched,gold/eventstream_with_identity}.sql` —
  the model-edge chain fixture (stable sessions rejoined on every touched partition).
- Prior art: Gupta/Mumick/Subrahmanian 1993; Griffin & Libkin 1995; Quass et al. 1996
  (self-maintainability); Labio & Garcia-Molina 1996 (snapshot differentials); DBToaster 2012;
  F-IVM 2018; DBSP 2023; Shapiro et al. 2011 + Power et al. 2024 (idempotent merge);
  Netflix IPS; Snowflake Dynamic Tables; Data Vault 2.0 HASHDIFF; dbt snapshots `check`;
  Databricks Enzyme 2026. (§9 has links.)
