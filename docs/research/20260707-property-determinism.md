# Property research: determinism (run vs row) and nondeterminism taint

**Date:** 2026-07-07
**Status:** research
**Related specs:** `docs/specs/model_properties.md` §"Determinism (run vs row) and the nondeterminism predicate"; `docs/specs/model_maintenance.md` §"The equivalence invariant"; `docs/specs/batched_models.md` §"Non-determinism and the payload rule"; `docs/specs/keyed_models.md` §"Ordering ties"; `docs/specs/model_transforms.md` §"Compile-time pinning"
**Related code:** `crates/smelt-logical/src/analysis/monotonicity.rs` (`classify_function_determinism`, `NONDETERMINISTIC_FUNCTIONS`); `crates/smelt-logical/src/rules/incremental.rs` (`check_nondeterminism`); `crates/smelt-logical/src/rules/cumulative.rs` (`KeyedForbidsNondeterministic`); `crates/smelt-runtime/src/transformer.rs` (`pin_run_deterministic_clocks`); `crates/smelt-runtime/src/execute.rs` (run-wide pin at `run_start`)
**Scope:** per-column (taint per output column) + per-model roll-up (skeleton-clean)

---

## 1. The property

### 1.1 The three-level classification

Every value-producing expression in a model's SQL sits at one of three determinism levels:

| Level | Definition | Canonical members | Salvageable? |
|---|---|---|---|
| **Deterministic** | Same inputs → same output, on every evaluation, in every run | `+`, `UPPER`, `DATE_TRUNC`, `HASH(col)`, literals, source columns | Trivially |
| **Run-deterministic** | One value per *run*, stable within the run, drifts *across* runs | `NOW()`, `CURRENT_TIMESTAMP`, `CURRENT_DATE` | Yes — **pin** to one literal at run start (`pin_run_deterministic_clocks`, `model_transforms.md`) |
| **Row-nondeterministic** | Fresh value per *row evaluation*; two evaluations of the same row disagree | `RANDOM()`, `RAND()`, `UUID()`, `GEN_RANDOM_UUID()`, `SETSEED()` | No — there is nothing to pin; the variance is per-row, not per-run |

The code home is `classify_function_determinism` (`monotonicity.rs`), the single run-vs-row classifier consumed by `rules/incremental.rs`, `rules/cumulative.rs`, and the monotonicity trace's own clock-function arm.

Why run-determinism is salvageable and row-nondeterminism is not: a run-deterministic function has *one* free variable — the wall clock at evaluation time — and that variable can be **bound once** before emission. `pin_run_deterministic_clocks(sql, run_start)` rewrites every parsed clock call to a typed literal derived from the time `execute_project` began, so a backfill spanning many internal chunks still produces exactly one literal for the whole run (comment at `execute.rs:1013-1026`). After pinning, the expression is fully deterministic *for that run*. A row-nondeterministic function has a free variable **per row**: no compile-time substitution can bind it, because there is no single value to substitute. `RANDOM()` pinned to `0.42` is not `RANDOM()` any more; that is a semantics change, not a pin.

### 1.2 What "deterministic" means here: replay-determinism across runs

The property is **not** "does DuckDB's optimizer treat this function as volatile". It is **replay-determinism**: *if the same input rows are re-evaluated by a later run (or by the full-refresh oracle), do they produce the same output rows?* This is exactly the determinism the executable equivalence oracle needs. The invariant `incremental_state(S) == full_refresh(source | input ∈ S)` (`model_maintenance.md` §"The equivalence invariant") compares two evaluations of the same SQL over the same inputs **at different times**; any expression whose value depends on *when* or *in which physical order* it is evaluated makes the two sides differ even when the inputs are identical. Run-determinism is salvageable precisely because pinning removes the "when"; row-nondeterminism is unsalvageable because the fresh-per-evaluation variance survives any amount of pinning.

### 1.3 Non-function sources of nondeterminism — the taxonomy beyond the name predicate

The function-name predicate catches only explicitly volatile *calls*. Replay-determinism can fail with **zero** nondeterministic functions in the text:

