# Property research: filter distributivity / pushdown depth (injection point)

- **Date:** 2026-07-07
- **Status:** research
- **Related specs:** `docs/specs/model_properties.md` (rows "Injection-point / pushdown-depth", "Set-operation distribution", "Partition alignment", "Unified bound / reach derivation", §Semantics "Unified bound / reach derivation"), `docs/specs/model_maintenance.md` (§"Windowed maintenance and the horizon" — widened scan + exact clamp, scan ⊇ write), `docs/specs/model_transforms.md` (§"Source-filter pushdown + the two clamps", "UNION-branch wrap-and-filter")
- **Related code:** `crates/smelt-logical/src/analysis/source_bounds.rs` (`InjectionPoint`, `BoundResult`, `derive_and_classify_bounds`, `BoundResult::merge`), `crates/smelt-logical/src/rules/incremental.rs` (`derive_model_source_bounds`, `restrict_ctx_for_constructs`, `restrict_ctx_for_union`, `restrict_ctx_for_join`, admission scans in `detect()`), `crates/smelt-logical/src/rules/rule_diagnostics.rs` (`check_event_time_injectable`), `crates/smelt-runtime/src/transformer.rs` (`inject_source_filters`, `inject_time_filter`, `is_transparent_single_source`)
- **Prior research:** `docs/research/20260701-monotonicity-primitive-research/research-pushdown-and-monotone-expressions.md` (the classical σ-commutation laws — GMUW §16.2 — and the monotone-rewrite prior art; cited below as *[pushdown-research]*, not restated); `docs/research/property-discovery/catalog.md` cells SC-1 (correlated `EXISTS` reach), G-06 (left-join null-preservation), G-09 (`UNION ALL` bound composition)

---

## 1. The property

Fix a model `Q` with projected event-time expression `e`, and a half-open time-window
predicate over the output:

```
σ ≡ σ_[t1,t2)  :  keep output rows where  e ∈ [t1, t2)
```

The **injection point** of `σ` for a given source `R` of `Q` is the deepest placement of
a time filter on `R`'s scan that preserves the query's meaning. It is a per-**(model,
source)** verdict — a model joining a fact to a dimension can be `Source`-transparent for
the fact and refuse any filter on the dimension. Three verdicts:

**`Source` — zero-margin transparent slice.** There is a rewritten filter `σ'` on the
source such that σ commutes with the *whole* query exactly:

```
σ_[t1,t2)( Q(R) )  =  Q( σ'(R) )                                (exact, no residue)
```

`σ'` is not σ verbatim: it is σ **rewritten onto the traced leaf column**. If the
monotonicity trace (`model_properties.md` §"Event-time monotonicity trace") establishes
`e = f(R.c)` for a single leaf column `c` and a monotone-non-decreasing `f`, then the
pre-image of the half-open interval under `f` is again an interval, and

```
σ' ≡ σ_{c ∈ f⁻¹([t1,t2))}      e.g.  e = ts + INTERVAL 2 HOUR  ⇒  σ' : ts ∈ [t1 − 2h, t2 − 2h)
```

For a *weakly* monotone `f` (`date_trunc`, `CAST(ts AS DATE)`) the pre-image endpoints
must be **bucket-rounded**, not copied (`f(x) ≥ t1 ⟺ x ≥ ceil_f(t1)`) — see §4 GROUP BY.
So `Source` already *presupposes* the monotonicity trace: without `f` and its inverse
there is no `σ'` to push.

**`OuterClamp(m)` — widened scan + exact clamp.** No exact commutation, but a finite
margin `m = (before, after)` exists such that

```
σ( Q(R) )  =  σ( Q( widen_m(σ')(R) ) )        widen_m(σ') : c ∈ [f⁻¹(t1) − before, f⁻¹(t2) + after)
```

The pushed filter is deliberately **wider** than the window (the reach `m` comes from
the unified bound derivation — frames, interval bands, `WHERE` shifts, plus the declared
`source_lateness` term), and σ is re-applied **exactly** on a wrapping projection over
the output schema (`model_transforms.md` §"Source-filter pushdown + the two clamps").
The margin is *read but never written*: `scan ⊇ write`.

**Refused.** No finite `m` exists (an `UNBOUNDED` frame, an unaligned aggregate, a
global scalar subquery), or none is provable (`NotDerivable`: a bare `LAG`, a symbolic
month/year interval, an untraceable `e`). Fail-closed: absence of proof is refusal.

### This property is a composition, not a primitive

The verdict is **derived** from three primitives plus per-operator commutation facts:

1. **Monotonicity trace** → supplies `σ'` (which leaf column, which rewrite, which
   endpoint handling). No trace ⇒ no filter to push at any depth.
2. **Reach** (`BoundResult`) → supplies `m`. Zero reach upgrades `OuterClamp` to
   `Source`; unbounded/non-derivable reach demotes to Refused.
3. **Alignment** (`PartitionAlignment`) → decides whether σ commutes with a grouping /
   dedup / window scope at all (the exact-vs-margin-vs-never split per operator).

Each relational operator between the output and the scan contributes one commutation
fact (does σ pass through exactly, with margin, or not at all), and the model's verdict
for a source is the **composition of the facts along the path** from the output
projection down to that source's scan (§5). This is the classical predicate-pushdown
law set of GMUW §16.2 specialised to a temporal range predicate — *[pushdown-research]*
grounds each per-operator law; this document works out the smelt-specific composition
algebra, the counterexamples, and the gap between the algebra and the shipped code.

### A useful error taxonomy

When `σ'` is pushed too deep, exactly three kinds of damage are possible, and they map
one-to-one onto the verdict lattice:

- **Row-set errors** — extra or missing *output rows*, values of surviving rows intact.
  Repairable by the exact outer clamp (it deletes rows; it cannot fix values). Example:
  margin rows emitted by a widened scan.
- **Bounded value errors** — in-window output rows computed from a truncated
  neighbourhood (a rolling window missing its left context). Repairable by widening the
  scan by the reach `m` — this is precisely what `OuterClamp(m)` is.
