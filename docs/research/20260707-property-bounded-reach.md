# Property research: bounded reach / temporal locality

- **Date**: 2026-07-07
- **Status**: research
- **Property scope**: per-(model, source) — a relation-level property parameterised by source
- **Related specs**: `docs/specs/model_properties.md` (§Surface "Unified bound / reach derivation", "Maintained-window / horizon derivation", "Frame-reach taxonomy", "Interval / temporal-join detection"; §Semantics "Unified bound / reach derivation"), `docs/specs/model_maintenance.md` (§"Windowed maintenance and the horizon"), `docs/specs/model_transforms.md` (widened-scan + exact clamp, dimension-driven horizon MERGE, settled-delay/tail-rewrite), `docs/specs/sources.md` (`source_lateness`)
- **Related code**: `crates/smelt-logical/src/analysis/source_bounds.rs` (`BoundResult`, `derive_model_bounds`, `derive_and_classify_bounds`, `parse_interval`, `InjectionPoint`), `crates/smelt-logical/src/analysis/temporal.rs` (`TemporalDependency`, `EffectiveWindow`, `compute_effective_window`), `crates/smelt-logical/src/analysis/window_independence.rs` (self-edge), `crates/smelt-runtime/src/compile.rs` (`build_source_bound_map`)
- **Related research**: `docs/research/property-discovery/catalog.md` (SC-1 lateness-clamp bug, SC-2, G-05 horizon-conditional enrichment)

---

## 1. The property

For each source `S` a model `M` reads, **bounded reach** is the answer to: *to produce `M`'s output rows whose event time falls in the run window `[start, end)`, how far outside `[start, end)` must `S` be read?* The verdict domain is

```
BoundResult = Bounded { source_partition_col, before, after }   -- finite reach in seconds
            | Unbounded                                          -- provably infinite reach
            | NotDerivable                                        -- no rule for the shape
```

`Bounded{before, after}` licenses reading `S` over `[start − before, end + after)` and nothing more. The property is **per-(model, source)**: one model joining three timeseries sources carries three independent verdicts, and a single source referenced by two models carries a different verdict in each. It is a property of the *relation the model denotes*, parameterised by which input you perturb — not a property of the source alone (that would be lateness, below) and not a property of the model alone (that composition is the horizon, below).

### The before/after asymmetry

`before` and `after` are named separately, not folded into one radius, because they have asymmetric operational cost:

- **Backward reach (`before`)** is cheap: the history already exists and is settled. The widened scan simply reads `before` seconds more of it. No waiting, no rewriting.
- **Forward reach (`after`)** interacts with **watermark settling**: an output row at event time `t` needs input up to `t + after`, so at the moment the run's input watermark stands at `w`, only output up to `w − after` is *finalizable*. A nonzero `after` forces one of two physical strategies (`model_transforms.md`, "Horizon settled-delay / tail-rewrite"): delay the write until the forward margin has settled, or write provisionally and rewrite the tail slice on a later run. Backward reach never forces either.

A `LEAD(x) OVER (… RANGE BETWEEN CURRENT ROW AND INTERVAL '2 hours' FOLLOWING)` and a `LAG(x) OVER (… RANGE BETWEEN INTERVAL '2 hours' PRECEDING AND CURRENT ROW)` are mirror images algebraically but not operationally — which is exactly why `BoundResult` carries the pair rather than `max(before, after)`.

### Seconds vs Symbolic offsets

Every `INTERVAL '<value>'` literal in a bound-relevant position is folded by one shared parser (`parse_interval`) into:

- `Offset::Seconds` — seconds/minutes/hours/days/weeks: **uniform** durations, exactly convertible;
- `Offset::Symbolic` — month/year: **non-uniform** (a month is 28–31 days, a year 365–366). A symbolic offset has no fixed length in seconds without a reference date, so it cannot populate `Bounded{before, after}`. Per the fail-closed constraint (`model_properties.md` §Constraints — absence of a proof is a rejection), a symbolic literal in a bound-relevant position forces `NotDerivable` for that source rather than an approximate fixed-day guess;
- `Offset::Integer` — a signed bare-integer shift over a monotone non-temporal partition key (sequence id / watermark), the non-temporal sibling.

Why refusal rather than the ~30d/~365d approximation? Because the *same* `Bounded` value serves consumers with opposite error tolerances. For pure scan-widening, over-approximating a month as 31 days would be safe (reading extra rows is harmless — the exact clamp discards them). But `Bounded` also feeds the injection-point classification (`before == after == 0` ⇒ push the filter to the source scan with **no** outer clamp) and, prospectively, the horizon write-eligibility clamp — consumers where an approximation in the wrong direction *drops rows*. `INTERVAL '1 month'` under-approximated as 30 days silently loses the 31st day of a January lookback. Rather than carry per-consumer error polarity through the type, the derivation refuses. (Whether a `Bounded`-with-upper-bound variant should exist for scan-only consumers is Open Question 1.)

The advisory walk in `temporal.rs` deliberately *does* approximate (month → 30 days, year → 365 days) — see §4 "bare LAG/LEAD" for why the two walks are allowed to diverge.

### Reach vs horizon vs lateness — three distinct quantities

These are routinely conflated; the specs keep them apart and this doc depends on the distinction:

| Quantity | Kind | Anchored on | Direction | Who states it |
|---|---|---|---|---|
| **Computation-reach** (`before`/`after`) | per-(model, source) | the model's SQL geometry: frames, bands, shifts | both | **derived** (`derive_model_bounds`) |
| **Source-lateness** (`source_lateness:`) | per-source world-fact | ingest skew: how long after event time `t` a row stamped `t` may still *arrive* | backward (arrival) | **declared** (`sources.md`), default 0 |
| **Horizon** | per-model (batched only) | write-eligibility: the far edge of the maintained window | forward from the write window's trailing edge | **derived** from all sources' reach + join contribution + lateness; `horizon_ceiling:` is warning-only |