1. **Underspecified ORDER BY + LIMIT (ties).** `SELECT * FROM events ORDER BY score DESC LIMIT 10` — if rows 10 and 11 tie on `score`, which one survives is an engine implementation detail (scan order, parallelism, merge order). Row *membership* of the output is nondeterministic. No function is volatile; the *specification* is incomplete.
2. **`ANY_VALUE` / `FIRST` / `LAST` without an ordering.** `ANY_VALUE(email)` per group returns "some" value — DuckDB picks whatever it saw first in its physical scan, which changes with data layout, vectorization, and thread count.
3. **Order-sensitive aggregates without `ORDER BY` inside the aggregate.** `STRING_AGG(name, ',')` and `ARRAY_AGG(tag)` produce a value whose *content* depends on input arrival order. `STRING_AGG(name, ',' ORDER BY name)` is deterministic; the bare form is not.
4. **Floating-point summation order.** `SUM(DOUBLE)` is not associative in IEEE-754: a parallel plan folds partials in a nondeterministic tree, so the low bits can differ run-to-run. Usually below the tolerance anyone cares about — but a `HAVING SUM(x) > 100.0` sitting exactly on the boundary turns bit-noise into row-membership nondeterminism.
5. **Hash/random sampling.** `SELECT * FROM events USING SAMPLE 10%` (or `TABLESAMPLE`) selects a nondeterministic subset — reservoir/bernoulli sampling is `RANDOM()` by another spelling, with no `RANDOM` token in the text. (`USING SAMPLE 10% (bernoulli, 42)` with a seed is *seed-deterministic* but still layout-sensitive in some engines.)
6. **Sequences, `rowid`, auto-increment.** `nextval('seq')` and DuckDB's implicit `rowid` assign values by physical insertion order — deterministic *given the exact physical history*, which is exactly what a replay does not have.
7. **Tie-ambiguous window ordering.** `ROW_NUMBER() OVER (ORDER BY updated_at)` where `updated_at` has duplicates: the numbering *within* a tie group is unspecified. See §4.8 — this is the highest-frequency practical case, because `ROW_NUMBER` over a non-unique ordering is the standard dedup/surrogate-key idiom.

The unifying view: **row-nondeterminism = any dependence on evaluation instance** (fresh randomness) **or on physical evaluation order** (ties, scan order, float folding), while run-determinism = dependence *only* on run start time. The function-name predicate is a sound-but-incomplete detector for the first sub-class and blind to the second.

---

## 2. Why maintenance needs it

The equivalence invariant is the parent contract of the whole refresh family (`model_maintenance.md`): an incremental run must produce what a full refresh over the processed inputs would. An incremental run and a full refresh evaluate the *same SQL* at *different times* over *different subsets*, and stitch results together by **address** — partition value, unique key, event time. Nondeterminism threatens the invariant exactly where it reaches an addressing position:

- **Nondeterministic `event_time` / `partition_column`**: rows get **re-windowed**. Run 1 computes a row's partition as `2026-07-06`; the full-refresh oracle (or a later rewrite of the same partition) computes `2026-07-07` for the same input row. The DELETE+INSERT rewrite of `batched` deletes the write window and re-inserts — a row whose partition value drifted lands *outside* the deleted window: the old copy is never deleted, the new copy is inserted elsewhere. Duplicate rows, in the wrong slice.
- **Nondeterministic `unique_key`**: `merge_into` matches on the key. A key that replays differently (`UUID()` as surrogate key is the classic) never matches its own previous row — every re-scan of an input row **double-inserts** it under a fresh key. The stored state diverges from the oracle by one phantom row per reprocessing.
- **Nondeterministic row membership** (`WHERE`/`HAVING`/`JOIN ON`/`DISTINCT`/`GROUP BY`/window `PARTITION BY`/`ORDER BY`/frame): the *set of rows* differs between the incremental evaluation and the oracle evaluation. There is no way to state equivalence over a set that isn't a function of the inputs.

These four roles — event-time, partition, unique-key, row-membership — are the **skeleton**. The skeleton must replay identically or rows are mis-addressed; everything else is **payload**. A payload column may be exempted by declaration (`nondeterministic_columns`, migrating to `columns.<c>.contract: plausible` per `batched_models.md`): an audit stamp or a surrogate the modeller accepts may vary is a value judgement only the author holds, so it is declared — but the declaration is only admitted when the taint flow proves the tolerated value flows *exclusively* into the listed column and never back into the skeleton. Listing a skeleton column is a configuration error, not a widening.

The per-model roll-up is then simply: **a model is skeleton-clean iff no skeleton position's column-level taint set contains a row-nondeterministic source** (with run-deterministic sources additionally excluded from event-time/partition even when pinned — see §4.10).

---

## 3. Per-construct analysis

