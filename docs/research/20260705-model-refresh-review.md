# Discussion paper: a fresh review of the model / refresh-mode framework

**Status:** research (review; no decisions made here)
**Date:** 2026-07-05
**Reviewer:** Claude (fresh-context review requested by andrew)
**Scope:** the maintenance framework and refresh-mode taxonomy as it stands after the
2026-07-04 re-cut — `model_maintenance.md`, `model_properties.md`,
`model_transforms.md`, `models.md`, the six mode specs, `sources.md`, and the
fundamentals research (`20260704-maintenance-fundamentals.md`). The question asked:
have we missed important use cases, is the taxonomy arranged well, have we missed
techniques? Emphasis on conceptual and factual problems, not presentation.

---

## 0. Overall assessment

The re-cut is genuinely good. The property/transform/mode layering, the
declared/derived/implied law, validator-not-chooser, the addressing axis
(partition-addressed vs key-addressed) as the real distinction between modes, and
the refusal to ship a `strategy:` sub-knob are all defensible and mostly
internally consistent. The findings below are not "start over" findings — they are
places where the framework's central claims are stated more strongly than they
hold, where the taxonomy has a seam that will leak, and where standard techniques
from the incremental-view-maintenance and streaming literature are absent and
would materially change some v1 "refuse / full refresh" verdicts.

I rank them: §1–§3 are the conceptual problems I'd want resolved in the specs
before more mode-vertical implementation lands; §4 is factual/implementability
problems; §5 is missed techniques; §6 is missed or under-served use cases.

---

## 1. The one-invariant claim does not survive its own mode list

`model_maintenance.md` stakes the whole family on one contract:

> for the processed input set `S`, `incremental_state(S) == full_refresh(source | partition ∈ S)`

Three of the six non-`full` modes cannot discharge this as written.

### 1.1 `latest_value` and `versioned`: the mode adds semantics the SQL does not have

For `batched`, `cumulative`, and `materialized_view`, the model SQL *is* the
meaning: run the SQL over all inputs and you get the stored relation. That is what
makes `full_refresh` an executable oracle, and it is the deepest smelt principle
in play ("logical SQL is pure; the framework injects filters, never semantics").

`latest_value` breaks this. The spec's example body is a bare projection:

```sql
SELECT customer_id, tier, region FROM smelt.customers_snapshot
```

A full refresh of *that SQL* returns every input row — not one row per key. The
one-row-per-key dedup, the "latest wins by ordering column" rule, and the
last-processed fallback are all semantics the **mode** layers on top of the SQL.
`versioned` goes further: the stored relation contains columns
(`valid_from`/`valid_to`/`is_current`) the SQL never projects. For both modes,
`full_refresh(model)` in the invariant silently means "the *mode's* semantics
applied to all inputs at once", a mode-specific re-definition that the framework
never acknowledges. The invariant text reads as if the SQL is the oracle for every
mode; for these two it is not.

This is fixable in two directions, and the choice is a real design decision:

- **(a) Make the SQL carry the semantics** (the derive-from-SQL direction the
  project already prefers). A `latest_value` body written as
  `SELECT key, MAX_BY(tier, updated_at) AS tier, … GROUP BY key` — or with
  `QUALIFY ROW_NUMBER() = 1` — *is* its own full-refresh oracle, and the classifier
  can verify the shape instead of imposing it. Note what falls out: written this
  way, `latest_value` is literally a cumulative-style keyed fold whose combiner is
  `MAX_BY` — which sharpens the question of whether it earns a peer mode at all
  (see §3).
- **(b) Keep the bare-projection surface** but state honestly, in
  `model_maintenance.md`, that for these modes the equivalence oracle is
  `mode_semantics(all inputs)`, not `sql(all inputs)`, and specify
  `mode_semantics` formally per mode. This is what dbt snapshots do implicitly;
  the cost is giving up "the SQL is the whole spec" for two modes.

Right now the specs do neither, and the generative equivalence oracle — named as
the family's regression net — has no defined reference implementation for these
two modes.

### 1.2 `versioned` over a mutable snapshot: the invariant is vacuous by construction

