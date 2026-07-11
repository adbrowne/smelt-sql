# Per-key constancy / functional dependencies (`key → column`)

- **Date**: 2026-07-07
- **Status**: research
- **Related specs**: `docs/specs/model_properties.md` (§"Model-scoped declarations" row "Functional dependency"; §Known Divergences on `functional_dependency_verdict`), `docs/specs/keyed_models.md` (§"The column-family catalogue" — the once-write `COALESCE` family; §"Key temporal locality" route 2 "Key-determined"), `docs/specs/model_transforms.md`, `docs/specs/maintenance_plan.md`
- **Related code**: `crates/smelt-logical/src/analysis/functional_dependency.rs` (`functional_dependency_verdict`), `crates/smelt-logical/src/analysis/join_shape.rs` (`fan_out`, `JoinContext`), `crates/smelt-core/src/config.rs` (`FunctionalDependency { key, determines }`)
- **Related research**: `docs/research/property-discovery/catalog.md` (cells G-05 inner-join enrichment, G-09 UNION ALL, G-10 composite-key fan-out)

---

## 1. The property

A **functional dependency** (FD) `K → c` on a relation `R` states: for any two rows `r₁, r₂ ∈ R`, if `r₁[K] = r₂[K]` then `r₁[c] = r₂[c]`. Equivalently: `c` is a *per-key constant* — the value of `c` is a (partial) function of the value of the key columns `K`. In smelt's surface this is per-column: an FD is a fact about one `determines` column relative to a key set, declared as

```yaml
functional_dependencies:
  - key: [customer_id]
    determines: customer_region
```

(`smelt-core/src/config.rs`: `FunctionalDependency { key: Vec<String>, determines: String }`, `deny_unknown_fields`.)

Three distinct epistemic grades must not be conflated:

**(a) Schema-level FD — holds for all instances by construction.** A declared primary/unique key `u` on a table gives `u → c` for *every* column `c` of that table, in every instance the constraint admits. This is the strongest grade: it survives arbitrary future data because the constraint is enforced (or, in smelt's catalog-free world, *declared* — `sources.md` unique keys, `unique_key:` on keyed models). These are the **axioms** of the inference system in §5.

**(b) Instance-level FD — happens to hold in the current data.** `zip_code → city` may hold in today's rows of a staging table with no constraint enforcing it. This grade is **never provable statically** from the SQL — no analysis of the query text can establish a fact about data content. It can only be *declared* (the `functional_dependencies:` frontmatter is exactly this: a world-fact the modeller asserts), and a declared instance FD carries the standing risk that a future load violates it. This is why the spec classifies it as a model-scoped declaration, not a derived proof, and why the widening rule (§6) exists.

**(c) Query-derived FD — provable from the query structure.** Some operators *manufacture* FDs regardless of input data. The canonical case: after `GROUP BY k`, the output has at most one row per `k` value, so `k → e` holds for **every** output expression `e` — by construction, for all possible inputs. Likewise `DISTINCT` on a projection makes the full column set a key, `FIRST_VALUE(x) OVER (PARTITION BY k)` is per-`k` constant, and a literal column is FD on anything. These are the **derivable** grade: static analysis over the AST proves them with no declaration and no data inspection.

The three grades order by trust: (a) and (c) are sound against all future data (given the declared constraints stay true / the query stays the same); (b) is sound only as long as the world-fact holds.

### The temporal wrinkle

"Constant per key" is ambiguous over time, and the ambiguity is load-bearing for maintenance:

1. **Constant within a run** — all rows *processed in one refresh* agree on `c` per key. Weakest; a snapshot of a mutable dimension trivially satisfies it (one row per key per snapshot).
2. **Constant over all time (write-once)** — once a key's `c` value is first observed, no later input ever carries a different value for that key. This is what the once-write `COALESCE(target, delta)` family needs: it writes `c` on the key's first appearance and *never re-merges it*, so any later mutation is silently lost unless the FD is of this grade.
3. **Eventually mutable** — the FD holds at each instant but the determined value *changes slowly*: `customer_id → customer_region` where customers move. This is exactly a **broken FD over time** — and it is precisely the *slowly changing dimension* problem. An SCD-2 model (`versioned_models.md`) is the honest representation of grade-3: it repairs the temporal FD by augmenting the key, `(customer_id, valid_from) → customer_region`, which *does* hold over all time. Grade 3 masquerading as grade 2 is the classic stale-dimension bug.

The `functional_dependencies:` declaration as consumed by the once-write family asserts grade 2 (write-once over all time), which is strictly stronger than the instantaneous FD most modellers have in mind. §6 discusses connecting this to the source **mutation profile** (`sources.md`): a grade-2 assertion about a column fed from a `mutable` source is suspicious on its face; from an `append_only` source whose first row per key carries the value, it is coherent.

---

## 2. Why maintenance needs it

Per-key constancy is not an optimisation nicety; it changes which maintenance plans are *correct*:

- **Once-write (the motivating consumer).** `keyed_models.md`'s once-write column family (`COALESCE(target, delta)` — first-non-null over the group) is admitted only under the once-write provenance proof: the value must be a per-key constant, either key-derived or covered by a declared FD. Without the FD, `COALESCE(target, delta)` computes "first observed", which a full refresh cannot reproduce if values vary per key — an equivalence failure, refused as `KeyedOnceWriteUnproven`. With the FD, first-observed = only-value = what full refresh computes, and the column can be written once and never re-merged.
- **Dimension enrichment without re-derivation.** If `order_id → customer_region` holds on the enriched fact (via `customer_id` and a keyed dimension, §4 join rule), an incremental run that touches an old key does not need to re-run the dimension join for that key's stored rows: the stored value is still the value. Conversely, absent the FD (a mutable dimension, grade 3), every touched key's enrichment columns must be re-derived — the difference between a cheap MERGE update-set and re-joining the dimension.
- **MERGE update-set minimisation.** In `WHEN MATCHED THEN UPDATE SET …`, any column FD-on-the-merge-key with grade-2 constancy can be dropped from the update set entirely: the incumbent value is provably equal to the incoming one. Fewer columns updated = less write amplification, and on engines with column-level change tracking, fewer spurious downstream invalidations.
- **Skipping columns in change detection.** Snapshot-diff input discovery (`input-delta discovery`, `model_properties.md`) compares old vs new rows to find changes. A grade-2 FD column cannot have changed for an existing key; excluding it from the comparison narrows the diff predicate and avoids false-positive deltas from e.g. re-derived-but-equal expressions.
- **Key-determined partition pruning.** `keyed_models.md` §"Key temporal locality" route 2: when the partition projection is a per-key constant, a delta row's partition value *is* its key's stored partition value, so the write slice is exactly the delta's own partitions — pruning that is exact regardless of key age.

All of these consume the same verdict; none is sound under grade 1 or grade 3 constancy. That is why the verdict must be precise about which grade it certifies.

---

## 3. Per-construct analysis: FD transfer rules

For each operator: given FDs on the input(s), which FDs hold on the output — with a minimal DuckDB-correct example and a concrete-row counterexample where the rule has a boundary. Throughout, `K → c` is the input FD under discussion.

### 3.1 Projection (`SELECT`)

**Rule.** `K → c` survives iff every column of `K` and `c` itself survive projection (possibly renamed). Additionally, **congruence**: any deterministic expression over FD-on-`K` columns is itself FD on `K` — if `K → a` and `K → b`, then `K → f(a, b)` for deterministic `f`.

```sql
-- input orders(order_id PK, customer_id, amount); order_id → customer_id, order_id → amount
SELECT order_id, customer_id, amount * 1.1 AS amount_with_tax
FROM orders;
-- order_id → customer_id survives; order_id → amount_with_tax by congruence.
```

**Counterexample (key projected away).** Project `customer_id, amount` only: `order_id → amount` is not stateable on the output (no `order_id` column). No FD on `customer_id` is implied — two orders of the same customer:

| customer_id | amount |
|---|---|
| c1 | 10 |
| c1 | 25 |

`customer_id → amount` fails. Losing the key column loses the FD; it does not transfer to a smaller key.

**Congruence caveat — determinism.** `f` must be row-deterministic. `SELECT order_id, amount + random() AS jitter` does **not** give `order_id → jitter` even though `order_id → amount`: the nondeterminism predicate (`model_properties.md` §Determinism) is exactly the gate here. Run-deterministic functions (`NOW()`) are per-run constants and preserve within-run FDs but not cross-run grade-2 constancy.

### 3.2 `WHERE` (selection)

**Rule.** Selection preserves **all** FDs: a subset of rows cannot introduce two rows agreeing on `K` but differing on `c` if the superset had none. Selection can also *create* instance FDs that did not hold before — but these are grade (b), **not statically provable** in general.

```sql
-- events(user_id, event_type, region): user_id → region does NOT hold
-- (u1, 'click', 'EU'), (u1, 'buy', 'US')
SELECT user_id, region FROM events WHERE event_type = 'buy';
-- On the filtered instance, user_id → region happens to hold (one row per user).
```

That created FD holds only because of data content (each user has ≤1 'buy'); a second 'buy' row for `u1` with region 'AP' breaks it. **Exception — provable creation:** a filter that pins the key columns to constants (`WHERE k = 5`) makes *every* column trivially FD on `∅` (hence on anything) within the output; and `WHERE c = <literal>` makes `c` a constant column, so `∅ → c`. These constant-propagation cases *are* statically derivable; the general "the filter thinned the data enough" case is not, and the analysis must not attempt it.

### 3.3 Inner join

**Rule (preservation).** FDs of both sides survive an inner join, keyed on their original key columns: joining does not merge rows, it pairs them, and within any pair-set agreeing on left-`K`, the left columns are unchanged. (Caveat: `K` here must still be interpreted over the *output* — see the fan-out discussion, because the same left row can appear many times, which is fine for FD-holding but fatal for aggregation.)

**Rule (transitive import) — the load-bearing one.** A fact table with `fact_key → fk` joined to a dimension declared unique on `fk` imports the dimension's FDs onto the fact rows:

- axiom (fact): `order_id → customer_id`
- axiom (dim, schema-level): `customer_id → customer_region` (declared unique key on `customers.customer_id`)
- join on `o.customer_id = d.customer_id` ⟹ transitivity: `order_id → customer_region` on the join output.

```sql
SELECT o.order_id, o.customer_id, d.customer_region
FROM orders o
JOIN customers d ON o.customer_id = d.customer_id;
```

This **requires the join to be one-to-one per fact row** — exactly the fan-out proof (`join_shape::fan_out`). If the equality does not hit a declared unique key of the dimension side, the join multiplies rows and the imported "FD" is disproven, not merely unproven:

**Counterexample (fan-out).** Join on `d.category` (not unique):

`orders`: (o1, cat=books). `dims`: (id=1, category=books, tag='new'), (id=2, category=books, tag='sale').

Output:

| order_id | category | tag |
|---|---|---|
| o1 | books | new |
| o1 | books | sale |

`order_id → tag` fails on the output — same key, two values. This is why `functional_dependency_verdict` refuses a declared FD against a proven `OneToMany`: the query itself manufactures per-key variance, and no world-fact about the sources can repair a variance the join *creates*. The declaration widens undecidability; it never overrides a disproof.

Note the asymmetry: fan-out is *safe for FD preservation of the dimension side* (`d.id → d.tag` still holds — duplicated pairs agree) but *destroys FDs keyed on the probe side* whose determined column comes from the many-side.

### 3.4 `LEFT JOIN`

**Rule.** Left join = inner join ∪ null-extended unmatched left rows. Left-side FDs survive unchanged. Imported dimension FDs (transitive rule, requires the same one-to-one proof) survive **weakened by NULL-extension**: `order_id → customer_region` still holds — an unmatched `order_id` gets `customer_region = NULL`, and NULL is a value; there is exactly one output row (given one-to-one), so per-key constancy is trivially maintained. So yes, null-padding is FD-preserving in the static, single-instance sense.

**But the once-write consumer cares about a different thing: null-then-filled evolution across runs.** Grade-2 constancy means the *sequence of observed values over runs* is constant. A late-arriving dimension row turns run-1's `(o1, NULL)` into run-2's `(o1, 'EU')` — the FD holds *within each run's output* but the per-key value **changed between runs**. For a once-write column this is not a violation but the *intended* pattern (`COALESCE(target, delta)` fills a NULL exactly once); for MERGE update-set skipping (§2) it is a soundness bug — the column *can* change (NULL → value) and must stay in the update set until non-null. The precise licence a left-join-imported FD supports is therefore **"null-or-constant per key, monotone null→value"**, a strictly weaker verdict than grade-2 constant, and consumers must be told which they got. (This is a semilattice: NULL ⊑ v, writes only ascend — which is exactly why the once-write family's `COALESCE` fold is its correct maintainer, and why skipping the column in change-detection is *not* licensed until the value is non-null.)