Computation-reach exists even for a perfectly punctual source (a 7-day rolling window forces 7 days of backward reach on data that arrived instantly). Lateness exists even for a reach-free model (a pass-through of a source whose rows arrive up to 2 hours late must re-scan 2 hours back to discover the late delta). The horizon is the *model-wide composition* of both across every source — the one number that answers "past which point may this run no longer write". The `model_properties.md` Surface table marks precisely this composition step `not-yet`: today the horizon-ceiling warning compares against the *per-source* reach; the model-wide fold does not exist.

The split matters for provability: reach is a theorem about the SQL, checkable at compile time and impossible to drift; lateness is an empirical claim about the world that smelt cannot derive and must trust (see §6).

---

## 2. Why maintenance needs it

Three consumers, in decreasing order of maturity:

**1. Widened scan + exact clamp** (`model_transforms.md`, built). Windowed maintenance evaluates the model over the scan window `[start − before − lateness, end + after)` and clamps the *write* to exactly `[start, end)` — "read the margin, never re-write it". Without a derived `Bounded{before, after}` this transform is not licensed: an `Unbounded` or `NotDerivable` source cannot have a pushed scan filter at all and either routes to per-partition/full evaluation or is refused for the mode. Getting `before` wrong is not a performance bug — an under-derived `before` computes window functions over a *truncated* frame inside the widened scan, and the wrong values flow into rows that **are** inside the write window (concrete rows in §5).

**2. Horizon clamp for `batched`** (`model_maintenance.md` §"Windowed maintenance and the horizon"). The write-eligibility clamp — past which partitions a run may no longer write — must be **derived from reach**, never trusted from a declaration, because a declared horizon smaller than the true reach silently drops rows that should have been rewritten. The declared `horizon_ceiling:` can only *warn*. So the horizon feature is downstream of this property being sound: every reach under-derivation is a future horizon under-derivation.

**3. Pushdown eligibility / injection point** (`model_properties.md` "Injection-point / pushdown-depth"). `Bounded{0, 0}` — the transparent slice — licenses pushing the event-time filter all the way to the source scan *and dropping the outer output clamp as redundant* (`InjectionPoint::Source`). Any nonzero margin keeps both layers (`OuterClamp`); `Unbounded`/`NotDerivable` forbid the pushed filter outright. Note the sharpened stakes: because a `(0,0)` verdict *removes* the outer clamp, a construct mis-derived as `(0,0)` has no safety net — this is exactly the SC-1 failure class (catalog: "`source_bounds` `(0,0)` fallback clamps the late conversion → REFUTED = bug").

`keyed` consumes reach differently — as observability plus the scan bound of the dimension-driven horizon MERGE — but never as a write clamp (`model_maintenance.md`: "only proofs prune").

---

## 3. Per-construct analysis

Throughout: source `events` has `timeseries.partition_column = event_ts` (TIMESTAMP); run window is `[start, end)`. "Reach" is stated for the named source.

### 3.1 Window frames

**`RANGE BETWEEN INTERVAL … PRECEDING/FOLLOWING`** — the one derivable frame family. The frame is stated *in event-time units against the ORDER BY column*, so its reach is literal:

```sql
SELECT user_id, event_ts,
       SUM(amount) OVER (PARTITION BY user_id ORDER BY event_ts
                         RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS spend_7d
FROM smelt.silver.events
```

Reach: `Bounded{before = 7d, after = 0}`. Symmetrically `… CURRENT ROW AND INTERVAL '2 hours' FOLLOWING` gives `Bounded{0, 2h}`. Side condition (not currently enforced — §7): the ORDER BY column must be the source's traced event-time column; a `RANGE` frame ordered by `amount` says nothing about temporal reach.

**`ROWS BETWEEN n PRECEDING …`** — `NotDerivable`. `n` rows is not `n` seconds: rows are arbitrarily sparse in event time. Concrete refutation: user 42 has events at `2026-01-01` and `2026-07-01`. `ROWS BETWEEN 1 PRECEDING AND CURRENT ROW` at the July row reaches back six months. Any finite `before` chosen in advance is defeated by a sparser user. (The advisory walk instead estimates `Periods(n)` — see LAG/LEAD below.)

**`GROUPS BETWEEN n PRECEDING …`** — `NotDerivable`, same argument one level up: a peer *group* is all rows sharing an ORDER BY value, and *gaps between successive groups* are unbounded in time (groups at Jan 1 and Jul 1, nothing between). The spec marks GROUPS "conservative" — there is no rule, so it falls into the fail-closed default rather than being positively classified.

**`UNBOUNDED PRECEDING` / `UNBOUNDED FOLLOWING`, and the default frame** — `Unbounded`. A window function with ORDER BY and *no* frame clause defaults to `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`; without ORDER BY, the whole partition. Both are unbounded backward reach — the classic running total:

```sql
SUM(amount) OVER (PARTITION BY user_id ORDER BY event_ts) AS lifetime_spend
```

The implementation checks the forward side *before* Form A/B accumulation, because `RANGE BETWEEN INTERVAL '1 day' PRECEDING AND UNBOUNDED FOLLOWING` has a derivable nonzero `before` that would otherwise accumulate into `Bounded` and mask the infinite `after` (`has_unbounded_forward_reach`).

### 3.2 Bare `LAG`/`LEAD` — the deliberate two-walk divergence

```sql
SELECT user_id, event_ts,
       LAG(amount) OVER (PARTITION BY user_id ORDER BY event_ts) AS prev_amount
FROM smelt.silver.events
```