For a snapshot-diff source, `source | partition ∈ S` does not exist: a mutable
snapshot is a *state*, not a log, and the earlier states in `S` are destroyed by
the time any full refresh could run. `versioned`'s history is strictly **more
informative** than anything recomputable from the current source — capturing
information a full refresh destroys is the entire point of the mode (it is why dbt
snapshots exist). So the claim in `versioned_models.md` that the stored history
"equals what a full rebuild would compute from the same set of processed
snapshots" is an oracle that can never be executed and never tested. The
observation-series insight already exists in the research (`20260703` §19.6:
non-replayable input) but was applied only to the batched/partitioned side.

Suggested reframe: split the family's contract by **input replayability**, which
is already a source world-fact (`mutation_profile`):

- **Replayable inputs** (clocked append-only feeds, change feeds): the invariant
  holds as stated and is testable. All modes qualify.
- **Non-replayable inputs** (mutable snapshots): the mode is an **observer** — the
  contract is *prefix consistency* (the stored state is exactly what the mode's
  semantics produce over the sequence of observations actually made), not
  equivalence to any recomputation. `versioned` and `latest_value` over
  snapshot-diff live here; `cumulative` over snapshot-diff does not exist at all
  (§2.2).

This costs one paragraph of nuance in `model_maintenance.md` and buys back
honesty for the family's central claim.

### 1.3 The formal statement quantifies over partitions that snapshot sources don't have

Even for the modes where the invariant is meaningful, `partition ∈ S` type-errors
for an unclocked source. The invariant needs to be stated over "processed inputs"
abstractly, with the partition-set form as the clocked specialisation. Minor, but
this is the family's load-bearing sentence and it should type-check.

---

## 2. Two holes in the input-consumption axis

The axis (window-forward / snapshot-diff / change-feed) is presented as purely a
scan-cost fact — "moving along this axis never changes what the stored relation
means." Two problems.

### 2.1 The axis classifies discovery mechanics but not the *semantic type* of the input

Keyed modes divide on whether they consume **events/deltas** or **state
observations**, and this is not the same axis:

- `cumulative` and `accumulating_snapshot` fold **deltas**: each input row is a
  contribution to be folded exactly once. Feeding them a *state* stream is
  incoherent — snapshot-diffing a mutable source and re-merging into a `SUM`
  double-counts everything from the first re-scan.
- `latest_value` and `versioned` consume **states** (or state-change events):
  each input row is an observation that supersedes, not a contribution that
  accumulates. Snapshot-diff is natural for them.

The axis table in `models.md` currently says snapshot-diff applies to "keyed modes
over a mutable snapshot source" — uniformly. For `cumulative` that cell is not
expensive, it is *wrong*, and nothing in the framework rules it out: a cumulative
model over an unclocked mutable source is today rejected only by the incidental
`CumulativeNoDrivingSource` (no `timeseries:`), which reads as a missing
declaration, not as the real reason (a fold cannot consume states). The framework
should name the event-vs-state input distinction explicitly — it is a per-mode
compatibility fact (a mode × discovery-cell matrix, mostly derivable from the
combiner shape) — rather than leaving `cumulative`'s protection to a diagnostic
that appears to be about a missing YAML block. Without naming it, the first person
to "fix" `CumulativeNoDrivingSource` by teaching cumulative to snapshot-diff will
ship a silent double-counter.

### 2.2 Snapshot-diff needs a diff *baseline*, which is state smelt says it doesn't keep

Snapshot-diff for `latest_value`/`versioned` compares the incoming scan against
"the stored current versions" — fine, the target is the baseline. But note this
quietly commits the framework to the target-as-replica assumption for the diff to
be correct: any out-of-band edit to the target corrupts future diffs. Worth one
line in the constraints; today it is implicit.

---

## 3. The cumulative / accumulating_snapshot seam, and the litmus rule turned on its authors

### 3.1 Overlapping admissible SQL, different verdicts