### 3.5 `GROUP BY` — the FD factory

**Rule.** After `GROUP BY g₁, …, gₙ`, the output has at most one row per grouping-key value; hence `{g₁…gₙ} → e` for **every** output expression `e` — aggregates and grouping keys alike, by construction, for all inputs. This is grade (c): the strongest, declaration-free source of FDs.

```sql
SELECT customer_id, SUM(amount) AS total, MAX(order_ts) AS last_order
FROM orders GROUP BY customer_id;
-- customer_id → total, customer_id → last_order: provable, no declaration needed.
```

No counterexample exists — this is the one unconditional rule. Two sharp edges:

- The factory certifies constancy of the output *relation*, i.e. within-run. Grade-2 (cross-run write-once) does **not** follow: `SUM(amount)` per customer changes every run as orders arrive. The factory proves "one row per key", not "value never changes". Consumers wanting once-write need the *value's* temporal stability separately (e.g. `MIN(created_at)` over an append-only source is both factory-FD and grade-2 stable; `SUM` is factory-FD but grade-3).
- `GROUP BY` on an expression (`GROUP BY date_trunc('day', ts)`) makes that *expression* the key; the underlying column is not recoverable as a key.

### 3.6 `DISTINCT`

`SELECT DISTINCT a, b, c` makes `{a, b, c}` (the full projected set) a key of the output — hence trivially `{a,b,c} → anything projected`. It creates **no smaller-key FD**: `DISTINCT customer_id, region` does not give `customer_id → region` — concrete rows (c1, EU), (c1, US) both survive DISTINCT. `DISTINCT` is `GROUP BY <all columns>` and inherits exactly that factory rule, nothing more. Pre-existing FDs among the projected columns survive (subset-of-rows argument, as WHERE).

