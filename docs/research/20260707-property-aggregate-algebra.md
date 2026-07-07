# Property research: aggregate algebra / combiner discriminants — and how they compose

- **Date**: 2026-07-07
- **Status**: research
- **Related specs**: `docs/specs/model_properties.md` (§Surface row "Algebraic discriminants", §Semantics "Algebraic discriminants (the raw facts, not the ladder)" — the owner of the discriminants); `docs/specs/model_maintenance.md` (§"The algebraic maintenance ladder", §"The equivalence invariant" — the owner of the *ordering* of these facts and the maintainable/delegated cutoff; this doc discusses ladder consequences only by citation); `docs/specs/keyed_models.md` (§"Column families" — the built consumer)
- **Related code**: `crates/smelt-logical/src/analysis/discriminants.rs` (`combiner_discriminants`, `Discriminants`, `Monotone`); `crates/smelt-types/src/functions.rs` (`SqlFunction`, `from_name`, `is_aggregate`); `crates/smelt-logical/src/analysis/bounded_domain.rs` (the holistic-widening declaration)
- **Prior research**: `docs/research/20260704-maintenance-fundamentals.md` (proofs / world-facts / transforms spine); `docs/research/property-discovery/catalog.md` (cells G-01…G-10 cited inline); sibling docs in this discovery series: `docs/research/20260707-property-delta-shape.md` (delta shapes — cited as sibling, not depended on) and the fan-out/join-cardinality property doc (same series)

Scope note: everything here is **per-column**. Each aggregate output column carries its own discriminant tuple; a model is a vector of columns, each independently classified, independently admitted (`maintenance_plan.md` §"Per-cell admission"), and independently maintainable. Nothing in this doc is a model-level verdict.

---

## 1. The property

### 1.1 The discriminant tuple

For each aggregate combiner (per output column), the raw algebraic facts are the tuple the code already carries (`analysis/discriminants.rs::Discriminants`):