`SELECT key, MIN(x) AS a, MAX(y) AS b FROM clocked_source GROUP BY key` is
admissible under **both** `refresh: cumulative` and
`refresh: accumulating_snapshot`, with the same combiners (`MIN→LEAST`,
`MAX→GREATEST`), the same driver, the same output shape, and the same end-state
contract. The observable differences are: (a) accumulating_snapshot requires a
bounded forward horizon `H` and clamps merge eligibility to `[run_start − H, …]`;
(b) accumulating_snapshot tolerates overlap/re-runs (its allowlist is all
idempotent) while cumulative admits non-idempotent `SUM`/`COUNT`; (c) the hot-key
cap.

Apply the project's own litmus rule (`models.md` §Design): a change that "changes
only how much is scanned" must be **derived**, never a peer mode. The horizon
clamp is a scan/write-work bound. On its face, accumulating_snapshot's
distinguishing machinery fails the peer test against cumulative — the two modes
are one keyed-fold family whose combiner allowlists differ, plus a horizon.

There *is* a defensible answer: the horizon is not just a scan bound, it is a
**completeness contract** — beyond `H`, late enriching facts are dropped, which
changes what the stored relation means relative to full refresh (§3.2). If that's
the justification, the litmus rule should say so: "changes the equivalence
contract, the output shape, **or the completeness bound**" — otherwise the rule as
written argues against the mode's existence. And the specs should stop describing
the two modes' overlap zone as if the choice between them were obvious; a
diagnostic ("this model is admissible as `cumulative`; note the differences…")
would be worth speccing, since the validator-not-chooser stance means users must
pick and can pick wrong.

### 3.2 The horizon clamp drops *scanned* inputs, and the declared-H path re-imports the exact failure the derived-horizon rule exists to prevent

`model_maintenance.md` defends silent late-arrival exclusion with: "smelt cannot
fail loud on a row it never scans." That defense is honest for `batched` — the
row is outside the scan window. It is **not true for accumulating_snapshot**: a
conversion row arriving in the current run window, targeting a key older than
`H`, is *scanned* (it is in the driving-source window being read) and then its
write is refused by the clamp. The framework can absolutely fail loud — or at
least count — here, and today the spec specifies a silent drop. Two consequences:

1. **The invariant is weakened in a way the framework doesn't formalise.** For a
   run set `S` containing a beyond-`H` enrichment,
   `incremental_state(S) ≠ full_refresh(source | partition ∈ S)` — the full
   refresh would apply the enrichment, the clamp drops it. "Processed inputs"
   silently becomes "processed inputs, minus scanned-but-clamped rows",
   per-key. This should be stated as the mode's *completeness specialisation* of
   the invariant, not left implicit under "watermark-style bound".
2. **The declared-`H` path contradicts the derived-horizon principle.**
   `model_maintenance.md`: "the horizon is derived, never trusted from a
   declaration… a declared horizon smaller than the true reach would make the
   clamp drop rows that should have been rewritten." `accumulating_snapshot.md`
   then accepts `H` **declared on the source** (`source_lateness`) when no forward
   predicate exists — and an under-declared source lateness produces precisely the
   silent row-dropping the principle names. Derive-else-declare justifies having a
   declared fallback; it does not justify having it *without the mitigation* that
   the rows in question, uniquely, are in hand.

