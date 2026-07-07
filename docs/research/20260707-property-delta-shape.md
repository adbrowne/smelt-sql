# Property research: delta shape propagation

- **Date:** 2026-07-07
- **Status:** research
- **Related specs:** `docs/specs/model_properties.md` (row "Input-delta discovery"; catalogued `mutation_profile` input), `docs/specs/models.md` (§"Input-consumption axis", three-state declaration law), `docs/specs/model_maintenance.md` (scope maps in the composition contract; the equivalence invariant), `docs/specs/model_transforms.md` (the transform catalogue each shape drives), `docs/specs/sources.md` (`mutation_profile:`)
- **Related code:** `crates/smelt-logical/src/analysis/input_delta.rs` (the built three-verdict discovery), `crates/smelt-core/src/sources.rs` (`MutationProfile`)
- **Prior research:** `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` (the `(column-group × input-delta)` plan framing — this doc supplies the *input-delta* axis of that product), `10-dependency-propagation.md` (edge model, forward propagation — this doc supplies the payload those edges carry), `docs/research/property-discovery/catalog.md` (cells SC-2, G-06)
- **Sibling property docs (2026-07-07 series):** `20260707-property-determinism.md` (run-varying expressions — the mechanism that manufactures deltas out of static inputs), the bounded-reach property doc (frame/horizon neighbourhoods that window functions and joins spread a delta over)

---

## 1. The property

**Delta shape** is a static classification of *how a relation is allowed to change between two runs*. Not how much it changes — how. It is a property of an edge in the dependency graph: "when this input's contents move from run *n* to run *n+1*, the difference is guaranteed to have this form."

### The lattice

```
none  ⊑  insert-only  ⊑  upsert-by-key(K, C)  ⊑  general
```

| Shape | Meaning | Z-set characterisation |
|---|---|---|
| `none` | The relation is identical across the two runs (static/settled). | Δ = 0 |
| `insert-only` | Rows may appear; no existing row is modified or removed. | Δ ≥ 0 (a non-negative multiset) |
| `upsert-by-key(K, C)` | Relative to a key set `K` with at most one live row per key: new keys may appear, and existing keys may have their values in column set `C ⊆ non-key columns` replaced. No key disappears; `K`-columns of existing rows never change. | Δ = Δ⁺ − Δ⁻ where every negative row in Δ⁻ is the *current* row at some key `k`, matched by exactly one positive row in Δ⁺ at the same `k` (differing only in `C`); unmatched positives are new keys |
| `general` | Arbitrary updates and deletes (including key changes, which are a delete plus an insert). | Δ an arbitrary signed multiset |

`upsert-by-key` carries **two parameters**: the key `K` (which columns identify a row) and the mutable-column set `C` (which columns can differ between the retracted and re-asserted row). `C` is the **per-column refinement**: `upsert-by-key(K, ∅)` collapses to `insert-only` restricted to new keys; `upsert-by-key(K, all)` with key deletion allowed collapses to `general`. Downstream, `C` decides whether a consumer that only projects columns *outside* `C` sees any change at all — a filter or projection can lower the shape it observes (§5).

The lattice order is "can be treated as": every `none` delta is a valid `insert-only` delta (the empty one), every insert-only delta is a valid upsert delta (no matched negatives), every upsert delta is a valid general delta. A maintenance plan admissible for shape `s` is therefore admissible for every shape below `s` — which is exactly why **fail-closed means round up to `general`**, never down.

### Why *shape*, not size, is the static property

Size (how many rows changed) is a runtime fact — it varies per run and can only be *observed*. Shape is a **guarantee over all runs**, so it can be proven at plan time from declared world-facts and the operator tree, and it is what admission needs: whether `INSERT INTO` is a correct write transform does not depend on whether tonight's delta has 10 rows or 10 million; it depends on whether a delta can ever contain a retraction. Shape is to deltas what a type is to values.

### The Z-set backbone (DBSP / differential dataflow framing)

Model every relation as a **Z-set** (multiset with integer multiplicities); a delta is a signed Z-set `ΔR = R_{n+1} − R_n`. Every relational operator `Q` has a **delta rule**: `Q(R + ΔR) = Q(R) + Δ_Q(R, ΔR)`, and the structure of `Δ_Q` is determined by the operator's algebra:

- **Linear** operators (projection, selection, `UNION ALL`) satisfy `Q(R + ΔR) = Q(R) + Q(ΔR)` — the delta rule is the operator itself, so signs pass through unchanged: linear operators are **sign-preserving**, hence shape-preserving (up to key survival, §4.1).
- **Bilinear** operators (join) satisfy `Δ(A ⋈ B) = ΔA ⋈ B + A_old ⋈ ΔB + ΔA ⋈ ΔB`. Products of non-negatives are non-negative, so inner join preserves insert-only — but the cross terms multiply a delta against *old* state, which is the addressing subtlety of §4.3.
- **Non-linear** operators (`DISTINCT`, `GROUP BY`, `EXCEPT`/`NOT EXISTS`, `LIMIT`/top-k, window functions) have delta rules that consult old state and can **emit signs the input delta did not contain**. This is where insert-only inputs manufacture retractions downstream — the entire interesting content of this document.

smelt's three-shape vocabulary is a deliberate coarsening of arbitrary Z-sets: `insert-only` = non-negative Δ; `upsert-by-key` = Δ whose negative part is exactly "the current row at a key being replaced" (so the write transform never needs a standalone `DELETE`, only `MERGE`); `general` = anything. The coarsening is what makes the property finitely checkable and what maps one-to-one onto the write-transform catalogue (§2).