| Field | Meaning | Algebraic name |
|---|---|---|
| `is_monoid` | the combiner is a **commutative monoid** on its state: closed, associative, has identity (= empty group's value). A fold of contributions is well-defined without replay, in any order, in any bracketing | commutative monoid |
| `needs_inverse` | the monoid has **no inverse**: a contribution cannot be un-seen; retraction requires replay. `needs_inverse = false` on a monoid means it **is a group** (every contribution has an inverse combine) | group vs bare monoid |
| `decomposable` | the *presented* value is not itself a monoid, but is `π(state)` for a richer state that **is** a commutative monoid and a pure presentation map `π` | homomorphic decomposition |
| `monotone` | `Value` (the presented value only ever moves one way under insert), `Order` (the value may switch, but only in step with a monotone ordering key — a semilattice fold on `(order, value)` pairs), or `None` | (semi)lattice facts |

Two further facts are **load-bearing for composition** but are *not* fields of the tuple today (this is a finding, §6):

- **Idempotence** (`x ⊕ x = x`): the discriminant that decides survival under duplication — `UNION DISTINCT`, fan-out joins, redelivered deltas (§4.5–4.6). Within today's catalogue it happens to coincide with `is_monoid ∧ needs_inverse` *minus BIT_XOR*, but that coincidence is an accident of which combiners are enumerated, not a theorem.
- **Multiset-function-ness / order-sensitivity**: whether the combiner is even a *function of the input multiset*. `STRING_AGG` without `ORDER BY`, `FIRST`/`LAST`/`ANY_VALUE` are not — their value depends on physical evaluation order. Every discriminant above presupposes multiset-function-ness; a non-function has no algebra to classify.

Also implicit in every row below: the algebra is stated over **exact value domains**. `SUM(DOUBLE)` and `PRODUCT(DOUBLE)` are not associative in IEEE-754; the equivalence invariant (`model_maintenance.md`) over floats holds only up to rounding, which the generative oracle must tolerate. Over `INTEGER`/`DECIMAL`/`HUGEINT` the algebra is exact (DuckDB sums integers into `INT128`).

### 1.2 The standard catalogue

Per combiner, `(is_monoid, needs_inverse, decomposable, monotone)` plus the two shadow facts (idempotent, multiset-function):

| Combiner | monoid | needs-inverse | decomposable | monotone | idempotent | multiset-fn | Notes |
|---|---|---|---|---|---|---|---|
| `SUM` | yes | no (**group**) | — | None | no | yes | identity 0; inverse = subtract |
| `COUNT(*)` | yes | no (**group**) | — | None | no | yes | counts rows; every row contributes 1 |
| `COUNT(col)` | yes | no (**group**) | — | None | no | yes | NULL rows contribute the identity (0) — same algebra as `COUNT(*)`, different **per-row map** (§3.1); the delta computation must apply the same null-skip or it double/under-counts |
| `BIT_XOR` | yes | no (**group**) | — | None | **no** | yes | self-inverse: `x ⊕ x = identity` — the *opposite* of idempotent. Survives retraction for free; corrupted by duplication |
| `MIN` / `MAX` | yes | **yes** | — | **Value** | **yes** | yes | semilattice (meet/join); identity ±∞ (SQL: NULL as empty-group value) |
| `BOOL_AND` / `BOOL_OR` | yes | **yes** | — | Value* | **yes** | yes | `BOOL_OR` is `MAX` on `{false<true}`; value-monotone under insert exactly like MIN/MAX (*code currently claims `None` — §6). `BOOL_OR ≅ EXISTS` |
| `BIT_AND` / `BIT_OR` | yes | **yes** | — | None† | **yes** | yes | †per-bit value-monotone, but the presented integer is not ordered-monotone; claiming `None` is correct at the value level |
| `AVG` | no | — | **yes** | None | no | yes | state `(sum, count)`, both group columns; `π = sum/count` |
| `VAR_*` / `STDDEV_*` | no | — | **yes** | None | no | yes | state = Welford/Chan triple `(count, mean, M2)` or the moment triple `(n, Σx, Σx²)`; the moment form is a **group** (all three components invertible), the Welford form merges but numerically better |
| `CORR` / `COVAR_*` / `REGR_*` | no | — | **yes** | None | no | yes | co-moment tuples; same story as variance (not in `SqlFunction`'s classified set today) |
| `APPROX_COUNT_DISTINCT` | no | — | **yes** | None | **state is idempotent** | yes | state = HLL register vector; merge = per-register `MAX` — an **idempotent** monoid, so the *state* survives dedup and fan-out even though the presented estimate is not additive |
| `MEDIAN` / `PERCENTILE_*` / quantiles | no | — | no (**holistic**) | None | — | yes | needs the full multiset; only rung-4 multiset state maintains it (`model_maintenance.md`) |
| `MODE` | no | — | no (**holistic**) | None | — | yes | value→count map is the natural (bounded-domain) state |
| exact `COUNT(DISTINCT x)` | no | — | no (**holistic**) | None | — | yes | = cardinality of the seen-set; the seen-set itself is an idempotent monoid but `O(domain)` — exactly the rung-4 bounded-domain multiset |
| any `agg(DISTINCT x)`, non-idempotent agg | no | — | no (**holistic**) | None | — | yes | §3.3/§4.5: DISTINCT strips multiplicity, which only idempotent combiners never depended on |
| `STRING_AGG(x, sep ORDER BY k)` / `ARRAY_AGG(x ORDER BY k)` | no | — | yes, **unbounded state** | None | no | yes (up to k-ties) | state = the ordered list keyed by `k`; merge = sorted-merge — associative and commutative *because the ordering key externalises the order*. Algebraically decomposable; practically holistic-priced (`O(n)` state), so it belongs on rung 4's opt-in side, not rung 2 |
| `STRING_AGG` / `ARRAY_AGG` **without** `ORDER BY` | — | — | — | — | — | **NO** | not a function of the multiset — value depends on scan order. Nothing to classify; fail-closed is the only sound verdict (compare `KeyedForbidsNondeterministic`) |
| `FIRST` / `LAST` / `ANY_VALUE` | — | — | — | — | — | **NO** | order-/plan-dependent. `ANY_VALUE` is sound only under the keyed *plain overwrite* family's snapshot semantics ("incoming row wins" — `keyed_models.md`), which is an addressing convention, not a fold. `FIRST(x ORDER BY k)` (where a dialect supports it) is `MIN_BY(x, k)` — see next row |
| `MAX_BY` / `MIN_BY` / `ARG_MAX` / `ARG_MIN` | no | — | yes: state `(k*, v*)` | **Order** | yes (on the pair state, up to k-ties) | up to ties | semilattice fold on pairs ordered by `k`; the presented `v` may jump arbitrarily, but only when `k` advances. Ties on `k` are the classifier's documented carve-out (`keyed_models.md` §"Ordering ties") |
| `PRODUCT` | yes | **no — except zero** | yes (repair) | None | no | yes | on ℝ\{0} a group (inverse = divide); **0 is absorbing** — once a zero enters, no inverse exists (`product(2,0,3) = 0`, verified DuckDB, and dividing by 0 cannot un-see it). Decomposition repairs it: state `(product of non-zeros, count of zeros)` — both group columns — with `π = if zeros>0 then 0 else product`. A textbook case of `decomposable` upgrading `needs_inverse` |
| `EXISTS` (join-existence) | — | — | — | Value | — | — | not an aggregate combiner; its value-monotonicity is a join-contribution fact (fan-out proof's territory), which is why `combiner_discriminants` deliberately does not classify it (doc comment in `discriminants.rs`) |

`COUNT(*)` **vs** `COUNT(col)` deserves the explicit sentence: they share the identical group algebra; they differ only in the per-row contribution map (`1` vs `CASE WHEN col IS NOT NULL THEN 1 ELSE 0 END`). The discriminant tuple is the same; what changes is that a *delta* for `COUNT(col)` must be computed with the same null-skip, and an UPDATE that flips a value between NULL and non-NULL is a genuine retraction+insert for `COUNT(col)` while being a no-op for `COUNT(*)`. Composition-relevant, discriminant-irrelevant.

---

## 2. Why maintenance needs it

The ladder (`model_maintenance.md` §"The algebraic maintenance ladder") is exactly these facts ordered by maintenance consequence — restated here only by citation, since the ordering is owned there:

- **`is_monoid`** is the licence for **fold-delta**: `state' = state ⊕ fold(delta)` with no replay, in any delivery order (order/set-determinacy corollary of the equivalence invariant). Catalog G-01 is the happy path; G-07 is the refusal for non-monoids (recompute-only).
- **`needs_inverse`** is the retraction boundary: with an inverse (rung 3's groups), an update/delete is `state ⊕ inverse(old) ⊕ new` — still fold-delta. Without one, a retraction forces replay of the affected key/partition. G-04 is the refuted case: `MIN` under a *mutable snapshot* — idempotent folding is fine for inserts but the source retracts, and MIN cannot un-see.
- **`decomposable`** is the licence for rung 2's state-table-plus-presentation-view: maintain the monoid state, present `π(state)`. Fidelity is exact — decomposition changes the *representation*, never the value.
- **holistic** (¬monoid ∧ ¬decomposable) is rung 4's opt-in bounded-domain multiset or delegation (`refresh: materialized_view`) or `refresh: full`.
- **`monotone`** is not a maintenance licence by itself; it is what makes the *overwrite* column families sound (`keyed_models.md`: extremal fold is re-run-safe because idempotent; order-monotone overwrite is merge-safe up to ties) and what downstream monotonicity traces consume.

The per-column framing matters here: one model can hold a `SUM` (group — absorbs corrections), a `MAX` (monoid — insert-only fine, corrections force replay), and a `MEDIAN` (holistic — refused without a budget) side by side; the plan is the meet of per-cell verdicts, not one model-level classification.

---

## 3. Per-construct analysis: how SQL constructs transform the discriminant

Each construct below is a function **Discriminants → Discriminants** (or → refusal). Minimal SQL and a counterexample per row; all SQL verified against DuckDB v1.5.

### 3.1 Per-row maps inside the aggregate: `CASE`, arithmetic, `FILTER` — preserving

```sql
SUM(CASE WHEN status = 'paid' THEN amount ELSE 0 END)
SUM(amount) FILTER (WHERE status = 'paid')
COUNT(col)          -- itself SUM(CASE WHEN col IS NOT NULL THEN 1 ELSE 0 END)
```

All three are `fold(⊕, h(row))` for a pure per-row map `h` into the same monoid. Precomposition with a pure map preserves **every** discriminant: associativity/commutativity/identity/inverse are properties of `⊕`, untouched by `h`. `FILTER (WHERE p)` is the special case `h(row) = if p(row) then f(row) else identity`.

**Condition, with counterexample**: `h` must be deterministic and row-local. `SUM(CASE WHEN random() < 0.5 THEN x END)` is not a function of the multiset at all — re-folding a delta yields a different contribution than the full refresh would, violating the equivalence invariant regardless of the monoid (this is why `KeyedForbidsNondeterministic` exists). Determinism of `h` is a *precondition* the discriminants sit on top of, not a discriminant.

### 3.2 Expressions **around** aggregates — a presentation map over a state vector

```sql
SELECT k,
       SUM(x) / COUNT(x)          AS avg_rederived,   -- AVG spelled out
       SUM(gross) / SUM(net)      AS ratio,
       MAX(t) - MIN(t)            AS span
FROM src GROUP BY k
```

Each such column is `π(c₁, …, cₙ)` for a pure scalar `π` over columns that are individually monoid/group/decomposable. **The composite is not a monoid** — there is no `⊕` such that `ratio(A ⊎ B) = ratio(A) ⊕ ratio(B)` (ratios don't add). But it never needs to be one: maintenance operates on the state vector `(SUM(gross), SUM(net))`, each maintained under its own discriminant, and `π` is applied at read time. This is precisely rung 2 generalised: *decomposability is closed under tupling* — a tuple of monoids is a monoid (componentwise), and any pure map over it is a valid presentation. So `SUM(x)/COUNT(x)` is literally `AVG` re-derived by hand: same `(sum, count)` state, same `π`.

Why it's fine, spelled out: the equivalence invariant quantifies over the *state*; `π` commutes with everything because it touches no input rows. The one condition is that `π` reads **only** the maintained state columns of the same row (no cross-row, no side inputs).

Today `keyed_models.md` rejects composite expressions over aggregates (`KeyedUnknownCombiner`: "`SUM(x) + 1` … add columns for the underlying aggregates and derive downstream") — sound and conservative; the derive-downstream workaround is exactly "make the presentation map a downstream model". Recognising in-model presentation maps is a widening this analysis licenses (Open Question 1).

### 3.3 `DISTINCT` modifier — group → holistic (except idempotent)

```sql
SELECT SUM(DISTINCT x) FROM (VALUES (5),(5),(3)) v(x);  -- 8, not 13
```

`DISTINCT` makes the aggregate a function of the **support set**, not the multiset. Counterexample to fold-delta: state 8 (having seen {5,3}); a delta row `5` arrives; the monoid fold says `8 + 5 = 13`; truth is 8. The fold cannot know the 5 was already seen without carrying the seen-set — which *is* the holistic multiset state. So `DISTINCT` sends `SUM`/`COUNT` (groups!) to **holistic** — the sharpest single-token discriminant demotion in SQL.

**Exception — idempotent combiners are DISTINCT-transparent**: `MIN(DISTINCT x) = MIN(x)`, `MAX(DISTINCT x) = MAX(x)`, `BOOL_OR(DISTINCT x) = BOOL_OR(x)`. Proof in §4.5 (same theorem as UNION-DISTINCT survival). The current classifier blanket-demotes on `distinct = true` (`discriminants.rs` line 80) — sound but over-conservative for the idempotent set (§6).

### 3.4 Aggregates over expressions of **joined** columns

```sql
SELECT f.k, SUM(f.qty * d.unit_price) AS revenue
FROM facts f JOIN dims d ON f.product_id = d.product_id
GROUP BY f.k
```

The combiner is still `SUM` — group, unchanged discriminant. What changes is **whose delta retracts it**: the per-row map `h(row) = qty * unit_price` now reads a join partner, so a *dimension* update is a retraction (`-qty*old_price`) plus insert (`+qty*new_price`) against every joined fact. Group columns absorb this (given the old value is recoverable); monoid-only columns (`MIN(f.ts * something(d))`) cannot and force replay. And if the join fans out (one fact ↔ many dims), the contribution is *duplicated* — §4.5. Catalog G-05 (mutable dimension under column re-derivation) is the refuted/conditional cell; the once-write and dimension-driven-horizon-MERGE machinery (`keyed_models.md`, `model_transforms.md`) exist precisely to license the recoverable cases. Discriminant verdict: **preserved on the combiner; the join moves the problem into delta shape and fan-out**, which are the sibling properties.

### 3.5 Window-function analogues — same algebra, different addressing

```sql
SUM(x) OVER (PARTITION BY k ORDER BY t ROWS UNBOUNDED PRECEDING)  -- running total
```

The running total is the **same** `SUM` monoid, folded per *prefix* instead of per group: row `i`'s value is `fold(⊕, rows ≤ t_i)`. Every discriminant carries over verbatim; what changes is the **addressing** of the output — one stored value per input row (a trajectory), not per key. Consequence (catalog G-08): a late row with timestamp `t*` changes the presented value of **every row with `t ≥ t*`** — the delta shape is a suffix rewrite, not a point merge, even though the algebra is a perfectly invertible group. This is the cleanest demonstration that discriminants and delta shape are orthogonal properties: the algebra says "fold-delta is sound per address"; the addressing says "one late input touches unboundedly many addresses". (`keyed_models.md` bans `OVER` in keyed bodies — "the keyed state *is* the window" — exactly because the keyed end-state grain is the one window shape whose delta stays point-addressed.) Bounded frames (`ROWS BETWEEN 6 PRECEDING AND CURRENT ROW`) bound the suffix to the frame reach — this is `derive_model_bounds`'s computation-reach, not a discriminant change.

### 3.6 `GROUPING SETS` / `ROLLUP` / `CUBE` — a rollup *is* a re-aggregation

```sql
SELECT region, product, SUM(rev) FROM s GROUP BY ROLLUP (region, product)
```

The `(region)` rows are exactly `SUM` re-aggregated over the `(region, product)` rows; the grand-total row re-aggregates those. So a rollup is a materialised **two-level fold**, and maintaining a rollup incrementally from its finest grain is admissible per column exactly by the re-aggregation rules of §4.1: `SUM`/`MIN`/`MAX`/`BOOL_*` cells maintain from the finer grain; an `AVG` cell must carry the decomposed `(sum, count)` at the finer grain (or recompute); a `MEDIAN` cell never maintains from the finer grain (§4.3, medians-of-medians). The engine computing a one-shot `ROLLUP` from base rows is always correct — the discriminant question only bites when the coarse cells are *maintained from* the fine cells rather than from base.

### 3.7 `HAVING` — a post-filter on the aggregate flips output-row membership

```sql
SELECT k, COUNT(*) AS n FROM s GROUP BY k HAVING COUNT(*) < 3
```

`HAVING` does not touch the combiner's discriminant — the state fold is unchanged. It changes what an *insert* does to the **output**: inserting `k`'s third row **deletes** `k`'s output row. Under append-only input the output is no longer grow-only, even though `COUNT` is a group and the input is monotone. Maintenance must therefore evaluate the predicate on the *post-fold* state and emit the membership flip (insert / update / delete of the output row) — "predicate-flip detection". With a value-monotone aggregate and a one-sided predicate (`HAVING MAX(t) >= X` under insert-only) the flip is itself monotone (rows only ever *enter*), recovering a grow-only output; with a group aggregate or a two-sided predicate, flips go both ways. Verdict: **discriminant preserved on the state; a new obligation (flip detection) added on the output**, and the output's own delta shape changes — sibling doc territory (`20260707-property-delta-shape.md`).

### 3.8 `ORDER BY` inside the aggregate

```sql
SELECT string_agg(x, ',' ORDER BY y) FROM (VALUES ('b',2),('a',1)) v(x,y);  -- 'a,b'
```

Concatenation is associative but **not commutative**; unordered `STRING_AGG` is therefore not even a multiset function. `ORDER BY y` externalises the order into data: the result becomes a genuine function of the multiset of `(x, y)` pairs (up to `y`-ties), and the natural state — the `y`-sorted list — has a commutative merge (sorted two-way merge). So `ORDER BY` inside the aggregate is a discriminant **upgrade**: non-function → decomposable-with-unbounded-state. The price is `O(n)` state per key, which puts it economically with rung 4's opt-in budget rather than rung 2, and retraction (delete an element from the middle) needs the list state anyway (making it a Z-set-of-pairs, retraction is free). Ties on `y` are the same carve-out as `MAX_BY` ties.

### 3.9 Transfer table (construct × discriminant)

| Construct | is_monoid | needs_inverse | decomposable | monotone | idempotent | Net verdict |
|---|---|---|---|---|---|---|
| pure row map inside agg (`CASE`, arithmetic) | = | = | = | = | = | preserving (precondition: deterministic, row-local) |
| `FILTER (WHERE p)` | = | = | = | = | = | preserving (same precondition on `p`) |
| scalar expr **around** agg(s) | composite: — | — | **yes** (tuple state + `π`) | value-level lost | — | maintain components, present `π`; composite column itself is only a presentation |
| `DISTINCT` modifier | **→ no** | — | **→ no** | → None | = (idempotent: entire row unchanged) | **holistic**, except idempotent combiners where it is the identity transform |
| agg over joined-column expr | = | = | = | = | = | discriminant preserved; retraction/duplication obligations move to join-shape + delta-shape properties |
| window prefix fold (`OVER … UNBOUNDED PRECEDING`) | = | = | = | = | = | same algebra; addressing change → suffix-rewrite delta (G-08) |
| `ROLLUP`/`CUBE`/`GROUPING SETS` | per §4.1 | per §4.1 | needed at fine grain for AVG-class | = | = | coarse cell = re-aggregation of fine cell; admissible iff §4.1 admits |
| `HAVING p(agg)` | = | = | = | output monotone only if agg value-monotone ∧ `p` one-sided | = | state preserved; adds predicate-flip obligation on output membership |
| `ORDER BY` inside agg | no → | — | **→ yes** (unbounded list state) | — | no | upgrade from non-function; rung-4 economics; ties carve-out |

---

## 4. Composition algebra — the heart

Notation: `A ∘ B` means outer aggregate `A` applied to the per-group results of inner aggregate `B` (a two-level `GROUP BY`, the finer grouping inside). `⊎` is multiset union.

### 4.1 Re-aggregation: when does `A ∘ B` collapse to a one-level fold?

**General condition.** Let `B` have monoid state `(S, ⊕, e)` (directly, or via decomposition). A two-level fold equals the one-level fold iff the outer combiner applied to the inner *states* is `⊕` itself:

```
fold⊕( { fold⊕(P₁), fold⊕(P₂), … } )  =  fold⊕(P₁ ⊎ P₂ ⊎ …)
```

which is exactly **associativity + commutativity of `⊕`** — always true for a commutative monoid. So *every* monoid re-aggregates **through its own merge operator**. The subtlety is that the SQL surface name of the merge operator may differ from the surface name of the aggregate:

| Inner `B` | Correct outer (the merge partner) | Same-name outer sound? |
|---|---|---|
| `SUM` | `SUM` | yes — `SUM ∘ SUM = SUM` |
| `COUNT` | **`SUM`** | **no** — `COUNT ∘ COUNT` counts *groups* (see 4.2); `SUM(cnt)` re-counts rows |
| `MIN` / `MAX` | `MIN` / `MAX` | yes — `MIN ∘ MIN = MIN` (verified: min over per-day mins = global min) |
| `BOOL_AND/OR`, `BIT_AND/OR` | same | yes (idempotent monoids nest like MIN/MAX) |
| `BIT_XOR` | `BIT_XOR` | yes |
| `AVG` | none at the value level | **no** — `AVG ∘ AVG ≠ AVG` (4.3); the *state pair* re-aggregates fine |
| `APPROX_COUNT_DISTINCT` | HLL register-max merge | no at value level (estimates don't add); yes at state level |
| `MAX_BY(v, o)` | `MAX_BY(v, o)` **with `o = MAX(o)` carried** | only if the winning ordering key is projected alongside (4.4) |
| `MEDIAN`, `MODE`, exact `COUNT(DISTINCT)` | — | **never** (4.3); holistic values carry no mergeable state |

So the crisp statement: **`A ∘ B` is maintainable iff `B` exposes (directly or by decomposition) a monoid state and `A` is that state's own merge — i.e., the pair `(A, B)` forms a valid two-level fold of one monoid.** Same-name nesting (`SUM∘SUM`, `MIN∘MIN`) is the special case where the value *is* the state and the merge has the same SQL spelling. Everything else is either a renaming (`SUM` of `COUNT`s), a state-level nesting (decomposed `AVG`), or a refusal (holistic).

### 4.2 `COUNT ∘ anything` — the outer COUNT counts groups

```sql
-- users with ≥1 event, per the two-level shape:
SELECT count(*) FROM (SELECT user_id FROM events GROUP BY user_id) g;   -- = 2 on the fixture
```

The outer `COUNT(*)` is a group combiner over the multiset of *groups* — its algebra is intact. What composition changes is the **delta shape**: a base-row insert contributes `+1` to the outer count **iff it creates a new inner group** (a first-seen `user_id`); otherwise it contributes 0. Equivalently, `COUNT(*)` over groups **is** `COUNT(DISTINCT user_id)` over the base — composition has manufactured a holistic-over-base aggregate out of two group combiners. The two readings reconcile: maintained *from the inner model's output delta* (group created / group destroyed events), the outer fold is a plain group fold; maintained *from the base*, it needs the seen-set. This is the clearest instance of the theme: **composition preserves each level's discriminant but transforms the delta shape between levels** — an insert below becomes {nothing | insert | (with HAVING/retraction) delete} above. The shape taxonomy is the sibling doc's subject (`docs/research/20260707-property-delta-shape.md`); this doc only fixes the algebraic side: the outer fold is sound for whatever delta stream of *group-level* changes reaches it, and the inner level is responsible for emitting that stream faithfully.

### 4.3 `AVG ∘ AVG ≠ AVG` — and why decomposition commutes with nesting

Concrete rows (verified in DuckDB): group `a` has subgroups `p = {1, 2, 3}` and `q = {100}`.

```sql
CREATE TABLE t(g TEXT, sg TEXT, x DOUBLE);
INSERT INTO t VALUES ('a','p',1),('a','p',2),('a','p',3),('a','q',100);

SELECT avg(x) FROM t WHERE g='a';                                          -- 26.5   (106/4)
SELECT avg(sub) FROM (SELECT sg, avg(x) AS sub FROM t GROUP BY sg);        -- 51.0   ((2+100)/2)  ✗
SELECT sum(s)/sum(c)
FROM (SELECT sg, sum(x) AS s, count(x) AS c FROM t GROUP BY sg);           -- 26.5   ✓
```

The naive nesting weights each subgroup equally instead of by size — the classic weighted-mean error. The decomposed form is exact because **decomposition commutes with nesting**: the state monoid `(sum, count)` is a product of groups, products of monoids re-aggregate componentwise (§4.1 applied twice), and the presentation map `π = s/c` is applied once, at the top. In general: *if `B = π ∘ fold_M`, then `B` nests through any number of levels by folding `M` at every level and applying `π` only at the outermost* — presentation maps never commute with folds (`π(a ⊕ b) ≠ π(a) ⊕ π(b)`), so the design rule is "fold state all the way up, present once". The same argument covers variance (moment triples), `CORR` (co-moments), and HLL (register vectors). This is also the correct semantics for an `AVG` cell under `ROLLUP` (§3.6).

For **holistic** combiners no such rescue exists — there is no decomposition to commute. `MEDIAN ∘ MEDIAN`: subgroups `{1,2,3}` and `{100}` have medians `2` and `100`; the median of medians is `51.0` (verified) but the true median of `{1,2,3,100}` is `2.5`. And unlike AVG there is no bounded summary that fixes it: the median of a union depends on the full interleaving of the two multisets. **Holistic never re-aggregates**; the only compositional state is the multiset itself (rung 4), whose union is — again — a monoid, at `O(domain)` cost.

### 4.4 `MAX_BY ∘ ?` — order-monotone nests iff the ordering key travels

Verified in DuckDB:

```sql
-- base: (g, o, v) = ('a',1,10), ('a',2,20), ('b',3,5); global answer: max_by(v,o) = 5 (o=3)
SELECT max_by(v, o) FROM (
  SELECT g, max_by(v, o) AS v, max(o) AS o        -- carry the winning ordering key!
  FROM base GROUP BY g
) s;                                               -- 5  ✓
```

The pair state `(o*, v*)` under "keep the pair with larger `o`" is a commutative semilattice (idempotent monoid), so §4.1 applies — *provided the state is what travels between levels*. Projecting only `v` at the inner level discards `o*`, and no outer combiner can recover which inner winner is globally latest: with inner outputs `20` and `5` alone, both `MAX` (→20, wrong) and any other value-level fold are unsound. So the transfer rule: **`MAX_BY` re-aggregates as `MAX_BY(v, o)` over inner `(max_by(v,o), max(o))`; without the carried key it is a refusal.** The ties carve-out compounds across levels (ties at any level propagate up). `MAX_BY ∘ MAX` (outer picks by a *different* key than the inner used) is simply a different aggregate — no general law.

### 4.5 Aggregation over `UNION ALL` vs `UNION DISTINCT` — idempotence is exactly the discriminant

**`UNION ALL`** is multiset union `⊎`. `fold(⊕, A ⊎ B) = fold(⊕, A) ⊕ fold(⊕, B)` is the *definition* of folding a monoid over a disjoint union — so **every monoid (and every decomposable state) folds across UNION ALL branches unconditionally**. This is the disjoint-union case that makes `batched` trivially order-independent (`model_maintenance.md`: "its combiner is disjoint union") and makes per-branch incremental maintenance of a UNION ALL model compose (catalog G-09: bound derivation composes across arms). Holistic columns still need the whole union, but the delta *stream* of a UNION ALL is just the concatenation of branch deltas — no new obstacle.

**`UNION` (DISTINCT)** dedups **across** branches. Verified:

```sql
SELECT sum(x) FROM (SELECT 5 AS x UNION     SELECT 5);   -- 5
SELECT sum(x) FROM (SELECT 5 AS x UNION ALL SELECT 5);   -- 10
```

Per-branch folds give `5 ⊕ 5 = 10 ≠ 5`: **SUM/COUNT (and BIT_XOR) break** — non-idempotent combiners see the duplicate twice. **MIN/MAX/BOOL_\*/BIT_AND/BIT_OR survive**: per-branch `MIN`s combine to the true `MIN` whatever the overlap.

**Theorem (idempotence ⟺ dedup-harmless).** For a commutative monoid `(S, ⊕, e)` and per-row map `h`, `fold(⊕, h·set(M)) = fold(⊕, h·M)` for every multiset `M` **iff** `⊕` is idempotent on the image of `h`.
*Proof.* (⇐) Idempotent + commutative + associative means the fold of a multiset depends only on its support: group equal elements, collapse each run `x ⊕ … ⊕ x = x`. Dedup — whether by `UNION DISTINCT` re-deduplicating cross-branch or by a `DISTINCT` modifier (§3.3) — changes only multiplicities, never support. (⇒) If `x ⊕ x ≠ x` for some `x` in the image, the multisets `{r, r}` and `{r}` with `h(r) = x` distinguish the two sides. ∎

So idempotence is *precisely* the discriminant that makes both `UNION DISTINCT` and duplicate delivery harmless — and it is the same fact that catalog G-02 probes (re-delivered delta double-counts SUM without a dedup ledger; `keyed_models.md`'s additive-fold family gets the ledger, the extremal family doesn't need it) and G-03 confirms (MAX/BOOL_OR hold under all schedules). Note the theorem needs idempotence **on the image of `h`**, not on all of `S` — but for the catalogue's combiners the two coincide.

Caveat for maintenance over a UNION-DISTINCT body: even for idempotent columns, an inner *retraction* interacts with cross-branch dedup (a row deleted from branch 1 may still exist in branch 2) — survival is an insert-only statement.

### 4.6 Aggregation over fan-out joins — duplication is dedup's twin

A 1:N join replays each left row N times — multiplicity inflation, the mirror image of dedup's multiplicity erasure. Verified fixture: facts `{(1,10),(2,20)}`, dims with key 1 appearing twice:

```sql
SELECT sum(v), min(v), max(v), count(*) FROM f JOIN d USING (k);
-- sum = 40 (true base sum 30 — corrupted), min = 10, max = 20 (robust), count(*) = 3 (counts join rows)
```

By the theorem in §4.5, **idempotent combiners are fan-out-robust** (support unchanged: a duplicated row adds no new values) and **non-idempotent combiners are corrupted** in proportion to fan-out degree. `COUNT(*)` deserves its own line: over a fan-out join it *correctly counts join rows* — corruption is only relative to the intent "count facts", which is a modelling question the fan-out/cardinality proof (`analysis/join_shape.rs::fan_out`, over a declared-unique-key `JoinContext`) exists to decide; catalog G-10 records the current single-column-unique-key blind spot. The full treatment of fan-out as a property — degrees, uniqueness evidence, the `functional_dependencies:` widening — is the sibling fan-out/cardinality doc in this discovery series; the discriminant-side contribution is exactly one bit: **idempotent ⇒ duplication-safe; otherwise the join must be proven fan-out-free (or the duplication factor must be exactly compensated).**

### 4.7 Retraction through a stack

Per-level facts compose into a stack rule. Write a two-level maintained pipeline `outer(A) ∘ inner(B)` where each level stores its fold.

- **group ∘ group** (`SUM` over per-subgroup `SUM`s): a base retraction is an inverse-combine at the inner level, which emits a *signed* inner delta (`new − old`), which the outer absorbs by another inverse-combine. Fully incremental at both levels — this is Z-set/DBSP-style change propagation, and it is exactly why rung 3 is stated in terms of groups: **signed deltas are the lingua franca of composable retraction, and only groups speak it natively.**
- **monoid ∘ group** (outer `MAX` over per-subgroup `SUM`s): the inner absorbs the retraction fine (group), but its output *may decrease*, and the outer `MAX` cannot un-see the old higher value. The outer must **replay — but only over the inner aggregates**, i.e. over `G` subgroup rows, not `N` base rows. A maintained lower level bounds the replay cost of a non-invertible upper level to the intermediate grain's cardinality. This is a genuinely useful stack even though its top rung is non-invertible.
- **group ∘ monoid** (outer `SUM` over per-subgroup `MIN`s): the *inner* is the blocker — a base retraction may raise a subgroup's `MIN`, and computing the new `MIN` requires replaying that subgroup's base rows (or rung-4 multiset state at the inner level). Once the inner delta is known, the outer absorbs it invertibly. Replay cost: one subgroup's base rows — bounded by subgroup size, not by `N`.
- **anything ∘ holistic / holistic ∘ anything**: the holistic level replays its full input at its grain (or carries rung-4 multiset state, whose Z-set signing makes retraction free at `O(domain)` cost — the `model_maintenance.md` rung-4 trade stated compositionally).

General rule: **a retraction entering a stack propagates as a signed delta upward until it hits the lowest non-invertible level; that level replays over its own input grain (the level below's output cardinality), and above it, propagation resumes as signed deltas if the change is expressible as one.** Idempotence, as usual, gives partial relief: an insert-only "retraction" (a correction that only ever raises MAX) is absorbed without replay — but that is a monotonicity fact about the *update stream*, a world-fact declaration (mutation profile, `sources.md`), not an algebraic one.

### 4.8 Composition transfer table (operator × discriminant)

Rows: composition operator. Columns: what happens to each discriminant class of the inner column.

| Operator | group (SUM/COUNT/BIT_XOR) | idempotent monoid (MIN/MAX/BOOL/BIT_AND/OR) | decomposable (AVG/VAR/HLL) | order-monotone (MAX_BY) | holistic |
|---|---|---|---|---|---|
| re-aggregate, same grain hierarchy (`A∘B`) | ✓ via merge partner (`SUM` of `COUNT`s) | ✓ same name | ✓ **state-level only**; present at top (4.3) | ✓ iff ordering key carried (4.4) | ✗ never (4.3) |
| outer `COUNT` of groups | Δ-shape becomes group-creation events; = `COUNT(DISTINCT key)` over base (4.2) | same | same | same | — |
| `UNION ALL` (disjoint ⊎) | ✓ | ✓ | ✓ (state) | ✓ (state) | ✗ value; Δ-stream concatenates |
| `UNION DISTINCT` (cross-branch dedup) | ✗ (4.5 theorem) | ✓ (idempotent) | HLL state ✓ (registers idempotent); AVG ✗ | pair-state ✓ up to ties | ✗ |
| fan-out join (duplication) | ✗ corrupted (4.6) | ✓ robust | HLL state ✓; AVG ✗ | ✓ up to ties | ✗ (multiplicities wrong) |
| retraction into the stack (4.7) | absorbed as signed delta | replay at own grain (bounded by level below) | moment-form absorbs (group components); Welford-form replays | replay at own grain | replay, or rung-4 Z-set |
| `DISTINCT` modifier (§3.3) | → holistic | identity (no-op) | → holistic | n/a | holistic already |
| `HAVING` post-filter (§3.7) | + flip-detection obligation (both directions) | flips one-way if predicate one-sided w.r.t. value-monotone | + flip detection on `π(state)` | + flip detection | — |

---

## 5. Static provability vs declaration

Everything in §1–§4 is **statically provable from the SQL**: the classifier is a pure function of `(resolved SqlFunction, distinct flag)` — no data, no statistics, no engine probe. That is why it lives with the proofs in `model_properties.md` and is fail-closed: an unrecognised or UDF combiner returns `holistic_or_unknown()`, never an optimistic monoid (`discriminants.rs` doc comment; consistent with `model_properties.md` "Proofs are fail-closed").

Exactly **one** declaration touches this property, and it only *widens the holistic case*: `bounded_domain:` (with mandatory `max_cardinality`) licenses rung-4 multiset state for a holistic column (`analysis/bounded_domain.rs::bounded_domain_verdict`; refused when applied to a monoid/decomposable combiner, which needs no licence; `MalformedBoundedDomain` on structural errors). Note what it declares: a **world-fact about the data's domain size** — the one input the SQL genuinely cannot yield — not an algebraic fact.

**Nothing is declarable into a group.** Invertibility is a mathematical property of the combiner; no frontmatter key can conjure an inverse for `MAX`, and per `model_properties.md`'s escape-hatch rule a declaration may only widen what a proof admits, never assert an algebra the proof rejects. The only honest routes from non-invertible to retraction-capable are the two this doc derives: **decomposition** into group components where one exists (`PRODUCT` → `(nonzero-product, zero-count)`; variance → moment triple), and **richer state** (the rung-4 Z-set, which makes even `MIN`/`MAX` retraction free by paying `O(domain)`). Both change the state representation, never the claimed algebra of the original state — consistent with the equivalence invariant's "state size changes across rungs, never fidelity".

Two boundary notes. (1) Determinism of the per-row map (§3.1) is a *precondition*, policed separately (`KeyedForbidsNondeterministic`, `classify_function_determinism`) — the discriminant classifier assumes it. (2) A `MAX_BY` ties carve-out is neither proof nor declaration but a documented weakening of the oracle ("equivalence up to ties", `model_maintenance.md`) — a third, honest category worth keeping rare.

## 6. Implementation gaps (`discriminants.rs` vs this catalogue)

Specific, in rough priority order:

1. **`BIT_XOR` is misclassified as non-invertible.** `discriminants.rs` line 104 puts `BitXor` in the `BoolAnd | BoolOr | BitAnd | BitOr | BitXor` arm (`needs_inverse: true`). `BIT_XOR` is self-inverse — a group — and both `model_maintenance.md` (rung 3: "`SUM`, `COUNT`, `BIT_XOR`") and `keyed_models.md` (additive-fold family, Invertible = yes) say so. The code disagrees with two normative specs. Conservative direction (denies a valid retraction path, admits nothing unsound), but it is real spec/code drift `/smelt:validate` should catch.
2. **No idempotence discriminant.** §4.5–4.6 show idempotence — not non-invertibility — is the fact that decides `UNION DISTINCT`, fan-out-join, and redelivery robustness (and DISTINCT-modifier transparency, gap 3). Today it can only be *inferred* as `is_monoid ∧ needs_inverse ∧ ≠BitXor`, which is exactly the kind of coincidence that breaks when the catalogue grows (a non-idempotent, non-invertible order-externalised list monoid — §3.8 — would break it). Add `idempotent: bool` to `Discriminants`.
3. **`distinct = true` blanket-demotes to holistic**, including `MIN(DISTINCT x)` ≡ `MIN(x)` (§3.3). Sound, over-conservative; once gap 2 lands, the rule is `if distinct && !idempotent { holistic }`.
4. **`ARG_MAX` only.** `SqlFunction` has no `ArgMin`, and `from_name`/`resolve_alias` map neither `MAX_BY` nor `MIN_BY` nor `ARG_MIN` (`functions.rs` — alias table is JSON/NVL only). Yet `keyed_models.md`'s order-monotone-overwrite family is *specified over* `MAX_BY(value, ordering)` / `MIN_BY(...)`, and DuckDB's primary spelling is `max_by`/`min_by`. As written, the spec's flagship family cannot be classified from its own canonical spelling — fail-closed today, but a name-resolution gap ahead of the classifier, not in it.
5. **`STRING_AGG`/`ARRAY_AGG` fall to `holistic_or_unknown` with no order-sensitivity input.** The classifier's signature (`function, distinct`) cannot see an `ORDER BY` inside the call, so it cannot distinguish the non-function form (must stay refused, ideally with a *why* — "order-nondeterministic" beats generic holistic for diagnostics) from the ordered form (§3.8: decomposable, unbounded state, ties carve-out). Same signature limitation blocks `FIRST(x ORDER BY k)` ≅ `MIN_BY`.
6. **No `PRODUCT` variant in `SqlFunction` at all** — DuckDB's `product()` is unresolvable, so unclassifiable (fail-closed via `from_name → None` upstream). Low priority, but it is this doc's cleanest example of decomposition repairing invertibility (§1.2) and would be a good test of gap-2's machinery.
7. **`BOOL_AND`/`BOOL_OR` claim `Monotone::None`.** `BOOL_OR` is `MAX` over `{false < true}` (and ≅ `EXISTS`, which the spec's own discriminant row lists as value-monotone); `BOOL_AND` is `MIN`. Conservative, not unsound — but inconsistent with treating MIN/MAX as `Value`, and it denies downstream monotone reasoning a fact it is entitled to.
8. **No re-aggregation / merge-partner knowledge.** Nothing in `smelt-logical` encodes §4.1's table — that `COUNT` re-aggregates as `SUM`, that `AVG` re-aggregates only as the `(sum,count)` state, that `MAX_BY` needs the ordering key carried. Nested-aggregate stacks (a keyed model reading another keyed model, `ROLLUP` maintenance, the `COUNT`-of-groups pattern of §4.2) are simply outside the current classifier's vocabulary; `keyed_models.md` sidesteps this by rejecting composite/nested shapes (`KeyedUnknownCombiner`), which is sound but leaves the entire §4 composition surface unimplemented.
9. **`COUNT(*)` vs `COUNT(col)` are one `SqlFunction::Count`.** Discriminant-identical (§1.2), so the *classifier* is fine — but any future delta-computation consumer needs the null-skip distinction from the call site, and nothing currently threads it.

## 7. Open questions

1. **Presentation maps in-model**: should `keyed_models.md` admit pure scalar expressions over admitted aggregates (`SUM(a)/SUM(b)`) as derived presentation columns over a maintained state vector (§3.2), rather than `KeyedUnknownCombiner` + derive-downstream? The algebra says yes; the cost is classifier scope (expression purity proof) and explain-surface complexity.
2. **Where does idempotence live** once added (gap 2): a new `Discriminants` field consumed by the fan-out and union proofs, or derived at those proofs from a per-combiner table? A field keeps one home (`model_properties.md`'s row would gain a fifth fact); the spec row and `keyed_models.md`'s family table would need syncing.
3. **Re-aggregation vocabulary**: is the merge-partner map (§4.1) a new *proof* in `model_properties.md` (classify a two-level fold as valid/invalid) or a `model_transforms.md` rewrite (rewrite `AVG` at a coarse grain into state columns at the fine grain)? It is both a fact and a transform; the split matters for who owns `ROLLUP` maintenance.
4. **Ordered-aggregate state** (§3.8): does `STRING_AGG(x ORDER BY k)` join rung 4 under the existing `bounded_domain:` budget (state is `O(group size)`, not `O(domain)` — a different bound with the same fail-loud shape), or does it need its own budget key?
5. **`HAVING` flip-detection** (§3.7): is predicate-flip emission a delta-shape concern (sibling doc), a new transform, or grounds to refuse `HAVING` in maintained bodies the way `OVER` is refused in keyed bodies today? The value-monotone ∧ one-sided-predicate special case (flips are monotone) suggests a provable admissible subset.