Recommended in both cases: **late-fact accounting**. The delta rows the clamp
excludes are already materialised in the run; count them (optionally quarantine
them to a side table — a dead-letter for enrichments) and surface the count in run
output / `smelt explain`. This converts the one silent-data-loss point in the
whole framework into an observable one at near-zero cost, and it gives the
declared-`H` path an empirical check ("you declared 3 days; 4 % of enrichments
arrive later").

### 3.3 `materialized_view`'s "keyed" output shape is a taxonomy overreach

The mode accepts *arbitrary engine-incrementalisable SQL* — no key requirement,
no GROUP BY requirement — yet `models.md` and the composition table class its
output shape as "keyed". That is not derived from anything; it is asserted so the
mode fits the partitioned/keyed dichotomy. Two concrete costs:

- **Factual**: an engine-maintained view need not have a unique key; the shape
  claim is simply false for, say, a maintained join without aggregation.
- **A missed use case**: forbidding `timeseries:` on it means an engine-maintained
  *daily aggregate* — a perfectly partitioned relation — cannot declare its
  partition shape, so downstream batched models must read it in full instead of
  receiving source-filter pushdown. The forbid rule is inherited from the keyed
  modes, where it is justified ("the output has no partition column"); here the
  output may well have one.

Suggested: output shape for `materialized_view` is "whatever the SQL produces"
(shape: *opaque* or *engine-defined*), and `timeseries:` on it should be
*allowed as a downstream-consumption declaration* (the same role it plays on a
source — it declares a partition shape consumers may rely on, it doesn't drive
smelt maintenance). That also generalises: any refresh mode's output that is in
fact time-partitioned could usefully carry a consumer-facing clock; see §6.4.

---

## 4. Factual / implementability problems

### 4.1 Cumulative's reprocessing refusal and partial-failure recovery need state that doesn't exist by default

Two spec claims collide:

- `batched_models.md` / design memory: **smelt does not own watermarks or run
  history**; run-state tracking is opt-in (`state.mode: intervals`).
- `cumulative_aggregate.md`: the rule "rejects reprocessing at planning time when
  it can detect it (the partition has been merged before and the run window
  includes it)".

A merged `SUM` leaves no trace in the target of *which* partitions produced it.
Without a run ledger, "has been merged before" is undetectable, so the
double-count guard — the thing standing between `SUM`/`COUNT` and silent
corruption — is best-effort at most and default-off. Worse, the failure story
composes badly: a cumulative run over N partitions that crashes at partition k
has merged 1…k−1; the documented recovery for non-idempotent combiners is the
reprocessing machinery — which in v1 is "refuse; run `--full-refresh`". So **v1
cumulative with `SUM`/`COUNT` turns any mid-run crash into a full rebuild**, and
can't reliably detect the alternative (a blind re-run) that would corrupt state.
The standard technique that fixes both is in §5.1 and is cheap. At minimum the
specs should state the dependency explicitly: cumulative's constraints 9 (refuse
reprocessing) and the partial-failure story are sound **only under run-state
tracking**, which should arguably be mandatory (not opt-in) for non-idempotent
combiners.

### 4.2 `Append` as a backend-chosen strategy contradicts the idempotence invariant

`batched_models.md` says strategy is backend-internal ("backends pick a strategy
from the model's config and their capabilities") and lists `Append` (insert-only,
no dedup) in the enum, while Constraint 7 requires idempotence under fixed input.
A backend that picks `Append` violates Constraint 7 on any re-run. Either
`Append` must be gated on run-state ledger semantics (only append windows the
ledger proves unwritten), or the spec should say a backend may only choose among
strategies that preserve the mode's invariants (making `Append` unreachable
today). As written, the door is open for a conforming backend to be
non-conforming.

### 4.3 Intra-window "last-processed" is undefined

`latest_value`'s fallback combiner keeps "whichever row the current run wrote
last", but within a single window/scan with multiple rows per key there is no
defined order at all (SQL gives none) — the winner is engine- and plan-dependent,
i.e. row-nondeterministic. The open question on ordering-key ties is noted in the
spec, but the no-ordering-column case is worse than a tie-break question: it makes
the stored value nondeterministic per run, which the determinism discipline
elsewhere in the family would reject. Consider requiring an ordering column for
window-forward consumption (where multiple rows per key per window are expected)
and permitting last-processed only for snapshot-diff (where each scan has at most
one row per key by construction — a snapshot is a function of key).

### 4.4 Spec drift (small, but you asked for factual)

- `model_transforms.md` marks **compile-time pinning** "built" in the Surface
  table and lists it under "Unbuilt" in Known Divergences. (Commit `69ed9611`
  suggests built; the divergence entry is stale.)
- `batched_models.md` Known Divergences says the two-layer widened-scan/exact-clamp
  is "marked *partial (redesign)* in `model_transforms.md`"; `model_transforms.md`
  marks it **built** (and F13 landed it). Stale cross-reference.
- `models.md`'s frontmatter table says `batched.safety_overrides` includes
  `allow_subqueries`; `batched_models.md`'s taint discussion also references
  `allow_nondeterministic` as a "blunt" override not present in the frontmatter
  example's closed set. Worth one pass to make the override list identical in
  both specs.