`LAG(x, n)` reaches back exactly `n` **rows** — the sparse-rows argument above applies verbatim, so for the pushdown proof this is `NotDerivable`: a time-bounded scan may cut off the predecessor row, and `LAG` then returns the wrong row's value (or NULL) *for a row inside the write window*. There is no widening that fixes it short of `Unbounded`.

Yet `analysis/temporal.rs` classifies the same construct as `TemporalOffset::Periods(n)` — a *bounded* estimate, converted to days via the model's granularity (`LAG(x, 3)` at weekly granularity → 21 days). This is not an inconsistency; it is two different questions with different failure costs, kept as two walks on purpose (`model_properties.md` §Known Divergences):

- `source_bounds::BoundResult` answers "may I **prune the scan** to this bound?" — an under-estimate silently corrupts output ⇒ fail-closed, refuse.
- `temporal::EffectiveWindow` answers "how big should a **backfill chunk / advisory filter widening** be?" — an under-estimate costs a wasted or re-run chunk, never a wrong row, and the per-period assumption (one row per grid period, the common shape for models that use bare `LAG` over a dense daily grid) makes `Periods(n)` a good estimate ⇒ heuristic, allowed.

Collapsing them into one verdict would either lose the chunking heuristic or loosen the proof; the spec tracks unification as future work, not a silent merge.

`LAG(x, n)` **with** an explicit `RANGE` frame is redundant-but-derivable: the frame supplies the time bound the offset lacks.

### 3.3 `WHERE` with interval shifts (Form B)

```sql
SELECT e.*
FROM smelt.silver.events e, smelt.silver.model_runs r
WHERE e.event_ts >= r.run_ts - INTERVAL '3 days'
  AND e.event_ts <  r.run_ts
```

A comparison whose LHS is the source's partition column and whose RHS carries a `± INTERVAL` shift contributes reach on that side: `>= … - INTERVAL '3 days'` → `before = 3d`; `< … + INTERVAL '1 hour'` → `after = 1h`; `BETWEEN x − i1 AND x + i2` → `(i1, i2)`. Cross-column rebase is in scope — only the LHS column is required to be the partition column, the anchor expression on the right may be another table's column (the timezone-rebase pattern `b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '14 hours' AND m.event_date_local + INTERVAL '14 hours'`).

Two hardening lessons already paid for here (regression-tested):

- **SC-1 cross-source leak**: a Form-B match must be attributed to the source whose column is on the LHS. Before `lhs_column_is_partition_col`, a correlated-`EXISTS` band on `conversions.conversion_date` was *also* attributed to the unrelated `events.event_date` source — spurious widening (and, worse, its dual: the source that deserved the band got the `(0,0)` fallback).
- **SC-1b interval absorption**: the extracted expression must stop at the first depth-0 boolean connective/clause keyword (`expression_prefix`), or a zero-margin upper bound absorbs `+ INTERVAL` literals from later, unrelated predicates.

### 3.4 Interval / temporal joins

Three shapes, three verdicts:

**Bounded band** — finite reach for the non-anchor side:

```sql
-- attribution: conversions within 7 days after the click
SELECT c.click_id, cv.conversion_id
FROM smelt.silver.clicks c
JOIN smelt.silver.conversions cv
  ON cv.user_id = c.user_id
 AND cv.conversion_ts BETWEEN c.click_ts AND c.click_ts + INTERVAL '7 days'
```

With `clicks` as the anchor (the driving fact, resolved by `resolve_join_driving_fact`), producing output for clicks in `[start, end)` needs `conversions` over `[start, end + 7d)`: reach of `conversions` = `Bounded{0, 7d}`. Note the band's polarity flips with perspective: the same predicate read as "each conversion looks back to a click ≤ 7 days before it" gives `clicks` reach `Bounded{7d, 0}` relative to a conversions-anchored window — reach is anchored, not symmetric (this is the composition rule in §5.3).

**Equi-join on the time column** — zero band, pass-through:

```sql
JOIN smelt.silver.weather w ON w.day = e.event_day
```

`weather` reach = `Bounded{0, 0}` relative to the anchor: perfectly partition-local.

**Unbounded inequality join** — `Unbounded`:

```sql
JOIN smelt.silver.deploys d ON d.deploy_ts <= e.event_ts   -- "all deploys so far"
```

One-sided inequality with no closing band reaches to the beginning of `deploys`. Concrete rows: a deploy at `2020-01-01` participates in the join for an event at `2026-07-07`; no finite `before` on `deploys` retains it.

A **pure key equi-join with no time predicate at all** (the dimension lookup) is a fourth case: the joined source is not a timeseries participant of this model — it is absent from the `BoundContext` and gets *no* bound. Its maintenance story is not reach but the mutable-dimension probe / horizon-bounded MERGE (`model_transforms.md`), which consumes the *anchor's* forward reach, not the dimension's.

### 3.5 Correlated subqueries with time predicates (the attribution pattern)

```sql
SELECT e.*,
       EXISTS (SELECT 1 FROM smelt.silver.conversions c
               WHERE c.user_id = e.user_id
                 AND c.conversion_ts BETWEEN e.event_ts AND e.event_ts + INTERVAL '7 days') AS converted_7d
FROM smelt.silver.events e
```

Semantically a semi-join with a bounded band: `conversions` reach = `Bounded{0, 7d}` relative to the events anchor. Two consequences worth separating:

- **Scan side**: writing events `[start, end)` correctly requires conversions up to `end + 7d` — a forward reach, hence the settling interaction: `converted_7d` for an event at time `t` is not final until the conversions watermark passes `t + 7d`.
- **Maintenance side (the SC-1 bug)**: when a conversion at time `t_c` *arrives between runs*, the output rows it changes are events in `[t_c − 7d, t_c]` — **behind** the new run window. A window-forward run that derives `(0,0)` for `events` (the fallback when the correlated band was attributed to the wrong source) clamps those events out and the late conversion is silently never reflected. This is the catalogued refutation, and it shows why the per-(model, source) framing matters: the *conversions* delta drives a write into the *events*-addressed past, at a distance equal to the band — exactly the shape the dimension-driven horizon-bounded MERGE (`model_transforms.md`) exists for.

A correlated subquery with **no** time bound (`EXISTS (… WHERE c.user_id = e.user_id)`) is `Unbounded` for the inner source, same as the inequality join.

### 3.6 Aggregates and `GROUP BY` — not reach-neutral when bucketing

A `GROUP BY` whose key set contains the partition column **untransformed** is reach-neutral: the output row at time `t` is a fold of exactly the input rows at time `t`.

`GROUP BY date_trunc(...)` is **not** neutral — bucketing adds reach up to the bucket width:

```sql
SELECT date_trunc('week', event_ts) AS week, SUM(amount) AS total
FROM smelt.silver.events
GROUP BY 1
```

The output row stamped `week = 2026-06-29` folds inputs from `[2026-06-29, 2026-07-06)`. A run window `[2026-07-01, 2026-07-02)` (daily cadence, mid-bucket) that scans only its own window computes `total` for week `06-29` from one day of data and — under DELETE+INSERT of the touched bucket — *overwrites the previously correct partial* with a worse one. Correct maintenance needs the scan (and the write) expanded to bucket boundaries: reach up to `(width − ε)` backward from `start` and forward to the bucket edge past `end` — i.e. `Bounded{≈1 week, ≈1 week}` in the unaligned case, and exactly `{0, 0}` when the run granularity is bucket-aligned. That alignment fact is precisely the `PartitionAlignment` proof (`model_properties.md`), which is the correct discharge condition; note that `date_trunc('month', …)` buckets have symbolic width and force `NotDerivable` by the same rule as symbolic intervals. Aggregates *themselves* (SUM/AVG/…) contribute nothing here — reach is a property of the grouping geometry, not the combiner; the combiner's algebra is a separate discriminant.

Today's derivation treats `date_trunc` GROUP BY as zero-reach (`temporal.rs` `test_simple_group_by_no_dependency` asserts exactly this) — sound only under bucket-aligned run windows, and nothing checks that. See §7.

### 3.7 `DISTINCT`

Reach-neutral, with a side condition. `SELECT DISTINCT user_id, event_ts, amount …`: duplicates by definition agree on *every* projected column, including `event_ts`, so whether a row at time `t` survives dedup depends only on rows at time `t` — no cross-window dependence. The side condition: the event-time column must be projected untransformed. `SELECT DISTINCT user_id, date_trunc('week', event_ts) …` inherits the bucketing reach of §3.6; a `DISTINCT` that drops the time column entirely has no timeseries output to window (StaticSeed territory), not a reach question.

### 3.8 Set operations

- **`UNION ALL`**: each output row comes from exactly one branch, so per source the reach is the **max** across branches in which that source appears (branch-wise trace, `model_properties.md` "Set-operation distribution"). Max is *exact* here, not conservative — no interaction between branches exists to sum over.
- **`UNION` (dedup)**, **`INTERSECT`**, **`EXCEPT`**: set ops compare **full rows**, so the event-time column is always part of the compared tuple, and membership of an output row at time `t` depends only on both branches' rows at time `t` — reach = max of branch reaches, by the same argument as `DISTINCT`, with the same untransformed-time side condition per branch. Example of the side condition biting: `SELECT user_id FROM a EXCEPT SELECT user_id FROM b` has no time column in the compared tuple; whether `user_id = 7` appears depends on *all* of `b`'s history — `Unbounded` for `b`. The spec classifies only `UNION ALL` today; `INTERSECT`/`EXCEPT` are unclassified and fall to the fail-closed default.

### 3.9 `ORDER BY` / `LIMIT`

A top-level `ORDER BY` alone is reach-neutral (it changes presentation order, not set membership; determinacy is a separate concern). A **global `LIMIT` is `Unbounded`**: membership in the top-`n` by some order depends on every other row in history.

```sql
SELECT * FROM smelt.silver.events ORDER BY amount DESC LIMIT 10
```

Rows: history holds amounts `{100, 90, …, 20}` (ten rows) and today's run window adds `amount = 95`. The new row *enters* the top-10 and evicts the historical `20` — an output-set change **outside** any finite window around the run. No `Bounded{before, after}` covers eviction. (A `LIMIT` *inside* a partition-aligned scope — `QUALIFY row_number() OVER (PARTITION BY event_day …) <= 10` — is partition-local and neutral; it is the global form that is unbounded.)

### 3.10 Self-joins

Nothing special: the same relation appears as anchor and non-anchor, and the band rule applies —

```sql
SELECT a.event_ts, a.amount, b.amount AS amount_prev_day
FROM smelt.silver.events a
JOIN smelt.silver.events b
  ON b.user_id = a.user_id AND b.event_ts = a.event_ts - INTERVAL '1 day'
```

`events` reach = merge of its anchor role `(0,0)` and its shifted role `(1d, 0)` = `Bounded{1d, 0}` (union semantics: `BoundResult::merge` takes componentwise max for the *same* source, which is exactly right for two roles of one relation). An inequality self-join (`b.event_ts <= a.event_ts`) is `Unbounded` per §3.4.

### 3.11 Self-referencing models (reading own prior output)