Notation: `T(c)` is the taint set of column `c` — the set of nondeterministic sources (each tagged run-det or row-nondet) that *may* influence `c`'s value. Taint is a **may-analysis**: over-approximation is sound (spurious rejection), under-approximation is unsound (silent mis-addressing).

### 3.1 Scalar expressions — taint is infectious

```sql
SELECT event_id, amount * (1 + random() * 0.01) AS jittered_amount FROM events
```

Any expression with a tainted operand is tainted: `T(f(a,b)) ⊇ T(a) ∪ T(b)`. There is no "sanitizing" scalar function — `ROUND(random(), 0)`, `CAST(random() AS INT)`, `random() > 0.5` all remain row-nondeterministic (coarsening the codomain reduces the *probability* of replay divergence, never to zero). Failure narrative: `jittered_amount` is payload here, fine if declared; move the same expression under an alias named in `unique_key` and every reprocessed row double-inserts.

### 3.2 CASE — tainted condition vs tainted branch

```sql
-- tainted branch: taint reaches the output only on some rows, but MAY-analysis taints the column
CASE WHEN status = 'test' THEN uuid() ELSE session_id END AS session_key
-- tainted condition: BOTH branches' selection is nondeterministic
CASE WHEN random() < 0.5 THEN 'A' ELSE 'B' END AS bucket
```