---

## 5. Missed techniques

Ordered by how much they'd change current verdicts.

### 5.1 A transactional run ledger (processed-window log) — the exactly-once pattern

The classic fix for non-idempotent folds: keep a tiny per-model ledger table
(`model, window, run_id`) in the backend, and make each `merge_into` and its
ledger insert **one transaction**. Re-running a window is then detected exactly
(fixes §4.1's detection hole), resume-after-crash is exact (merge only unledgered
windows — no full rebuild), and `Append` becomes safely available (§4.2). This is
not "smelt owning a watermark store" in the sense the design rejected — the state
lives in the backend, transactional with the write it describes, so there is no
sync-correctness window (the rejection rationale). It is the idempotent-consumer
pattern every streaming system uses. The `run_state.md` intervals machinery is
adjacent but opt-in and not transactional-with-the-merge; this needs to be the
default for `SUM`/`COUNT` cumulative, or those combiners arguably shouldn't ship.

### 5.2 Per-key targeted recompute — the missing rung between group and multiset

The ladder jumps from rung 3 (invertible combiners; subtract-then-add) to rung 4
(bounded-domain multiset) and declares `MIN`/`MAX`/`BOOL_*` unreprocessable
without a full refresh. There is a standard middle technique the ladder misses:
**recompute only the affected keys from history**. A reprocessed partition names
exactly which keys it touches; for those keys — and only those — re-run the
model's aggregation over the full source history and overwrite the state rows.
Cost is `O(history of affected keys)`, not `O(table)`, and it needs *no stored
state at all* beyond the target. This is how many IVM engines handle
non-invertible aggregates under deletion (lazy recompute on retraction of the
current extremum), and it converts cumulative's harshest verdict ("a corrected
partition under `MIN` ⇒ rebuild the world") into a targeted repair. It also
serves `accumulating_snapshot`'s correction story (the
`AccumulatingSnapshotCorrectableMilestone` refusal could instead offer per-key
repair) and `versioned`'s late-correction open question. It belongs in
`model_transforms.md` as a catalogued transform ("keyed targeted recompute",
licensed by: key-addressed output + replayable source), and in the ladder text as
the acknowledged alternative to rung 4 when the domain is unbounded but affected
keys are few.

### 5.3 Self-emitted change feeds — incrementality currently dies at every DAG edge

Today every keyed output is a lookup read **in full** by every downstream model,
every run; and a batched model downstream of a cumulative one has no way to know
which keys changed. The framework's own input-consumption axis names the fix:
"an update-events *table* is a change feed reified as a window-forward source."
Smelt is in the unique position to make its *own maintained models* emit that
feed — `merge_into` knows exactly which keys it touched; appending `(key,
run_window)` to a companion delta table makes every maintained model a
`change_feed`-profile source for its consumers, and cascade maintenance becomes
uniform. This is the single technique that would most change the framework's
scalability story (it is the core move of Materialize/DBSP/SQLMesh cascades), and
nothing in the current architecture is hostile to it — it is literally the third
cell of an axis you already built. Worth a research note even if deferred; the
danger of not naming it is designing mode-local observability that later conflicts
with it.

### 5.4 Late-fact accounting / dead-letter for the horizon clamp

Covered in §3.2 — the clamped rows are in hand; count them, optionally quarantine
them. Cheap, converts the framework's one silent-loss point into telemetry.

### 5.5 Hash-diff for snapshot-diff

Snapshot-diff as specced compares incoming rows against stored current versions
column-by-column. The standard cost reduction (Data Vault's hash-diff; dbt
snapshot `check_cols` with a hash): store one digest column per row of tracked
attributes and compare digests. Turns the diff into a single-column comparison and
makes "did any tracked attribute change" O(1) per key. Belongs in
`versioned`/`latest_value` semantics when snapshot-diff is specced properly (it
also interacts with tracked-attribute selection — the digest defines the tracked
set operationally).

### 5.6 Bitemporality as the shape of the late-correction answer