### Relation to `mutation_profile` — the source axiom

`sources.md`'s `mutation_profile:` is the **declared world-fact** at the leaves; delta shape is its derived, propagated form:

| Declared profile | Leaf delta shape |
|---|---|
| `append_only` | `insert-only` |
| `mutable` (`mutable_snapshot`) | `general` (a snapshot read observes arbitrary in-place edits and deletes) — refinable to `upsert-by-key(unique_key, C)` when the source additionally declares `unique_key:` and, per the structured block direction (`sources.md` §"`mutation_profile` — the structured block"), a no-delete sub-fact |
| `change_feed` | the feed's own shape: `retractions: true` ⇒ `general` carried explicitly; keyed CDC upserts ⇒ `upsert-by-key`; insert-only feed ⇒ `insert-only` |
| *(undeclared)* | `general` — fail-closed, per `models.md`'s three-state declaration law (undeclared = strictest) |

A settled/frozen input (a fully-built upstream partition inside its watermark, a static seed) has shape `none`. Note the profile is declared **on the source and shared by every consumer** (`models.md` §"Vertical is declared, horizontal is derived") — shape is then *derived* per model per input edge; nothing above the leaf is ever declared.

One more leaf that is easy to miss: **the model's own text**. A run-varying expression in the model (`now()`, `current_date`) is an implicit input whose value changes every run — it can give a model a non-trivial output delta even over `none`-shaped relational inputs (§4.2; sibling determinism doc).

---

## 2. Why maintenance needs it

The output delta shape **selects the write transform** (`model_transforms.md` catalogue):