### 3.7 Window functions

**Rule.** `f(...) OVER (PARTITION BY k)` with a **whole-partition, order-insensitive-or-fully-framed** window creates `k → result` by construction — every row of a partition receives the same value. This makes windows a second FD factory:

```sql
SELECT order_id, customer_id,
       FIRST_VALUE(region) OVER (PARTITION BY customer_id ORDER BY ts
                                 ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS first_region,
       SUM(amount)         OVER (PARTITION BY customer_id) AS customer_total
FROM orders;
-- customer_id → first_region, customer_id → customer_total (per-partition constants).
```

**Counterexample (running frame).** `SUM(amount) OVER (PARTITION BY customer_id ORDER BY ts)` — the default frame with `ORDER BY` is `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`, a *running* sum: rows of the same partition carry different values. Rows (c1, ts=1, 10)→10, (c1, ts=2, 5)→15: `customer_id → running_total` fails. The transfer rule must therefore inspect the frame: per-partition-constant ⇔ the frame is the whole partition (no `ORDER BY`, or explicit `UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING`) — the same frame taxonomy `model_properties.md`'s frame-reach proof already classifies. `ROW_NUMBER`/`RANK`/`LAG` are never per-partition constants. Note DuckDB's `FIRST_VALUE` default frame with `ORDER BY` ends at `CURRENT ROW`, which for `FIRST_VALUE` specifically is still partition-constant (the first value doesn't depend on the frame end) — a per-function refinement the general rule may conservatively skip.

Pre-existing FDs pass through unchanged (windows add columns, never merge or drop rows).

### 3.8 `UNION ALL` — the star case

**Rule.** An FD `K → c` holding in **each branch does not hold in the union.** This is the central destructive operator and the motivating example. Concrete rows:

```sql
-- branch A: current CRM
SELECT customer_id, region FROM crm_current      -- (c1, 'EU')
UNION ALL
-- branch B: legacy import
SELECT customer_id, region FROM crm_legacy;      -- (c1, 'US')
```

Each branch is keyed on `customer_id` (say each has a declared unique key), so `customer_id → region` holds in A and in B individually. The union:

| customer_id | region |
|---|---|
| c1 | EU |
| c1 | US |

FD destroyed — same key, different values across branches. Any once-write column licensed off a branch-level FD and consumed on the union would freeze whichever branch's row is folded first: nondeterministic, and not what full refresh (which sees both) computes.

**When the FD does survive the union — three provable cases:**

1. **Disjoint key spaces, provable from a constant discriminator embedded in the key.** If each branch tags its rows and the tag is part of the key, keys cannot collide:

   ```sql
   SELECT 'crm'    AS src, customer_id, region FROM crm_current
   UNION ALL
   SELECT 'legacy' AS src, customer_id, region FROM crm_legacy;
   -- (src, customer_id) → region holds: the src literal differs per branch,
   -- so no two rows from different branches share the composite key.
   ```

   Statically provable: each branch projects a distinct literal into a key column (constant-folding per branch + literal inequality). This is the standard "union of partitions" idiom.

2. **Disjoint sources by world-fact.** The branches read provably-disjoint key ranges (e.g. `WHERE customer_id < 1000` / `WHERE customer_id >= 1000` — statically checkable predicate disjointness on the key) or the modeller declares source disjointness. Predicate-based disjointness on key columns is derivable for simple range/equality predicates; anything else is a declaration.

3. **Identical derivation from a shared upstream.** Both branches compute `c` by the *same deterministic expression over the same upstream relation's key row* — e.g. both branches join the same keyed dimension and project `d.region`. Then colliding keys agree on `c` by congruence through the shared upstream, even though the key spaces overlap. Provable in principle (expression + provenance equality), but the provenance-equality check is substantially harder than cases 1–2 and is realistically a later rung.

Absent one of these, the transfer rule for `UNION ALL` must **fail closed**: branch FDs are dropped at the union node.

### 3.9 `UNION` (DISTINCT)

Same failure. `UNION` dedups *entire rows*; (c1, 'EU') and (c1, 'US') are distinct rows and both survive. The dedup gives the full column set as a key (as `DISTINCT`, §3.6) but repairs no per-key collision on a proper key subset. The same three survival cases apply.

### 3.10 `INTERSECT` / `EXCEPT`

**Rule.** Both preserve all FDs of the **left** input (and `INTERSECT` those of the right too): the output is a subset of the left input's rows (set-semantics subset; `EXCEPT ALL`/`INTERSECT ALL` outputs are sub-multisets), and the WHERE argument applies — a subset cannot manufacture a collision. Additionally the output of set-semantic `INTERSECT`/`EXCEPT` is duplicate-free, so the full column set is a key. Note `model_properties.md` records `INTERSECT`/`EXCEPT` as unclassified in the set-operation-distribution proof — that gap is about *filter pushdown*, but an FD transfer analysis would need the same AST coverage.

### 3.11 `VALUES` / constant columns

A literal column is FD on anything, including the empty key: `SELECT customer_id, 'EU' AS region FROM t` gives `∅ → region`, hence by augmentation `K → region` for any `K`. A `VALUES` list is a relation whose FDs are instance-facts of the listed rows — decidable by direct inspection since the rows are in the query text (e.g. `VALUES (1,'a'),(2,'b')` provably has `col0 → col1`; `VALUES (1,'a'),(1,'b')` provably has not).

### 3.12 Correlated / scalar subqueries

A scalar subquery in the select list, correlated on the outer key, is FD-on-that-key **by construction iff it returns ≤ 1 row per correlation value** — which is exactly what "scalar" enforces at runtime (DuckDB errors on >1 row) and what a `GROUP BY`d or aggregate-only body proves statically:

```sql
SELECT o.customer_id,
       (SELECT MAX(v.score) FROM visits v WHERE v.customer_id = o.customer_id) AS best_score
FROM orders o;
-- customer_id → best_score: the subquery is an aggregate keyed on the correlation
-- column — a GROUP-BY-factory FD in disguise (it IS a grouped aggregate per key).
```

Two sub-cases: (i) aggregate body with the correlation predicate on the outer key — always ≤1 row, FD provable; (ii) bare `SELECT c FROM d WHERE d.k = o.k` — ≤1 row iff `d.k` is a declared unique key of `d`, i.e. the same fan-out question as §3.3 in subquery clothing (and it should share the proof). `EXISTS` subqueries yield a boolean that is a deterministic function of the outer key and the inner relation — FD on the outer key within a run, but grade-3 across runs unless the inner source is append-only and the predicate monotone (the re-scanned-existence-flag condition in `keyed_models.md`).

---

## 4. Summary transfer table

| Operator | FD transfer rule | Creates FDs? |
|---|---|---|
| Projection | survives iff `K ∪ {c}` projected; congruence: deterministic `f(FD cols)` is FD | via congruence |
| WHERE | preserves all; creation only via constant-pinning (provable) — data-thinning creation not provable | `∅ → c` for pinned/eq-literal cols |
| Inner join (1:1 proven) | both sides' FDs survive; transitive import `K→fk, fk→c ⟹ K→c` | transitive imports |
| Inner join (fan-out / unproven) | probe-side FDs into many-side columns **disproven**; dim-side own FDs survive | no |
| LEFT JOIN (1:1 proven) | imports as *null-or-constant, monotone null→value* — weaker than grade-2 constant | weakened imports |
| GROUP BY | **factory**: group key → every output; within-run grade only | yes, unconditionally |
| DISTINCT | full projected set becomes a key; no smaller-key FD; input FDs survive | full-set key only |
| Window, whole-partition frame | `PARTITION BY k` → result is FD on `k`; running frames do **not** qualify | yes, frame-gated |
| UNION ALL | branch FDs **dropped** unless: literal-discriminator-in-key, provably disjoint key predicates, or identical shared-upstream derivation | no |
| UNION (distinct) | same as UNION ALL + full-set key | full-set key only |
| INTERSECT / EXCEPT | left input's FDs survive (subset); output duplicate-free (set semantics) | full-set key |
| VALUES / literals | literal column: `∅ → c`; VALUES rows: decide by inspection | yes |
| Scalar correlated subquery | FD on correlation key iff ≤1 row provable (aggregate body, or unique-key equality — the fan-out proof again) | yes, conditionally |

---

## 5. Composition algebra

FDs form a closure system. The complete inference has two layers:

**Layer 1 — per-operator transfer rules** (§4): each relational operator maps the FD set of its input(s) to a (sound, possibly incomplete) FD set of its output. These are the *structural* rules; each one either fires (with a proof obligation, e.g. fan-out `OneToOne`) or does not, in which case the FDs it would have produced are simply absent from the output set.

**Layer 2 — Armstrong closure at every node.** Within any single relation's FD set, Armstrong's axioms are sound and complete:

- **Reflexivity**: `Y ⊆ X ⟹ X → Y`.
- **Augmentation**: `X → Y ⟹ XZ → YZ` (in the per-column framing: `K → c ⟹ K∪Z → c`; any superkey of a key also determines `c`).
- **Transitivity**: `X → Y ∧ Y → Z ⟹ X → Z`.

Derived rules worth naming for the implementation: **union** (`K → a ∧ K → b ⟹ K → {a,b}` — needed to assemble composite determined sets), **decomposition**, and the **congruence** rule of §3.1 (an extension beyond classical Armstrong, since it introduces expression columns: `K → cols(e), e deterministic ⟹ K → e`).

The full analysis is then: seed the leaves with **axioms** (declared unique keys ⟹ schema FDs; declared `functional_dependencies:` ⟹ instance FDs, tagged as declared; literals ⟹ `∅ → c`), propagate bottom-up through the plan applying the transfer rule at each operator, and close under Armstrong + congruence at each node before propagating further. Each derived FD should carry its **provenance grade** (schema / declared / query-derived) and **temporal grade** (within-run / null-monotone / write-once), because consumers gate on different grades (§2, §3.4).

### Worked pipeline (all four phenomena)

```sql
-- axioms: orders.order_id unique (schema); customers.customer_id unique (schema);
--         crm_legacy.customer_id unique (schema)
WITH enriched AS (                                   -- [1] transitive import
  SELECT o.order_id, o.customer_id, o.amount, d.region
  FROM orders o JOIN customers d ON o.customer_id = d.customer_id
),                                                    -- fan_out=OneToOne (d.customer_id unique)
per_customer AS (                                     -- [2] GROUP BY factory
  SELECT customer_id, region, SUM(amount) AS total
  FROM enriched
  GROUP BY customer_id, region
),
unified AS (                                          -- [3] UNION ALL destruction…
  SELECT 'live'   AS src, customer_id, region, total FROM per_customer
  UNION ALL
  SELECT 'legacy' AS src, customer_id, region, total FROM crm_legacy_totals
)
SELECT src, customer_id, region, total FROM unified;  -- [4] …repaired by discriminator
```

- **[1]** `orders`: `order_id → customer_id, amount` (reflexive from the unique key). Join proof: `d.customer_id` declared unique + `ON` equality ⟹ `fan_out = OneToOne` ⟹ transitive import `order_id → region`. Node FD set: `order_id → {customer_id, amount, region}`.
- **[2]** Factory: `{customer_id, region} → total` — and since `customer_id → region` also holds here (transitivity through [1]'s import: within `enriched`, `customer_id → region` via the dimension axiom), Armstrong gives the reduced key `customer_id → {region, total}`. Note `total` is within-run grade only (SUM).
- **[3]** Branch FDs: `customer_id → {region, total}` in each branch. At the union node the bare transfer rule drops them — same `customer_id` can appear in both branches with different totals.
- **[4]** But each branch projects a distinct literal into `src` (survival case 1, §3.8): `(src, customer_id)` keys the union, so `{src, customer_id} → {region, total}` survives. If a consumer needs `customer_id → region` on the final output, the analysis must **fail closed and report the union node** as the point where the narrower FD died — the actionable diagnostic being either "add `src` to your key" or "prove/declare branch key-disjointness".

### Fail-closed discipline

The analysis must return "FD does not hold" whenever **any operator on the derivation path has no firing transfer rule** — an unproven join cardinality, a running window frame, a bare `UNION ALL`, an opaque function in a congruence position (nondeterminism unknown), an unclassified set operation. Absence of proof is rejection (`model_properties.md` §Constraints), and — matching the existing verdict design — the result should distinguish `NotProven` (no rule fired; a declaration may widen) from `Refused` (a rule fired *negatively*: proven fan-out, proven branch collision by construction; no declaration may override). The UNION ALL case is instructive: a bare union is `NotProven` (the branches *might* be key-disjoint as a world-fact — declarable), whereas two branches that provably emit the same key with different literal values for `c` would be `Refused`.

---

## 6. Static provability vs declaration

The three-grade structure of §1 maps directly onto smelt's derive-vs-declare law (`models.md`, `model_properties.md` §Design "Derive where decidable, declare where not"):

- **Query-derived FDs are proofs.** The GROUP BY factory, whole-partition windows, congruence, transitive join import over declared keys, literal columns, discriminator-tagged unions — all decidable from the AST plus the declared-key context. These should never require a declaration, and today several of them silently fall into the "declare it" bucket only because the derivation is unbuilt (§7).
- **Source-instance FDs need declaration; declared unique keys are the axioms.** The catalog-free layering (`join_shape.rs` doc comment: callers inject `JoinContext` unique keys the way `BoundContext` injects partition columns) means every schema-level axiom arrives as a declaration on the source or model (`unique_key:`, source YAML). The `functional_dependencies:` frontmatter is the instance-level analogue for non-key FDs.
- **The widening rule.** `functional_dependency_verdict(determines_fan_out, declared)` implements the spec's only-widen constraint exactly: `OneToMany` ⟹ `Refused` regardless of declaration (a positive structural disproof can never be overridden by a world-fact — the query manufactures the variance); `OneToOne` ⟹ `Constant` without any declaration (the proof suffices); `None` (no traceable join origin — the undecidable case) ⟹ `Constant` iff declared, else `NotProven`. Declarations widen undecidability only; they never narrow (never skip a dedup the proof requires) and never overturn a disproof.
- **Temporal mutation of a declared FD is a world-fact — the mutation profile is the honest cross-check.** A declared `customer_id → customer_region` consumed at grade 2 (once-write) is an assertion that the region *never changes for a customer* — over all future loads. smelt cannot verify this, but it can check *coherence*: the column's source carries a `mutation_profile:` (`sources.md`). Declared FD at grade 2 over a `mutable` source is the modeller asserting a stronger fact than the source's own profile suggests — worth at least a warning, plausibly a refusal with an explicit override. Over `append_only`, grade 2 reduces to "the first row per key carries the final value", which is the natural reading. And when the world-fact is genuinely grade 3 (slowly changing), the correct modelling answer is not a declaration but a key repair: versioned/SCD-2 (`versioned_models.md`), where `(key, valid_from) → attr` is a true grade-2 FD.

---

## 7. Implementation gaps (as of 2026-07-07)

What exists (all in `crates/smelt-logical/src/analysis/`):

- **`functional_dependency.rs`** — `functional_dependency_verdict(Option<Cardinality>, bool) → {Constant, NotProven, Refused}`. It is the *composition* of exactly one transfer rule (join fan-out into `determines`) with the declaration, and nothing else. Per its own doc comment, no consumer (the once-write enrichment transform) is wired yet; `model_properties.md` §Known Divergences confirms.
- **`join_shape.rs`** — `fan_out(join, ctx)`: `OneToOne` iff a top-level ANDed `=` (or `USING`) column matches a declared unique key of the joined side; fail-closed to `OneToMany` for `CROSS JOIN`, missing condition, unqualified columns, or no key match.
- **`config.rs`** — the `FunctionalDependency { key, determines }` frontmatter shape, single-column `determines`, `deny_unknown_fields`.

Specific gaps against §§3–5:

1. **No Armstrong closure.** There is no FD-set data structure at all — the verdict is per-(column, single-join) with no reflexivity/augmentation/transitivity. Consequence: the transitive import of §3.3 (`order_id → customer_id → region`) is not representable; the current code answers only "is the join sourcing `determines` one-to-one", keyed implicitly, and cannot chain two hops or reduce a composite key.
2. **No GROUP-BY factory.** The single strongest, cheapest derivation — group key → every output — is not implemented anywhere; a `GROUP BY`-produced column with an FD consumer would today need a *declaration* for a fact that is provable from the AST. (The `unique_key`-must-restate-`GROUP BY` check in `keyed_models.md`'s classifier is adjacent but produces no FD facts.)
3. **No union analysis.** `UNION ALL` — the star destructive case — is invisible: `functional_dependency_verdict` receives one `Option<Cardinality>` and cannot express "the determines column crosses a set-operation node". A declared FD on a model whose body unions two branches would today widen `None` to `Constant` with **no check that the union preserves it** — the declaration is doing exactly the work §3.8 shows is unsound to assume, and there is no discriminator-in-key or disjointness derivation to catch the safe cases. Relatedly, set-operation distribution in `model_properties.md` covers UNION ALL for *filter pushdown* only; `INTERSECT`/`EXCEPT` are unclassified for everything.
4. **No projection/congruence/window/subquery rules.** `determines_fan_out = None` conflates "plain pass-through column" (arguably fine to widen) with "column computed through arbitrary constructs the analysis never looked at" (windows with running frames, expressions over multi-source columns, scalar subqueries). The `None` arm is the whole non-join SQL surface, undifferentiated.
5. **Composite keys.** Two independent restrictions: (a) `FunctionalDependency.key` is a `Vec<String>` in frontmatter, but nothing downstream consumes the key columns at all — `functional_dependency_verdict` takes only `declared: bool`, so the declared key set is never checked against the join's equality columns or the model's `unique_key`; (b) `JoinContext.unique_keys` holds columns "each of which **alone** uniquely identifies a row" — a composite unique key `(a, b)` is not expressible, so a join on `ON e.a = d.a AND e.b = d.b` against a composite-keyed dimension fails closed to `OneToMany` (over-conservative; catalogued as property-discovery cell **G-10**: "join fan-out on composite unique key → mislabeled fan-out").
6. **No temporal-grade distinction.** The verdict is a single `Constant` with no within-run / null-monotone / write-once tag; the LEFT-JOIN weakening of §3.4 and the SUM-vs-MIN factory caveat of §3.5 are not expressible, so every consumer must assume the strongest grade the once-write family needs — correct for that one consumer, over-strong for a within-run consumer and a latent trap when the MERGE-update-set consumer arrives.
7. **No mutation-profile coherence check** (§6): a declared FD over a `mutable`-profiled source is admitted with no warning.

Gaps 1–3 are the substance of "composition algebra unbuilt"; gap 5(a) is arguably a latent bug rather than a missing feature (a declared key of `[order_id]` and a declared key of `[customer_id]` for the same `determines` are indistinguishable to the verdict).

---

## 8. Open questions

1. **Verdict granularity: should `Constant` split into temporal grades?** Once-write needs write-once; MERGE update-set skipping needs write-once-or-non-null-yet; partition pruning (key-determined route) needs per-key-constant-once-written. One enum with `{WithinRun, NullMonotone, WriteOnce}` grades, or separate proofs per consumer?
2. **Where does the FD set live in the plan?** Armstrong closure per plan node implies an FD-set annotation propagated bottom-up (like nullability/type inference). Is this a `smelt-logical` analysis pass over the parsed body, or a per-node annotation in the planner's `LogicalNode` — and does the Salsa layer cache per-model FD sets for cross-model transitive import (a downstream model reading a keyed upstream inherits its axioms)?
3. **UNION disjointness: derive, declare, or both?** The literal-discriminator case is cheaply derivable; predicate-disjointness on key ranges is a small constraint solver; general source disjointness is a world-fact. Is a `disjoint_keys:` declaration on the union (or across sources) worth its surface area, or should the answer stay "add the discriminator to your key" (fail closed with that diagnostic)?
4. **Should a declared FD be *checked* at run time?** A cheap post-write assertion (`SELECT key FROM t GROUP BY key HAVING COUNT(DISTINCT c) > 1 LIMIT 1`) would fail-loud a violated grade-2 declaration on the run that violates it, instead of silently freezing a wrong once-write value forever. Cost/opt-in shape undecided; interacts with the mutation-profile coherence warning (§6).
5. **Composite-key semantics of the declaration.** When `key: [a, b]`, what must the verdict check the key *against* — the model's `unique_key`, the join equality set, the GROUP BY set? Today nothing reads it (gap 5a); defining the matching rule is prerequisite to fixing both the declaration and G-10's composite `JoinContext`.