A model whose SQL refs its own name is a self-edge (`window_independence`). Reach applies with the model's own output as the source: a **backward-bounded** self-read (`prev.event_ts >= cur.event_ts - INTERVAL '1 day'`) yields `Ordered` execution — runs must proceed in window order, each reading `k` back into its own committed output; the stored output *is* the state that stands in for deeper history, so the per-run scan stays bounded even though the transitive data dependence is unbounded (that is the whole point of stateful maintenance). A forward or unbounded self-read is refused fail-closed — there is no execution order under which the needed output already exists.

### 3.12 Session windows / gap-based constructs — inherently unbounded

```sql
-- session id: new session when gap from previous event > 30 minutes
SELECT user_id, event_ts,
       SUM(CASE WHEN event_ts - LAG(event_ts) OVER w > INTERVAL '30 minutes'
                OR LAG(event_ts) OVER w IS NULL THEN 1 ELSE 0 END)
           OVER (PARTITION BY user_id ORDER BY event_ts) AS session_id
FROM smelt.silver.events
WINDOW w AS (PARTITION BY user_id ORDER BY event_ts)
```

The gap parameter (30 min) bounds the *step*, not the *reach*: a session extends as long as consecutive events keep arriving within the gap, so the session an in-window row belongs to can have started arbitrarily long ago. Concrete chain: events every 20 minutes from `2026-01-01` to `2026-07-07` form **one** session; the `session_id` of today's row depends on a row six months back. Additionally the outer `SUM … OVER (ORDER BY …)` is itself a running total (`Unbounded` already by §3.1). Sessionization is inherently `Unbounded` for pushdown; the escape hatches are structural (a declared max-session-length world-fact — Open Question 4) or stateful (the self-referencing form of §3.11, carrying open sessions forward as state).

### 3.13 Non-equi joins generally