| Output delta shape | Admissible write transform | Catalogue entry |
|---|---|---|
| `none` | no-op (skip the run for this edge) | — |
| `insert-only` | plain `INSERT INTO` / append | append (windowed fold's write half) |
| `upsert-by-key(K, C)` | `MERGE` on `K`, updating only `C` | keyed `merge_into` (target-as-replica); generic column-scoped merge when `C` is one mutation-sensitivity group |
| `general`, bounded footprint | delete+insert over the affected region | region recompute (`DELETE` covering the write window + `INSERT`, cf. `filter_range`) |
| `general`, unbounded/unknown footprint | full recompute | `full` |

Choosing a transform *above* the true shape is a correctness bug (an append cannot express a retraction — the stranded row of G-06 stays wrong forever); choosing one *below* is only a cost bug. Hence the derivation must be sound-upward: every uncertainty rounds toward `general`.

**Scope maps are exactly this property, per input edge.** `model_maintenance.md` defines a scope map as "for each input of a model, the derived mapping from that input's delta to the affected output addresses and the transform that runs for it." Decomposed:

```
scope map(input i) = ( shape_i --[transfer rules along the operator path from i to the output]--> output shape
                     , delta addresses --[the same path]--> output addresses )
```

The first component is this document; the second (addressing/footprint) is the bounded-reach sibling. They are distinct — §4.3 shows an edge where the shape stays `insert-only` but the addresses escape the input's window — and both are needed: shape picks the transform, addressing picks the region it runs over. A run is the union of its inputs' scope maps; the equivalence invariant (`model_maintenance.md` §"The equivalence invariant") is the theorem each cell must satisfy: at a fixed processed-input set, the incremental transform and the from-scratch recompute produce identical state.

This is also precisely the `input-delta` axis of the `(output-column-group × input-delta)` plan object of `01-framework.md`: each column of that matrix is one input edge's derived shape, and per-column refinement (`C`) is what makes the *row* axis (column groups) interact with it — an input whose upsert-mutable set `C` misses a column group entirely contributes shape `none` to that group's cell.

---

## 3. Notation for the two-run narratives

Each per-construct section below uses the same discipline: **run 1** materialises the model over input state `S₁`; the input then changes by delta `Δ` (whose shape we state); **run 2** must leave the output equal to a from-scratch evaluation over `S₂ = S₁ + Δ` (the equivalence invariant). The question in every case: *what shape is the required output change* `Q(S₂) − Q(S₁)`? All SQL is DuckDB-correct.

Shared fixtures (used with variations):

```sql
-- fact: append-only clickstream            -- dim: mutable keyed reference
CREATE TABLE events (                        CREATE TABLE users (
  event_id INT, user_id INT,                  user_id INT PRIMARY KEY,
  amount DECIMAL(10,2), ts TIMESTAMP          country TEXT, tier TEXT
);                                           );
```

---

## 4. Per-construct analysis: delta-transfer rules

### 4.1 Projection and scalar expressions — shape-preserving, with a key-survival side condition

```sql
SELECT event_id, amount * 1.1 AS amount_gross FROM events
```

Projection with row-deterministic scalar expressions is linear: `π(S + Δ) = π(S) + π(Δ)`. Signs pass through, so the shape passes through: `none → none`, `insert-only → insert-only`, `general → general`.

**Side conditions.**
- *Key survival:* `upsert-by-key(K, C)` stays upsert **only if `K` survives the projection**. If the key columns are projected away, the matched-retraction structure is no longer expressible in the output schema — two input rows that differed only in `K` collapse, multiplicities matter, and the honest output shape is `general` (or one re-keys on a surviving candidate key, if provable).
- *Per-column refinement (shape can decrease):* if the projection keeps `K` but drops every column of `C`, the delta becomes invisible — output shape `none`. `SELECT user_id, country FROM users` over `upsert-by-key(user_id, {tier})` is static. This is the cheapest and most consequential refinement in the whole algebra: it turns "mutable dimension upstream" into "no maintenance work at all" for consumers that don't read the mutable columns.
- *Determinism:* the scalar must be row-deterministic. `SELECT event_id, now() AS seen_at` re-derives a different value for *every* row on every run — a from-scratch evaluation of run 2 differs from run 1 on all pre-existing rows, i.e. output shape `general` from a `none` input. Sibling doc `20260707-property-determinism.md`; the append transform is admissible here only under the weaker snapshot-consistency contract (the stored value is "the run that inserted it"), which is a *deliberate* relaxation, not a derivation.

Two-run narrative (refinement case): run 1, `users = {(1,'AU','gold')}`, output of `SELECT user_id, country FROM users` is `{(1,'AU')}`. Delta: tier of user 1 flips to `'silver'` — `upsert-by-key(user_id, {tier})`. Run 2 correct output: `{(1,'AU')}` — unchanged. Output shape `none`; the scope map for this edge is empty.

### 4.2 WHERE — sign-preserving, but boundary-crossing updates and run-varying predicates

**Static row-deterministic predicate.** `σ_p` is linear: `σ(S+Δ)=σ(S)+σ(Δ)`. So:

- `insert-only → insert-only` (some inserts filtered out, none inverted).
- `upsert-by-key(K, C)` where `p` reads **no column of `C`**: predicate membership is stable per key → `upsert-by-key(K, C)` preserved (some keys invisible on both sides).
- `upsert-by-key(K, C)` where `p` reads a column of `C`: an update can move a row **across the predicate boundary**. Run 1: `users = {(1,'AU','gold'),(2,'AU','silver')}`, model `SELECT * FROM users WHERE tier = 'gold'` outputs `{(1,…)}`. Delta: user 1 → `'silver'`, user 2 → `'gold'`. Run 2 correct output: `{(2,…)}` — the change is *delete key 1, insert key 2*. A deletion of a key is not expressible as an upsert; output shape is **`general`** (key-scoped, so delete+insert by key — a `MERGE … WHEN MATCHED AND NOT p THEN DELETE` — is the natural transform, but plain `merge_into` is not).
- `general → general`.

**Run-varying predicate — the sliding filter.** `WHERE ts > now() - INTERVAL 7 DAY`. Even over a **`none`** input the output changes every run: run 1 (evaluated 2026-07-01) contains rows with `ts` in `[06-24, 07-01]`; run 2 (2026-07-08) must *not* contain the `[06-24, 07-01)` rows and must contain `[07-01, 07-08]`. The correct output change includes **deletes of rows the input never touched**: `none → general`. The delta was manufactured by the model's own non-determinism, not by any input edge — which is why the determinism property is a *prerequisite* input to this algebra: every transfer rule below assumes row-deterministic operators, and the determinism doc identifies where that assumption fails. (A monotone clock comparison `WHERE ts <= run_watermark` against a settled watermark is the benign special case: rows only ever *enter*, giving `none → insert-only` — the windowed-fold read scope in disguise.)

**Transfer rule (WHERE, deterministic p):** `shape_out = shape_in`, except `upsert-by-key(K,C) → general` when `columns(p) ∩ C ≠ ∅`. **(WHERE, run-varying p):** `shape_out = general` regardless of input.

### 4.3 Inner join — bilinear; insert-only preserved, but addressing escapes

`Δ(A ⋈ B) = ΔA ⋈ B_old + A_old ⋈ ΔB + ΔA ⋈ ΔB`.

- `insert-only ⋈ none = insert-only` (only the first term is non-zero; non-negative ⋈ anything-fixed-non-negative ≥ 0).
- `insert-only ⋈ insert-only = insert-only` (all three terms non-negative).

**The late-pairing subtlety — shape vs addressing.** Run 1: `events = {(e1, u1, 07-01)}`, `orders(right) = {}` (an inner join `events ⋈ orders ON user_id` outputs nothing). Delta: one **new** right row `(u1, order o9, 07-08)` — insert-only, timestamped in the new window. Run 2 correct output: insert `(e1 ⋈ o9)`. Shape: pure insert — but the inserted output row **lives at e1's address** (July 1's partition / key-space), *outside* the delta's own window. The `A_old ⋈ ΔB` term pairs new right rows with **old left rows**. So:

- the **write transform** may still be append (`insert-only` shape holds), but
- the **write addresses** are not bounded by the input delta's window — a window-forward read of the left side would miss `e1`, and a partition-clamped append (batched's write-eligibility clamp) would drop the row.