Both taint the output: `T(CASE) = T(cond) ∪ T(then) ∪ T(else)`. The distinction matters only for precision arguments a v2 analysis might attempt (a tainted branch guarded by a deterministic condition taints only rows satisfying the condition — but since row identity is what's at stake, per-row refinement buys nothing for skeleton positions). Failure narrative for the first form: `session_key` as `unique_key` double-inserts exactly the `status='test'` rows — an intermittent, data-dependent duplication that is miserable to debug, which is why may-analysis (reject both) is right.

### 3.3 WHERE with RANDOM() — row-membership taint (sampling, the classic)

```sql
SELECT * FROM events WHERE random() < 0.10   -- "10% sample"
```

No output *column* is tainted — every projected value is a clean source column. What is tainted is the **relation's membership**: which rows exist. This is why the taint domain cannot be columns alone; it needs one extra per-relation bit, `T_membership`. An incremental run samples its window; the oracle re-samples everything and gets a *different* 10%. Equivalence is violated for every mode, unconditionally — there is no payload-column declaration that can absorb membership taint, so `WHERE` is a hard exclusion in `check_nondeterminism` (position "a WHERE clause") regardless of any opt-in. Same for `USING SAMPLE` (§1.3.5), which today's text scan does not see.

### 3.4 JOIN ON tainted key

```sql
SELECT e.*, d.label FROM events e JOIN dims d ON e.bucket = floor(random()*10)
```

A tainted join predicate taints **membership of both sides' contributions** (which pairs match) — so it taints every output column and the row set at once. Hard exclusion ("a JOIN ... ON clause"). Subtler real-world variant: joining on a column that is *upstream-tainted* (model A projected `uuid() AS join_key`; model B joins on it) — the predicate text in B is clean, the taint arrives through lineage. Cross-model taint propagation (§4) is required to catch it; today's per-model text scan cannot.

### 3.5 GROUP BY tainted key

```sql
SELECT floor(random()*10) AS bucket, count(*) AS n FROM events GROUP BY 1
```

Grouping by a tainted key nondeterministically **partitions the input multiset**: both the group keys and every aggregate over them are tainted. Since `GROUP BY` keys are almost definitionally the output's addressing columns (the unique key of an aggregate model), this is skeleton taint twice over. Hard exclusion ("a GROUP BY key").

### 3.6 Aggregates OF tainted values — and aggregates that launder run-determinism

```sql
SELECT user_id, sum(random()) AS noise FROM events GROUP BY user_id
```

Is `noise` tainted? **Yes** — the aggregate of row-nondeterministic inputs is a value-nondeterministic output (`T(agg(e)) ⊇ T(e)`). Aggregation *collapses row-taint into value-taint*: the output column has one nondeterministic value per group rather than per row, but replays still differ, which is all that matters. Whether it's a *violation* depends on where the value flows: as a declared payload column, tolerable; feeding a `HAVING sum(random()) > 5`, membership taint — rejected.

The interesting asymmetry is **run-deterministic inputs**: `MIN(now())` = `MAX(now())` = `now()` — the aggregate output is still exactly run-deterministic, because every row of a (pinned) run sees the same literal. Aggregation never *escalates* run-taint to row-taint; some aggregates even erase determinism *structure* while preserving the level (`count(*) FILTER (WHERE ts > now())` — run-det membership inside the aggregate → run-det value out). Formally: the taint *level* is the max of the input levels; aggregation is level-monotone but can be level-preserving in ways a naive "any nondet inside an aggregate → reject" rule over-penalizes. `check_nondeterminism` today does not special-case this: `sum(random()) AS noise` in the SELECT list is treated as `random()` flowing into `noise` (listable), which is the right answer by accident of the text scan.

### 3.7 Window functions — ROW_NUMBER over a tie-ambiguous ORDER

```sql
SELECT *, row_number() OVER (PARTITION BY user_id ORDER BY updated_at) AS rn
FROM events QUALIFY rn = 1
```

Two distinct hazards:

1. **Explicitly nondeterministic window clauses** — `OVER (ORDER BY random())` (random shuffle numbering), `PARTITION BY uuid()`. Caught today: `collect_over_contents` scans every `OVER (...)` and hard-excludes any predicate hit ("a window's PARTITION BY / ORDER BY / frame").
2. **Tie-ambiguity** — the example above with duplicate `updated_at` per user. The numbering within a tie group is engine order; `rn` is **row-nondeterministic in effect** with zero volatile functions. This is the single most consequential practical case: `ROW_NUMBER ... QUALIFY rn = 1` is the standard dedup idiom, and `rn` (or the surviving row's identity) routinely becomes the unique key or decides row membership (`QUALIFY` = membership taint). An incremental run and the oracle can keep *different* representatives of the tie group. Not caught by any function predicate; requires an order-key uniqueness proof (§6). `keyed_models.md` §"Ordering ties" makes the honest version of this call for `MAX_BY`: equivalence holds "up to ordering-key ties", carved out rather than falsely proven.

`FIRST_VALUE`/`LAG` inherit the same split: deterministic iff the window ordering is total over the partition.

### 3.8 DISTINCT / UNION over tainted columns

```sql
SELECT DISTINCT user_id, uuid() AS tag FROM events
```

Under `DISTINCT` the **whole projected row is the dedup key**, so any tainted projected column is membership taint — every row's `tag` is unique, `DISTINCT` becomes a no-op, and the deduplication the modeller wrote is silently defeated (row count changes vs. the intended semantics; replay changes it again). Hard exclusion in code ("SELECT DISTINCT (the whole row is the dedup key)") — correctly *independent* of the `nondeterministic_columns` listing. `UNION` (distinct union) is the same operator in disguise: `A UNION B` dedups across the combined projection, so a tainted column in either branch is membership taint; `UNION ALL` is not — it is pure concatenation, and taint stays columnar: `T_union_all(c) = T_A(c) ∪ T_B(c)`.

### 3.9 ORDER BY / LIMIT with ties — membership nondeterminism with no volatile function

```sql
SELECT * FROM scores ORDER BY score DESC LIMIT 100
```

If the 100th and 101st rows tie, membership is engine order. The batched rule sidesteps this wholesale: `LIMIT` is rejected outright for incremental materialization (`incremental.rs` step 2c) — sound, and incidentally closes the tie case without needing a uniqueness proof. A future relaxation (`LIMIT` admitted when the `ORDER BY` key is proven unique) would need exactly the order-key uniqueness analysis of §6. `ORDER BY` without `LIMIT`/`FETCH` is semantically irrelevant to a table materialization (row sets are unordered) and needs no check.

### 3.10 NOW() in a filter — the run-deterministic sliding window

```sql
SELECT * FROM events WHERE ts > now() - INTERVAL 7 DAY
```

This deserves its own treatment because it is *the* place where run-determinism, despite being pinnable, still breaks the executable oracle.

**Pinning fixes intra-run coherence, not cross-run equivalence.** Pinning guarantees every chunk of one run sees the same window — without it, a chunked backfill would evaluate `now()` per chunk and each chunk would keep a slightly different trailing window (rows near the boundary appear in some chunks' outputs and not others: incoherent within a single run). That is what `pin_run_deterministic_clocks` at run-wide `run_start` buys, and it is necessary.

**But the pinned filter makes the full-refresh oracle time-dependent.** The invariant compares `incremental_state(S)` with `full_refresh(source | input ∈ S)` — and `full_refresh` executed *today* evaluates `now()` as today. A row ingested 6 days ago passed run 1's filter and was written; today's oracle run excludes it (it is now 8 days old). The stored table and the oracle disagree **with both sides behaving correctly** — the model's specification itself is a moving target. Formally: the model SQL is not a function of the inputs, it is a function `f(inputs, t_run)`; equivalence is only stateable per fixed `t`.

Consequences for the design space:
- The incremental writes are **append-monotone but never retract**: rows that age out of the 7-day window are excluded from *new* windows' computation, but a batched DELETE+INSERT only rewrites the run's write window — old partitions keep rows the oracle would now drop. The stored table drifts toward "everything that was ever within 7 days at its write time", which is a *different model* ("event was recent when observed") than the SQL claims ("event is recent").
- Hence the hard exclusion of run-deterministic functions from `WHERE` is **correct even though pinning exists** — `check_nondeterminism` rejects `NOW` in a WHERE clause via the shared predicate list, and this is not over-caution: no pin makes a sliding-window filter replay-equivalent.
- The *legitimate* spelling of the intent is a **derived retention/horizon** on the time axis smelt already owns: filter on the partition/event-time column against the *run window* (which smelt injects and records), not against the wall clock — then the oracle's restriction `input ∈ S` carries the window with it, and equivalence is restored because the window is part of the processed-set bookkeeping, not of the SQL.
- The only unconditionally safe home for a pinned clock is a **direct payload projection** (`now() AS loaded_at`): each row's value is the pin of the run that wrote it, never re-read to place/filter/group — exactly the carve-out the code admits (run-clock direct projection into an unlisted column).

### 3.11 Subqueries and CTEs

A scalar subquery importing taint taints the enclosing expression; an `IN (SELECT ...)` / `EXISTS` whose subquery has membership or value taint on the compared column is membership taint on the outer query. Today's analysis does not do any of this per-column: it **fails closed on indirection** — any predicate hit inside a CTE body is rejected outright regardless of what the outer query does with it (`check_nondeterminism` CTE arm), and any occurrence not attributable to a single SELECT-list item hits the step-5 catch-all. Sound; imprecise (a CTE projecting `now() AS loaded_at` as pure payload is rejected where the same text in the outer SELECT is admitted).

### 3.12 Self-reference

A model reading its own previous state (`smelt.ref('self')` / the keyed fold) **launders history into fact**: a nondeterministic value written in run N is a *stored, deterministic input* to run N+1. Taint does not flow backward through materialization — the stored `uuid` column is replay-stable from run N+1's perspective. This is exactly why the check must run *before first materialization* and why declared exemptions are per-column contracts rather than per-run waivers: the divergence is baked in at write time, and only the write-time gate can prevent it. (It is also why the oracle carve-outs in `model_maintenance.md` are stated as *permanent* documented divergences, not transient ones.)

---

## 4. Composition algebra

Taint is a forward may-analysis over operator lineage. Domain per relation `R`: a map `T: columns(R) → 𝒫(Sources)` where each source is tagged `run` or `row`, plus one membership bit `T_m(R) ∈ {clean, run, row}` (the determinism level of *which rows exist*). Ordering: `clean < run < row`; all transfer functions are monotone in this lattice, so the analysis has a least fixed point over any DAG of operators and models.

### 4.1 Operator × taint-transfer table

| Operator | Column taint out | Membership taint out |
|---|---|---|
| **Project / scalar expr** `SELECT e₁ AS c₁, …` | `T(cᵢ) = ⋃ T(col) for col ∈ inputs(eᵢ)`, plus the level of any volatile call in `eᵢ` | `T_m` unchanged |
| **Projection DROP** (column not re-projected) | tainted column **leaves the taint set** — see §4.4 | `T_m` unchanged |
| **Filter** `WHERE p` | per-column unchanged | `T_m ⊔ level(p)` — a tainted predicate taints membership |
| **Join** `A ⋈_p B` | output col from A keeps `T_A(col)`; from B keeps `T_B(col)` — taint imported from either side **per column**, never smeared across sides | `T_m = T_m(A) ⊔ T_m(B) ⊔ level(p)` |
| **Aggregate** `GROUP BY k; agg(e)` | `T(k)` as projected; `T(agg(e)) = T(e)` — row-taint collapses to value-taint, level preserved (never escalated: `MIN(now())` stays run) | `T_m = T_m(in) ⊔ level(k)` — tainted keys nondeterministically partition |
| **DISTINCT** | unchanged | `T_m ⊔ max over ALL projected columns` — the row is the key |
| **UNION (distinct)** | per-column `T_A(c) ⊔ T_B(c)` | like DISTINCT over the concatenation: `⊔` of everything |
| **UNION ALL** | `T_A(c) ⊔ T_B(c)` | `T_m(A) ⊔ T_m(B)` — union of two clean CTEs is clean |
| **Window** `f() OVER (PARTITION BY p ORDER BY o …)` | `T(out) = T(args) ⊔ level(p) ⊔ level(o) ⊔ tie-ambiguity(o)` — an order-sensitive `f` over a non-unique `o` is row-level even when `level(o)=clean` | unchanged (QUALIFY then converts it to membership) |
| **ORDER BY + LIMIT** | unchanged | `T_m ⊔ (row if order key not proven unique, else clean)` |
| **SAMPLE** | unchanged | `row` (unconditionally, absent a seed+stable-layout proof) |
| **Materialize** (model boundary, written state) | resets to `clean` for *already-written* rows (§3.12) — but the write-time gate must have passed first | resets |

### 4.2 Membership taint is relation-level, column taint is column-level

The two must not be conflated in either direction. Collapsing column taint to a per-relation bit ("this CTE contains RANDOM somewhere → poison everything downstream") is sound but destroys the precision that makes the payload exemption usable at all — see §4.4. Ignoring the membership bit (tracking columns only) is *unsound*: §3.3's sampler taints no column.

### 4.3 Aggregation collapses row-taint into value-taint

After `GROUP BY user_id → sum(random()) AS noise`, the group row's identity (`user_id`) is clean; only the value in `noise` varies across replays. Downstream, `noise` behaves like any tainted scalar column: droppable, declarable as payload, fatal if it reaches a skeleton position. The collapse matters because it means aggregation is a *containment boundary in one direction*: row-level chaos upstream becomes a single tainted column that projection can then discard.

### 4.4 Dropping a tainted column clears it — taint is per-column, not per-relation

```sql
WITH scored AS (SELECT event_id, ts, random() AS score FROM events)
SELECT event_id, ts FROM scored          -- clean: score is dead
```

If `scored` never *used* `score` to filter/group/order/join (membership stays clean), the final projection carries no taint at all. This precision is what makes the algebra worth having: real pipelines constantly compute exploratory or debug columns that die before the model output. The current implementation's CTE arm rejects this query (any predicate hit inside a CTE body → hard error), which is the largest precision gap between the algebra and the code (§7).

One trap: the drop clears *column* taint only. `SELECT DISTINCT event_id FROM scored` after the drop is clean; but `SELECT DISTINCT event_id, score FROM scored` then dropping `score` in an outer query does **not** clear — DISTINCT already consumed `score` into membership, and membership taint survives projection. Order of operations in the lineage is everything: the algebra evaluates operator by operator, not over the final projection list.

### 4.5 Run-det pinning composition — within a run and across models

Within one `execute_project` run, the pin is **run-wide**: `run_start` is captured once at the top of `execute_project` (`execute.rs:106`) and every model, batch, and chunk of that run pins to the same literal. So two models both projecting `now()` in one pipeline run agree exactly — pinning composes trivially *intra-run*.

**Across runs it does not.** If model B reads model A and they were last built in different runs (A yesterday, B today — the normal state of an incremental DAG, where each run rebuilds only what changed), A's stored `loaded_at` values carry yesterday's pin while B's carry today's. Any B-side logic *comparing* its own pin to A's stored pin (e.g. `WHERE a.loaded_at > now() - INTERVAL 1 HOUR` — "only fresh A rows") reintroduces the sliding-window problem of §3.10 through the back door: the comparison result depends on the *skew between two runs' pins*, which is scheduling, not data. The algebra should therefore treat a **stored pinned column as an ordinary deterministic data column with no relationship to the current run's pin** — pins are opaque timestamps once written; only their *provenance within the current run* is coherent.

What a pipeline-wide per-run pin buys, precisely: (a) intra-run chunk coherence (necessary for the one-literal-per-run invariant the admission gate assumes); (b) cross-model *same-run* consistency, so a run's audit stamps mutually agree and a run is identifiable by its pin; (c) a well-defined replay hook — re-running with a recorded `run_start` reproduces the run's pinned values exactly, which is what makes run-determinism compatible with the replay oracle *per run*. What it cannot buy: cross-run agreement (staleness skew is intrinsic to incremental scheduling) or oracle-equivalence for pinned values used in filters (§3.10).

### 4.6 Cross-model propagation

The fixed point over the model DAG: a model's *output* taint map (post-gate: skeleton positions proven clean, declared payload columns tagged) becomes the *input* taint map of its readers. Today no cross-model propagation exists — each model's check is local text analysis, and §3.4's laundered join key (upstream `uuid() AS join_key`) passes silently. The declared exemption should export: a column listed in A's `nondeterministic_columns` is a `row`-tainted input column in every reader B, so B joining or grouping on it is a skeleton violation *in B*. Without this, the per-column opt-in is only as strong as the single model boundary.

---

## 5. Static provability vs declaration

**The function-name predicate is an allowlist/denylist with an open middle.** `classify_function_determinism` returns `Neither` for any unrecognised name, and the two consumers handle `Neither` in *opposite* ways:

- The **monotonicity trace** (event-time position) is fail-closed: an unrecognised function is `Undecidable → NotTraceable`, refused unless declared (`timeseries.assert_monotonic` — and even that declaration cannot widen a *Disproven* verdict, e.g. a known row-nondeterministic function in the event-time chain).
- The **taint check** (`check_nondeterminism`, `KeyedForbidsNondeterministic`) is effectively **fail-open for UDFs**: it scans for the eight names in `NONDETERMINISTIC_FUNCTIONS` and treats everything else — including a user-defined or macro-expanded function that wraps `random()` — as deterministic. A UDF is not classified fail-closed-nondeterministic; it is silently trusted. This is an asymmetry worth naming: the event-time skeleton position gets fail-closed treatment via a *different* proof, but WHERE/GROUP BY/key positions get only the denylist.

**The declaration (`nondeterministic_columns` / `contract: plausible`) widens, never substitutes.** Its admission conditions are exactly the taint algebra's: the tolerated value must flow *exclusively* into the listed column (the exclusive-flow requirement — enforced today by the fail-closed-on-indirection step: any occurrence not attributable to one SELECT-list item is rejected), and a skeleton column (`event_time_column`, `partition_column`, `unique_key`) is never listable — declaring one is a configuration error, not an override (`batched_models.md` Constraint 12). This is the "declared escape hatches may only widen" invariant of `model_properties.md` applied to this property: the declaration widens the *payload* verdict, never the skeleton one.

**Tie-nondeterminism is invisible to any function predicate.** §1.3's cases 1–4 and 6–7 contain no volatile name. What would catch them is an **order-key uniqueness proof**: given `ORDER BY e₁, …, eₙ` (in a window, an aggregate's internal ORDER BY, or under LIMIT), prove the tuple is unique per partition/group — via a declared/derived unique key on the source relation reaching the ordering columns injectively, or a trailing tiebreaker column proven unique. Where the proof fails, the honest options are exactly the ones the specs already model: refuse (batched's blanket LIMIT rejection), carve out (keyed's `MAX_BY` "equivalence up to ordering-key ties", `keyed_models.md` §"Ordering ties" — incumbent-wins plus mandatory sequential execution makes it deterministic-given-history without claiming a proof), or demand a remodel (the composite provably-tie-free ordering expression the keyed spec recommends, e.g. `(updated_at, source_seq)`). Notably the keyed classifier "cannot verify uniqueness and does not claim to" — the carve-out is documented rather than the proof faked, which is the right posture until a uniqueness proof exists.

**Order-sensitive aggregates** (`STRING_AGG`, `ARRAY_AGG` without internal ORDER BY, `ANY_VALUE`, `FIRST`) sit between the two mechanisms: they are *names*, so a predicate extension could classify them — but as **conditionally deterministic** (deterministic iff an internal `ORDER BY` with a unique key is present), a third predicate class the current two-way enum cannot express. Keyed mode dodges this today by whitelisting combiners (`STRING_AGG` → `KeyedUnknownCombiner`, refused; `ANY_VALUE` admitted only under the snapshot posture where one-row-per-key-per-scan makes it well-defined); batched mode has no check at all for them.

---

## 6. Implementation gaps (vs the §1.3 taxonomy and §4 algebra)

What `check_nondeterminism` + `KeyedForbidsNondeterministic` + the LIMIT rejection cover today, against the full taxonomy:

| Taxonomy item | Batched (`check_nondeterminism`) | Keyed (`cumulative.rs`) |
|---|---|---|
| Volatile functions in skeleton positions | **caught** (hard exclusions: partition/event-time expr, unique_key, WHERE, HAVING, GROUP BY, JOIN ON, OVER-contents, DISTINCT, CTE bodies fail-closed) | **caught** (blanket: any of the eight names anywhere in the outer body → refused, no payload exemption at all) |
| Volatile functions in payload (declared) | **caught + widened** (`nondeterministic_columns`; run-clock direct projection admitted unlisted) | n/a (blanket refusal; no declaration surface) |
| Run-det pin | **built**, run-wide `run_start` (`execute.rs:1026`) | keyed path: refused before pinning is relevant |
| ORDER BY + LIMIT ties | **incidentally closed** — LIMIT rejected outright (2c) | LIMIT not specifically checked; keyed body shape rules mostly preclude it |
| `ANY_VALUE`/`FIRST` without order | **missed** (no check) | **incidentally closed** — combiner whitelist; `ANY_VALUE` only under snapshot posture |
| `ARRAY_AGG`/`STRING_AGG` without ORDER BY | **missed** | **incidentally closed** — `KeyedUnknownCombiner` |
| Window tie-ambiguity (`ROW_NUMBER` over non-unique order) | **missed** — `collect_over_contents` checks only for volatile *names* inside OVER; a tie-ambiguous clean ordering passes | **closed by shape** — window functions banned outright in keyed outer body |
| `USING SAMPLE` / `TABLESAMPLE` | **missed** — no `SAMPLE` token in the predicate list; membership taint passes silently | **missed** |
| Float summation order | **missed** (arguably below the line; becomes real only at HAVING boundaries) | missed |
| Sequences / `nextval` / `rowid` | **missed** — `NEXTVAL` not in the list | missed |
| UDF / unrecognised function | **fail-open** in taint positions (§5); fail-closed only at event-time via the trace | fail-open likewise |
| Cross-model taint (upstream-tainted join key, exported `nondeterministic_columns`) | **missed** — analysis is per-model text | missed |
| Tainted-column drop precision (§4.4) | **over-rejects** — CTE-body hit is fatal even for a dead column | over-rejects (blanket) |
| String-literal / identifier false positives | text scan is word-boundary (`has_keyword_at_boundary`) on stripped SQL — a column *named* `uuid` or a literal `'RANDOM'` can still false-positive; contrast the pin transform, which is parser-based and never touches literals | same class of risk |

Summary shape: the **function-predicate core is solid and consistently fail-closed for the skeleton**, the **non-function taxonomy is mostly uncovered in batched mode** (SAMPLE and window tie-ambiguity being the two with real mis-addressing potential), keyed mode is protected by its restrictive body shape rather than by taint analysis, and the **precision ceiling** (CTE fail-closed, no per-column lineage, no cross-model flow) is the cost of the text-scan implementation rather than of the algebra.

## 7. Open questions

1. **Should unrecognised functions fail closed in taint positions?** The event-time trace already does; the WHERE/key scan does not. A blanket flip breaks every model using any of the hundreds of ordinary deterministic builtins not on an allowlist — so this really asks: build a deterministic-builtin *allowlist* (large, engine-specific, maintenance burden) or keep the denylist and accept the UDF hole?
2. **What is the third predicate class for conditionally-deterministic constructs** (`STRING_AGG`/`ARRAY_AGG`/window functions: deterministic iff their ordering key is proven unique), and does the order-key uniqueness proof reuse the unique-key machinery (`JoinContext`, declared `unique_key`) or need its own injectivity trace?
3. **Cross-model taint export**: should a column listed in `nondeterministic_columns` (`contract: plausible`) propagate as a row-tainted input column to every reader, making a downstream join/group on it a *downstream* skeleton violation — and if so, where does that check live (per-model with imported facts, or a graph-level pass in `maintenance_plan.md`'s graph layer)?
4. **Recorded-pin replay**: should `run_start` be persisted in the run ledger so the equivalence oracle can replay a historical run with its original pin (making run-det payload columns oracle-checkable per run), rather than only checking runs against a fresh-pinned full refresh?
5. **Upgrading from text scan to lineage**: is the per-column taint algebra (§4) worth implementing over `SelectAnalysis`/the logical plan — recovering the dead-tainted-column and CTE-payload precision (§4.4, §3.11) and enabling items 2–3 — or does the fail-closed text scan's simplicity win until a real model is over-rejected?