Any join predicate on the time columns that fails to close a band on both sides is `Unbounded`; any band expressed with symbolic intervals is `NotDerivable`; a band on *non*-time columns (`b.amount BETWEEN a.amount - 5 AND a.amount + 5`) says nothing about time and contributes nothing (the time reach is then whatever other predicates supply — with none, the joined timeseries source is unbounded within the anchor's window join, i.e. `(0,0)` only if a separate time predicate ties it down; absent one, fail-closed).

---

## 4. Composition algebra

This is the property's real strength: **reach composes through the operator tree**, mechanically, under conditions that are themselves statically checkable. The composed object is, per source `S`, a pair `r_S(R) = (before, after)` in the extended domain `D = (ℝ≥0 × ℝ≥0) ∪ {Unbounded} ∪ {NotDerivable}`, read as: *to compute relation `R`'s rows with (traced) event time in `[s, e)`, read `S` over `[s − before, e + after)`.*

### 4.1 Series composition ADDS — window-over-window, carefully

The single most important rule, and the one the current implementation gets wrong (§7). Stacked operators each with their own reach **add**, they do not max:

```sql
WITH daily AS (                        -- inner: 7-day rolling sum
  SELECT user_id, event_ts,
         SUM(amount) OVER (PARTITION BY user_id ORDER BY event_ts
                           RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS spend_7d
  FROM smelt.silver.events
)
SELECT user_id, event_ts,              -- outer: 7-day rolling avg of the rolling sum
       AVG(spend_7d) OVER (PARTITION BY user_id ORDER BY event_ts
                           RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS smooth
FROM daily
```

Reason it through concrete rows. Take one user with events: day 1 → `amount 100`, day 4 → `10`, day 10 → `1`; run window `[day 10, day 11)`.

- **True values.** `spend_7d(day 4)` covers days `−3..4` ⊇ {day 1, day 4} = **110**. `spend_7d(day 10)` covers days `3..10` ⊇ {day 4, day 10} = **11** (day 1 is out of its 7-day frame). `smooth(day 10)` averages `spend_7d` over `daily` rows in days `3..10` = avg(110, 11) = **60.5**.
- **Under max-composition** (`before = max(7d, 7d) = 7d`), the widened scan reads events in `[day 3, day 11)` = {day 4, day 10}. Inside that scan, `spend_7d(day 4)` is computed over a truncated frame = **10** (day 1 is missing from the scan, not from the frame). `smooth(day 10)` = avg(10, 11) = **10.5** ≠ 60.5 — and day 10 is squarely inside the write window, so the wrong value is *written*.
- **Under sum-composition** (`before = 7d + 7d = 14d`), the scan reads `[day −4, day 11)` ⊇ {day 1, day 4, day 10}; every `daily` row the outer frame touches is computed over its full frame; `smooth(day 10)` = 60.5. Correct.

The general rule: output row at `t` reads inner rows in `[t − p_out, t + f_out]`; each inner row at `u` reads source in `[u − p_in, u + f_in]`; the union over `u` is `[t − p_out − p_in, t + f_out + f_in]`. **Series: `(b, a) ∘ (b', a') = (b + b', a + a')`.** Depth-`n` stacks of 7d windows reach `7n` days — reach grows linearly with pipeline depth, which is itself a design signal (deep window pipelines quietly become expensive to maintain even when every stage looks cheap).

**Side condition — the traced time column must be the SAME through the stack.** The rule is stated against one clock. If the outer window orders by a *different* time column than the one the inner reach was derived against, the sum is meaningless. Breaking example: inner reach derived against `event_ts`, outer window `ORDER BY ingest_ts RANGE BETWEEN INTERVAL '7 days' PRECEDING …` — 7 days of *ingest* time corresponds to an unbounded span of *event* time (a backfill batch ingested in one hour can carry years of events). The composed verdict must be `NotDerivable` unless the two clocks are related by a derived/declared bound (which is exactly what `source_lateness` is: a declared `|ingest_ts − event_ts|` bound — a legitimate future bridge, not a today rule). A **constant** column shift is fine and is what the monotonicity trace's offset folding handles: projecting `event_ts + INTERVAL '2 hours' AS local_ts` and windowing over `local_ts` composes as a translation — it shifts the anchor, adding the constant to one side of the reach and subtracting from the other, never changing finiteness.

### 4.2 Parallel composition MAXES — UNION ALL branches

Per source `S`, `r_S(UNION ALL(R1, …, Rn)) = max_i r_S(Ri)` componentwise (branches absent `S` contribute nothing). Exact, not conservative: each output row is produced by exactly one branch, so the scan for `S` must satisfy the hungriest branch and no interaction term exists. The same max rule extends to `UNION`/`INTERSECT`/`EXCEPT` **only** under the projected-untransformed-time side condition of §3.8 — the counterexample there (`EXCEPT` over `user_id` alone) breaks it to `Unbounded`.

Series-add and parallel-max, with `Unbounded` as the additive absorber and `NotDerivable` absorbing everything (matching `BoundResult::merge`'s precedence: `NotDerivable > Unbounded > Bounded`), make the algebra a tropical-flavoured semiring over `D`. That structure is why a fold over the operator tree is the natural implementation shape.

### 4.3 Join contribution — bands add along the path

A join contributes the band as a *translation of the anchor's window onto the joined source's clock*. If the anchor relation `A` has reach `r_S(A) = (b, a)` for source `S`, and `B` joins to `A` with band `B.ts ∈ [A.ts − β, A.ts + α]`, then for a source `S'` reached *through* `B` with `r_{S'}(B) = (b', a')`:

```
r_{S'}(A ⋈ B) = (β + b', α + a')      -- relative to the anchor's output window
```

and **chaining adds along the join path**: `A ⋈[±1d] B ⋈[±2d] C` gives `C` a reach of `(3d, 3d)` relative to `A`-anchored output. Counterexample for the tempting "max of the two bands" rule, with rows: `A` row at day 10; `B` matches at `A.ts − 1d` = day 9; `C` matches at `B.ts − 2d` = day 7. A scan of `C` widened by only `max(1, 2) = 2` days from the day-10 window start reads `[day 8, …)` and misses the day-7 `C` row that genuinely joins into the day-10 output. Bands accumulate exactly like series windows — a join is series composition through the ON clause.

Equi-join on time = band `(0, 0)` = pass-through. The anchor itself must be unique (driving-fact resolution: exactly one traceable input, else fail-closed) — with two candidate anchors the "relative to the run window" anchor is ambiguous and no sound translation exists.

### 4.4 Aggregate-then-window vs window-then-aggregate

Both orders compose by the same two rules; they differ only in which widths appear:

- **Aggregate (bucket) then window**: `GROUP BY date_trunc('day', ts)` (bucket width `w = 1d`, reach `(w, w)` unaligned / `(0,0)` aligned per §3.6) then `RANGE '7 days' PRECEDING` over the bucket timestamps → series sum = `(7d + w, w)` unaligned, `(7d, 0)` aligned. This is the common "roll up to daily grain, then rolling week" shape, and note the window now ranges over *bucket* timestamps — still the same traced column family only because `date_trunc` is a monotone bucketing of it; a window over buckets of a *symbolic* width (`date_trunc('month', …)`) poisons the stack to `NotDerivable` at the bucketing step.
- **Window then aggregate**: `RANGE '7 days'` per row, then `GROUP BY date_trunc('day', ts)` → `(7d, 0)` series-plus-bucket = `(7d + w, w)` unaligned. Same totals, same rules; the orders are not semantically interchangeable (avg-of-sums ≠ sum-of-avgs) but their *reach arithmetic* is uniform, which is what makes a mechanical fold trustworthy.

### 4.5 The operator × rule table

`(b, a)` = componentwise reach per source; `⊥` = `NotDerivable`; `∞` = `Unbounded`; both absorb (⊥ dominates ∞).

| Operator | Output reach per source `S` | Side conditions | Counterexample when the condition fails |
|---|---|---|---|
| Scan of `S` | `(0, 0)` | `S` has a timeseries clock | clockless source: no reach question, snapshot-diff path |
| WHERE, row-local predicate | input reach (identity) | predicate reads only current row's columns | predicate is `EXISTS(…)` → see correlated rows |
| WHERE, interval band `col θ x ± INTERVAL k` | input `+ (k_−, k_+)` on the LHS column's source | LHS is that source's partition column; `k` uniform | SC-1: band attributed to the wrong source both over-widens it and leaves the right one at `(0,0)` |
| Projection, time col untouched or `+ const` uniform shift | identity (shift translates the anchor) | shift is a uniform-`Seconds` constant | `event_ts + INTERVAL '1 month'` → ⊥ (symbolic); `ts1 - ts2` two-column arithmetic → ⊥ (no single clock) |
| Window, `RANGE INTERVAL p PRECEDING / f FOLLOWING` | input `+ (p, f)` — **series-add** | ORDER BY column = the traced time column; `p`,`f` uniform | ordered by `ingest_ts` while reach is in `event_ts`: 7d of ingest ≠ any bound of event time → ⊥ |
| Window, `ROWS n` / `GROUPS n` / bare `LAG`/`LEAD(n)` | ⊥ (pushdown); `Periods(n)` (advisory chunking only) | — | two rows six months apart defeat any fixed seconds bound |
| Window, no frame + ORDER BY / `UNBOUNDED` bound | ∞ on that side | — | running total needs full history |
| GROUP BY containing time col untransformed | identity | — | — |
| GROUP BY `date_trunc(g, ts)` | input `+ (w_g, w_g)`; `(0,0)` if run window bucket-aligned | `w_g` uniform (`g` ≠ month/quarter/year); alignment = `PartitionAlignment` proof | mid-bucket daily run over weekly buckets overwrites a correct partial with a truncated one (§3.6) |
| Aggregate combiner (SUM/AVG/…) as such | neutral | — | (algebra is a separate discriminant; reach lives in the grouping geometry) |
| DISTINCT | identity | time column projected untransformed | time col dropped/truncated → bucketing rule or ⊥ |
| UNION ALL | `max_i` over branches — **parallel-max**, exact | — | — |
| UNION / INTERSECT / EXCEPT | `max_i` over branches | each branch projects its time column untransformed (it is then in the compared tuple) | `EXCEPT` over `user_id` only: membership depends on all of the second branch's history → ∞ |
| ORDER BY (no LIMIT) | identity | — | — |
| Global LIMIT | ∞ | — | new row evicts a years-old row from the top-10 (§3.9) |
| Partition-aligned QUALIFY/LIMIT | identity | partition key ⊇ run partition column | global `row_number()` is the LIMIT case |
| Join, band `[−β, +α]` on time vs anchor | non-anchor subtree's reach `+ (β, α)` — series-add along the path | exactly one resolved anchor; band closed on both sides; uniform | chained `1d`,`2d` bands need `3d`, not `max = 2d` (§4.3); one-sided band → ∞ |
| Join, time-equi | non-anchor reach `+ (0, 0)` | as above | — |
| Join, key-only (dimension) | no time reach derived (lookup; mutable-dimension machinery instead) | — | treating a mutable dimension as reach-free *for delta purposes* is SC-2's cousin |
| Correlated subquery, bounded time band | inner source `+ (β, α)` (semi-join rule) | band closed, uniform | unbounded correlation → ∞ |
| Self-reference, backward-bounded `k` | `(k, 0)` vs own output + `Ordered` execution | read strictly backward | forward self-read: no valid execution order, refused |
| Session / gap-based | ∞ | — | 20-minute chain spanning six months (§3.12) |

### 4.6 Is reach compositional? Yes — precisely when the clock survives

The verdict: **reach is compositional** — the output reach of every operator above is a function of (i) its inputs' reaches and (ii) the operator's own locally-readable reach contribution, with no whole-program analysis required. That is the property's strength: it turns "how far must I read?" into a bottom-up fold with two combinators (series-add, parallel-max) and two absorbers. The conditions, stated once:

1. **One traced clock per stack.** Every reach-bearing operator (frame ORDER BY, band predicate, bucketing key, shift) must act on the *same* event-time column family — the source's partition column threaded through the tree, possibly under monotone uniform transforms (constant `Seconds` shifts; uniform-width `date_trunc`). The moment two unrelated time columns meet (ingest vs event, `ts1 − ts2` arithmetic), the fold's unit of account is gone → `NotDerivable`. This is the same single-column discipline the event-time monotonicity trace already enforces, and it is why the two proofs share the interval parser and should share the trace.
2. **Uniform offsets only.** Symbolic (month/year) widths poison the arithmetic (`b + Symbolic` has no value) → `NotDerivable`, at the first symbolic contribution.
3. **Anchored joins.** Series-adding a band requires knowing which side is the anchor; ambiguous anchors fail closed before composition begins.
4. **Set-op / DISTINCT time-visibility.** Parallel-max over comparing operators needs the clock inside the compared tuple.

Under 1–4 the fold is exact (not merely sound) for every row of the table except the deliberately-conservative ones (ROWS/GROUPS, unaligned buckets' `±w` which over-approximates partial alignment).

---

## 5. Static provability vs declaration

The property is split along the derive-where-decidable / declare-where-not line (`model_properties.md` §Design), and each half has the right failure mode:

- **Computation-reach is derived, and only derived.** It is a theorem about the SQL text — there is nothing for a modeller to know that the analyzer cannot see, so a declaration could only introduce drift (`feedback_derive_dont_declare`: window properties declared in YAML rot when the SQL changes). Fail-closed on everything outside the rule table: `ROWS`/`GROUPS`/bare `LAG`/`LEAD` → `NotDerivable`; symbolic intervals anywhere in bound position → `NotDerivable`; default frames / one-sided bands / global LIMIT / gap chains → `Unbounded`. Both reject verdicts forbid the pushed filter and force the outer clamp; neither is ever approximated past.
- **Source-lateness is declared, and only declared.** How late the upstream pipeline delivers is a world-fact about systems smelt does not control; deriving it is impossible and guessing violates fail-loud. It defaults to zero (absent = punctual), lives on the source (`sources.md`), and is the *only* declared term in the reach split.
- **`horizon_ceiling:` is warning-only** — the unique declaration in the `model_properties.md` table that widens *nothing*: the clamp always uses the derived reach; the ceiling only licenses a compile-time warning when the derived value exceeds it (`check_horizon_ceiling`). The asymmetry is deliberate: an over-derived horizon costs scan width; an under-*declared* horizon trusted by the clamp drops in-reach rows silently, the one thing the equivalence invariant forbids.
- **No declared escape hatch for reach exists today**, and any future one must follow the widening law: it may relax only a `NotDerivable` the proof could not decide (e.g. a declared max-session-length turning §3.12 finite, or a declared ingest-vs-event skew bridging the two-clock case), never a positively-derived `Unbounded`.

---

## 6. Implementation gaps

Read against §4's algebra, `source_bounds.rs` today is a **flat per-source text scan, not a compositional fold**. `derive_bound_for_source(sql, partition_col)` uppercases the *entire* statement, collects every Form-A frame and every LHS-scoped Form-B band found anywhere in it, and combines them with `BoundResult::merge` — **componentwise max**. Specifically:

1. **No series-add: nested/stacked reach is under-derived.** The §4.1 window-over-window model derives `before = max(7d, 7d) = 7d` where correctness needs `14d`. Nothing in the walk knows one frame consumes the other's output — there is no operator tree at all. Whether this is *reachable* unsoundness today depends on the admission gates in `rules/incremental.rs` (the window-`OVER` admission scans may refuse multi-window models before the bound is consumed); that interaction is exactly what a regression test should pin down before the widened-scan transform trusts `Bounded` values from stacked-CTE models. `merge`'s max is the right operation for its documented purpose — union of *parallel* constraints on one source (two roles of a self-join, two UNION ALL branches) — it is being applied to series cases it was never sound for.
2. **No join-path chaining.** A Form-B band is attributed to its LHS column's source with the anchor implicitly assumed to be the run clock. A band whose anchor expression is itself a shifted/derived column of another reach-bearing source (§4.3's `A ⋈ B ⋈ C`) contributes only its own hop; the path sum is never formed. Same root cause as (1).
3. **Form A is not source-scoped.** Unlike Form B (post-SC-1 `lhs_column_is_partition_col`), *any* `RANGE BETWEEN INTERVAL` frame anywhere in the statement contributes to **every** source in the `BoundContext` (the code comment calls it a heuristic). Over-widening is the safe direction, but it also means a frame over source A's clock inflates source B's scan, and — more importantly — it forfeits the per-source precision the horizon composition (`not-yet` row) will need. The ORDER-BY-column ≡ traced-clock side condition of §4.5 is not checked at all.
4. **Reach and lateness combine by `max`, not `+`.** `compute_effective_window` sets `effective_lookback = max(ast_lookback, data_latency)`. But a row arriving `L` late lands in a partition up to `L` behind the run window, and *rewriting that partition* needs the computation's own `before` behind *it*: the sound scan lookback is `L + before`, and the two quantities max'd only coincide when one is zero. The advisory path under-widens whenever both are nonzero (e.g. 2h lateness + 7d frame → 7d, missing the frame tail of the late partition). The `BoundResult` path never combines them at all — `source_lateness` is parsed (`smelt-core/src/sources.rs`) but not folded into `derive_model_bounds`'s output; the spec's "splits computation-reach from declared source-lateness" is currently a split with only one populated half at the proof layer.
5. **Bucketing GROUP BY derives zero reach unconditionally.** `test_simple_group_by_no_dependency` pins `date_trunc('day', …) GROUP BY` at zero lookback/lookahead, with no `PartitionAlignment` discharge of the §3.6 side condition. Sound iff run windows are always bucket-aligned; nothing enforces or even records that assumption.
6. **Whole-statement symbolic refusal is over-broad by design.** One `INTERVAL '1 month'` anywhere — even in a projected literal that touches no bound position — refuses the source (`has_symbolic_interval_in_bound_position` deliberately scans everything). Fail-closed and documented, but it converts a precision limitation into a usability cliff for calendar-aware models.
7. **The two-walk divergence is deliberate and should stay** (bare `LAG` = `NotDerivable` for pushdown vs `Periods(n)` for chunking; month = refusal vs ≈30d advisory), but the walks now share only the interval parser. As (1)–(3) push `source_bounds` toward an AST fold, the shared substrate should grow (frame extraction, scope attribution) while the *fail-closure policies* remain distinct — the spec's "not silently merged".
8. **Coverage cliffs inherited from text-scanning**: `INTERSECT`/`EXCEPT` unclassified (fail-closed, fine but coarse); session/gap shapes only refused via their component constructs (the running-total default frame), not recognised as a pattern that could carry a targeted diagnostic; `GROUPS` refused by omission rather than classification.

The through-line: every gap except (4) and (6) is the same missing piece — an operator-tree fold with series-add and parallel-max, replacing one flat max over textual matches. The algebra in §4 is the specification of that fold.

## 7. Open questions

1. **Safe upper bounds for symbolic offsets, per consumer?** For scan-widening only, `INTERVAL '1 month'` ≤ 31 days is a sound over-approximation; the exact clamp discards the excess. Should `BoundResult` grow a `BoundedAtMost` variant admissible for scan widening but refused by injection-point/horizon consumers, or is the two-consumer error-polarity split too easy to get wrong to be worth the cliff it removes?
2. **What keeps flat-max sound today, and what is the migration path to the fold?** Enumerate the admission gates (window-`OVER` scans, single-anchor resolution) that currently exclude stacked-window/join-chained models from consuming `Bounded` values; pin them with regression tests; then rebuild `derive_bound_for_source` as the §4.5 fold over the parsed tree, with the clock-identity side condition checked via the existing monotonicity trace.
3. **`max` vs `+` for lateness × reach** (gap 4): confirm with a concrete refutation harness case (SC-1-style, late row + 7d frame) whether `compute_effective_window`'s `max` produces a wrong widened DELETE/filter range in a real incremental run, and whether the fix belongs in the advisory path, the `BoundResult` split, or both.
4. **Declared max-session-length as a widening world-fact?** Sessionization is the highest-value inherently-`Unbounded` shape. A declared cap ("no session exceeds 24h") would license `Bounded{24h, 0}` — but per the widening law it must widen a `NotDerivable`/pattern-recognised case, and it needs a runtime fail-loud check (a session touching the cap aborts rather than truncates). Is that check cheaply expressible?
5. **The model-wide horizon fold across differently-anchored sources** (the spec's `not-yet` row): per-source reaches are each anchored on their own clock via the join translation of §4.3 — what is the composition when the driving anchor's clock and a banded source's clock have independent lateness declarations, and does the horizon need to be a per-source vector rather than one scalar?
