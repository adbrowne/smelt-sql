# Property research: key grain, cardinality & fan-out

- **Date**: 2026-07-07
- **Status**: research
- **Related specs**: `docs/specs/model_properties.md` (rows "Fan-out / cardinality", "Join-contribution monotonicity", "Driving-fact / anchor resolution", "Functional dependency"), `docs/specs/keyed_models.md` (key-addressed shape, `unique_key`, once-write, join-contribution consumer), `docs/specs/model_maintenance.md` (key-addressing, dimension-driven horizon MERGE)
- **Related code**: `crates/smelt-logical/src/analysis/join_shape.rs` (`Cardinality`, `JoinContext`, `fan_out`, `join_contribution_monotone`), `crates/smelt-logical/src/analysis/discriminants.rs`
- **Related research**: `docs/research/property-discovery/catalog.md` (G-05 inner-join enrichment, G-06 left-join null preservation, G-10 composite-key fan-out); `docs/research/20260707-property-aggregate-algebra.md` (the combiner-algebra side of the composition)

---

## 1. The property

**Grain** of a relation R is a set of *candidate keys*: column sets K such that no two rows of R agree on all columns of K. `{order_id}` is a grain; `{customer_id, order_date}` is a (composite) grain; a relation may hold several simultaneously (`{order_id}` and `{order_number}`); a relation with duplicate rows has *no* grain, not even the all-columns set.

Three distinct questions live under this one property and must not be conflated:

**(a) Key preservation of a relation through an operator.** Given input grain(s), what is the output grain? Operators fall into four classes:

- **preserve** — output keys ⊇ some function of input keys (WHERE, semi/anti join, many-to-one join on the probe side, window functions, INTERSECT/EXCEPT, LIMIT);
- **coarsen/compose** — output key exists but is a *different* (usually wider) set (one-to-many join → composite of both grains; UNION ALL of disjoint branches → key + discriminator);
- **destroy** — no derivable output key (UNION ALL in general, unnest, cross join, projection dropping key columns);
- **establish** — the operator *creates* a key regardless of input grain (GROUP BY, DISTINCT, `ROW_NUMBER()=1` dedup). These are the axiom-generators of the whole calculus: they are the only places uniqueness comes from other than a declaration.

**(b) The pairwise join cardinality verdict.** `Cardinality = OneToOne | OneToMany` (join_shape.rs) is *not* the output grain — it is a statement about one join clause: "does joining side S onto probe P match at most one S-row per P-row?" `OneToOne` means the probe's grain survives the join (enrichment in place); `OneToMany` means rows multiply and the probe grain is lost (or becomes composite). The verdict is per-join and directional: it says nothing about how many P-rows match each S-row, which is why LEFT vs INNER matters separately (§4).

**(c) Fan-out INTO an aggregate.** Even when the *net* relation grain is fine (e.g., a fan-out followed by GROUP BY back to the original key), the row multiplication upstream is a per-column lineage fact that survives. `SUM`/`COUNT` over a fanned-out column count each contribution once per duplicate — the result changes; `MIN`/`MAX` are duplicate-insensitive (idempotent: `max(x, x) = x`) — the result does not. Which aggregates are corrupted by duplication is exactly the idempotence/needs-inverse discriminant classified in `docs/research/20260707-property-aggregate-algebra.md`; this doc treats those discriminants as an opaque input (via `Discriminants`) and contributes the multiplicity fact they compose with. `join_contribution_monotone` is precisely that composition: `OneToMany` × decrementing aggregate → refused with the fan-out-specific reason; `OneToMany` × anything → refused; `OneToOne` × monotone combiner → admitted.

The scope here is **per-model, relation-level**: the grain of the model's output relation, derived from declared unique keys on its inputs (`unique_key` on models, key declarations on sources) plus what the model's own SQL establishes.

## 2. Why maintenance needs it

Four concrete consumers, all already named in the specs:

1. **`merge_into` needs a proven key.** The keyed write primitive (`model_transforms.md`, key-addressed modes in `model_maintenance.md` §"Key-addressed") merges a delta into stored state *by key*. If the delta relation is not actually unique on `unique_key`, DuckDB's `MERGE INTO` (and Delta's) errors at runtime on multiple source matches — or worse, a backend silently picks one. `keyed_models.md` gets this today by *construction* (the body must be a `GROUP BY` on `unique_key`, so the grain is established, not checked); a general grain proof would let key-addressed writes be admitted for bodies that are not literally one GROUP BY — e.g., a GROUP BY followed by an enrichment join, provided the join is proven `OneToOne`.