`versioned`'s open questions (late corrections to a closed interval; hard-delete
opt-in) are the exact problem bitemporal modelling exists for: keep *valid time*
(source event time — already the spec's stamp) and *system time* (when smelt
learned it). A correction then never rewrites history — it appends a new
system-time version of a valid-time interval, and "as-was" queries remain stable.
Full bitemporality may be more than smelt wants, but the open question should at
least be argued against it, because the alternative (in-place rewrite of closed
intervals) silently breaks the replay-safety the mode's §"Validity stamped from
source event-time" fought for.

### 5.7 Quantile sketches alongside HLL

Rung 2 names decomposed monoids: Welford for variance, HLL for approximate
distinct. The same slot for approximate quantiles (t-digest / KLL sketches —
mergeable, bounded-size) is the standard answer to "exact `MEDIAN` needs a
multiset": most users asked for a percentile will accept a sketch. The
bounded-domain multiset (rung 4) is then reserved for the genuinely-exact cases.
The refusal diagnostic for a holistic aggregate should suggest the sketch form by
name once it exists; today the spec text says "suggesting the approximate form"
without there being one for quantiles.

### 5.8 Versioned-history compaction

`versioned` accumulates versions forever; `accumulating_snapshot` explicitly never
GCs. Fine for v1, but there is no catalogued transform for *history compaction*
(collapse versions older than a retention horizon; keep last-per-key before time
T). It will be asked for the first time a versioned model meets a chatty source.
One row in the transforms catalogue ("history compaction — licensed by a declared
retention world-fact") reserves the concept.

---

## 6. Missed or under-served use cases

### 6.1 Multiple event streams driving one keyed model

`CumulativeMultipleDrivingSources` rejects the honest and common shape "one keyed
state fed by the union of N event tables" (device edges from web events + app
events; a milestone fed by two conversion feeds). The batched side already solved
the analogous problem (UNION ALL with per-branch traceable clocks is admitted,
per-branch pushdown). The keyed driver could accept a UNION ALL of same-clock
sources as *one* driving input — the per-window delta SELECT unions the branches
— without touching the multi-granularity question that motivated the deferral.
Today the workaround is an intermediate batched union model, which materialises a
full copy of the event stream just to satisfy the exactly-one-anchor rule. Worth
distinguishing in the specs: "N joined candidate anchors" (genuinely ambiguous —
reject) vs "N unioned same-shape streams" (not ambiguous — admit).

### 6.2 Periodic snapshot facts

Kimball's three fact types are transaction (→ `batched`), accumulating snapshot
(→ named mode), and **periodic snapshot** (one row per entity per period — daily
account balances, weekly inventory levels). The framework covers it — it is a
self-referential ordered batched model (balance = yesterday's balance + today's
deltas) — but nothing *names* it, the self-referential path is specced-not-built,
and it is the shape most likely to send a user hunting for a mode that doesn't
exist. Deserves a named recipe in the user docs (and is the strongest concrete
motivation for prioritising the ordered-execution enforcement noted as unbuilt in
`batched_models.md`).

### 6.3 Deletion/erasure propagation (GDPR) as a first-class driver of retraction

