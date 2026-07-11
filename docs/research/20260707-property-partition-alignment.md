# Property research: partition alignment (scoped)

- **Date**: 2026-07-07
- **Status**: research
- **Property row**: `docs/specs/model_properties.md` §Capability table, row "Partition alignment (scoped)" — `PartitionAlignment = Aligned | NotAligned{reason}`, judged per scope
- **Related specs**: `docs/specs/batched_models.md` (§"Safety checks", §"Run window vs partition granularity"), `docs/specs/keyed_models.md` (§Constraint violations `KeyedGroupByContainsPartitionColumn`), `docs/specs/model_maintenance.md` (§"The equivalence invariant" — per-partition strengthening), `docs/specs/timeseries.md` (the clock declaration and granularity ordering)
- **Related code**: `crates/smelt-logical/src/analysis/mod.rs` (`PartitionAlignment`, `scope_group_by_alignment`, `scope_distinct_alignment`, `scope_over_alignment`, `resolve_scope_group_by`); `crates/smelt-logical/src/rules/incremental.rs` (the consuming admission checks, including the not-yet-rewired uppercase-substring `OVER`/`LIMIT` scans)
- **Related research**: `docs/research/property-discovery/ledger.md` (correlated-EXISTS bound extraction finding, `source_bounds.rs:589`), `docs/research/20260705-keyed-collapse-application.md` (the opposite-polarity consumer's decision record)

---

## 1. The property

**Definition.** For a model whose timeseries clock declares `partition_column` `p` at partition grain `g_p` (`timeseries.md`), and for each *scope* in the model's SQL that partitions its input rows into equivalence classes — a `GROUP BY` (classes = groups), a `DISTINCT` (classes = duplicate sets), a window `OVER` (classes = window partitions) — the scope is

> **Aligned** iff the scope's key set `K` *refines* the partition function: for all input rows `r1, r2`, `K(r1) = K(r2) ⟹ part(r1) = part(r2)`, where `part(r)` is `r`'s value of `p` truncated to grain `g_p`.

Equivalently: no equivalence class of the scope ever contains rows from two different partitions. When that holds, the scope's computation *factors* — evaluating it over any union of whole partitions produces, for the rows of each partition, exactly the rows a full evaluation would produce. That factoring is the entire value of the property.

**Why containment is the right practical test.** Refinement is semantic and undecidable in general; the decidable sufficient condition is that `K` **contains** `p` (or a partition-determining transform of `p`, next paragraph). If `p ∈ K`, two rows in the same class agree on `p` outright, hence on `part(r)`. The check is therefore stated as containment and evaluated fail-closed: containment proven ⇒ `Aligned`; anything else ⇒ `NotAligned{reason}` naming the scope's actual keys. `NotAligned` is *not* a claim that the scope crosses partitions — it is the absence of a proof that it cannot. This matters for the polarity discussion (§3): the keyed consumer must not read `NotAligned` as a positive proof of cross-partition folding.

Containment is deliberately a **raw fact, not a mode verdict**. `model_properties.md` §Constraints (placement criterion) keeps the batch-safety roll-up in `batched_models.md`; this property only answers "does this scope's key pin the partition?" per scope, and each mode composes the answers with its own polarity.

### The transformed-column subtlety

The partition column is very often *itself* a transform in the model (`DATE_TRUNC('day', event_ts) AS event_date`), and scopes may key on further transforms of the underlying timestamp. Containment must then be judged up to **granularity compatibility**, and the direction matters:

Let the model's partition grid be truncation to `g_p` (say `day`), and let a scope key on `t(ts)` where `t` is a truncation to grain `g_k`.

- **`g_k ≤ g_p` (finer or equal) — preserves alignment.** Every `g_k`-bucket lies inside exactly one `g_p`-bucket: knowing `date_trunc('hour', ts)` determines `date_trunc('day', ts)`. `GROUP BY date_trunc('hour', ts)` on a day-partitioned model never merges rows from two days. Equal grain (`GROUP BY date_trunc('day', ts)` where `p = date_trunc('day', ts)`) is the identity case.
- **`g_k > g_p` (coarser) — breaks alignment.** A month-bucket contains many day-partitions; `GROUP BY date_trunc('month', ts)` on a day-partitioned model merges rows from ~30 partitions into one group. Recomputing one day's partition cannot reproduce that group.
- **Non-grid transforms break alignment even when they look "fine-grained".** `EXTRACT(dow FROM ts)` yields 7 buckets each of which unions rows from *every* week — it is not a refinement of any calendar grid. `ts::DATE + INTERVAL 1 day` (a constant shift) *is* grid-compatible (it is a bijection on day-buckets) but shifts which partition a group lands in — it preserves the factoring but moves the write target; this is exactly the write-rebasing case `batched_models.md` §Execution model step 1 handles via the widened write window, and the monotonicity trace already classifies constant shifts. The precise criterion is: `t` preserves alignment iff `t` **refines the `g_p` grid** (each fiber of `t` lies within one partition); it additionally keeps the write window unshifted iff `t`'s fibers are *within the same* partition they came from.

The coarseness order is the closed enum ordering `hour < day < week < month < quarter < year` (`timeseries.md` §"Granularity values"; note `week`/`month` are incomparable-in-spirit — a week straddles month boundaries — so refinement must be judged on the *grid*, not merely the ordinal: `date_trunc('week', ts)` does **not** refine the month grid even though `week < month` in the enum. The enum ordering is safe for the `g_run ≥ g_part` run-window check in `batched_models.md`, which only needs "coarser run window over finer partitions"; alignment refinement needs the stronger grid-containment fact, for which `week` refines only `day`/`hour`).

Two adjacent checks that must not be conflated with this property:

- `batched_models.md` §"Run window vs partition granularity" checks the *declared* `granularity` (`g_run`) against the grain implied by the `partition_column` projection (`g_part`), requiring `g_run ≥ g_part`. That is a declaration-consistency check on the clock itself.
- Partition alignment checks each *scope's keys* against the (already-validated) partition grid. Same granularity machinery, different subject.

## 2. Why maintenance needs it — and the opposite polarity

**Per-partition equivalence.** `model_maintenance.md` §"Addressing" (lines 41–46): partition-addressed output enjoys *per-partition equivalence* — the single processed-input equivalence invariant, checkable slice-by-slice — "available because each output slice depends only on its own source partition". Partition alignment is precisely the per-scope obligation that makes this true through the model's own computation tree: the bound/reach property bounds which *source rows* a partition reads; alignment guarantees that no *intermediate scope* re-mixes rows across partitions after they are read. Together they license the wholesale-rewrite transform: `DELETE WHERE p IN write_window; INSERT SELECT …` is legal because every deleted row's replacement is computable from the write window's own input alone.

Concretely, an unaligned scope breaks each admitted construct's contract:

- an unaligned `GROUP BY` puts rows from inside and outside the write window into one group — the partial run sees a different group composition than a full refresh, so any `HAVING` filters differently and any aggregate value differs;
- an unaligned `DISTINCT` dedups a window-internal row against a row outside the window — the partial run keeps a row the full refresh would drop (or vice versa);
- an unaligned window `OVER` gives a row a frame that reaches outside the write window — the recomputed row's value depends on rows the run did not (and per the pushdown clamp, *cannot*) see identically.

This is why `batched_models.md` §"Safety checks" admits `HAVING`/`DISTINCT`/windows *iff* the owning scope is `Aligned`, and refuses (with a `safety_overrides` escape) otherwise.

**The opposite polarity: keyed.** The key-grain shape (`keyed_models.md`) exists to compute *cross-partition folds*: `GROUP BY user_id` deliberately omits the clock, and the maintained state is exactly the accumulation of contributions from many partitions, kept current by `merge_into` rather than partition rewrite. For that consumer the same containment fact is read inverted:

- `NotAligned` scopes are the *expected, definitive* shape — they are what makes the fold necessary (a run's delta rows update keys whose stored state came from other partitions). The keyed collapse wants these scopes *surfaced*, because each cross-partition scope is a column family the classifier must assign a combiner to.
- `Aligned` aggregation is the *anomaly signal*: a keyed model whose `GROUP BY` contains the driving source's `partition_column` with no `timeseries:` block is ambiguous between the partition-grain shape and the key-embedded time-partitioned key-grain shape, and is a hard error (`KeyedGroupByContainsPartitionColumn`, `keyed_models.md` §Constraint violations) suggesting both resolutions.

One raw fact, two consumers with opposite polarity — the reason the property row insists it is "a raw containment fact, not a mode verdict", and why the enum carries no "safe"/"unsafe" vocabulary. A caution that falls out of the fail-closed encoding: since `NotAligned` is absence-of-proof, the keyed consumer must treat it as "not proven partition-local" (sufficient for surfacing a scope for classification) but must not use it as a positive proof that the fold *does* cross partitions — e.g. `GROUP BY user_id` on a source where `user_id` happens to functionally determine `p` is semantically aligned but reported `NotAligned`. The batched side loses an admission (sound); the keyed side must not gain a conclusion.

## 3. Per-construct analysis

Running example schema (DuckDB), day-partitioned on `event_date = DATE_TRUNC('day', event_ts)`:

```sql
CREATE TABLE events (user_id INT, event_ts TIMESTAMP, amount DECIMAL(10,2));
INSERT INTO events VALUES
  (1, TIMESTAMP '2026-07-01 09:00', 10.00),
  (1, TIMESTAMP '2026-07-02 09:00', 20.00),
  (2, TIMESTAMP '2026-07-02 10:00',  5.00);
```

Write window for the counterexamples: the single partition `2026-07-02` (i.e. a run recomputing only that day).

### 3.1 Plain projection / filter — alignment-neutral

`SELECT user_id, DATE_TRUNC('day', event_ts) AS event_date, amount FROM events WHERE amount > 1` creates no scope: rows pass through individually, each carrying its own partition value. There is nothing to align; the construct contributes no verdict to the roll-up. Its one obligation is *carriage* (§4): if the projection drops `event_date`, every downstream scope becomes unalignable — the implementation's `NotAligned { reason: "partition_column … is not projected in this scope" }` arm.

### 3.2 GROUP BY containing / omitting the partition column

Aligned:

```sql
SELECT DATE_TRUNC('day', event_ts) AS event_date, user_id, SUM(amount) AS total
FROM events GROUP BY 1, 2
```

Every group is pinned to one day. Recomputing 2026-07-02 alone reproduces `(2026-07-02, 1, 20.00)` and `(2026-07-02, 2, 5.00)` exactly.

Counterexample (omitting):

```sql
SELECT user_id, SUM(amount) AS total, MAX(DATE_TRUNC('day', event_ts)) AS event_date
FROM events GROUP BY user_id
```

Full refresh: user 1 → `total = 30.00`. A run over partition 2026-07-02 sees only the 07-02 rows: user 1 → `total = 20.00`. The group for user 1 straddles the write-window edge; DELETE+INSERT of the 07-02 partition writes `30.00 → 20.00` corruption (and note the group's `event_date` is itself an aggregate — the row's partition address is unstable). This is exactly the shape that belongs in keyed mode.

### 3.3 GROUP BY on an expression of the partition column — granularity cases

Finer (preserves):

```sql
SELECT DATE_TRUNC('hour', event_ts) AS event_hour,
       MIN(DATE_TRUNC('day', event_ts)) AS event_date,   -- constant per group
       SUM(amount) AS total
FROM events GROUP BY 1
```

Each hour-group lies inside one day; the per-partition recompute reproduces every group whole. (Semantically aligned; the current implementation reports `NotAligned` because it matches key text against the projected `event_date` expression — sound, incomplete; §7.)

Coarser (breaks):

```sql
SELECT DATE_TRUNC('month', event_ts) AS event_month, SUM(amount) AS total
FROM events GROUP BY 1
```

Full refresh: `(2026-07, 35.00)`. The July group aggregates rows from partitions 07-01 and 07-02; a 07-02-only run computes `(2026-07, 25.00)`. Worse, the output has no day-grain partition column at all, so the DELETE cannot even address the stale row. Coarser bucketing is a grain change — the model's real partition grain *is* month, and declaring `granularity: day` on it should fail the `g_run ≥ g_part`-style consistency check, not be papered over by alignment.

Non-grid (breaks despite small buckets): `GROUP BY EXTRACT(dow FROM event_ts)` — the Wednesday bucket unions rows from every Wednesday in history; recomputing one day rewrites a bucket whose other contributors are invisible.

### 3.4 GROUPING SETS / ROLLUP / CUBE — per-set alignment

```sql
SELECT DATE_TRUNC('day', event_ts) AS event_date, user_id, SUM(amount) AS total
FROM events GROUP BY GROUPING SETS ((event_date, user_id), (event_date), ())
```

The first two sets contain `event_date` — each of their output rows is partition-pinned. The `()` set is a **global scope**: its single grand-total row (`event_date IS NULL`, `total = 35.00`) aggregates every partition. Two independent failures: (a) its value changes whenever *any* partition changes, so no per-partition rewrite maintains it; (b) its `event_date` is `NULL` — the row has no partition address, so the DELETE clause can never target it (it survives every run and duplicates). `ROLLUP(event_date, user_id)` includes the `()` set, so it is unaligned; `ROLLUP(user_id) ` nested under an outer aligned key is fine only if spelled `GROUPING SETS ((event_date, user_id), (event_date))`. Verdict rule: the construct is `Aligned` iff **every** grouping set contains the partition column (or a refining transform). CUBE never qualifies (it always includes `()`).

### 3.5 DISTINCT — whole row vs projected subset

`SELECT DISTINCT` dedups on the whole projected row, so it is `Aligned` exactly when `p` is projected: two rows can only collide when they agree on every column, including `p`, hence lie in the same partition. `SELECT DISTINCT user_id, DATE_TRUNC('day', event_ts) AS event_date FROM events` — aligned.

Counterexample (dropped column):

```sql
SELECT DISTINCT user_id FROM events
```

Full refresh: `{1, 2}` — user 1 appears once. Per-partition runs: partition 07-01 yields `{1}`, partition 07-02 yields `{1, 2}` — user 1 written by both runs. Beyond the duplicate, the output has no partition column, so successive DELETE+INSERT runs cannot address prior rows at all. (The timeseries surface already requires `p` projected at the outer scope; the per-scope check exists so an *inner* scope that drops it is still caught.)

### 3.6 Window OVER

Aligned — `PARTITION BY` a superset containing `p`:

```sql
SELECT user_id, event_date, amount,
       SUM(amount) OVER (PARTITION BY event_date, user_id ORDER BY event_ts) AS running_in_day
FROM (SELECT *, DATE_TRUNC('day', event_ts) AS event_date FROM events)
```

Every frame is contained in one day-partition; recomputing 07-02 reproduces its rows' window values exactly.

Counterexamples:

```sql
-- PARTITION BY omitting p: user 1's frame spans 07-01 and 07-02.
ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_ts)
-- full refresh gives user 1's 07-02 row rn = 2; a 07-02-only run gives rn = 1.

-- Empty OVER (): the maximally unaligned scope — one global window.
SUM(amount) OVER ()   -- 35.00 on full refresh; 25.00 on a 07-02-only run, on every row.

-- OVER (ORDER BY event_ts): global running total, same failure.
```

One refinement the reach machinery owns, not alignment: a *bounded-frame* window over the event time (`ROWS/RANGE BETWEEN … PRECEDING`) can be admitted even when unaligned, because the frame-reach taxonomy converts it into a finite lookback widening the scan window (`incremental.rs` explicitly exempts bounded-`PRECEDING` frames from the OVER admission scan). Alignment is the *zero-lookback* license; frame-reach is the *bounded-lookback* one.

### 3.7 Aggregates without GROUP BY — the global scope

`SELECT COUNT(*) AS n, MAX(DATE_TRUNC('day', event_ts)) AS event_date FROM events` collapses all partitions into one row: the degenerate `GROUP BY ()`, unaligned by the same argument as the `()` grouping set. `scope_group_by_alignment` returns `NotAligned { reason: "this scope has no GROUP BY" }` for it — correct — but note the consumer-side guard in `incremental.rs` only invokes the check when a `GROUP BY` clause exists (§7).

### 3.8 Joins — data movement across partitions

A join creates no key-equivalence scope, so alignment is not judged *on* it — but it can move rows across partitions, which is the neighbouring **bound/reach** property's jurisdiction. The division of labour, precisely:

- The output's partition value comes from exactly one projected expression (say `e.event_date` from the driving side). Downstream scope alignment is judged against *that* column. A `GROUP BY d.other_date` where the model's `partition_column` is `e.event_date` is simply unaligned (its keys don't contain the partition expression).
- **Lookup join (unclocked side)**: `events e JOIN users u ON e.user_id = u.user_id` — each output row keeps `e`'s partition; the scope-factoring story is untouched. What per-partition *equivalence* additionally needs is that the lookup side is read in full every run (it is: sources without a clock have no bound — `batched_models.md` step 3) and that a changed lookup row is handled by staleness, not by the partition contract.
- **Equi-join on the partition column**: `e JOIN f ON e.event_date = f.event_date AND …` — the join key pins both sides to the *same* partition; either side's `event_date` is a valid partition projection, and no row of a foreign partition can influence a given output partition. This is the join shape that preserves partition-locality outright.
- **Clocked join on non-time keys**: `events e JOIN payments p ON e.txn_id = p.txn_id` — an output row in partition 07-02 may read a `payments` row from any partition. Not an alignment failure (downstream `GROUP BY e.event_date` is still `Aligned` as a scope) but a reach obligation: per-partition equivalence holds only if the reach derivation can bound how far `p` is scanned (`Bounded(before, after)`), else the model is `PerPartitionOnly`/refused. Counterexample for the composed claim: with a payment row at 06-01 joined to an event at 07-02, recomputing 07-02 with `payments` clamped to 07-02 silently drops the join match a full refresh finds.

### 3.9 UNION ALL / UNION / INTERSECT / EXCEPT

**UNION ALL** is transparent — it creates no scope; each branch's rows pass through with their own partition values. Alignment is judged per branch, plus a *branch-consistency* condition on the output column (worked in §4, the motivating example).

**UNION / INTERSECT / EXCEPT are global scopes**: their DISTINCT/membership semantics compare rows *across both branches over all partitions*. They must therefore be judged, not passed through. The saving grace mirrors §3.5: the comparison key is the whole row, so **if `p` is projected in both branches at the model's partition grain, the scope factors** — rows can only interact when equal, hence co-partitioned. `SELECT user_id, event_date FROM a UNION SELECT user_id, event_date FROM b` is aligned. The genuine hazards:

- `p` not projected: `SELECT user_id FROM a EXCEPT SELECT user_id FROM b` — membership of user 1 depends on `b` rows in every partition.
- Cross-branch grain mismatch, concretely:

```sql
SELECT user_id, DATE_TRUNC('day',   event_ts) AS event_date FROM events
EXCEPT
SELECT user_id, DATE_TRUNC('month', event_ts) AS event_date FROM refunds
-- refunds: (1, TIMESTAMP '2026-07-15 00:00')
```

The right branch produces `(1, 2026-07-01)` (month-truncated) — which cancels the *left* branch's genuine 07-01 day row for user 1. A run recomputing partition 07-01 must read July refunds from *every* day to know this; the right branch's re-bucketing makes the equal-rows-are-co-partitioned argument false because the right branch's `event_date` is not its rows' partition value. Rule: a DISTINCT-flavoured set op is `Aligned` iff `p` is projected and **every branch's projection at that position is that branch's own partition value at grain equal to `g_p`** (finer is not enough here — equality of the *column values* is what confines interaction, so the grains must literally match).

Multiplicity note: `EXCEPT ALL`/`INTERSECT ALL` (bag semantics) obey the same rule — matching is still per-equal-row.

### 3.10 Correlated subqueries

A correlated predicate is a semi-join; the scope question is whether the correlation pins the inner scan to the outer row's partition.

Aligned (correlation includes the partition column):

```sql
SELECT * FROM daily d
WHERE EXISTS (SELECT 1 FROM events e
              WHERE e.user_id = d.user_id
                AND DATE_TRUNC('day', e.event_ts) = d.event_date)
```

Each outer row's probe reads only its own partition of `events`; recomputing 07-02 is self-contained.

Counterexample (correlation omits it):

```sql
SELECT * FROM daily d
WHERE EXISTS (SELECT 1 FROM refunds r WHERE r.user_id = d.user_id)
```

With `refunds = {(1, 2026-07-01 …)}`, the 07-02 row for user 1 is kept because of a 07-01 refund. If `refunds` is scanned clamped to the write window (pushdown), the recomputed 07-02 partition drops the row a full refresh keeps. If `refunds` is unclocked and read in full, the row is *correct* but the model's staleness now depends on refunds changes — the reach property's territory again. Ledger finding worth carrying: the correlated-predicate bound extraction in `source_bounds.rs:589` takes `_partition_col_upper` but never verifies the matched columns are the source's own partition column (`docs/research/property-discovery/ledger.md` cell around line 228) — the alignment analogue of that check should not repeat the mistake.

### 3.11 LIMIT / ORDER BY / QUALIFY

- `LIMIT` (with or without `ORDER BY`) is a global top-k scope: which rows survive depends on the whole relation. `SELECT * FROM events ORDER BY amount DESC LIMIT 2` keeps `(1, 07-02, 20.00)` and `(1, 07-01, 10.00)` on full refresh, but a 07-02-only run keeps `(2, 07-02, 5.00)` instead of the 07-01 row — different row *set* per partition. Unaligned, no aligned form short of a partition-local spelling (`QUALIFY row_number() OVER (PARTITION BY event_date …) <= k`).
- Bare `ORDER BY` with no `LIMIT` is presentation only — set-determinacy handles it; no alignment scope.
- `QUALIFY` filters on a window value, so its alignment *is* the referenced window's alignment: `QUALIFY ROW_NUMBER() OVER (PARTITION BY event_date, user_id ORDER BY event_ts) = 1` — aligned (partition-local top-1); the same over `PARTITION BY user_id` — unaligned (§3.6's counterexample, now changing row membership rather than a value, which is strictly worse).

## 4. Composition algebra — the model-level roll-up

Alignment is judged per scope; the model verdict is a **meet (AND) over every scope in the query tree**, threaded by a *carriage* relation: each scope is judged against the partition expression *as visible in that scope*, so the roll-up must track the partition column's identity through projections, and one broken link severs everything downstream.

**The carriage rule (stacked CTEs).** Alignment survives re-projection iff (a) each SELECT list carries `p` — or a grid-refining transform of it — into an output column, (b) every intervening scope is itself aligned, and (c) downstream scopes key on the carried column. Formally: if scope `S2` reads relation `R` produced by scope `S1`, and `S1` is aligned, then every `R`-row is partition-pinned and `S2`'s alignment reduces to containment of the carried column in `S2`'s keys. If `S1` is unaligned, `R`'s rows have no honest partition provenance and `S2`'s "alignment" is vacuous — the meet is already `NotAligned`. Transform composition: a chain of grid-refining transforms is grid-refining; a single coarser step anywhere (a later `DATE_TRUNC('month', event_date)`) breaks the chain at that point and everything below.

```sql
WITH by_day AS (          -- S1: aligned
  SELECT DATE_TRUNC('day', event_ts) AS event_date, user_id, SUM(amount) AS total
  FROM events GROUP BY 1, 2
), by_user AS (           -- S2: drops event_date → carriage severed
  SELECT user_id, SUM(total) AS lifetime FROM by_day GROUP BY user_id
)
SELECT * FROM by_user     -- no downstream scope can be aligned; model NotAligned
```

**Operator × rule table.**

| Operator | Preserves model alignment iff | Counterexample when the side condition fails |
|---|---|---|
| Projection / filter | `p` (or grid-refining transform) is carried in the SELECT list | drop `event_date` → §4 CTE example |
| `GROUP BY` | keys ⊇ {carried `p`-expression} up to grid refinement | §3.2 lifetime-per-user; §3.3 monthly rollup |
| `GROUPING SETS`/`ROLLUP`/`CUBE` | *every* grouping set contains it | §3.4 grand-total row with `NULL` address |
| `DISTINCT` | `p` projected in this scope | §3.5 `DISTINCT user_id` |
| Window `OVER` | every window's `PARTITION BY` ⊇ {`p`} — or the frame is bounded (then it is a reach obligation, not alignment) | §3.6 `OVER ()`, per-user `ROW_NUMBER` |
| Join | output projects one side's `p`; other clocked side either equi-joined on `p` or covered by a derivable reach bound; unclocked side read in full | §3.8 `txn_id` join reading a 06-01 payment |
| `UNION ALL` | every branch aligned **and** every branch's output column at `p`'s position is that branch's own partition value at grain ≤ `g_p` | below — the re-bucketing branch |
| `UNION`/`INTERSECT`/`EXCEPT` (± `ALL`) | `p` projected in every branch **at exactly grain `g_p`** in each | §3.9 day-vs-month `EXCEPT` |
| Correlated subquery | correlation predicate pins the inner scan to the outer partition (equality on `p` at compatible grain) | §3.10 refunds `EXISTS` |
| `LIMIT` (+`ORDER BY`) | never (only the `QUALIFY`-partition-local respelling) | §3.11 |
| Aggregate-over-aggregate | inner aligned and outer keys ⊇ carried `p` at grain ≤ `g_p` — outer coarser breaks | below |

**UNION ALL of two aligned CTEs — the motivating example, worked.** Both branches internally aligned; the union re-buckets:

```sql
WITH a AS (   -- aligned: GROUP BY its own day grain
  SELECT DATE_TRUNC('day', event_ts) AS event_date, SUM(amount) AS total
  FROM events GROUP BY 1
), b AS (     -- ALSO internally aligned — but at month grain
  SELECT DATE_TRUNC('month', refund_ts) AS event_date, -SUM(amount) AS total
  FROM refunds GROUP BY 1
)
SELECT event_date, SUM(total) AS net FROM (
  SELECT * FROM a UNION ALL SELECT * FROM b
) GROUP BY event_date
```

With `refunds = {(2026-07-15, 3.00)}`: branch `b` emits `(2026-07-01, -3.00)` — a row whose `event_date` is **not** its input rows' day-partition. The outer `GROUP BY event_date` is textually aligned, and each branch is internally aligned, yet the model is broken: recomputing partition 07-01 must re-derive `b`'s contribution, which requires scanning refunds from *all of July* — outside every derivable 07-01-window bound. Full refresh: `net(07-01) = 10.00 − 3.00 = 7.00`; a 07-01 run with refunds clamped to 07-01 writes `10.00`. The union rule's side condition — each branch's output column must be *that branch's* partition value at grain ≤ `g_p` — is exactly what catches this: `b`'s `event_date` is month-grained (`g_k = month > g_p = day`). Per-branch alignment does not compose through a shared output column unless the column means the same (grain-compatible) thing in every branch.

**Aggregate over aggregate.** Inner `GROUP BY (event_date, user_id)` aligned; outer `GROUP BY event_date` over it — both aligned, composes (daily actives from daily-user grain). Outer `GROUP BY DATE_TRUNC('month', event_date)` — outer scope coarser, breaks with §3.3's arithmetic lifted one level: the monthly group unions ~30 partition-pinned inner rows. Note the inner aggregation being aligned buys nothing for an unaligned outer scope — the meet is not redeemed by any aligned sub-tree.

**Roll-up polarity, restated per consumer.** Batched needs the meet: *all* scopes aligned (or individually excused by a reach proof / override) before the wholesale-rewrite plan is admitted; that composed verdict is the batch-safety roll-up and lives in `batched_models.md`, not here. Keyed wants the *list*: every `NotAligned` scope, with its reason and its keys, because those are the fold points the column-family classifier assigns combiners to. The property layer should therefore expose the per-scope verdict *set* (scope id → verdict), not only a pre-folded boolean — folding is the consumer's move.

## 5. Static provability vs declaration

- **Literal containment is fully derivable** from the AST: resolve the scope's own keys (including `GROUP BY 1` ordinals against the scope's own SELECT list — already implemented), find the partition expression among the scope's projections, test membership. No declaration needed, no engine consulted.
- **Transformed containment needs the clock declaration but nothing else.** Deciding whether `DATE_TRUNC('hour', ts)` in the keys refines the partition grid requires knowing `g_p` — which comes from the `timeseries:` declaration plus the independently-derived `g_part` of the partition projection (`batched_models.md` already derives `g_part` for the `g_run ≥ g_part` check; alignment should consume the same derivation, not re-derive). The transform classifier is a closed list — truncations (`DATE_TRUNC`, `time_bucket` with grid-anchored buckets, `CAST(ts AS DATE)`), constant shifts (already recognised by the monotonicity trace), and the grid-refinement judgment on the closed granularity enum with the `week`⋢`month` caveat from §1. Anything outside the list is `NotAligned{reason naming the construct}` — fail-closed, matching the fail-loud discipline.
- **Functional-dependency widening is a legitimate future escape hatch, with guard rails already modelled.** `GROUP BY session_id` where `session_id → event_date` is semantically aligned but unprovable from the SQL alone. The existing `functional_dependency` declaration machinery (`analysis/functional_dependency.rs`: declaration may widen only the undecidable case, never override a positive disproof) is the right template if this pain materialises; per the derive-don't-declare stance it should not be added speculatively.
- **Alignment itself is never declared.** It is a pure SQL fact; the only declared inputs are the clock (`partition_column`/`granularity`) and, on the consumer side, the `safety_overrides.allow_*` escapes — which override the *admission*, not the fact (the property still reports `NotAligned`; the mode chooses to proceed).

## 6. Implementation gaps (as of this worktree)

What exists (`crates/smelt-logical/src/analysis/mod.rs`):

- `PartitionAlignment` enum with reasoned `NotAligned`; `scope_group_by_alignment` (per-scope, AST-based, ordinal-resolving via `resolve_scope_group_by`, judged against the scope's **own** projection of the partition expression); `scope_distinct_alignment` (projection-containment, per the whole-row argument); `scope_over_alignment` (AST-based, every window in the scope must have a `PARTITION BY` containing `p`, fail-closed on missing `PARTITION BY`). All three are per-scope and correctly refuse to look at outer scopes (unit-tested: `test_scope_over_alignment_is_per_scope_not_outer`).
- Consumers wired (`rules/incremental.rs`): outer-scope `GROUP BY` containment; `HAVING` admission via `check_having_alignment_all_scopes`; `DISTINCT` admission via `check_distinct_alignment_all_scopes` — both walked across the `UNION` branch chain so a later branch is judged by its own keys.

Gaps, in decreasing severity:

1. **The window-`OVER` and `LIMIT` admission scans are still uppercase-substring based.** `incremental.rs` §2a gates on `upper_sql.contains("OVER(") || upper_sql.contains("OVER (")` and dispatches to `find_inadmissible_over` — a hand-rolled byte scanner over the uppercased SQL (`next_over_position`, `find_partition_by_keys_end`, balanced-paren extraction) — despite the AST-based `scope_over_alignment` existing and being tested. `model_properties.md` §Known Divergences records this exactly: "The admission scans in `incremental.rs` have not yet been rewired onto the AST-based `PartitionAlignment` signal — that consumer wiring is a mode-composition concern." Substring hazards: `OVER` inside a string literal or identifier (the scanner does check word boundaries but not literal contexts), and the `PARTITION BY` key-list trimming being textual.
2. **No roll-up across nested CTE / derived-table scopes.** The HAVING/DISTINCT walks cover the outer SELECT and its UNION chain only. CTE bodies are *deliberately not* gated by the `allow_subqueries` structural check (the "CTE bypass" comment in §2d, kept to avoid regressing `web_analytics/sessions.sql`), so a `HAVING` or `DISTINCT` living inside a CTE body is judged by **no one** — neither the alignment walk (doesn't descend) nor the subquery gate (exempts CTEs). This is the one place the current wiring is optimistic rather than fail-closed. Derived tables in FROM are refused wholesale by §2d rather than judged — fail-closed but coarse (an aligned subquery scope needs the override).
3. **Transformed-column containment is textual, not granularity-aware.** `scope_group_by_alignment` compares the GROUP BY key text against the projected partition expression text. Exact-expression and ordinal spellings work; a grid-refining but textually different key (`GROUP BY DATE_TRUNC('hour', event_ts)` on a day-partitioned model, §3.3) is `NotAligned` — sound but incomplete. No connection yet to the `g_part` derivation or the granularity enum; the `week`⋢`month` grid subtlety is moot until that lands but must be encoded when it does. Textual comparison is also whitespace/case-fragile (`date_trunc('day',event_ts)` vs the projected spelling).
4. **Global-aggregate scopes skip the outer check.** The outer containment check runs only `if select.group_by_clause().is_some()`; a no-GROUP-BY model is assumed per-row. A global aggregate (`SELECT MAX(event_date) AS event_date, COUNT(*) …`) has no GROUP BY, projects the partition alias, and slides past this check — other gates (the monotonicity trace on the projection, which would classify an aggregate-valued partition column) are relied on to catch it, but no test pins that.
5. **Construct coverage.** GROUPING SETS/ROLLUP/CUBE have no per-set handling (whatever the parser yields for `GROUP BY ROLLUP(...)` is compared textually — at best accidentally `NotAligned`); `INTERSECT`/`EXCEPT` are unclassified (spec-acknowledged, set-op distribution covers `UNION ALL` only); correlated-subquery alignment is not judged (and the neighbouring bound-extraction bug — `_partition_col_upper` unused at `source_bounds.rs:589` — shows the correlation-pins-partition check must verify *which* column is matched, not just that an equality exists); `QUALIFY` is not judged as a scope.
6. **The verdict is consumed point-wise, not exposed as a set.** Each consumer calls the scope functions ad hoc; there is no "all scopes with verdicts" query for the keyed consumer's opposite-polarity use (§4 roll-up note) — today the keyed side has only the single `KeyedGroupByContainsPartitionColumn` outer-scope check.

## 7. Open questions

1. **Where does the branch-consistency rule for UNION ALL live?** Per-branch alignment composes only with the shared-output-column grain condition (§4). Is that side condition part of this property (a per-*model* fact about the carried column) or part of the batched roll-up? Leaning: the carriage relation — including cross-branch consistency — is property-layer (both consumers need honest provenance); the meet/list fold is mode-layer.
2. **Should grid refinement admit `finer-than-`g_p`` keys at all**, given the output row then lands in a partition its key doesn't literally name? An hour-grained group on a day-partitioned model is alignment-safe, but the *output* must still project a day-grain partition column for the DELETE to address rows — is "aligned scope + coarser projected address" a shape we ever want, or should the check demand grain-equality for the *outermost* scope and allow refinement only for inner scopes?
3. **The CTE-body blind spot (gap 2)**: close it by descending the alignment walks into `WITH` bodies (risking regressions on models relying on the bypass), or by routing CTE scopes through the coming `maintenance_plan.md` per-cell admission instead of the legacy incremental gate?
4. **Semantic alignment via functional dependency** (`GROUP BY session_id` with `session_id → event_date`): wait for concrete pain and reuse the declaration-widening pattern of `functional_dependency.rs`, or rule it out to keep alignment purely structural?
5. **Exposure shape for the keyed consumer**: a `Vec<(ScopeId, PartitionAlignment)>` per model (letting keyed enumerate fold points and batched take the meet) versus today's per-construct entry points — and if the set form lands, does the batch-safety roll-up in `batched_models.md` become a trivial fold over it?