2. **Enrichment transforms need `OneToOne`.** Column re-derivation for a mutable dimension (catalog G-05; the generic column-scoped merge in `model_transforms.md`) re-derives the enriched columns for affected keys. That is only a column-scoped operation if the dimension join did not multiply fact rows — a `OneToMany` enrichment changes the *row set*, not just column values, and cannot be maintained as an UPDATE-shaped merge. The once-write / functional-dependency proof (`model_properties.md` row "Functional dependency") composes with fan-out the same way: a declared FD is refused when fan-out positively proves the column multi-valued per key.

3. **Dimension-driven horizon MERGE needs contribution monotonicity.** F15 (`model_transforms.md` row "Dimension-driven horizon-bounded MERGE") merges a dimension batch straight into the target slice without re-reading the fact. That is only equivalent to recompute when the dimension's per-key contribution folds monotonically — `join_contribution_monotone`'s verdict, which has fan-out as one of its two inputs. A row-multiplying join into `SUM` means the dimension update must retract *N* copies of the old contribution, and N is not recoverable from the target.

4. **Keyed collapse needs the output grain.** The key-temporal-locality routes and the snapshot-reconcile run shape (`keyed_models.md`) reason about "one stored row per key"; the equivalence oracle ("the SQL is the oracle") is only meaningful because the output grain is `unique_key`. Any future relaxation of the GROUP-BY-body requirement moves the burden from syntactic construction to grain inference.

## 3. Per-construct analysis

Throughout: `orders(order_id, customer_id, amount)` with declared grain `{order_id}`; `customers(customer_id, region)` with declared grain `{customer_id}`. Concrete rows:

```sql
CREATE TABLE orders AS FROM (VALUES
  (1, 10, 100.0), (2, 10, 50.0), (3, 20, 75.0)
) t(order_id, customer_id, amount);
CREATE TABLE customers AS FROM (VALUES
  (10, 'AU'), (20, 'NZ')
) t(customer_id, region);
```

### 3.1 Projection

Key survives iff every key column is projected (possibly renamed). `SELECT order_id, amount FROM orders` — grain `{order_id}` preserved. `SELECT customer_id, amount FROM orders` — grain destroyed: rows `(10, 100.0)` and `(10, 50.0)` collide on any candidate subset once `order_id` is gone (they differ on `amount`, but `{customer_id, amount}` is only unique by accident of this data — not provable).

Injective *expressions* of key columns preserve the key in principle (`order_id + 1000000`, `CAST(order_id AS VARCHAR)`), but injectivity is undecidable in general (`order_id % 10` is not injective; `UPPER(code)` is not injective on mixed-case data; even `a || '-' || b` over two columns is non-injective without a separator guarantee: `('ab','c')` vs `('a','bc')` both yield `'ab-c'` under `||`). **Rule: key survives only through bare column references (renames allowed); any non-trivial expression over a key column drops that column from the surviving key.** A small allowlist of provably-injective forms (cast to a wider type, `+ constant` on integers) is a possible later widening, not a v1 rule.

### 3.2 WHERE

Filtering preserves every input key (a subset of a duplicate-free set is duplicate-free). It never *coarsens*.

Can it *establish* a key? Semantically yes: `WHERE order_id = 3` yields at most one row per anything, and `WHERE is_current` on an SCD2 table yields one row per business key. But statically this requires knowing the predicate selects ≤1 row per candidate key — undecidable in general, and even the SCD2 case rests on an *invariant of the data* (at most one open version per key), not on the predicate's text. **Rule: WHERE preserves, never establishes.** The two special forms worth naming as future declarations rather than inference: (i) equality on a full key (`WHERE k = <const>` → whole relation has ≤1 row — cheap and sound to recognise); (ii) a declared "current-flag" invariant on a versioned input, which is exactly what `versioned_models.md`'s shape profile should export as a derived fact about its own presentation view rather than something consumers re-prove.

### 3.3 INNER JOIN

Direction matters. With F the probe (left) and D the joined side:

- **Many-to-one** (D unique on the join columns): probe grain preserved. `orders o JOIN customers c ON o.customer_id = c.customer_id` — each order matches ≤1 customer (grain `{customer_id}` declared on customers, join equality covers it) → output grain `{order_id}`. This is `fan_out = OneToOne` and the enrichment pattern. Caveat vs LEFT JOIN: INNER also *drops* probe rows with no match — grain-preserving (still unique) but not row-set-preserving, which matters for the aggregate side (a dropped fact row changes SUM too).
- **One-to-many** (D not unique on join columns): probe grain lost. Join orders to a `payments(payment_id, order_id, paid)` table on `order_id` where an order has two payments: order 1's row appears twice. Output grain is the *composite* `{order_id, payment_id}` — derivable as (probe key ∪ joined-side key) whenever both sides have keys, because a pair of rows agreeing on both keys is the same probe row joined to the same D row. If D has no key at all, output has no grain. `fan_out = OneToMany` is the verdict either way; the composite-grain refinement is what a grain *calculus* adds over the binary verdict.

### 3.4 LEFT JOIN

Preserves the left grain **iff** the right side is unique on the join columns — same condition as many-to-one INNER, plus it keeps unmatched left rows (NULL-extended). If the right side is *not* unique, duplicates appear exactly as in INNER:

```sql
-- customer_tags(customer_id, tag): customer 10 has tags 'vip' and 'churn-risk'
SELECT o.order_id, t.tag
FROM orders o LEFT JOIN customer_tags t USING (customer_id);
-- order_id=1 appears twice → grain {order_id} destroyed
```

The NULL-extension is grain-neutral (one NULL-extended row per unmatched left row) but is the lineage fact behind catalog G-06: a *late-arriving* right side means the stored NULL-extended row is stale — a recompute-region concern, not a grain concern. For grain purposes: **LEFT JOIN ≡ INNER JOIN's preservation rule, with the row-set additionally a superset-per-left-row of the INNER result.**

### 3.5 FULL and CROSS

- **FULL OUTER**: output grain is preservable only as the composite of both keys with NULL-extension on each side; the left grain alone is *not* a key (an unmatched right row has NULL in every left-key column, and two unmatched right rows collide on the left key `NULL…NULL` under the "no two rows agree" reading only if NULLs are treated as agreeing — SQL UNIQUE treats NULLs as distinct, but a MERGE key does not). Fail-closed: no useful single-side grain.
- **CROSS**: grain is the composite of both keys if both exist; there is no join predicate to prove `OneToOne` against, so `fan_out` correctly fails closed to `OneToMany` (a one-row dimension would make it 1:1 in fact, but cardinality-of-data is not a static property).

### 3.6 Semi and anti joins (EXISTS / NOT EXISTS / IN)

Always grain-preserving — by *shape*, not by proof about the other side:

```sql
SELECT o.* FROM orders o
WHERE EXISTS (SELECT 1 FROM customers c WHERE c.customer_id = o.customer_id AND c.region = 'AU');
```

A semi-join emits each probe row at most once no matter how many inner rows match; it is a filter. This is why **semi-join enrichment is the safe pattern**: when a maintenance transform needs "which fact keys are affected by this dimension delta", probing with EXISTS (or `IN (SELECT …)`) can never fan out, whereas an inner join to the delta can. `NOT EXISTS` identically. The grain rule is unconditional — no `JoinContext` needed — which makes semi-joins the cheapest positive verdict in the whole calculus and the recommended rewrite target when `fan_out` fails closed on an affected-key probe.