- **Unbounded value errors** — in-window values depend on arbitrarily distant input (a
  lifetime aggregate, a global denominator). No finite widening repairs them ⇒ Refused.

`Source` = the push induces no error at all. `OuterClamp(m)` = the push induces only
bounded value errors (fixed by `widen_m`) and row-set errors (fixed by the clamp).
Refused = an unbounded value error somewhere on the path.

## 2. Why maintenance needs it

The injection point is the **scan-cost dial of every incremental run**. A `batched` run
for window `[t1,t2)` must produce, for its write window, exactly what a full refresh
restricted to the processed inputs would (`model_maintenance.md` §"The equivalence
invariant"). The three verdicts are three cost regimes:

- `Source`: the run reads `O(|window|)` rows from each transparent source, computes, and
  writes. One filter, at the scan; the outer clamp is textually redundant and dropped
  (`is_transparent_single_source`). This is the regime that makes daily incremental
  runs on multi-year tables cheap.
- `OuterClamp(m)`: the run reads `O(|window| + m)` — still window-proportional. The
  clamp costs one wrapping projection. The margin is re-read every run but never
  re-written (no double-count at partition edges — the exact defect the old
  auto-widened *write* window had, `model_transforms.md` §Design "Rejected").
- Refused: the honest fallbacks are per-partition execution (for `Unbounded`, where
  each partition still only *writes* itself but must *read* history) or full refresh.
  Cost is `O(|history|)` per run — the regime incremental maintenance exists to escape.

The alternative to pushdown — "compute the whole query, clamp at the top" — is not a
cheaper `OuterClamp`; it is a full refresh per run that throws away most of its output.
Worse, it is not even always *correct* to describe `OuterClamp` that way: the clamp
alone repairs only row-set errors. Without the widened scan a window function at the
left edge of an unwidened scan produces a *wrong value* on an *in-window* row, which
the clamp happily keeps. Scan ⊇ write is a correctness obligation, not an optimisation.

The property also feeds the derived horizon (`model_maintenance.md`): the same reach
`m` that widens the scan bounds the write-eligibility clamp, and both must come from
one derivation — which is why `derive_and_classify_bounds` pairs the `BoundResult` and
the `InjectionPoint` in a single walk (the two windows can never disagree).

## 3. Per-construct analysis

Notation: `R` is the anchor (driving, clock-bearing) source; σ is the output window
filter; `σ'` the leaf rewrite. Every SQL example is DuckDB-valid. "Exact" =
contributes `Source` (margin 0); "margin `k`" = contributes `OuterClamp` with reach
`k`; "refused" = ⊥.

### 3.1 Projection (monotone rewrite) — exact, conditional on the trace

```sql
SELECT event_ts + INTERVAL 2 HOUR AS event_time, user_id
FROM smelt.raw.events
```

`e = f(ts)`, `f` strictly increasing ⇒ `σ' : event_ts ∈ [t1−2h, t2−2h)` and
`σ(π(R)) = π(σ'(R))` exactly.

**Counterexample — a non-monotone projection.** `e = EXTRACT(hour FROM event_ts)` is
periodic. Rows: `('2026-06-03 05:10', u1)` and `('2026-07-05 05:20', u2)` both project
`e = 5`. The pre-image of `[5, 6)` is *every day's* 05:00 hour — not an interval, so
no `σ'` exists on `event_ts`. The trace returns `NotTraceable{Disproven}` (periodic
construct) and no declaration may widen it. Refused.

### 3.2 WHERE — exact, trivially

Selections commute: `σ(σ_p(R)) = σ_p(σ(R))` for any per-row `p` (GMUW 16.2.2). A
model's own `WHERE` never blocks pushdown; its interval shifts *contribute reach*
(Form B: `WHERE ts BETWEEN other_ts - INTERVAL 1 DAY AND other_ts` gives the shifted
source a 1-day margin, not a refusal).

**The one genuine hazard is NULL, and it is not a WHERE hazard per se.** For a pure
per-row pipeline, a NULL leaf is harmless: `NULL ≥ t1` is `NULL`, so the outer σ drops
the row exactly as the pushed `σ'` does. The row-influence problem appears only once a
non-per-row operator sits above the leaf:

```sql
SELECT user_id, MAX(event_ts) AS event_time, COUNT(*) AS n
FROM smelt.raw.events GROUP BY user_id
```

Rows: `(u1, NULL)`, `(u1, DATE '2026-07-05')`. Full compute: `(u1, 2026-07-05, 2)`.
Push `σ' : event_ts >= DATE '2026-07-01'` to the scan: the NULL row is dropped
*pre-aggregation* → `(u1, 2026-07-05, 1)`. In-window row, wrong value — a value error
the clamp cannot repair. This is why the **column nullability gate** downgrades
`Traceable → NotTraceable` when the leaf is nullable or unknown
(`model_properties.md` §"Column nullability gate"): the gate is conservative for pure
per-row models, but sound composition demands it because the trace cannot see what
sits above it.

### 3.3 Inner join — exact on the anchor side only

`σ_θ(R ⋈ S) = σ_θ(R) ⋈ S` iff θ references only `R`'s attributes — the driving-fact
condition. The anchor is the exactly-one-`Traceable` joined input
(`resolve_join_driving_fact`); the filter goes to `R`'s scan, and the join contributes
margin 0 *for R*.

**The non-anchor side.** By default it gets **no filter** (full scan) — sound, since
an unfiltered `S` is exactly what the full refresh joins against. Two upgrades exist:

1. A **time-band join condition** gives `S` its own derived bound: `ON s.uid = e.uid
   AND s.ts BETWEEN e.ts - INTERVAL 1 DAY AND e.ts` lets `S` be scanned over
   `[t1 − 1d, t2)` — margin = band width (the interval/temporal-join detection row,
   `partial` today).
2. Equality of clock columns (`ON r.ts = s.ts`) would license transitive pushdown;
   smelt does not derive this (DuckDB's statistics propagation may recover it at the
   engine level — *[pushdown-research]* Area 2, DuckDB).

**Counterexample — pushing the window onto a non-anchor dimension.** Fact
`(order_id 1, ts 2026-07-05, cust 9)`; dimension `(cust 9, updated_at 2026-03-01,
tier 'gold')`, joined `ON o.cust = d.cust`. Push `d.updated_at >= DATE '2026-07-01'`
onto the dimension: the dim row vanishes, the join loses the match, the in-window
order row disappears (inner join) — a *missing* row the clamp cannot restore. The
dimension's time column is simply unrelated to the output window.

### 3.4 LEFT JOIN — exact to the preserved side; the null-supplying side is the classic invalidity

**Preserved side = anchor:** `σ_p(R ⟕ S) = σ_p(R) ⟕ S` when `p` is over `R`'s columns —
filtering preserved rows before or after the join yields the same set (an R row failing
`p` contributes nothing either way; one passing `p` gets the same matches). Exact.

**Null-supplying side.** The textbook invalidity, worked concretely. Model:

```sql
SELECT e.event_ts AS event_time, e.user_id, c.amount
FROM smelt.raw.events e
LEFT JOIN smelt.raw.conversions c
  ON c.user_id = e.user_id
 AND c.conv_ts BETWEEN e.event_ts AND e.event_ts + INTERVAL 7 DAY
```

Rows: event `(u1, 2026-07-02)`; conversion `(u1, 2026-07-08, 40.0)`. Run window
`[2026-07-01, 2026-07-05)`. Full refresh: the band matches (07-08 ≤ 07-02 + 7d), output
`(2026-07-02, u1, 40.0)`.

Push the *unwidened* window onto the null-supplying side (`c.conv_ts ∈ [07-01, 07-05)`):
the conversion (07-08) is filtered out **pre-join**, so the preserved event doesn't lose
its output row — it degrades to `(2026-07-02, u1, NULL)`. In-window row, wrong value.
The clamp (on `event_time` = 07-02, in window) keeps it. This is the sharp reason LEFT
JOIN pushdown to the null-supplying side is invalid *without the full derived margin*:
filtering `S` never removes output rows, it **converts matches into NULL-padding** —
always a value error, never a row-set error, hence never clamp-repairable.

With the band-derived margin it is sound: scan `c` over `[t1, t2 + 7d)` (forward reach
7d — note `before`/`after` are split for exactly this reason) and every match of an
in-window event is provably inside the widened scan; the NULL-padding pattern of the
full refresh is reproduced exactly, then the clamp trims the margin's own output rows.
So: null-supplying side = margin(band reach), refused when no band is derivable.

(A σ placed on the null-supplying side's *own* column — `WHERE c.amount > 10` after the
LEFT JOIN — is the other classic case: `σ_p(R ⟕ S) ≠ R ⟕ σ_p(S)` because the RHS keeps
`(r, NULL)` for an `r` whose only matches fail `p` while the LHS drops the joined row;
that is the model author's semantics question, not smelt's time filter, but it is why
no generic "push any predicate into S" rule is admissible.)

### 3.5 FULL JOIN — refused; CROSS JOIN — exact on the anchor, in principle

**FULL JOIN.** Both sides are null-supplying, and the event-time is typically
`COALESCE(l.ts, r.ts)` — two-column arithmetic, `NotTraceable{Disproven}`, so the trace
already refuses. The refusal is genuine, not conservatism — pushdown *creates* rows:

Left `(k1, 2026-06-20)`, right `(k1, 2026-07-03)`, `FULL JOIN ... USING (k)`,
`event_time = COALESCE(l.ts, r.ts)`. Full refresh: one matched row, `event_time =
2026-06-20` — **outside** window `[07-01, 07-08)`, so σ drops it; output empty. Push
`σ'` onto the left side: the left row (06-20) is filtered pre-join, the right row
becomes unmatched and NULL-padded → `(NULL, 2026-07-03)` with `event_time = 07-03` —
**inside** the window. The clamp *keeps* the spurious row. Pushdown into a
null-supplying side can turn an out-of-window match into an in-window fabrication; no
margin fixes a row that shouldn't exist. Refused.

**CROSS JOIN.** σ over the anchor commutes with a product exactly:
`σ_p(R × S) = σ_p(R) × S`. The σ-commutation fact is benign; what fails closed in smelt
is the *cardinality* proof (`fan_out` treats `CROSS JOIN` as `OneToMany`), which gates
different transforms. Worth keeping the two properties distinct: a cross join against a
small calendar table is `Source`-transparent for the anchor's filter.

### 3.6 GROUP BY — exact iff partition-aligned, with bucket-boundary care

**Aligned case.** `GROUP BY` keys ⊇ the time bucket, e.g.:

```sql
SELECT date_trunc('week', event_ts) AS event_week, user_id, SUM(amount) AS total
FROM smelt.raw.events
GROUP BY 1, 2
```

σ on `event_week` selects whole groups; each group's rows share one bucket; pushing the
**bucket-rounded pre-image** is exact. The rounding is load-bearing:
`date_trunc('week', ts) >= t1 ⟺ ts >= ceil_to_week(t1)` and
`date_trunc('week', ts) < t2 ⟺ ts < ceil_to_week(t2)` — the pushed endpoints are the
*next bucket boundaries*, not `t1`/`t2` verbatim.

**Counterexample — naive (unrounded) endpoints truncate a group.** Weekly buckets, week
of Mon 2026-06-29 contains events `2026-06-30 (amount 10)` and `2026-07-04 (amount 5)`.
Run window `[2026-06-29, 2026-07-01)` (the bucket 06-29 is in-window; the window edge
07-01 is mid-bucket). Naive push `ts < 2026-07-01` scans only the 06-30 row → group
`(2026-06-29, u1, 10)`. Full refresh over the same processed partition (the whole
bucket) gives `(2026-06-29, u1, 15)`. In-window group, wrong aggregate — value error,
clamp keeps it. Correct push: `ts ∈ [2026-06-29, 2026-07-06)` (bucket-rounded), i.e.
the scan set is a *partition set*, which is exactly why `batched` demands run windows
aligned to the model's granularity and why the weakly-monotone rewrite must round
outward. (This is prior-art-settled: push a closed range covering the truncation
bucket — *[pushdown-research]* design guidance 3.)

**Unaligned case.** `GROUP BY user_id` with `event_time = MAX(event_ts)`: the group
folds all history. Rows `(u1, 2026-06-01)`, `(u1, 2026-07-05)`: full `COUNT(*) = 2`;
any windowed scan gives 1. Unbounded value error ⇒ Refused for pushdown (the property
verdict; `keyed` mode maintains this shape by a different mechanism — merge, not
slice — which is the "opposite polarity" note on the `PartitionAlignment` row).

**Empty grouping set** (global aggregate, `SELECT COUNT(*) FROM …`): never pushable —
also the one case where σ below γ can *change group existence* (a group surviving with
zero rows; the PrestoDB #11297 caveat, *[pushdown-research]* Area 1). Refused.

### 3.7 HAVING — inherits the GROUP BY verdict

`HAVING` is a selection on the grouped relation; σ (also a selection on the grouped
relation, when aligned) commutes with it. So: aligned scope ⇒ exact; unaligned ⇒
refused, and the *reason* is the group's cross-window content, not HAVING itself.
Counterexample: `GROUP BY user_id HAVING COUNT(*) > 3` with u1 having 2 June + 2 July
events — full refresh emits u1 (4 > 3); a July-window run computes 2 and drops it.
Code: `check_having_alignment_all_scopes` licenses HAVING per-scope via
`scope_group_by_alignment`, across every UNION branch — matching the algebra.

### 3.8 Window functions — exact only when partition-aligned; else margin = frame reach; UNBOUNDED refuses

Three regimes:

- **`OVER (PARTITION BY pk …)` with pk ⊇ partition_column** — the window is
  partition-local; no frame can cross a bucket, so a bucket-rounded push is exact.
  (`find_inadmissible_over` admits this.)
- **Bounded `RANGE` frame** — `SUM(amount) OVER (ORDER BY event_ts RANGE BETWEEN
  INTERVAL 7 DAY PRECEDING AND CURRENT ROW)`: an in-window row's value reads back 7
  days ⇒ margin `before = 7d`. Push `widen_7d(σ')`, clamp exactly. Counterexample
  without the margin: row at 2026-07-01 with a contributing event at 2026-06-27 —
  unwidened scan starts 07-01, rolling sum misses the 06-27 amount; wrong value on an
  in-window row. (`FOLLOWING` symmetrically populates `after`.)
- **`ROWS` / `GROUPS` / bare `LAG`/`LEAD`** — reach is in *rows*, not time; no finite
  time margin covers it. Rows `( 2019-01-04 )`, `( 2026-07-02 )`:
  `LAG(event_ts) OVER (ORDER BY event_ts)` on the July row reads a value seven years
  back. `NotDerivable` ⇒ refused (`has_bare_lag_lead_over`). Note the deliberate
  fork from `temporal.rs`'s advisory estimate, which *does* guess a period for
  chunk sizing — the pushdown proof must not (`model_properties.md` §Known
  Divergences).
- **`UNBOUNDED PRECEDING/FOLLOWING`** — reach ∞ ⇒ `Unbounded` ⇒ refused for pushdown
  (routes to per-partition execution).

"Never exact" is therefore one qualifier too strong: the *aligned* window is exact; the
*framed unaligned* window is the canonical `OuterClamp(m)` citizen; everything else
refuses.

### 3.9 DISTINCT — exact (δ commutes with σ), gated on the time column surviving the dedup

`δ(σ_p(R)) = σ_p(δ(R))` for deterministic `p` (GMUW 16.2.6): duplicates agree on every
column, hence on `event_time`, so both copies pass or fail σ together — dedup-then-
filter equals filter-then-dedup, exactly. The precondition is that the traced time
column is *part of the deduped row* — if an inner `SELECT DISTINCT user_id` scope
doesn't project it, there is nothing to trace through and the trace already fails.
smelt's per-scope `scope_distinct_alignment` (partition_column projected in the
DISTINCT scope) is the same condition stated on the partition column. Verdict: exact.

### 3.10 UNION ALL — exact, branch-wise

Bag union distributes σ: `σ(R ⊎ S) = σ(R) ⊎ σ(S)`. Each branch gets its **own** `σ'`
via its own trace (branch 1 may project `ts`, branch 2 `ts + INTERVAL 1 HOUR` — the two
rewrites differ and must; `trace_union_branches` scopes each branch's trace to its own
FROM sources precisely because branches routinely share a partition-column *name*
across distinct sources). Catalog cell G-09 records the corresponding harness
hypothesis (bound derivation composes across arms — HOLDS).

**Counterexample inside the family — the StaticSeed branch:**
`SELECT DATE '2020-01-01' AS event_date, … UNION ALL SELECT event_date, …` — branch 1
is not a stream (every row lands in one frozen partition); a pushed filter on its
source is meaningless and the branch poisons the union (`restrict_ctx_for_union`
rejects `StaticSeed` by name). A `NotTraceable` branch is merely conservative: the
whole union falls back to the outer clamp.

### 3.11 UNION (DISTINCT) — σ does distribute; smelt still refuses, deliberately

Set union: `σ(δ(R ∪ S)) = δ(σ(R) ∪ σ(S))`. Proof: σ distributes over ∪ elementwise
(membership is `x∈R ∨ x∈S`, and `p(x)` depends only on the row value), and σ commutes
with δ (§3.9 — cross-branch duplicates are *identical rows*, so they share
`event_time` and pass/fail together). So mathematically UNION DISTINCT is as
transparent as UNION ALL, including under widened branch scans (widen ⊇ window means
every copy of an in-window row is scanned in both branches; the margin's extra rows are
clamped).

Why smelt refuses today, and why that is right for now:

1. **Unclassified, hence fail-closed.** `restrict_ctx_for_constructs` takes the union
   path only under `has_union() && is_union_all()`; `check_event_time_injectable`
   unconditionally errors on non-ALL set ops (`rule_diagnostics.rs:256-277`). The
   per-branch machinery (`trace_union_branches`) walks `union_select`, whose "simple
   per-branch append" assumption is documented as the reason plain UNION keeps the
   unconditional error.
2. **The maintenance transform is not value-preserving under partial evaluation the
   way the *algebra* is.** The σ-distribution proof is about one query evaluation; the
   DELETE+INSERT contract also needs the dedup outcome per *partition* to be stable
   across runs. Since duplicates share their partition value, this is in fact fine —
   but nobody has written that proof into a classifier, and per the constraint
   ("absence of a proof is a rejection") the refusal stands until it exists.

### 3.12 INTERSECT / EXCEPT — σ distributes over both; unclassified in smelt; ALL variants also distribute

**Set semantics, proofs.** For any per-row `p`:

- `σ_p(R ∩ S) = σ_p(R) ∩ σ_p(S)`: `x ∈ LHS ⟺ p(x) ∧ x∈R ∧ x∈S ⟺ x ∈ RHS`. Also
  `= σ_p(R) ∩ S` (one-sided suffices).
- `σ_p(R − S) = σ_p(R) − S = σ_p(R) − σ_p(S)`: `x ∈ LHS ⟺ p(x) ∧ x∈R ∧ x∉S`; the
  first equality is immediate; the second because for `p(x)`-rows, `x∈σ_p(S) ⟺ x∈S`.
- **Direction matters for EXCEPT:** pushing into the *subtrahend alone* is invalid.
  `R − σ_p(S) ≠ σ_p(R − S)`: take `x ∈ R ∩ S` with `¬p(x)` — LHS keeps `x` (it was
  filtered out of `S`), RHS drops it. The minuend must always carry the filter.

**Bag (`ALL`) semantics.** `INTERSECT ALL` yields multiplicity `min(m_R(x), m_S(x))`
per row-value `x`; `EXCEPT ALL` yields `max(m_R(x) − m_S(x), 0)`. σ filters by row
value, so it acts on whole value-classes: for `p(x)`-values the multiplicity arithmetic
is untouched; for `¬p(x)`-values both sides yield zero. Distribution holds for both
`ALL` forms — with the same minuend-side caveat for `EXCEPT ALL`, for the same reason.

**Why unclassified in smelt.** The parser's branch iterator (`union_select`) follows
`UNION_KW` only; `has_set_operation` exists but no consumer classifies
INTERSECT/EXCEPT branches; the spec row says exactly this ("`INTERSECT`/`EXCEPT` not
yet classified — partial (UNION ALL)"). The refusal is the diagnostic in
`check_event_time_injectable`. Given the proofs above, admission is a
per-branch-tracing extension plus the minuend-side rule — no new algebra needed — but
note both operands are *whole-row* comparisons, so like DISTINCT they additionally
require the traced time column to survive into the compared row (it does, by the trace
precondition) and determinism of every projected column (a `RANDOM()` payload would
make the row-value comparison itself unstable; the nondeterminism taint already
rejects that independently).

### 3.13 ORDER BY / LIMIT — ORDER BY is vacuous; LIMIT breaks

`ORDER BY` on a materialized table result carries no semantics (bag output); σ commutes
trivially. `LIMIT` destroys commutation:

```sql
SELECT event_ts AS event_time, amount FROM smelt.raw.events ORDER BY event_ts DESC LIMIT 1
```

Rows: `(2026-06-01, 10)`, `(2026-07-05, 99)`. Full refresh: `{(2026-07-05, 99)}`. Push
`σ' : ts ∈ [2026-06-01, 2026-06-02)`: the run computes and writes `(2026-06-01, 10)` —
a row the full refresh does not contain, in *its own* window, so the clamp keeps it.
`LIMIT n` is a global row-set operator: σ(LIMIT(R)) ⊆ LIMIT(σ(R)) fails in both
directions, and without ORDER BY it is nondeterministic besides. Refused
(`detect()` 2c; `allow_limit` is an explicit unsafe override).

### 3.14 Correlated subqueries / EXISTS — push into the outer only; the inner needs its own reach

`EXISTS` is a per-row predicate on the outer relation, so σ on the outer commutes with
it like any WHERE (§3.2): push into the **outer** scan exactly. The **inner** relation
is a second source whose scan needs its own bound, derived from the correlation band:

```sql
SELECT e.event_ts AS event_time, e.user_id,
       EXISTS (SELECT 1 FROM smelt.raw.conversions c
               WHERE c.user_id = e.user_id
                 AND c.conv_ts BETWEEN e.event_ts AND e.event_ts + INTERVAL 7 DAY) AS converted
FROM smelt.raw.events e
```

For outer window `[t1,t2)` the inner scan must cover `[t1, t2 + 7d)` — forward reach
7d on `conversions`. Restricting the inner to `[t1,t2)` misses a conversion at
`event + 6d` landing after `t2`; restricting it not at all is correct but unbounded.
The un-widened case is exactly the seed-bug hypothesis **SC-1** in the property-
discovery catalog (`source_bounds` `(0,0)` fallback clamps the late conversion —
expected REFUTED = bug): the textual Form B walk keys bounds off the *source's own
partition column name* appearing in a shift pattern, and a correlation buried in an
EXISTS body is precisely the shape it can miss, silently defaulting the inner source to
zero margin. Verdict: outer exact; inner margin(band reach), refuse when the band is
not derivable — and today's derivation must be assumed unsound here until SC-1 is run.

### 3.15 Scalar subqueries (global aggregates) — refused

```sql
SELECT event_ts AS event_time,
       amount / (SELECT SUM(amount) FROM smelt.raw.sales) AS share
FROM smelt.raw.sales
```

Rows: `(2026-06-01, 100)`, `(2026-07-05, 300)`. Full refresh: July row's `share =
300/400 = 0.75`. Push `σ'` (window `[07-01, 07-08)`) into *the same source feeding the
subquery*: denominator becomes 300, `share = 1.0`. In-window row, wrong value,
unbounded reach (every historical row contributes) ⇒ Refused. This is the empty-
grouping-set case (§3.6) appearing in expression position; any pushdown machinery that
rewrites source refs textually (as `inject_source_filters` does — it wraps **every**
occurrence of the ref) would corrupt the subquery too, which makes the blunt
subquery gate in `detect()` (2d) load-bearing, not just conservative.

### 3.16 Self-reference — a different mechanism, not a pushdown depth

A model reading its own prior output (`smelt.<self>` in refs) is not a σ-commutation
question: the "source" is the model's own history, and correctness is about
*evaluation order*, owned by the window-independence proof — a backward-bounded
self-read admits ordered (partition-sequential) execution; a forward or unbounded
self-read refuses. The time filter on the self-edge is the run scheduler's window
arithmetic, not a pushed σ'. Out of scope for this property beyond noting the clamp's
wrapping projection was moved above the model precisely so a self-referential model's
duplicated column names cannot capture the clamp binder (`model_transforms.md`
§Known Divergences, resolved item).

## 4. Summary table (operator × verdict)

| Operator | σ commutes | Condition / margin contribution |
|---|---|---|
| Projection `e = f(c)` | exact | `f` monotone non-decreasing (trace); weakly monotone ⇒ bucket-rounded `σ'` |
| WHERE | exact | always; interval shifts feed reach, NULLable leaf gated |
| Inner JOIN, anchor side | exact | θ resolves to exactly one `Traceable` input |
| Inner JOIN, non-anchor | margin(band) / no filter | derivable time band ⇒ widened scan; else full scan (sound) |
| LEFT JOIN, preserved (anchor) | exact | filter over preserved side's columns |
| LEFT JOIN, null-supplying | margin(band) | full derived band mandatory — narrower push NULL-pads matches (value error) |
| FULL JOIN | refused | pushdown fabricates in-window NULL-padded rows; trace refuses `COALESCE` anyway |
| CROSS JOIN, anchor side | exact | σ-commutation only; cardinality proof is separate |
| GROUP BY, aligned | exact | keys ⊇ time bucket; push bucket-rounded pre-image |
| GROUP BY, unaligned / global | refused | unbounded value error |
| HAVING | inherits γ | aligned scope exact; else refused |
| Window, `PARTITION BY ⊇ partition_col` | exact | partition-local |
| Window, bounded `RANGE k` | margin(k) | before/after from frame direction |
| Window, `ROWS`/`GROUPS`/bare `LAG`/`LEAD` | refused | row-count reach has no time bound (`NotDerivable`) |
| Window, `UNBOUNDED` | refused | reach ∞ |
| DISTINCT | exact | time column in dedup key (guaranteed by trace) |
| UNION ALL | exact, branch-wise | per-branch `σ'`; `StaticSeed` branch rejects |
| UNION (DISTINCT) | exact (proven §3.11) | unclassified in smelt ⇒ refused today |
| INTERSECT [ALL] | exact (proven §3.12) | unclassified ⇒ refused today |
| EXCEPT [ALL] | exact on minuend (proven §3.12) | subtrahend-only push invalid; unclassified ⇒ refused today |
| ORDER BY (no LIMIT) | exact (vacuous) | |
| LIMIT | refused | global row-set operator |
| EXISTS / correlated subquery | exact (outer) | inner source: margin(correlation band); SC-1 flags today's derivation |
| Scalar subquery (global agg) | refused | unbounded value error; textual injection would corrupt it |
| Self-reference | n/a | ordered-execution property, not pushdown |

## 5. Composition algebra

### The lattice

Per (model, source), verdicts order as:

```
Source   ≻   OuterClamp(m)  [smaller m ≻ larger m, componentwise (before, after)]   ≻   Refused
```

`Source` behaves as `OuterClamp(0,0)` semantically; the distinction is operational —
it licenses dropping the outer wrap entirely (`is_transparent_single_source`), which is
why the code keeps it a separate classification rather than a zero margin.
`Refused` is ⊥ and absorbing. `Unbounded` and `NotDerivable` are two *routes* to ⊥
with different fallbacks (per-partition vs outright refusal) — the lattice does not
distinguish them but the diagnostics must.

### Composition rules

The verdict for a source is computed over the **path** from the output projection to
that source's scan, one operator at a time:

1. **Sequential composition (operator stacking): margins add.** If operator `A` sits
   above operator `B` on the path and each individually contributes reach `m_A`, `m_B`,
   the composed reach is `m_A + m_B` — an in-window output row reads `B`-rows up to
   `m_A` away, each of which reads source rows up to `m_B` further. Formally:
   `widen_{m_A}` applied to a query needing `widen_{m_B}` requires the source scanned
   over the `m_A + m_B` widening. Any ⊥ on the path makes the whole path ⊥
   (absorbing); `Source` is the identity.
2. **Parallel composition (branches of a set operation): per-branch paths are
   independent.** Each branch keeps its own path verdict and its own `σ'`. Only if the
   physical plan applies a *single* widened window at the union level does the union
   take `max` over branch margins (each branch's scan must cover its own reach; a
   shared window must cover the worst).
3. **Rewrites compose functionally.** `σ'` at depth `k` is the pre-image under the
   composition of the projections crossed so far; a weakly monotone stage inserts a
   bucket-rounding that later stages must respect (round *outward*, never inward).
4. **The clamp is applied once**, at the top, over the output schema, regardless of
   how many margined stages contributed — margins add into one scan widening; there
   is never a per-stage clamp (an inner clamp would re-introduce the truncated-context
   value errors the margin exists to avoid).

### Worked example 1 — two transparent CTEs, UNION ALL: still Source

```sql
WITH a AS (SELECT event_date, amount FROM smelt.raw.pos_sales),
     b AS (SELECT event_date, amount FROM smelt.raw.web_sales)
SELECT event_date, amount FROM a
UNION ALL
SELECT event_date, amount FROM b
```

Both branch paths are (projection: identity) ∘ (UNION ALL branch): exact ∘ exact.
Per-branch trace resolves `pos_sales.event_date` / `web_sales.event_date`; both
`Bounded(0,0)` ⇒ both `Source`. Two filters, one per source scan, no outer clamp
needed. Code agrees: `restrict_ctx_for_union` admits both `Traceable` branches, each
bound classifies `Source`. (One caveat: the *runtime* transparent fast path
`is_transparent_single_source` requires a **single** source, so this two-source model
keeps a redundant outer clamp today — harmless, mildly wasteful.)

### Worked example 2 — same union, one branch carries a 3-day window: per-branch depths

```sql
WITH a AS (SELECT event_date, amount FROM smelt.raw.pos_sales),
     b AS (SELECT event_date,
              SUM(amount) OVER (ORDER BY event_date
                                RANGE BETWEEN INTERVAL 3 DAY PRECEDING AND CURRENT ROW)
                AS amount
           FROM smelt.raw.web_sales)
SELECT event_date, amount FROM a
UNION ALL
SELECT event_date, amount FROM b
```

Algebra: branch `a` = `Source`; branch `b` = `OuterClamp(before=3d)`. The correct plan
is **per-branch injection**: exact filter on `pos_sales`, widened filter on
`web_sales`, one clamp above the union (needed only because of branch `b`). The whole-
model verdict is the branch meet — `OuterClamp(3d)` — but stating it only model-wide
throws away the fact that branch `a` needed no margin.

What the code does today: `restrict_ctx_for_union` admits both sources into one ctx;
then `derive_bound_for_source` runs its **global textual scan per source** — "any
`RANGE BETWEEN INTERVAL` in the SQL contributes a bound" (`source_bounds.rs`, Form A
heuristic) — so *both* sources are assigned `before = 3d`. Branch `a`'s scan is
over-widened by 3 days (sound: scan ⊇ write still holds; wasteful: extra margin read
every run) and both sources classify `OuterClamp`, keeping the clamp (correct).
Per-branch injection depth is representable in the algebra but not in the
implementation: the verdict map is keyed by source only, and the bound walk cannot
attribute a frame to a branch.

### Worked example 3 — an unaligned inner GROUP BY poisons everything above it

```sql
WITH lifetime AS (
  SELECT user_id, COUNT(*) AS lifetime_events
  FROM smelt.raw.events
  GROUP BY user_id                        -- unaligned: no time bucket in the keys
)
SELECT e.event_date, e.user_id, l.lifetime_events
FROM smelt.raw.events e
JOIN lifetime l USING (user_id)
```

The path from the output to the `events` scan *through `lifetime`* crosses an
unaligned γ ⇒ ⊥, absorbing: no margin placed above the CTE repairs it (the
`lifetime_events` value of an in-window row depends on all history — unbounded value
error), and clamping above it cannot help because clamps only delete rows. The *other*
path (through `e`) is exact — but both paths terminate in the **same source**, and the
per-source verdict must be the meet over **all** paths to that source: ⊥. So the model
refuses batched pushdown outright (or the CTE is materialised as its own upstream model
with `refresh: keyed`, restoring a clean single-path verdict per model — the DAG-
composition answer the litmus rule prefers). Had `lifetime` read a *different* source,
`e`'s source would remain `Source` and only the dimension-like source would refuse —
per-(model, source) scope earning its keep.

### Worked example 4 — stacked margins: the algebra says add, the code takes max

```sql
WITH daily AS (
  SELECT event_date,
         SUM(amount) OVER (ORDER BY event_date
                           RANGE BETWEEN INTERVAL 7 DAY PRECEDING AND CURRENT ROW) AS s7
  FROM smelt.raw.sales
)
SELECT event_date,
       AVG(s7) OVER (ORDER BY event_date
                     RANGE BETWEEN INTERVAL 3 DAY PRECEDING AND CURRENT ROW) AS smooth
FROM daily
```

An output row at `t` reads `daily` rows over `[t−3d, t]`; the `daily` row at `t−3d`
reads source rows over `[t−10d, t−3d]`. True backward reach = **10 days** (rule 1:
margins add along a path). `BoundResult::merge` is documented and implemented as
`before = max(before_i)` ("union semantics" — correct for *parallel* frames in one
scope, e.g. a 7d and a 3d window side-by-side in the same SELECT), and
`derive_bound_for_source` accumulates every Form A pattern found in the flat SQL text
through that same max-merge. Result: `before = 7d` — a scan two runs' rows **too
narrow**, producing wrong `smooth` values on the first three days of every run window.
This is a candidate **soundness gap**, not just an imprecision: the textual walk has no
scope structure, so it cannot tell nested from parallel frames. Without AST scoping the
safe merge is **sum** (an over-approximation: sum ≥ true reach whether frames nest or
sit in parallel); with AST scoping, add along nesting and max across siblings. Either
way this deserves a failing equivalence-harness case before any fix (red-green), and
until then multi-window stacking should arguably be `NotDerivable`.

## 6. Static provability vs declaration

Every input to this property follows derive-else-declare, and every declaration only
widens:

- **`σ'` (the rewrite) is derived only.** The trace's whitelist (identity, constant
  `INTERVAL` shifts, recognised monotone functions) is the decidable core; Richardson's
  theorem is why there is no general prover (*[pushdown-research]* Area 5). The one
  declaration, `timeseries.assert_monotonic`, widens exactly the
  `NotTraceable{Undecidable}` verdict (an opaque UDF wrapping a single column-bearing
  argument) and is threaded in at `restrict_ctx_for_join`; a `Disproven` shape (periodic
  `EXTRACT`, piecewise `CASE`, two-column arithmetic, row-nondeterminism) or a
  `StaticSeed` refuses regardless. The nullability gate then sits on top: a declared-
  monotone but nullable leaf still refuses (§3.2's aggregation counterexample is why).
- **`m` (the reach) splits into a derived and a declared term.** Computation-reach
  (frames, bands, WHERE shifts) is derived; **`source_lateness`** is a declared
  world-fact on the source that *adds to* the margin — widening only, so an honest
  declaration can never narrow a scan. A symbolic (month/year) interval in a
  bound-relevant position is `NotDerivable`, never approximated to fixed days.
- **Alignment is derived** from the AST (`scope_group_by_alignment`,
  `scope_distinct_alignment`, window-`OVER` scope) with per-scope judgement across
  UNION branches. The `safety_overrides.allow_*` flags are the *un-proved* escape
  hatches — unlike the declarations above they assert nothing about the world and
  simply disable a check; they are the surface where fail-closed is traded away
  explicitly by the modeller.
- **Fail-closed at every meet.** `NotDerivable` refuses the model
  (`derive_model_source_bounds` returns `Err` before any transform runs;
  `batch_safety_from_bounds` checks `NotDerivable` *before* `Unbounded` so a refusal
  can never be outclassed by a coarser-but-buildable verdict from another source);
  `Unbounded` routes to per-partition; an unrecognised construct never defaults to
  `Source`. The `horizon_ceiling` declaration is the deliberate odd one out — it can
  widen nothing and only licenses a compile-time warning.

## 7. Implementation gaps (specific, from the code)

1. **The verdict is binary and margin-less.** `InjectionPoint` (`source_bounds.rs:176`)
   is `Source | OuterClamp`; `Source` iff `Bounded{0,0}`. The margin lives in the
   paired `BoundResult`, ⊥ has no variant (refusal happens earlier as `Err`), and
   there is no per-branch or per-path dimension. Adequate for today's two consumers;
   the composition algebra of §5 (per-branch depth, additive path margins) does not fit
   in it.
2. **Set-operation distribution covers `UNION ALL` only.** The union path in
   `restrict_ctx_for_constructs` gates on `has_union() && is_union_all()`
   (`incremental.rs:1170`); `check_event_time_injectable` unconditionally errors for
   plain UNION/INTERSECT/EXCEPT (`rule_diagnostics.rs:256-277`); the parser's
   `union_select` iterator follows `UNION_KW` only, and `has_set_operation` has no
   classifier consumer. §3.11–3.12 prove σ distributes over all of them (minuend-side
   rule for EXCEPT), so this is unclassified-not-unsound — the spec row already says
   `partial (UNION ALL)`.
3. **Per-branch injection is unimplemented.** `restrict_ctx_for_union` merges all
   branch sources into one flat ctx; one `NotTraceable` branch returns an *empty* ctx
   (no pushdown anywhere in the model — outer clamp + full scans), and the UNION-branch
   wrap-and-filter transform is catalogued **unbuilt** (`model_transforms.md`).
4. **Bound attribution is global-textual, causing cross-branch/cross-source margin
   contamination.** `derive_bound_for_source` scans the whole SQL per source ("any
   `RANGE BETWEEN INTERVAL` … contributes a bound"), so a frame in one branch widens
   every source's scan (safe, wasteful — worked example 2), and Form B matches by
   partition-column *name*, so two sources sharing a column name cross-attribute
   bounds.
5. **Margin merge is `max`, not path-additive.** `BoundResult::merge`
   (`source_bounds.rs:203-229`) + the flat Form A accumulation under-widen nested
   window stacks (worked example 4) — the one identified *candidate unsoundness* in
   this analysis; needs a red harness case (the property-discovery loop's linkC
   apparatus is the natural home).
6. **Admission scans on uppercase substrings, not AST `PartitionAlignment`.** The
   window-`OVER` scan (`find_inadmissible_over`, `has_bounded_range_interval_frame`)
   and the `LIMIT` scan (`has_keyword_at_boundary`) in `detect()` are textual;
   `model_properties.md` §Known Divergences records that they have not been rewired
   onto the AST window-`OVER` `PartitionAlignment` signal (HAVING and DISTINCT already
   are, per-scope across UNION branches). Textual scans over-reject (an `OVER` inside a
   string literal or comment) and under-analyse (no per-scope judgement for windows).
7. **The transparent fast path is single-source only.** `is_transparent_single_source`
   requires exactly one source; a multi-source all-zero-margin model keeps a redundant
   outer clamp (worked example 1's caveat). Sound; a small completeness gap.
8. **Correlated-subquery reach is untrusted.** SC-1 (catalog) hypothesises the
   `(0,0)` fallback silently clamps a late conversion inside an `EXISTS` band —
   i.e. §3.14's inner-source margin is not reliably derived today. Also
   `inject_source_filters` rewrites *every* textual occurrence of a source ref, so a
   scalar-subquery global aggregate over the same source would be silently corrupted
   if the subquery gate (2d) were relaxed without making the injection scope-aware.
9. **Day-granular rounding at the runtime edge.** `inject_source_filters` converts
   margins to whole days with `div_ceil` and outward rounding (`transformer.rs`) —
   conservative-correct for date partitions; will need revisiting for sub-day
   granularities.

## 8. Open questions

1. **Per-branch injection representation.** Should the verdict be keyed
   `(model, source, path)` rather than `(model, source)` — and is the UNION-branch
   wrap-and-filter transform the forcing consumer, or does the flat over-widened form
   stay good enough until sub-day granularities make margin waste expensive?
2. **Margin composition.** Confirm worked example 4 against the equivalence harness;
   then choose: refuse multi-window stacking (`NotDerivable`), sum-merge flat bounds
   (safe over-approximation), or move bound derivation onto AST scopes (add along
   nesting, max across siblings).
3. **Admitting the proven set-ops.** UNION DISTINCT / INTERSECT [ALL] / EXCEPT [ALL]
   are σ-distributive (§3.11–3.12). What is the actual admission bar — per-branch
   trace over a generalised branch iterator plus the EXCEPT minuend rule, plus a
   determinism check on the compared row — and is there demand before the machinery is
   built?
4. **`Source` vs `OuterClamp(0,0)`.** Should the multi-source zero-margin case drop
   the outer clamp (generalising `is_transparent_single_source`), collapsing the
   operational distinction into "clamp iff any source has nonzero margin"?
5. **Who owns bucket-rounding of `σ'`?** The trace's `offset` models constant shifts
   only; a weakly-monotone stage (`date_trunc` in the projection, not the GROUP BY
   key) needs the pre-image endpoints rounded outward at injection time. Today this is
   implicit in granularity-aligned run windows — should the trace carry an explicit
   `bucket` component so misaligned windows are rejected (or rounded) by proof rather
   than by convention?