This is why shape and footprint are separate components of the scope map: `insert-only ⋈ insert-only` is admissible for *append* but not for *window-clamped append* unless a join-window bound is separately proven (the bounded-reach sibling; `model_transforms.md`'s dimension-driven horizon-bounded MERGE is the built instance of proving such a bound for the recompute read).

- `insert-only ⋈ upsert-by-key(K_B, C)`: an update to right row `b@k` rewrites **every** old output row that joined `b` — an upsert on the output key `(K_A-part, K_B-part)` touching the `C`-derived output columns, **provided the join column ∉ C**. If the join key itself is mutable (`join_col ∈ C`), an update re-routes rows — outputs at old pairings must be deleted → `general`. Fan-out multiplies the *count* of touched rows, never the shape.
- Anything ⋈ `general` = `general` (a right-side delete removes every output row it fed — deletes appear).

**Transfer rule (inner ⋈):** `shape_out = lub(shape_A, shape_B)` with two demotions: `upsert → general` if the join column is in the upsert's mutable set; and output-`upsert` requires the output key to embed both sides' keys. Addressing: never bounded by the delta's own address-space without a separately proven join bound.

### 4.4 LEFT JOIN — the stranded-NULL retraction (G-06)

```sql
SELECT e.event_id, e.user_id, o.order_id
FROM events e LEFT JOIN orders o USING (user_id)
```

Run 1: `events = {(e1, u1)}`, `orders = {}`. Output: `{(e1, u1, NULL)}` — the null-padded row. Delta: **insert-only** right row `(u1, o9)`. Run 2 correct output: `{(e1, u1, o9)}`. The required change: **retract** `(e1, u1, NULL)`, insert `(e1, u1, o9)`.

An insert-only input has produced an output delta with a negative part. But the negative part is structured: the retracted row is exactly the current row at left-key `e1`, replaced by a row at the same left key. So:

**Transfer rule (LEFT JOIN, insert-only right delta):** `insert-only(B) → upsert-by-key(K_A, right-side columns)` — *not* insert-only, *not* general. This is the canonical "non-linear operator manufactures retractions" case, and it is a real recorded refutation: catalog cell **G-06** (`docs/research/property-discovery/catalog.md`) — "append-only + late right side: HOLDS for recompute; **fold strands the NULL row**." A fold that appends `ΔA ⋈ B ∪ A ⋈ ΔB` leaves `(e1, u1, NULL)` in place *alongside* `(e1, u1, o9)`: a duplicate left row and a wrong NULL, permanently. The admissible transforms are region recompute (delete+insert over the affected left rows — what the catalogue verified) or a `MERGE` keyed on the left key.

Left-side insert-only delta is benign (`insert-only`): a new left row joins the old right state or gets NULL, either way a pure insert (subject to §4.3's addressing caveat in reverse — it reads *old right* state, fine for footprint since the write lands at the new left row). Right-side deletes (`general` right) re-strand: removing `o9` must put the NULL row *back* — still upsert-by-left-key if the right side is keyed, `general` (multiplicity accounting) otherwise. `FULL OUTER JOIN` symmetrises: insert-only on *either* side yields upsert on the other side's null-padding.

### 4.5 Anti-join / NOT EXISTS — anti-monotone: shape inversion

```sql
SELECT u.user_id FROM users u
WHERE NOT EXISTS (SELECT 1 FROM orders o WHERE o.user_id = u.user_id)
```

Run 1: `users = {u1, u2}`, `orders = {}`. Output (`never_ordered`): `{u1, u2}`. Delta: **insert-only** — `orders` gains `(u1, o9)`. Run 2 correct output: `{u2}`. The output change is a **pure delete**.

This is the sharpest case in the algebra: the right side of an anti-join is **anti-monotone** — an upstream INSERT causes a downstream DELETE, an upstream DELETE causes a downstream INSERT. Insert-only inverts into delete-only. Our lattice has no delete-only point, so the verdict rounds to **`general`** (though the finer fact — "deletes keyed by the outer key, no updates" — would license a keyed `MERGE … WHEN MATCHED THEN DELETE`, cheaper than delete+insert; a candidate lattice extension, §8). Left-side insert-only remains insert-only (a new outer row probes old right state — with the §4.3 caveat that the probe must read the *whole* right state, not a window). `EXCEPT` is the set-operator spelling of the same rule (§4.10). Any pipeline whose path to the output passes through the right side of an anti-join can never be maintained by append, whatever the sources declare — this single rule is undecidable from source profiles alone and absent from today's classifier (§7).

### 4.6 GROUP BY + aggregates — insert-only becomes upsert-by-group-key; the per-column showcase

```sql
SELECT user_id, count(*) AS n, sum(amount) AS total
FROM events GROUP BY user_id
```

Run 1: `events = {(e1,u1,10.00),(e2,u1,5.00),(e3,u2,7.00)}` → `{(u1,2,15.00),(u2,1,7.00)}`. Delta: insert-only `{(e4,u1,3.00),(e5,u3,9.00)}`. Run 2 correct output: `{(u1,3,18.00),(u2,1,7.00),(u3,1,9.00)}` — **update** group u1's aggregate columns, **insert** group u3, leave u2 alone.

**Transfer rule:** `insert-only → upsert-by-key(group key, aggregate columns)`. The group key never changes on an existing output row (it *is* the identity); only aggregate columns move. This is per-column refinement at its cleanest: `C =` exactly the aggregate columns, so a downstream consumer of the group key alone sees `none`, and the `(column-group × input-delta)` matrix of `01-framework.md` gets a genuinely different cell per column group of the same model.

Refinements and the shape/transform split:

- Whether the *update* is a cheap fold or a group recompute is the **algebraic-ladder** question (`model_maintenance.md`), orthogonal to shape: `count`/`sum` fold new rows in (monoid); `min`/`max` fold under insert-only but need group recompute under retractions; `avg` folds via `(sum, count)` state. Shape says *upsert on the group key*; the ladder says *how to compute the new value*.
- `general → upsert-by-key(group key, aggs) ∪ {group deletions}`: aggregation **absorbs** — an arbitrary input delta still only ever touches the output at group-key granularity, so composition here can *decrease* effective shape (§5). But a delete of a group's last row deletes the output row, so strictly `general → general`-restricted-to-key-grain: keyed delete+insert / MERGE-with-delete, never full-table chaos. Under `general` input, non-invertible aggregates force per-touched-group recompute (read the group's rows whole) — read scope widens even though write shape is keyed.
- The group key columns must be deterministic; grouping on a run-varying expression (`GROUP BY date_trunc('week', now()) …`) reintroduces §4.2's manufactured deltas.

### 4.7 DISTINCT — insert-only survives, but only with an existence probe; deletes need counts

`SELECT DISTINCT user_id, country FROM events_geo`.

Run 1: input `{(u1,'AU'),(u1,'AU'),(u2,'NZ')}` → output `{(u1,'AU'),(u2,'NZ')}`. Delta: insert-only `{(u1,'AU'),(u3,'US')}`. Run 2 correct output: `{(u1,'AU'),(u2,'NZ'),(u3,'US')}` — one insert; the duplicate `(u1,'AU')` must be **suppressed against stored state**, not just within the batch.

**Transfer rule:** `insert-only → insert-only`, with the side condition that the transform is not blind append but **append-with-membership-probe** (`INSERT … SELECT DISTINCT d.* FROM delta d ANTI JOIN target t USING (…)` — i.e. the delta rule consults old output). `upsert/general → general` in output terms — worse, correctness needs **multiplicity counting**: deleting one of the two `(u1,'AU')` input rows must *not* delete the output row; deleting both must. Without a stored count per distinct tuple (the classic DBSP `distinct` incrementalisation state), the only honest transforms are recompute-region or keeping the count as auxiliary state. `GROUP BY all-columns` view of `DISTINCT` makes this §4.6 with `count(*)` as hidden state.

### 4.8 Window functions — an insert rewrites its frame-mates

```sql
SELECT event_id, user_id, ts,
       row_number() OVER (PARTITION BY user_id ORDER BY ts)          AS seq,
       sum(amount)  OVER (PARTITION BY user_id ORDER BY ts
                          ROWS BETWEEN 2 PRECEDING AND CURRENT ROW)  AS roll3
FROM events
```

Run 1: `events(u1) = {(e1,07-01,10),(e2,07-02,5)}` → `seq: e1→1, e2→2; roll3: e1→10, e2→15`. Delta: insert-only, one **late** row `(e0, u1, 06-30, 2.00)`. Run 2 correct output: `e0→(1, 2)`, `e1→(2, 12)`, `e2→(3, 17)` — the insert changed **other rows'** window outputs. `e1` and `e2` never appeared in any input delta, yet their `seq` and `roll3` values are stale.

**Transfer rule:** `insert-only → upsert-by-key(row identity, window-output columns)` over the **frame-reach neighbourhood** of the inserted rows: the set of existing rows whose frames the insert enters. For `ROWS BETWEEN 2 PRECEDING …` inserting at position *p* reaches forward ≤ 2 rows *if the insert is at the frontier*; a **late** insert plus a rank-like function (`row_number`, unbounded-preceding running sums) reaches **every subsequent row in the partition** — the neighbourhood is unbounded backward-in-effect. Whether that neighbourhood is finite/derivable is exactly the bounded-reach sibling property; shape-wise the verdict is uniform: window functions demote insert-only to upsert (on the row identity, on the window columns only — per-column refinement again: non-window pass-through columns stay `insert-only`-clean). If inserts are provably frontier-only (monotone clock + zero lateness watermark, `sources.md`) and the frame never looks forward, the neighbourhood is empty and `insert-only` survives — the assumption the windowed fold silently makes today.

`upsert/general` input: any touched row invalidates its whole partition's order-dependent outputs → per-touched-partition recompute (write shape: keyed to the partition — bounded, but the whole partition, not the row).

### 4.9 UNION ALL — least upper bound

Linear in both branches: `Δ(A ∪ₐₗₗ B) = ΔA ∪ₐₗₗ ΔB`. **Transfer rule:** `shape_out = lub(shape_A, shape_B)` — the lattice join, computed branchwise. `insert-only ∪ₐₗₗ none = insert-only`; `insert-only ∪ₐₗₗ upsert = upsert` only if both branches share a compatible key with disjoint or branch-tagged key-spaces (otherwise cross-branch duplicates make the key claim unprovable → `general`); anything with `general` = `general`. `UNION` (distinct) = `UNION ALL` then §4.7 — lub, then the DISTINCT rule (probe under insert-only; counts under retractions).

### 4.10 INTERSECT and EXCEPT

`INTERSECT` (set semantics) is monotone in both arguments: an insert to `A` can only add to `A ∩ B` (if already in `B` — an existence probe against the *other* side's old state, cf. §4.3's cross term), a delete to either side can only remove. So `insert-only ∩ insert-only = insert-only` (with probes both ways: a new `A`-row entering, and a new `B`-row *activating* an old `A`-row — old-address inserts again); any retraction on either side → deletes → `general`.

`EXCEPT` (`A \ B`) is monotone in `A`, **anti-monotone in `B`** — `NOT EXISTS` in set-operator clothing (§4.5). Run 1: `A = {x, y}`, `B = {}` → output `{x, y}`. Delta: insert-only `B += {x}`. Run 2: `{y}` — a delete. **Transfer rule:** `shape_out = general` whenever `shape_B ≠ none`; `= shape_A` (with DISTINCT-style probes) when `B` is static. Any operator with an anti-monotone argument position poisons append-admissibility through that edge no matter how well-behaved the source is.

### 4.11 ORDER BY … LIMIT (top-k) — insertion causes eviction

```sql
SELECT user_id, total FROM user_totals ORDER BY total DESC LIMIT 3
```

Run 1: totals `{(u1,90),(u2,80),(u3,70),(u4,60)}` → output `{u1,u2,u3}`. Delta: insert-only `{(u5, 85)}`. Run 2 correct output: `{u1, u5, u2}` — `u5` entered and **`u3` was evicted**. **Transfer rule:** `insert-only → general` (delete of an arbitrary existing row not identified by any input key). Top-k is non-monotone by construction; the honest transform is recompute of the k rows — cheap because the footprint is `k`, which is the redeeming observation: shape is `general` but the *region* is tiny and closed (the stored top-k plus the delta suffices under insert-only input; under retractions even that fails — evicting a retracted row needs rows *below* the old cut, i.e. state outside the output). A bare `ORDER BY` without `LIMIT` on a materialised table is a no-op for shape (stored relations are unordered; determinacy concerns live in `model_maintenance.md`'s order/set-determinacy corollary).

### 4.12 Self-reference — the model's prior output as an input

A model that reads its own previous materialisation (`smelt.this()`-style, or the keyed mode's implicit target-as-replica read) has an input edge whose shape is **defined by the model's own output shape last run** — a fixpoint, resolved by ordered execution (`model_maintenance.md`: "a self-edge engages ordered execution"). Two stable patterns: the **fold** (`new = f(old_state, input_delta)` — e.g. cumulative totals: old edge shape is `none` *within the run* because the run reads a fixed snapshot of it, and the output upserts by key: this is exactly keyed `merge_into`'s contract, old state as the left operand of a monoid step); and the **anti-join dedup** (`WHERE NOT EXISTS (SELECT 1 FROM this t WHERE t.id = new.id)` — the self-edge sits in an anti-monotone position, but against *own past output*, which the run only appends to *after* reading: benign under ordered execution, incoherent without it). The rule: a self-edge is admissible only when execution ordering makes the read snapshot well-defined; then its contribution to the composition is the snapshot's shape (`none` per-run), and correctness across runs is the equivalence invariant applied inductively (run *n*'s output correct ⇒ run *n+1* reads correct state).

---

## 5. Composition algebra

### The fold

For a model `M` with inputs `i₁…iₙ` and operator tree `T`:

```
output_shape(M, iⱼ) = fold of the per-operator transfer rules (§4) along the path from iⱼ's leaf to T's root,
                      each non-leaf operator combining its children's *propagated* shapes
                      (join/set ops take multiple children; the rules above say how)
output_shape(M)     = lub over j of output_shape(M, iⱼ)   ⊔ general-if-any-run-varying-expression (determinism doc)
```

The per-edge value — *before* the lub — is the shape column of that edge's **scope map**; it is deliberately not collapsed, because the transform dispatch is per-edge ("what runs when *this* input changes", `model_maintenance.md`).

### Worked pipeline

```sql
-- source events: mutation_profile: append_only, timeseries clock, watermark: 0-lateness  → leaf shape: insert-only (frontier-only)
-- source users:  mutable, unique_key: user_id, no-delete; mutable cols {tier}            → leaf shape: upsert-by-key(user_id, {tier})

WITH enriched AS (                                        -- (1) window over events
  SELECT event_id, user_id, amount, ts,
         sum(amount) OVER (PARTITION BY user_id ORDER BY ts) AS running_total
  FROM events
),
joined AS (                                               -- (2) LEFT JOIN mutable dim
  SELECT e.*, u.tier
  FROM enriched e LEFT JOIN users u USING (user_id)
)
SELECT user_id, tier, count(*) AS n, max(running_total) AS peak   -- (3) GROUP BY
FROM joined
GROUP BY user_id, tier
```

**Edge A (events):**
1. Window (§4.8): frontier-only insert-only + `ORDER BY ts` running frame that never looks forward ⇒ neighbourhood empty ⇒ **insert-only** survives. (Drop the 0-lateness watermark and this step alone becomes `upsert-by-key(event_id, {running_total})` — one declared world-fact flips the whole downstream plan.)
2. LEFT JOIN, *left*-side delta (§4.4): new left rows probe old `users` ⇒ **insert-only** (read footprint: whole dim — bounded-reach concern, not shape).
3. GROUP BY (§4.6): **upsert-by-key((user_id, tier), {n, peak})** — `max` folds under insert-only.
   **Edge-A scope map:** shape upsert-by-group-key; transform: keyed `merge_into`; addresses: groups of the delta's `(user_id, tier)` pairs.

**Edge B (users):**
1. Window: not on this path — `enriched` unchanged by a `users` delta.
2. LEFT JOIN, *right*-side keyed update, mutable col `{tier}` (§4.3/4.4): join column `user_id ∉ C` ⇒ every old output row of the updated user gets a new `tier` ⇒ **upsert-by-key(event_id, {tier})** on `joined` (a right-side *insert* — a previously unknown user — would instead be G-06's stranded-NULL retraction, also upsert-by-left-key; the no-delete sub-fact spares us right-deletes re-stranding).
3. GROUP BY — and here is the demotion: `tier` **is a group-key column**. An updated `tier` moves rows from group `(u1,'gold')` to `(u1,'silver')`: old group shrinks (possibly to deletion), new group appears ⇒ **general** (key-grain: delete+insert on the touched `(user_id, tier_old/new)` groups). §4.2's boundary-crossing rule, appearing as key-crossing.
   **Edge-B scope map:** shape general-at-group-grain; transform: delete+insert (or MERGE-with-delete) over groups of touched users; addresses: both old- and new-tier groups of each updated `user_id` — old-address writes, needing the dim delta to expose old *and* new values (change_feed) or a snapshot-diff.

**Model output shape:** `lub(upsert-by-key, general) = general` — but the *plan* is not "recompute": it is the union of a cheap keyed merge (edge A, every run) and a keyed delete+insert (edge B, only when the dim moved). Collapsing to the lub before dispatch is precisely the lossy projection `01-framework.md` warns `refresh:` is.

### Monotonicity — composition can decrease shape

The fold is *not* monotone-increasing along the tree, and that is a feature:

- **Aggregation absorbs** (§4.6): `general` in, keyed-grain out — arbitrary upstream chaos exits at group-key granularity.
- **Projection/filter over the mutable-column complement** (§4.1/4.2): `upsert-by-key(K, C)` in, **`none`** out when no column of `C` survives — the strongest argument that the shape must carry `C` and not just a three-value tag: without per-column refinement, every consumer of a mutable dimension is condemned to snapshot-diff even when it reads only immutable columns.
- **Top-k truncation** (§4.11) decreases *footprint* to `k` while increasing shape — the two components move independently, again confirming the shape/addressing split.

What composition can never do is *soundly* decrease shape by luck: every decrease above is licensed by a provable structural fact (key absorption, column disjointness). Absent the proof, round up.

### Transfer-rule table

Cells: output shape (side-conditions). `up(K,C)` = upsert-by-key. `⊥` = none.

| Operator ↓ / input shape → | `none` | `insert-only` | `up(K,C)` | `general` |
|---|---|---|---|---|
| π / scalar (row-det.) | ⊥ | ins-only | up(K,C∩kept) if K kept, ⊥ if C∩kept=∅, general if K dropped | general |
| π with run-varying scalar | general | general | general | general |
| σ (det., cols(p)∩C=∅) | ⊥ | ins-only | up(K,C) | general |
| σ (det., cols(p)∩C≠∅) | ⊥ | ins-only | **general** (boundary-crossing) | general |
| σ (run-varying p) | **general** | general | general | general |
| ⋈ inner (per edge; other side old state) | ⊥ | ins-only (**old-address writes**) | up if join-col∉C ∧ output key ⊇ both keys; else general | general |
| LEFT ⋈, left-edge delta | ⊥ | ins-only | as inner | general |
| LEFT ⋈, right-edge delta | ⊥ | **up(K_left, right-cols)** (stranded-NULL retraction, G-06) | up(K_left, right-cols) if keyed right; else general | general |
| anti-join / NOT EXISTS / EXCEPT, right edge | ⊥ | **general** (shape inversion: insert→delete) | general | general |
| anti-join, left edge | ⊥ | ins-only (whole-right probe) | as σ | general |
| GROUP BY + aggs | ⊥ | **up(group-key, agg-cols)** (fold if monoid, else group recompute) | up(gk, aggs) if changed cols ∉ group key; **general at group grain** if group-key col ∈ C | general at group grain (group deletes possible; non-invertible aggs ⇒ per-group recompute) |
| DISTINCT / UNION-distinct | ⊥ | ins-only (**membership probe**) | general (needs multiplicity counts) | general (counts) |
| window fn | ⊥ | up(row-id, window-cols) over frame-reach; ins-only iff frontier-only ∧ backward-only frame | per-touched-partition recompute (keyed to partition) | same |
| UNION ALL | branchwise lub | lub | lub (compatible keys, disjoint/tagged key-spaces; else general) | general |
| INTERSECT (per edge) | ⊥ | ins-only (cross-probe; old-address) | general | general |
| ORDER BY+LIMIT (top-k) | ⊥ | **general** (eviction), footprint ≤ k | general | general |
| self-edge (ordered exec.) | per-run ⊥ (fixed snapshot) | — | — | — |

---

## 6. Static provability vs declaration

The three-state declaration law (`models.md`) applies cleanly:

1. **The only declared fact is the leaf axiom** — `mutation_profile` (with its sub-facts: `unique_key`, `retractions`, `delta_identity`, watermark/lateness) on the source, shared by all consumers. Per `sources.md` §Semantics, a narrowing declaration is admitted only **paired with verification** (tripwire/probe) — a violated `append_only` must fail the consuming run loudly, because the append transforms it licensed have already written state that a violation makes wrong.
2. **Everything above the leaf is derived** — the transfer rules of §4 are pure functions of the operator tree plus the leaf shapes plus two sibling properties (determinism of expressions; frame/join reach). Nothing per model is declarable: a per-model "this output is insert-only" annotation would be exactly the drift-prone declaration `feedback_derive_dont_declare` rules out.
3. **Fail-closed lands at `general` → recompute.** Undeclared profile ⇒ leaf `general`; unknown operator / unanalysable expression ⇒ transfer rule `⊤` ⇒ output `general`; `general` with unprovable footprint ⇒ full recompute. This is the existing constraint (`model_properties.md` §Constraints; `input_delta.rs` doc-comment: "never an unsound optimistic delta") restated in shape vocabulary — today's `SnapshotDiff` fallback *is* leaf-`general` with a diff-based delta extraction.

The **litmus** for where a fact lives: if two consumers of the same source could honestly disagree about it, it is derived (it depends on their operator trees); if all consumers must agree, it is a world-fact on the source. Delta shape at the leaf is the latter; delta shape everywhere else is the former.

---

## 7. Implementation gaps

What exists (`crates/smelt-logical/src/analysis/input_delta.rs`) against this algebra:

1. **The built discovery is a leaf read-scope verdict, not a shape.** `input_delta_discovery(SourceShape) -> {WindowForward, SnapshotDiff, ChangeFeed}` classifies *how to find* the delta of one driving source. It never computes what the delta's shape **is** (`ChangeFeed` says the source reports changes, not whether they include retractions — `sources.md`'s `retractions:` sub-fact is unparsed), and nothing propagates through operators: there is no transfer rule, no per-operator analysis, no lattice.
2. **The clocked-mutable cell is unsound — the built refutation.** `input_delta.rs:88-94`: `Some(ChangeFeed) => ChangeFeed, _ if shape.has_clock => WindowForward, _ => SnapshotDiff` — a source that is *declared `Mutable` but carries a clock* gets `WindowForward`, silently missing back-dated in-place updates. This is catalog cell **SC-2** ("clocked mutable, in-place update between runs → window-forward misses it → REFUTED = bug"). In shape vocabulary the fix is mechanical: the verdict must be `min(read-scope-from-clock, read-scope-required-by-shape)` — leaf shape `general` forbids window-forward regardless of the clock. The current code lets a *scoping* fact (has-clock) override a *shape* fact (mutable).
3. **Declared profiles license almost nothing** (`sources.md` §Known Divergences): only `change_feed` changes the verdict; `append_only` on an unclocked source still falls to `SnapshotDiff` (test `append_only_without_clock_still_falls_back_to_snapshot_diff`) — sound but pessimal; the structured block (`unique_key`, `retractions`, `delta_identity`) doesn't parse, so `upsert-by-key` leaves are **inexpressible** today.
4. **No per-column refinement anywhere.** No `C` on any shape, no mutation-sensitivity column-group interaction with input deltas — `01-framework.md`'s `(column-group × input-delta)` matrix has neither axis materialised in code. Consequence: a consumer projecting only immutable dim columns still pays the mutable-dim path.
5. **Anti-monotone positions are unhandled and unguarded.** Nothing in `smelt-logical` detects `NOT EXISTS`/`EXCEPT`/anti-join argument polarity; the cumulative/keyed classifier (`rules/cumulative.rs`) gates on its own structural checks, but there is no general "this path inverts inserts into deletes" refusal. Today this is masked because non-window cells recompute unconditionally (`sources.md` §Known Divergences: "every partition-grain cell is served by unconditional recompute regardless of profile") — the moment fold transforms widen, polarity analysis becomes load-bearing.
6. **Scope maps exist in spec, not in code or `smelt explain`** (`model_maintenance.md` §Known Divergences): the per-input-edge dispatch this property feeds has no runtime object; the transform union of §5's worked pipeline is not constructible.
7. **The stranded-NULL cell is verified only for recompute** (G-06): correct today because the transform *is* recompute; the LEFT-JOIN transfer rule (§4.4) is the admission check that must exist before any fold serves that cell.

Placement note: the natural home is a new pure analysis in `smelt-logical` (sibling to `analysis/input_delta.rs`), folding over `LogicalNode` — per the layered single-ownership invariant, below both `smelt-db` and `smelt-planner`, consumable by explain and by admission.

---

## 8. Open questions

1. **Should the lattice grow a `delete-only` / keyed-delete point?** Anti-monotone edges (§4.5, §4.10) and boundary-crossing filters (§4.2) all land at `general` today, yet their negatives are keyed and update-free — a `MERGE … WHEN MATCHED THEN DELETE` is admissible and much cheaper than delete+insert. Cost: a 5-point lattice (and `up(K,C)`+deletes as a 6th?) versus the three-shape vocabulary already woven through the specs.
2. **How is `C` (the mutable-column set) represented across renames/expressions?** Per-column refinement must survive projection renaming and expression derivation (`tier` feeds `CASE WHEN tier='gold'…`) — this is column-level lineage; does it reuse the mutation-sensitivity column-group machinery (`model_transforms.md`'s generic column-scoped merge) or need its own tracking?
3. **Where does the shape × footprint product live?** §4.3/4.11 show the two components moving independently; is the scope-map object `(shape, footprint)` per edge computed by one fused tree-fold with the bounded-reach analysis, or two analyses joined at explain/admission — and which spec owns the product (`model_properties.md` rows are per-property; the product smells like `model_maintenance.md`'s)?
4. **What is the verification story for derived upsert leaves?** `upsert-by-key` from `mutable + unique_key + no-delete` rests on three narrowing declarations; `sources.md` demands paired tripwires — is a per-run key-uniqueness + no-disappearance probe on the read snapshot cheap enough to be default-on, and what does the run do when it trips (the state already merged under the violated assumption)?
5. **Fixpoint semantics for self-edges beyond the two blessed patterns** (§4.12): should the analysis *classify* arbitrary self-references (fold vs anti-join-dedup vs unsound) and refuse the rest, or is the keyed mode's structural gate the only self-edge ever admitted?