(DuckDB's explicit `SEMI JOIN` / `ANTI JOIN` syntax is the same shape and should get the same rule.)

### 3.7 GROUP BY — the establishing operator

`GROUP BY k1, …, kn` establishes output grain `{k1..kn}` *unconditionally* — the one operator that creates a key out of nothing. This is the axiom `keyed_models.md` leans on when it requires the body to be the aggregation itself: `unique_key` = GROUP BY list is checked syntactically, and the grain follows by construction. Grouping on *expressions* establishes the grain on those expressions; it maps back to a column key only when the expressions are bare columns (same injectivity caveat as §3.1: `GROUP BY date_trunc('month', ts)` establishes a grain on the truncated value, not on `ts`).

### 3.8 DISTINCT

`SELECT DISTINCT …` establishes the whole projected row as a key. Note what it does *not* establish: the intended business key. `SELECT DISTINCT customer_id, region FROM stg` is unique on `{customer_id, region}`; it is unique on `{customer_id}` only if `customer_id → region` functionally — which DISTINCT does not prove (a customer with two regions yields two rows, silently). `DISTINCT ON (k)` (DuckDB supports it) *does* establish `{k}` — it is the dedup idiom in disguise, with engine-chosen survivor unless ORDER BY is given.

### 3.9 Window functions and the QUALIFY dedup idiom

Window functions are grain-preserving: they add columns, one output row per input row. No window function changes cardinality.

But one *composition* is an establishing operator and is statically recognisable:

```sql
SELECT * FROM events
QUALIFY row_number() OVER (PARTITION BY event_id ORDER BY ingested_at DESC) = 1;
```

`ROW_NUMBER() OVER (PARTITION BY k …) = 1` (via QUALIFY, or a subquery + `WHERE rn = 1`) establishes grain `{k}`: row_number assigns distinct values within each partition, so filtering to `= 1` keeps exactly one row per partition key. Recognition conditions: the function is `ROW_NUMBER` (not `RANK`/`DENSE_RANK` — ties give multiple 1s: with two rows tied on the ORDER BY, `RANK()` assigns 1 to both and the "dedup" emits two rows per key), the comparison is `= 1` (or `<= 1`), and the partition list is the candidate key. This is the second axiom-generator after GROUP BY and the standard latest-record idiom; recognising it is high-value because it is how modellers spell dedup when they need non-aggregated columns from the surviving row.

### 3.10 UNION ALL — the motivating destroyer

UNION ALL destroys uniqueness **even when both branches are unique on the same key**. Concrete rows:

```sql
-- online_orders grain {order_id}:  (1, 'web'), (2, 'web')
-- store_orders  grain {order_id}:  (1, 'pos'), (7, 'pos')   -- order 1 exists in both systems
SELECT order_id, channel FROM online_orders
UNION ALL
SELECT order_id, channel FROM store_orders;
-- rows: (1,'web'), (2,'web'), (1,'pos'), (7,'pos')  → order_id=1 twice. No grain.
```

Each branch is duplicate-free; the union is not, because uniqueness within a branch says nothing about the *intersection of key domains across branches*. This is the composition failure that motivates the whole calculus: two individually-keyed CTEs union'd is the everyday spelling of "combine two sources", and any grain inference that naively took `key ∪ key = key` would be unsound.

**When it IS safe: provably disjoint branches.** Two statically recognisable disjointness proofs:

1. **Constant discriminator column in the key.** If each branch projects a *distinct literal* into a column and that column is part of the claimed key, branches cannot collide:

```sql
SELECT 'online' AS src, order_id, channel FROM online_orders
UNION ALL
SELECT 'store' AS src, order_id, channel FROM store_orders;
-- grain {src, order_id}: within a branch, src is constant and order_id unique;
-- across branches, src differs on every pair. Proven — see §5.
```

2. **Disjoint source partitions**: both branches read the same keyed source under complementary constant predicates on a key column (`WHERE region = 'AU'` vs `WHERE region = 'NZ'`, region ∈ key). Sound but requires predicate disjointness reasoning; the discriminator form is the cheap v1 rule.

Absent a proof: **fail-closed, no grain** — matching `fan_out`'s discipline.

### 3.11 UNION [DISTINCT]

Restores *whole-row* uniqueness — the DISTINCT of the concatenation — but not the intended business key. On the rows above, `UNION` yields all four rows (they differ in `channel`), so `{order_id}` is still not a grain; only `{order_id, channel, …all columns}` is. UNION DISTINCT is DISTINCT's rule (§3.8) applied post-concatenation: establishes the all-projected-columns key, nothing narrower.

### 3.12 INTERSECT / EXCEPT

Both preserve any key of the **left** input: their output is a subset of the left input's distinct rows (both operators are set-semantic in SQL — they also deduplicate, so they additionally establish the whole-row key like DISTINCT). `INTERSECT ALL`/`EXCEPT ALL` preserve left keys by the subset argument (per-row multiplicity can only shrink, and a duplicate-free input has multiplicity ≤1 everywhere).

### 3.13 LIMIT / OFFSET

Subset of input rows → preserves all input keys. Establishes nothing (`LIMIT 1` semantically yields a ≤1-row relation — the same cheap special case as `WHERE key = const`, safe to recognise but marginal).

### 3.14 VALUES

A literal `VALUES` list has whatever grain its literal rows exhibit — statically checkable by inspecting the literals (all constants). Cheap and occasionally useful (seed/dimension enums); a duplicate literal key should probably be a diagnostic in its own right when the VALUES feeds a keyed model.

### 3.15 UNNEST / LATERAL

Explicit fan-out by construction: `SELECT o.order_id, u.item FROM orders o, UNNEST(o.items) AS u(item)` emits one row per element. Probe grain is destroyed; output grain is `{order_id, ordinality}` **only** when `UNNEST … WITH ORDINALITY` projects the ordinal (elements themselves may repeat: `items = ['a','a']`). A LATERAL subquery is `OneToOne` only if the subquery provably yields ≤1 row per invocation (e.g., it is a scalar-aggregate subquery — `SELECT max(…) FROM …` with no GROUP BY yields exactly one row); otherwise fail-closed.

## 4. Composition algebra

Grain inference is a bottom-up pass over the operator tree carrying, per node, a set of candidate keys (each a set of columns, tracked through renames). Declared `unique_key`s on inputs are the leaves' axioms; §3's rules are the transfer functions. Worked compositions:

**Chained many-to-one joins (star).** Fact F `{order_id}` joins dim D1 on `customer_id = D1.customer_id` (D1 unique on it), then dim D2 on `product_id = D2.product_id` (D2 unique). Each join is `OneToOne` against the *current* probe; the probe grain flows through unchanged. Net grain: `{order_id}` — the fact grain, regardless of how many dimensions are chained. Rule: `OneToOne` joins compose; grain inference is a left fold with identity on the probe key set.

**Snowflake chains (transitive OneToOne).** F → D1 (`OneToOne`), D1 → D2 on D1's `category_id` where D2 is unique on `category_id`: the second join's probe is the *already-joined* relation, whose grain is still `{order_id}`; the join is again `OneToOne`, so `{order_id}` survives transitively. The subtlety today's `fan_out` cannot see: the second join's equality references a column (`d1.category_id`) of an intermediate relation, not of a declared leaf — the proof needs the intermediate's column provenance ("`category_id` here is D1's `category_id`, and D1 is being probed at its key so its columns ride along 1:1"), i.e., grain inference must run interleaved with column-lineage through the tree, not per-clause.

**Fan-out then GROUP BY back — grain restored, lineage multiplicity persists.** F `{order_id}` joins payments (`OneToMany`, two payments for order 1), then:

```sql
SELECT order_id, sum(p.paid) AS total_paid, max(o.amount) AS amount
FROM orders o JOIN payments p USING (order_id)
GROUP BY order_id;
```

The GROUP BY re-establishes `{order_id}` — the *relation grain* is fine, and a `merge_into` on `order_id` is well-keyed. But per column: `sum(p.paid)` folds payment rows — correct, payments are the thing being summed; `max(o.amount)` sees `o.amount` **duplicated once per payment** — harmless only because `max` is idempotent. Replace it with `sum(o.amount)` and order 1 contributes 200.0 instead of 100.0. So the calculus must carry **two facts** through the tree: the relation's grain (a per-node property) and, per column, a *lineage multiplicity* flag ("has this column's source row been multiplied since its origin grain?"). The relation-level verdict alone would wrongly bless the composition; this is exactly the `join_contribution_monotone` composition (fan-out × discriminant), generalised from one join clause to the tree. The refusal condition: a duplicated-lineage column feeding a non-idempotent (duplicate-sensitive) combiner.

**Union of two keyed CTEs.** Fails (§3.10). **Discriminated union — proof.** Claim: if branch Bi has grain Ki, projects a literal constant ci into column `d`, all ci pairwise distinct, then `UNION ALL` of the Bi has grain `{d} ∪ K` (K the shared key columns). Proof: take rows r, s agreeing on `{d} ∪ K`. Agreement on `d` means `r.d = s.d = ci` for a single i (the ci are distinct constants and each branch emits only its own), so r and s come from the same branch Bi; within Bi they agree on Ki ⊆ K's image, and Bi is duplicate-free on Ki, so r = s. ∎ — Both premises are syntactically checkable: literal-constant projection per branch, pairwise-distinct literals.

**Join of two GROUP BY outputs on their full keys.** 

```sql
WITH by_customer AS (SELECT customer_id, sum(amount) s FROM orders GROUP BY customer_id),
     by_customer_ret AS (SELECT customer_id, count(*) n FROM returns GROUP BY customer_id)
SELECT * FROM by_customer b JOIN by_customer_ret r USING (customer_id);
```

Both sides have grain `{customer_id}` — **derived** (by GROUP BY), declared nowhere. The join is `OneToOne` in both directions; output grain `{customer_id}`. This is the key point for the implementation: `fan_out` today can only prove this if someone injects `customer_id` into `JoinContext` by hand. Grain inference makes `fan_out` a *consumer of derived keys* — the establishing operators (§3.7–3.9) feed the same `unique_keys` map that declarations do, and the join proof is unchanged. Derived and declared keys are the same currency.

### Operator × rule table

| Operator | Grain rule | Establishes? |
|---|---|---|
| Projection | key survives iff all key columns projected as bare refs (renames ok) | no |
| WHERE / HAVING | preserves all keys | no (except `key = const` ⇒ ≤1 row) |
| INNER JOIN, joined side unique on join cols | probe keys preserved (row set may shrink) | no |
| INNER JOIN otherwise | probe keys lost; composite (probeK ∪ joinedK) if both keyed, else none | no |
| LEFT JOIN | as INNER on preservation; keeps unmatched (NULL-extended) left rows | no |
| FULL OUTER | no single-side grain; composite-with-NULLs only, fail-closed | no |
| CROSS JOIN | composite if both keyed; never OneToOne statically | no |
| SEMI / ANTI (EXISTS/IN) | preserves all probe keys, unconditionally | no |
| GROUP BY k… | — | **yes: {k…}** (bare columns) |
| DISTINCT | — | **yes: all projected columns**; `DISTINCT ON (k)` ⇒ {k} |
| ROW_NUMBER PARTITION BY k … QUALIFY =1 | — | **yes: {k}** (ROW_NUMBER only, not RANK) |
| Other window fns | preserves all keys | no |
| UNION ALL | destroys, even when both branches keyed | discriminated form ⇒ {d}∪K |
| UNION (DISTINCT) | — | yes: all projected columns (not the business key) |
| INTERSECT / EXCEPT | preserves left keys | whole-row key (set semantics) |
| LIMIT/OFFSET | preserves all keys | no (`LIMIT 1` ⇒ ≤1 row) |
| VALUES | grain of the literal rows (checkable) | per-literal |
| UNNEST / LATERAL | destroys probe grain; `{probeK, ordinality}` WITH ORDINALITY; LATERAL scalar subquery ⇒ preserve | no |

## 5. Static provability vs declaration

The calculus has exactly three sources of truth, in trust order:

1. **Declared unique keys — the axioms.** `unique_key` on models, key declarations on sources. These are *asserted*, not proven; a wrong declaration makes every downstream verdict wrong. (The checked-declaration principle from `model_maintenance.md` — "a declared bound is admitted only checked" — suggests the same posture here eventually: a cheap runtime uniqueness assertion at write time, fail-loud on violation.)
2. **Derived keys — provable extensions.** GROUP BY, DISTINCT / DISTINCT ON, the ROW_NUMBER-=1 idiom, discriminated UNION ALL, and the preservation rules that carry both kinds through the tree. These are theorems given the axioms (GROUP BY and DISTINCT need no axioms at all). They are strictly more trustworthy than declarations and should be preferred when both exist.
3. **Everything else fails closed.** Unrecognised operator, expression-valued keys, RANK-based "dedup", undecidable predicate disjointness → the key set is dropped, joins against the relation get `OneToMany`, keyed admission is refused with a reason naming the operator that lost the grain. This matches join_shape.rs's existing discipline (CROSS JOIN / missing condition / unmatched equality → `OneToMany`, "never optimistically skipped") and the project-wide fail-loud rule.

Declarations should remain *only* for what is genuinely unprovable: leaf-source uniqueness, and (per `feedback_derive_dont_declare`) nothing that the SQL itself can establish — a modeller should never re-declare the grain their GROUP BY already creates, and the classifier should reject a `unique_key` that contradicts the derived grain rather than trust it.

## 6. Implementation gaps (join_shape.rs today)

Specific deltas between the calculus above and `crates/smelt-logical/src/analysis/join_shape.rs`:

1. **Declared keys only, injected per call.** `JoinContext.unique_keys` is a `source-name → set<column>` map callers populate by hand; nothing wires model/source `unique_key` declarations into it, and no consumer is wired at all (`model_properties.md` Known Divergences: "no consumer wires them yet — that is F15").
2. **Single-column keys only.** The map's contract is "columns each of which *alone* uniquely identifies a row" and `fan_out` checks `equality_columns.iter().any(|col| keys.contains(col))`. A composite key `(a, b)` is inexpressible: declaring both columns would unsoundly claim each alone is unique, and not declaring them fails closed — catalog **G-10** ("join fan-out on composite unique key … mislabeled"). Fix: keys must be `HashSet<BTreeSet<String>>` (sets of column-sets) and the check becomes "some declared key ⊆ the ANDed equality columns".
3. **Per-join-clause only — no grain propagation.** `fan_out` takes one `JoinClause`; there is no operator-tree pass, so: the probe side's grain after a prior join is unknown (snowflake chains unprovable, §4); a joined side that is itself a subquery/CTE with a GROUP BY gets no key; multi-join FROM clauses evaluate each join against leaf declarations only.
4. **No derived-key recognition.** GROUP BY, DISTINCT, DISTINCT ON, QUALIFY ROW_NUMBER=1, discriminated UNION ALL — none establish entries in any key context. The "join of two GROUP BY outputs" case (§4) — provable with zero declarations — currently yields `OneToMany`.
5. **Unqualified ON columns never attribute** (`collect_equality_columns`: `None => false`). Conservative-correct, but `USING` gets a pass while the equivalent unqualified `ON a = b` fails; a two-source scope could attribute unqualified names uniquely by schema lookup.
6. **Equality-shape only**: only top-level ANDed `=` between column refs. `ON o.cid = c.cid AND c.region = 'AU'` works (the extra conjunct is ignored, harmlessly); `ON (o.cid, o.pid) = (c.cid, c.pid)` (row-value equality) and `ON o.cid = c.cid OR …` do not (OR correctly fails closed; row-value is a recognisable gap).
7. **No relation-level output.** The API returns the pairwise verdict; there is no `Grain`/`KeySet` type for "the grain of this model's output", which is what keyed admission (§2 item 1/4) and the composite-grain refinement (§3.3) need. The natural shape: `grain_of(select, ctx) -> KeySet` as a new pure analysis that *calls* `fan_out` per join and applies §4's table, with `join_shape` keeping the pairwise proof.
8. **No lineage-multiplicity tracking.** `join_contribution_monotone` composes fan-out with discriminants for one join feeding one aggregate; the tree-level fact "this column's lineage was multiplied upstream of the GROUP BY" (§4, fan-out-then-regroup) has no carrier.

None of these require changing the fail-closed posture — every gap currently errs conservative (`OneToMany` / no key), so the work is pure widening.

## 7. Open questions

1. **Should declared unique keys be runtime-checked at write time** (uniqueness assertion in the merge transaction, fail-loud), matching the "declared bounds are admitted only checked" principle — or trusted as axioms? Cost is one aggregation over the delta per run.
2. **Where does the grain calculus live relative to column lineage?** Snowflake chains and injective-projection tracking both need column provenance through intermediate relations; is grain inference a client of a shared lineage pass, or does it carry its own minimal rename map?
3. **How far to take derived-key recognition in v1** — GROUP BY + DISTINCT ON + QUALIFY ROW_NUMBER=1 seem clearly in; discriminated UNION ALL and predicate-disjoint unions are sound but need constant-folding; is the discriminator form common enough in real models to justify it first?
4. **Composite-key `JoinContext` (G-10) vs a full `KeySet` type:** fix the map contract in place (minimal, unblocks composite `unique_key` models) or land the relation-level `grain_of` API in the same change so `fan_out` becomes its internal step?
5. **Lineage multiplicity as a per-column bit or a count bound?** A boolean ("possibly multiplied") suffices for the idempotence composition; a bounded multiplicity (e.g., "×1 per matched payment") would additionally license compensating rewrites — is there a consumer that justifies the stronger form?