The retraction discussion is framed around corrections and reprocessing. The
stronger real-world forcing function is **right-to-erasure**: rows deleted
upstream for compliance must vanish from maintained state, on a deadline, in
every keyed model — including the non-invertible ones. Under the current
framework the honest answer is "full refresh of every keyed model that ever
folded the subject's rows", which is operationally brutal at exactly the moment
it's legally required. §5.2's per-key targeted recompute is the right-sized tool
(the erased subject's keys are known). Naming the use case matters because it
converts retraction from "nice-to-have rung 3" into a requirement with a
deadline, and it argues for the erasure path to work on *every* rung, not only
the group rung.

### 6.4 Time-aware downstream consumption of versioned output

A `versioned` table has a real event-time column (`valid_from`) and is typically
huge; downstream models (e.g. "changes this week") must read it in full because
keyed outputs forbid `timeseries:`. Same shape as §3.3's materialized-view point:
the forbid rule conflates "smelt doesn't partition-maintain this output" (true)
with "consumers can't be told about its time column" (unnecessary). A
consumer-facing clock declaration on keyed outputs whose maintained columns are
provably append-monotone (`valid_from` is) would restore pushdown for a common
consumption pattern.

### 6.5 Cross-partition dedup ingest

At-least-once delivery lands duplicate events in *different* partitions;
partition-local DELETE+INSERT can't dedup across them, and `batched.unique_key`
is documented as a backend MERGE hint rather than a dedup contract. The framework
has the right primitive (a keyed mode — `latest_value` by `event_id` — is exactly
"keep one row per event"), but no spec or user doc connects "my source has dupes
across days" to that answer, and a `latest_value` model whose key is an event id
over a high-volume stream stresses the keyed path in a way none of the motivating
examples do. Worth a documented pattern plus a look at whether the keyed driver's
per-window `merge_into` is acceptable at event-grain cardinalities.

---

## 7. Summary of recommendations

| # | Finding | Suggested action | Where |
|---|---|---|---|
| 1.1 | `latest_value`/`versioned` add semantics beyond the SQL; invariant's oracle undefined for them | Decide: SQL-carries-semantics (MAX_BY/QUALIFY shape) vs formal per-mode `mode_semantics`; spec it | `model_maintenance.md`, both mode specs |
| 1.2 | Invariant vacuous for non-replayable inputs | Split contract by replayability: equivalence vs observer/prefix-consistency | `model_maintenance.md` |
| 2.1 | Event-vs-state input type missing from input-consumption axis; cumulative × snapshot-diff incoherent | Add the distinction + a mode×cell compatibility matrix | `models.md` |
| 3.1 | cumulative/accumulating_snapshot overlap; litmus rule as written argues against the latter's peer status | Extend litmus rule with completeness bound; spec the overlap-zone guidance | `models.md`, both mode specs |
| 3.2 | Horizon clamp drops *scanned* rows silently; declared-H re-imports the silent-drop failure | Late-fact accounting (count + optional quarantine); state the completeness specialisation of the invariant | `accumulating_snapshot.md`, `model_maintenance.md` |
| 3.3 | `materialized_view` output shape is not "keyed"; `timeseries:` forbid blocks downstream pushdown | Shape = engine-defined; allow consumer-facing `timeseries:` | `materialized_view.md`, `models.md` |
| 4.1 | Reprocessing detection + crash recovery for `SUM`/`COUNT` need state that's opt-in/absent | Transactional run ledger, default-on for non-idempotent combiners (§5.1) | `cumulative_aggregate.md`, `run_state.md` |
| 4.2 | `Append` strategy vs idempotence invariant | Gate `Append` on the ledger, or constrain backend strategy choice | `batched_models.md` |
| 4.3 | Last-processed `latest_value` is row-nondeterministic intra-window | Require ordering column for window-forward; last-processed only for snapshot-diff | `latest_value_models.md` |
| 5.2 | Ladder misses per-key targeted recompute | Catalogue the transform; soften the "non-invertible ⇒ full refresh" verdicts | `model_transforms.md`, `model_maintenance.md` |
| 5.3 | Incrementality dies at DAG edges | Research note: self-emitted change feeds from maintained models | new research doc |
| 6.1 | Unioned same-clock streams rejected as "multiple driving sources" | Admit UNION ALL of same-shape clocked sources as one anchor | `cumulative_aggregate.md` + keyed siblings |
| 6.2 | Periodic snapshot facts unnamed | User-doc recipe; prioritise ordered self-referential batched | docs-site, `batched_models.md` |
| 6.3 | Erasure/GDPR unaddressed as retraction driver | Name it; route to per-key recompute | `model_maintenance.md` |

The pattern across the strongest findings (§1, §3.2, §4.1) is worth naming: the
framework is at its best where a claim is *checkable* (the batched oracle, the
fail-closed proofs) and at its weakest where a claim is stated with the same
confidence but has no executable oracle (the keyed invariant for
latest_value/versioned, the reprocessing guard, the silent horizon drop). The
next unit of spec work I'd fund is not another mode — it is making the invariant
statement itself honest per mode (replayability, mode semantics, completeness
bound), because every composition table in the family cites it.
