# Property research: event-time monotonicity / traceability

- **Date**: 2026-07-07
- **Status**: research
- **Related specs**: `docs/specs/model_properties.md` (§Surface rows "Event-time monotonicity trace", "Column nullability gate", "Driving-fact / anchor resolution", "Set-operation distribution", "Static-seed detection"; §Semantics "Event-time monotonicity trace"), `docs/specs/model_maintenance.md` (§"The equivalence invariant", §"Windowed maintenance and the horizon"), `docs/specs/model_transforms.md` (pushdown / clamps / UNION-branch rows), `docs/specs/timeseries.md` (`assert_monotonic`)
- **Related code**: `crates/smelt-logical/src/analysis/monotonicity.rs` (the pure trace), `crates/smelt-logical/src/analysis/source_bounds.rs` (`Offset`, `BoundContext`, `resolve_single_anchor`, `resolve_join_driving_fact`), `crates/smelt-db/src/queries/monotonicity.rs` (nullability gate, `trace_event_time_checked`)
- **Prior research** (build-on, not repeated here): `docs/research/20260701-monotonicity-primitive-research/` — the primitive deep-dive (Part 6: definition, whitelist, declared guarantee), pushdown soundness laws + ClickHouse/Iceberg/Delta analogs, IVM/decidability theory. Empirical cells: `docs/research/property-discovery/catalog.md` (SC-1, G-06, G-09).

---

## 1. The property

**Scope: per-column.** The property is stated of a single projected column expression — specifically the expression the model projects into its `event_time` slot — never of the model as a whole. Any column expression can be asked the same question; the `event_time` slot is where the answer is load-bearing.

**Definition.** Let source `S` carry a clock/partition column `p` (its `timeseries.partition_column`), and let the model project `event_time` through expression `e`. The property holds when `e` is, row-by-row, a **monotone non-decreasing, total, source-traceable image of exactly one source column**: there is a function `f` such that `e = f(p)` with

> `p₁ ≤ p₂ ⟹ f(p₁) ≤ f(p₂)`   (monotone non-decreasing in the *value order* of `p`).

The operationally equivalent formulation (from the 20260701 deep-dive §6.1) is **interval-preimage-is-an-interval**: for every window `[lo, hi)` on `e`, `{rows : f(p) ∈ [lo, hi)}` = `{rows : p ∈ [a, b)}` for some thresholds. This is what makes a filter on the *output* expression relocatable to a filter on the *source* column — the entire point of the proof.

Two refinements the naive "monotone" gloss misses, both of which the analysis below leans on:

1. **Non-decreasing suffices; strictness is tracked, not required.** `DATE_TRUNC` and `CAST(… AS DATE)` are many-to-one plateaus yet push cleanly because window boundaries are grid-aligned; the verdict records this as `is_strict = false` plus a `grid_unit`.
2. **Monotonicity alone is not the whole licence — the offset must be constant (bounded).** The bound derivation assumes the preimage of `[lo, hi)` is `[lo − δ, hi − δ)` for a *constant* shift `δ`. `GREATEST(ts, c)` is genuinely monotone non-decreasing but its pointwise shift `f(p) − p = max(0, c − p)` is unbounded — see §4. `Traceable` therefore certifies *monotone + constant offset*, not bare monotonicity.

**Three distinct "monotone"s — do not conflate.** (i) **Value monotonicity of the projection function** `f` — what this property is. (ii) **Arrival monotonicity of the source** (rows arrive in roughly clock order; watermarks/lateness) — a *world-fact* on the source (`sources.md`), not provable from SQL. (iii) **Evolution monotonicity of an aggregate value** (`MAX(ts)` per key only ever grows as rows arrive) — the *value-monotone discriminant* (`model_properties.md` §"Algebraic discriminants"), a property of a combiner, not of a projection. §4's GROUP BY entry shows why (iii) must not be smuggled into (i).

**Verdict domain** (`EventTimeTrace`, `monotonicity.rs`):

- `Traceable { source, source_column, offset, monotonicity }` — `e` is a monotone non-decreasing image of exactly one source column, with constant shift `offset` (`Seconds` where uniform, `Symbolic` for calendar-variable month/year, `Integer` for non-temporal monotone keys) and a ClickHouse-shaped `Monotonicity { is_monotonic, is_positive, is_always_monotonic, is_strict, grid_unit }`.
- `StaticSeed { reason }` — a constant or `NULL` in the slot (or `COALESCE(col, const)`): not a stream at all. One seed row breaks incremental ≡ full (a constant lands in one partition forever; `NULL` never passes `e ≥ start`).
- `NotTraceable { reason, kind }` with `kind ∈ {Disproven, Undecidable}` — `Disproven`: the classifier *positively* knows the shape is not a monotone constant-shift chain (periodic function, piecewise `CASE`, two-column arithmetic, non-temporal cast, nondeterministic function, unresolvable/ambiguous leaf). `Undecidable`: no rule for the shape (an opaque UDF). The distinction is load-bearing: `timeseries.assert_monotonic` may widen **only** `Undecidable` (§6).

**The nullability gate** is a fourth judgment layered above the pure trace: `Traceable` is downgraded to `NotTraceable(Disproven)` when the traced leaf column is nullable *or its nullability is unknown* (`gate_nullable_leaf`, smelt-db). Rationale: a pushed `WHERE p ≥ start` silently drops `p IS NULL` rows that a full refresh keeps. The gate only ever narrows — sound by construction.

**Fail-closed contract.** The primitive may return `NotTraceable` for a form that is in fact safe (missed optimisation — consumer stays at the outer clamp), but must never return `Traceable` for a form that is not monotone-constant-shift-traceable (unsound pushed filter). Absence of a proof is a rejection.

## 2. Why maintenance needs it

The trace is the licensing property for every transform that relocates or bounds a time filter, and the disambiguator for every multi-input scan (`model_transforms.md` rows in parentheses):

- **Source-filter pushdown** ("Source-filter pushdown (window-an-input)"): the trace *is* the licence to rewrite the output-window filter as a source-scan filter on `source.source_column`, folding `offset` into the bound. Without it, only the outer clamp is safe.
- **Two-layer widened-scan + exact output clamp**: the scan reads `[start − k − offset, end)`; the `offset` term is the trace's constant shift. The **outer output-clamp itself needs no trace** — it filters the projected `e` verbatim; the trace is what the *pushdown* half needs. This asymmetry is why the fail-closed fallback ("stay at the outer clamp") is always available.
- **Partition DELETE+INSERT** (batched): the write address is the projected event-time's partition; the trace's `grid_unit` feeds the run-window/partition-granularity alignment check, and the `StaticSeed` verdict is what catches a constant/`NULL` clock before it corrupts partition addressing.
- **Driving-fact / anchor resolution** (`model_properties.md`): among joined inputs, the trace is re-run per alias-scoped input; **exactly one** `Traceable` input is the anchor whose scan is windowed (all others full-scanned). Zero or ≥2 fail closed. The windowed-keyed-maintenance driver and the batched step loop both consume this.
- **UNION-branch wrap-and-filter** (unbuilt): per-branch traces license injecting the filter independently into each set-operation branch; a `StaticSeed` branch is named and rejected rather than silently dropped.
- **Horizon / reach derivation** (`model_maintenance.md` §"Windowed maintenance and the horizon"): the derived clamp composes the reach with the trace's offset; a `Symbolic` offset in a bound-relevant position forces `NotDerivable` rather than an approximate guess.
- **Equivalence invariant** (`model_maintenance.md`): every one of the above is licensed *because* it preserves `incremental_state(S) == full_refresh(source | input ∈ S)`. The trace is the proof that windowing an input selects exactly the processed-input set the invariant quantifies over. Empirically, the `(0,0)` fallback that clamps a late conversion when the bound is *not* honestly derived is catalog cell SC-1 — the negative image of this property.

## 3–4. Per-construct analysis

Ground context for all examples: source `events(event_ts TIMESTAMP NOT NULL, …)` with `timeseries.partition_column = event_ts`; dimension `users(user_id, signup_ts, updated_at)`. "Today" columns state what `monotonicity.rs` actually does.

### Scalar arithmetic

**`col ± INTERVAL const` — holds (strict, offset folds).**
```sql
SELECT event_ts + INTERVAL '1 day' AS event_time FROM events
```
Strictly monotone; `offset = Seconds(86400)`. Month/year intervals stay monotone but the offset is `Symbolic('1 month')` (28–31 days) — traceable, yet a bound-relevant symbolic offset forces `NotDerivable` downstream (fail-closed, no ≈30d guess). `const − col` reverses direction and is `Disproven` (`5 - batch_id` is strictly *decreasing*); `col * n` and `col % n` are `Disproven` by name (not a constant shift; periodic). All implemented.

**Two-column arithmetic — breaks (`Disproven`).**
```sql
SELECT end_ts - start_ts AS event_time FROM sessions   -- duration, not a clock
```
Counterexample: rows A `(start '2026-01-01', end '2026-01-05')` → 4 days, B `(start '2026-01-04', end '2026-01-05')` → 1 day. Ordered by `end_ts`, A and B tie; the expression differs — not a function of either column alone, so no single-column preimage exists. Implemented (`Disproven`).

### Casts

**Temporal target — holds.** `CAST(event_ts AS DATE)` is monotone many-to-one: `is_strict = false`, `grid_unit = Day`. `TIMESTAMP`/`TIMESTAMPTZ`/`DATETIME` targets preserve the child's strictness.

**Non-temporal target — breaks (`Disproven`).**
```sql
SELECT CAST(seq_id AS VARCHAR) AS event_time FROM batches
```
Counterexample: `9 < 10` numerically but `'9' > '10'` lexically — lexical order agrees with value order only for fixed-width forms. Blanket-rejecting non-temporal targets is the right whitelist posture (ISO-8601 text *happens* to sort correctly, but proving the format is out of scope). Implemented.

### CASE / piecewise

**Breaks in general (`Disproven`, all CASE).**
```sql
SELECT CASE WHEN priority = 1 THEN event_ts
            ELSE event_ts - INTERVAL '30 days' END AS event_time
FROM events
```
Counterexample: row A `(event_ts '2026-06-30', priority 1)` → `2026-06-30`; row B `(event_ts '2026-07-01', priority 2)` → `2026-06-01`. `p_A < p_B` but `e_A > e_B`. A window `['2026-06-01','2026-06-02')` on `e` has preimage `{p = '2026-07-01' ∧ priority≠1} ∪ …` — not an interval in `p`.

Note the classification tension: *some* CASEs are monotone (`CASE WHEN event_ts < DATE '2026-01-01' THEN event_ts ELSE event_ts + INTERVAL '1 hour' END` is monotone non-decreasing — each branch is monotone and the pieces are ordered at the boundary). Deciding piecewise monotonicity requires comparing branch images at every predicate boundary; the classifier instead positively rejects the whole shape as `Disproven` — which also makes it **un-widenable by declaration**. That is a deliberate hard line (a piecewise clock is nearly always a modelling smell), but it is stricter than "no rule for this shape" — see Open Question 4.

### COALESCE

**`COALESCE(col, const)` — `StaticSeed`.**
```sql
SELECT COALESCE(event_ts, TIMESTAMP '1970-01-01') AS event_time FROM events
```
Every `NULL` row is stamped with the epoch constant — those rows land in one 1970 partition forever; an incremental run maintaining recent windows never revisits it, a full refresh recomputes it. This is the P3 seed hazard *in function form*, which is why the verdict is `StaticSeed` (never widenable) rather than `NotTraceable`. Nuance: if the leaf is provably `NOT NULL`, `COALESCE(event_ts, c)` is pointwise identical to `event_ts` and genuinely traceable — but nullability is invisible at the pure-trace layer (`smelt-logical` sits below the schema), and the gate architecture only *downgrades*, never upgrades, so the shape is refused unconditionally. Over-conservative by design; implemented.

### GREATEST / LEAST

**Breaks — but for a sharper reason than the code comment states.**
```sql
SELECT GREATEST(event_ts, TIMESTAMP '2026-01-01') AS event_time FROM events
```
`max(p, c)` **is** monotone non-decreasing in `p` — the disproof is not non-monotonicity. The real failure is the **unbounded offset**: `f(p) − p = max(0, c − p)` grows without bound as `p` recedes. Concretely: rows `p = '2019-03-03'` and `p = '2020-05-05'` both map to `e = '2026-01-01'`; the window `['2026-01-01','2026-01-02')` on `e` has preimage `(−∞, '2026-01-02')` on `p` — an interval, but a *half-infinite* one. Any pushed filter `p ≥ '2026-01-01'` drops both rows; the honest scan for that one window is all history. So the licensing property is "monotone **and** constant shift", and `GREATEST(col, const)` fails the second conjunct. `GREATEST(ts1, ts2)` of two monotone columns additionally fails single-column traceability (two leaves — the join multi-clock case in expression form). Both `Disproven`; implemented (message says "plateau can straddle a window boundary", which is the same fact seen from the window side).

`LEAST(event_ts, c)` is symmetric: unbounded *forward* preimage (`e ∈ [lo,hi)` with `hi > c` pulls in all `p > c` — every future row lands at `c`).

### DATE_TRUNC / EXTRACT — periodic vs monotone parts

**`DATE_TRUNC(unit, col)` / `DATE_BIN` / `TIME_BUCKET` — holds**, non-strict, `grid_unit` recorded from the unit literal. Implemented.

**`EXTRACT(part FROM col)` — periodic parts break; monotone parts are over-rejected today.**
```sql
SELECT EXTRACT(HOUR FROM event_ts) AS event_time FROM events   -- periodic: Disproven, correctly
```
Counterexample: `'2026-07-01 23:00'` → 23, `'2026-07-02 01:00'` → 1. The preimage of `[22, 24)` is a union of one interval *per day* — never a single interval. Same for `DOW`, `MONTH`-of-year, `MOD`. But `EXTRACT(EPOCH FROM ts)` is *strictly* monotone (the identity in another unit) and `EXTRACT(YEAR FROM ts)` is monotone non-strict with `grid_unit = Year` — both are genuinely traceable, yet today's classifier blanket-`Disproven`s every `EXTRACT` (and `Disproven` is declaration-proof, so `assert_monotonic` cannot rescue them either). Sound but incomplete; see §7.

### Opaque functions / UDFs

**`Undecidable` — the one declaration-widenable verdict.**
```sql
SELECT my_parse_ts(raw_payload_ts) AS event_time FROM events
```
No rule → `NotTraceable(Undecidable)`. Under `timeseries.assert_monotonic`, the trace recurses into the function's *single* column-bearing argument and admits with `is_strict = false` (the opaque body's own shape is unproven). Zero or ≥2 column-bearing arguments stays `Undecidable` even when declared — the declaration cannot conjure a leaf. Row-nondeterministic names (`RANDOM`, `UUID`, …) are carved out *before* the unknown-function arm and `Disproven` — a fresh-per-row value in the event-time slot is a positive hazard, not an unknown. Run-deterministic clocks (`NOW()`, `CURRENT_DATE`) are `Disproven` too: constant within a run but shifting *between* runs — not source-traceable (each run would re-address old rows). All implemented.

### WHERE filters

**Holds — selection never breaks the trace.** `σ_pred` removes rows; it changes no row's value, so `e = f(p)` holds on the surviving subset and the preimage property is inherited (a sub-interval of an interval restricted by an independent predicate is still exactly `σ_pred` applied after the window — the classical σ-commutation law). A `WHERE` on the event-time itself contributes to *bound* derivation (a `ts >= x - INTERVAL …` shift is reach), not to the trace. No counterexample exists at this operator; that is the point.

### Joins — anchor resolution

**Conditional: exactly one alias-scoped `Traceable` input, on a null-safe side.**
```sql
SELECT f.event_ts AS event_time, f.amount, u.plan
FROM events f JOIN users u ON f.user_id = u.user_id
```
The trace is re-run against each join input's own `BoundContext` (alias-scoped via `find_leaf_column_ref`'s qualifier + `resolve_single_anchor`); here `f` is `Traceable`, `u` is not → `f` is the anchor: window `f`'s scan, full-scan `u`. **Why exactly-one**: two `Traceable` inputs is the multi-clock hazard — with `SELECT GREATEST(f.event_ts, u.updated_at)` (or an `event_time` from whichever side "wins"), windowing `f` misses a row whose `u.updated_at` moved it into the window; windowing both misses join partners outside either window. Zero `Traceable` inputs (a dimension-side clock) leaves nothing to window. Implemented (`resolve_join_driving_fact`, fail-closed both ways).

**LEFT/FULL join — the null-supplying side breaks the gate's assumptions.**
```sql
SELECT o.order_ts AS order_time, s.shipped_ts AS event_time
FROM orders o LEFT JOIN shipments s ON o.order_id = s.order_id
```
Even with `shipments.shipped_ts NOT NULL` *declared*, an unmatched order row emits `event_time = NULL`. A full refresh keeps that row; the outer clamp `event_time ≥ start` drops it, and a pushed filter on `s.shipped_ts` prunes it out of the join altogether — incremental ≠ full by exactly the stranded-NULL row (catalog cell **G-06**: "HOLDS for recompute; fold strands the NULL row"). The pure trace cannot see join shape; the nullability gate today resolves the leaf's *declared/inferred* nullability at the source — it does not know the leaf sits on the null-supplying side of an outer join. Nullability is itself a property that **propagates through operators** (left join nullifies the right side; full join nullifies both; inner join with an equality on the column implies not-null on output), and the gate is only sound when fed the *post-join* nullability. This is a real gap (§7, item 4). Cross join: the trace itself composes (one side's clock is still per-row monotone) but a cross join has no `ON` — the fan-out proof fails closed to `OneToMany`, and anchor resolution still requires the exactly-one rule; the trace is necessary, not sufficient.

### GROUP BY with MIN(ts) / MAX(ts) as the projected event_time

**Not this property — a different monotonicity.**
```sql
SELECT user_id, MIN(event_ts) AS event_time, COUNT(*) AS n
FROM events GROUP BY user_id
```
`MIN(event_ts)` is a function of the group's *row-set*, not of any single source row — the per-row trace is category-mismatched here. In what sense is it monotone? **Evolution-monotone** (sense (iii) of §1): under insertion, per-key `MAX(ts)` only ever grows and `MIN(ts)` only ever shrinks (value-monotone / anti-monotone discriminants). Neither gives interval-preimage traceability:

- *Pushdown counterexample*: with rows `('u1', '2026-01-01')` and `('u1', '2026-07-01')`, full refresh gives `(u1, event_time = '2026-01-01', n = 2)`. A scan windowed to `['2026-07-01', …)` computes `(u1, '2026-07-01', n = 1)` — wrong address *and* wrong payload. Filtering output on `MIN(ts) ≥ start` is not scanning `ts ≥ start`.
- *Address instability*: as event_time, `MAX(ts)` migrates a group's output row **forward** every time the key recurs (the old partition retains a stale copy under partition-addressed DELETE+INSERT); `MIN(ts)` is stable under in-order arrival but migrates **backward** on a late row. Both violate the partition-addressed assumption that a written row's address is final.
- *What is sound instead*: keyed maintenance — `MIN`/`MAX` are idempotent monoids, so `merge_into` folding `min(stored, delta_min)` is exactly right (the ladder, rung 1). The aggregate event-time question belongs to the value-monotone discriminant + key-temporal-locality machinery, not to the trace.

**Today's classifier gets this wrong in a subtle way**: `MIN`/`MAX` are not in any match arm, so they fall to the *unknown-function* arm → `Undecidable` — which `assert_monotonic` is permitted to widen, recursing into `event_ts` and certifying a per-row trace for an aggregate expression. In group context that licence is unsound (the counterexample above). Aggregate names should be `Disproven` at the trace layer (the typed `SqlFunction::is_aggregate` classifier already exists). Sharpest gap found; §7 item 1.

### Window functions

**Breaks (relation-dependent value).**
```sql
SELECT LAG(event_ts) OVER (ORDER BY event_ts) AS event_time FROM events
```
`LAG(ts)`'s value at the window's left edge depends on rows *outside* any restricted scan: under full evaluation the first in-window row sees the last pre-window row's `ts`; under a windowed scan it sees `NULL`. Same for `FIRST_VALUE(ts) OVER (PARTITION BY user_id ORDER BY ts)` — a per-key constant equal to the key's first-ever `ts`, which a window that misses the key's birth row computes incorrectly. These are *reach* problems, correctly owned by the frame-reach/bound derivation (`RANGE … INTERVAL` → finite `k`, widened scan; bare `LAG` → `NotDerivable`), never by the trace. Today the `OVER` shape falls through to the unrecognised-head/unknown-function rejection — fail-closed, correct.

### DISTINCT

**Holds (value-neutral).** `SELECT DISTINCT date_trunc('day', event_ts) AS event_time FROM events` — dedup removes rows, changes no values; σ commutes with DISTINCT. The trace passes through. (Whether *other* projected columns survive dedup grain changes is `PartitionAlignment`'s question, not the trace's.)

### UNION ALL / UNION / INTERSECT / EXCEPT

**Branch-wise conditional.** Each branch is traced independently against *its own* sources — there is no exactly-one-anchor rule here (contrast joins): every branch is its own anchor.

```sql
SELECT event_ts AS event_time, amount FROM web_orders
UNION ALL
SELECT settled_ts + INTERVAL '1 day' AS event_time, amount FROM batch_orders
```
Both branches `Traceable` (to different sources, different offsets) → the *UNION-branch wrap-and-filter* transform injects the filter per branch with per-branch offsets; the model then windows two sources (the multi-source bound derivation is a separate, per-source computation — catalog cell **G-09** confirms bound derivation composes across `UNION ALL` arms). Note the composed verdict is not a single `Traceable{source, column, offset}` — it is a *vector* of per-branch verdicts (§5). Same source with different offsets is the same story in miniature: two branch filters with two folded offsets, no single model-level offset.

A `StaticSeed` branch is the hazard the per-branch trace exists to name:
```sql
SELECT event_ts AS event_time FROM events
UNION ALL
SELECT TIMESTAMP '2026-01-01' AS event_time FROM manual_adjustments   -- seed branch: refuse
```
Filter distribution itself: `σ(A ∪ B) = σ(A) ∪ σ(B)` holds for `UNION ALL`, `UNION` (dedup commutes with a value predicate), `INTERSECT` (`σ(A ∩ B) = σ(A) ∩ B` — one side suffices), and `EXCEPT` (`σ(A − B) = σ(A) − σ(B)`; pushing into *only* `B` is wrong — `σ(A) − σ(B)` needs the `A` side filtered, and dropping the `B`-side filter is safe for `EXCEPT` but changes nothing). The per-branch *expressions* differ by position, so the trace must run per branch against the branch's projection. Today: set-operation distribution is classified for `UNION ALL` only; `INTERSECT`/`EXCEPT` unclassified (fail-closed), transform unbuilt.

### ORDER BY / LIMIT

**ORDER BY alone: holds** (no value or membership change for a set/bag result). **LIMIT: breaks — non-local row membership.**
```sql
SELECT event_ts AS event_time FROM events ORDER BY event_ts DESC LIMIT 10
```
Which 10 rows survive depends on the *whole relation*: over full history it is the global top-10; over a windowed scan it is the window's top-10 — different sets whenever ≥1 of the global top-10 lies outside the window. σ does not commute with LIMIT in either order. No trace verdict can license a push; the construct must be refused at admission (today: the uppercase-substring `LIMIT` admission scan in `incremental.rs`, fail-closed).

### Subqueries and CTE nesting

**Conditional — composes by re-projection (§5 owns the rule).**
```sql
WITH hourly AS (
  SELECT date_trunc('hour', event_ts) AS bucket_ts, amount FROM events
)
SELECT bucket_ts + INTERVAL '1 hour' AS event_time, amount FROM hourly
```
Inner: `bucket_ts = f₁(event_ts)`, monotone non-strict, grid Hour. Outer: `e = f₂(bucket_ts)`, strict shift +1h. Composition `f₂∘f₁` is monotone non-decreasing (closure under composition), offset = 0 + 1h, `is_strict = strict₁ ∧ strict₂ = false`, grid = outermost grid layer — `Traceable(events.event_ts, +1h)`. The side-conditions that make this valid are exactly where CTEs go wrong: the inner body must be *transparent* for the traced column (no aggregate over it, no `LIMIT`, no set-op re-anchoring), and name resolution through the re-projection must be unambiguous. **Today the trace does none of this**: it walks a single expression and resolves the leaf *by name* against `ctx.source_partition_cols` — `bucket_ts` matches no source partition column → `NotTraceable(Disproven, "leaf does not match")`. Through-CTE composition is analysed in §5 and is unimplemented (§7 item 2). The body-structure classifier (`SelectItemKind`) is the existing per-body transparency check the composed walk would consume.

### Self-referencing models

A model reading its own prior output (`smelt.ref` to itself) has a self-edge; its `event_time` may trace to its *own* output column. The trace is then inductive: sound iff the column was `Traceable` at the model's seed and every run preserves it — which holds trivially because the model writes the very expression being traced. What actually gates the construct is not the trace but *window-independence/ordered-execution* (the self-read must be backward-bounded; forward/unbounded self-reads are refused) — the trace contributes the clock the ordering steps along. The outer output-clamp's wrapping projection exists partly for this case (the clamp column must bind to the output schema, not to an ambiguous inner alias when the model's own name appears in FROM).

### Constants / VALUES — StaticSeed

```sql
SELECT TIMESTAMP '2026-01-01' AS event_time, 'seed' AS kind
-- or: FROM (VALUES ('2026-01-01'::TIMESTAMP, 'a'), ('2026-01-02'::TIMESTAMP, 'b')) v(event_time, kind)
```
A literal in the slot is `StaticSeed` — a fixed table, not a stream. Correct treatment is "this is not a partitionable input", distinct from a real low-volume stream (which still traces to a genuine clock and is fine). `StaticSeed` is never widened by declaration. A `VALUES`-sourced column is not a *constant expression* (rows differ), but there is no source clock in `ctx` for it to resolve to → `NotTraceable` today; morally it is seed-like (finite, never grows by clock) — a classification refinement worth a line in the spec someday, not a soundness issue (both verdicts refuse).

## 5. Composition algebra

Two layers compose differently, and being precise about the split is most of the design.

### 5.1 Expression layer — fully compositional (implemented)

Judgments have the form `⊢ e ⇒ V` where `V ∈ {Trace(col, δ, m), Seed, NT(kind)}`. The implemented rules, written as inference rules:

```
──────────────────────────── (COL)
⊢ col ⇒ Trace(col, 0, strict)

⊢ e ⇒ Trace(c, δ, m)    g ∈ {DATE_TRUNC u, DATE_BIN u, TIME_BUCKET u, FLOOR, CAST→DATE}
────────────────────────────────────────────────────────────────── (GRID)
⊢ g(e) ⇒ Trace(c, δ, m ⊓ nonstrict, grid := u)      -- outermost grid wins

⊢ e ⇒ Trace(c, δ, m)    k a constant INTERVAL/integer literal
──────────────────────────────────────────────── (SHIFT)
⊢ e ± k ⇒ Trace(c, δ ⊕ k, m)        -- ⊕: Seconds+Seconds fold; month/year ⇒ Symbolic

⊢ e ⇒ Trace(c, δ, m)    f unknown, single column-bearing arg, declared_monotonic
────────────────────────────────────────────────────────── (DECL)
⊢ f(…e…) ⇒ Trace(c, δ, m ⊓ nonstrict)

everything else ⇒ Seed (literal/NULL/COALESCE-const) or NT(Disproven/Undecidable)
```

Soundness rests on two closure facts: **(a)** composition of monotone non-decreasing functions is monotone non-decreasing, and **(b)** constant shifts compose additively and commute with grid truncation *up to the grid unit* (which is why the widened scan carries `− offset` explicitly). `Seed` and `NT` are **absorbing**: any layer over them stays refused (one seed sub-expression poisons the chain). Strictness is a meet (`∧`); `grid_unit` is *last-writer-wins* (outermost governs the final grid). This layer is a straightforward bottom-up fold over the AST — the whole-expression verdict is computable from sub-expression verdicts with no side channel. **Fully compositional.**

One deliberate loss: `Offset::Seconds` is an unsigned magnitude — `+1d` and `−1d` fold identically; direction is recoverable from the AST but not carried in the verdict. Fine for "is this a constant shift", lossy for a signed bound consumer (§7 item 6).

### 5.2 Relational layer — compositional per-operator, with side-conditions and two genuine breaks

Judgments lift to relations: `⊢ R ⇒ V(R.et)` — the verdict for the event-time column *as projected by* `R`. The composition question: given verdicts for operand relations, what is the verdict of the composed query?

| Operator | Composed verdict | Side-conditions | Status today |
|---|---|---|---|
| `σ_pred(R)` (WHERE) | `V(R)` unchanged | none — selection is value-neutral | implicit (trace ignores WHERE) |
| `π` / SELECT re-projection, CTE/subquery stacking | `Trace(c, δ₁⊕δ₂, m₁⊓m₂)` when inner ⇒ `Trace(c, δ₁, m₁)` and the outer expression over the inner's output column ⇒ `Trace(inner_col, δ₂, m₂)` by §5.1 | inner body **transparent** for the traced column (`SelectItemKind`: no aggregate over it, no LIMIT, not re-anchored by a set op); unambiguous name resolution through the alias | **unimplemented** — leaf matched by name against source partition cols only |
| `R ⋈ S` (inner) | `V(anchor)` | **exactly one** of the alias-scoped inputs `Traceable` (anchor); others full-scanned | built (`resolve_join_driving_fact`) |
| `R ⟕ S` (left/right/full outer) | `V(anchor)` gated | anchor must sit on a **null-preserving side**; a null-supplied anchor ⇒ nullability-gate downgrade — requires *post-join* nullability, not source-declared | **gap** — gate reads source nullability only (G-06) |
| `R × S` (cross) | `V(anchor)` for the trace itself | trace composes; fan-out fails closed separately (`OneToMany`) — trace necessary, not sufficient for the write licence | built (trace) / built (fan-out refusal) |
| `UNION ALL` | **vector** `⟨V(B₁), …, V(Bₙ)⟩`; licence = ∀i `V(Bᵢ)` is `Traceable`; any `Seed`/`NT` branch refuses the whole | per-branch trace against the branch's own sources; branches may anchor to *different* sources and carry *different* offsets — no single-verdict reduction exists in general | classified (UNION ALL); transform unbuilt |
| `UNION` (distinct) | as UNION ALL | dedup commutes with a value predicate; same vector shape | unclassified (fail-closed) |
| `INTERSECT` / `EXCEPT` | as UNION ALL, with `EXCEPT` requiring the filter on the **left** arm (pushing only into `B` is unsound; `σ(A−B) = σ(A) − σ(B)`) | positional column correspondence across arms | unclassified (fail-closed) |
| `GROUP BY k…` — `event_time ∈ k` and `Traceable` | composes; grid alignment against partition grain checked separately | `PartitionAlignment` | built (alignment proof) |
| `GROUP BY` — `event_time = MIN/MAX(ts)` | **NT** at this layer — property changes *kind* (per-row trace → evolution-monotone discriminant; keyed-mode territory) | never reducible to a per-row trace (counterexample §4) | **mis-verdict today**: falls to `Undecidable` (widenable) — should be `Disproven` |
| aggregate-over-aggregate | outer key traced through inner key: composes as two π-steps (grid coarsens outward, e.g. day over hour). Outer `MAX` over inner `MAX`: associative *as a combiner* (regroup law) but never a trace | same transparency conditions per layer | unimplemented (falls out of the π rule) |
| window fn over `R` | `NT` for the windowed expression itself; a `Traceable` event_time *alongside* a window fn survives, with reach `k` owed to bound derivation | finite `RANGE` frame ⇒ widened scan; bare `LAG`/`ROWS` ⇒ `NotDerivable` | built (frame-reach, separate proof) |
| `DISTINCT` | `V(R)` unchanged | event_time among the distinct columns | built (alignment scan) |
| `ORDER BY` | `V(R)` unchanged | — | n/a |
| `LIMIT` | **refuse** — row membership is a whole-relation fact; σ never commutes | none exist | admission scan (text-based), fail-closed |

**Is the property compositional?** At the expression layer: yes, unconditionally — a bottom-up fold. At the relational layer: **yes per-operator with side-conditions, except at three points**:

1. **LIMIT** (and any TOP-N / QUALIFY-rank construct): the verdict of the whole is not a function of operand verdicts at all — membership is non-local. Only refusal is available.
2. **Aggregation over the clock**: the composed "verdict" is not in this property's domain — the question changes from per-row traceability to combiner evolution-monotonicity. A composition algebra that pretended `V(GROUP BY, MAX(ts))` reduces to `V(ts)` would be *wrong*, not merely incomplete (§4's counterexample). The algebra must return "different property" here, which `NT(Disproven)` encodes operationally.
3. **Nullability**: the gate's input is not compositional bottom-up from source declarations — it must be computed by an operator-level nullability propagation (outer joins nullify sides; `ON`-equality and `WHERE col IS NOT NULL` de-nullify; `COALESCE` de-nullifies) that mirrors the trace walk. Without it the gate is sound only for single-relation queries.

And one point where the verdict is compositional but its *shape* changes: set operations yield a per-branch **vector**, reducible to a single `Traceable` only in the degenerate all-branches-same-`(source, column, offset)` case. Consumers (pushdown) are naturally per-branch, so the vector is the honest type; forcing a scalar verdict would either over-refuse or lie.

**Stacked-CTE reduction, stated precisely.** `Traceable(cte2.ts)` reduces to `Traceable(source.ts)` iff each intervening layer is (a) a SELECT whose projection of the traced column is a §5.1-monotone chain over the layer below's traced column, (b) transparent for that column (no aggregation over it, no LIMIT, no set-op whose branches disagree), and (c) name-unambiguous. Offsets add along the stack; strictness meets; grid is the outermost layer's. The proof obligation is a chain of π-rule applications — exactly the "operator-by-operator pushdown walk" the 20260701 doc's Part 4 frames, and the classical selection-pushdown law set restated (Garcia-Molina/Ullman/Widom §16.2, per the pushdown research file).

## 6. Static provability vs declaration

**What is decidable.** General monotonicity of expressions is undecidable (Richardson's theorem — see `research-pushdown-and-monotone-expressions.md` Area 5), so the classifier is a **fixed whitelist**, never a prover: identity, grid truncations, temporal casts, constant shifts, and their compositions. Everything production engines do here is the same shape (ClickHouse `getMonotonicityForRange`, Iceberg `preserves_order` partition transforms, Delta generated-column filters). The whitelist is deliberately the *intersection* across target backends — eligibility must be a property of the plan, not of the engine.

**The three-way verdict split is the declaration interface.** `Disproven` = positive structural knowledge (periodic, piecewise, two-column, seed-adjacent, nondeterministic, unresolvable) — refused *regardless* of any declaration. `Undecidable` = no rule (opaque function) — the **only** verdict `timeseries.assert_monotonic` may widen, and even then only by resolving a single column-bearing argument (weakened strictness); it cannot conjure a leaf, cannot rescue a `StaticSeed`, cannot override a disproof. This is the "declared escape hatches may only widen" constraint (`model_properties.md` §Constraints) applied per-verdict-kind, and it is what makes trusting a declaration *for correctness* (higher stakes than the `joins:` cardinality precedent, which is trusted only for optimisation) tolerable: the declaration's blast radius is exactly the shapes the classifier admits to knowing nothing about.

**Fail-closed defaults.** Every unmatched shape → `NotTraceable`; ambiguous or unresolvable leaf → `Disproven`; unknown nullability → gate downgrade; symbolic offset in a bound position → `NotDerivable`. The one-directional soundness contract: false negatives cost a missed pushdown (outer clamp remains available and needs no proof); a false positive is a silently wrong table.

**The nullability gate in composition.** The gate is the reminder that the trace certifies a *value* property while the pushed filter is a *membership* operation — `NULL` is where they diverge (`NULL ≥ start` is not true; the row vanishes). Two composition facts follow. First, nullability must be propagated through the same operator tree the trace walks: a `NOT NULL` source column is nullable after sitting on the null-supplied side of an outer join (G-06's stranded row), and conversely a nullable column is effectively non-null below `WHERE col IS NOT NULL` or above `COALESCE` — the gate fed source-declared nullability is over-strict in the second case and, worse, **under-strict in the first**. Second, the gate is the model for how *any* schema-layer fact joins the pure trace: the pure verdict is computed below the schema (smelt-logical), and a thin smelt-db wrapper narrows it (`trace_event_time_checked`) — downgrade-only, so layering errors can only lose optimisations, never soundness. Any future post-join nullability computation should keep that shape.

## 7. Implementation gaps

Where this doc's analysis exceeds `monotonicity.rs` + `queries/monotonicity.rs` today, most severe first:

1. **Aggregate names reach the declaration-widenable arm.** `MIN(ts)`/`MAX(ts)` (any aggregate) fall through to the unknown-function arm → `Undecidable`; under `assert_monotonic` the trace recurses into `ts` and certifies a per-row trace for an aggregate expression — unsound in GROUP BY context (§4 counterexample: windowed scan miscomputes both the address and co-projected aggregates). Fix is small: consult `SqlFunction::is_aggregate` in `classify_function` and return `Disproven` ("aggregate event-time is a combiner property, not a trace"). Severity is bounded by consumers (the cumulative/keyed classifiers own the aggregate path and don't route `MAX(ts)` through the trace), but the primitive's own contract — `Undecidable` means *safe to widen* — is violated at this arm.
2. **No relational composition.** The trace walks one expression and resolves the leaf by bare name against `ctx.source_partition_cols`. §5.2's π-rule (through-CTE/subquery re-projection), the per-branch set-op vector, and aggregate-over-aggregate stacking are all unimplemented; a two-layer CTE with a renamed clock column is `Disproven("leaf does not match")` today even when §5.2 licenses it. This is the known "trace composes through re-projection" frontier the 20260701 doc's §4.6/§2.5 anticipated; `SelectItemKind` and `resolve_join_driving_fact` are the existing pieces a composed walk would assemble.
3. **Leaf resolution is name-based, not scope-based.** Cross-source column-name collisions are `Disproven(ambiguous)` (sound), but a *same-name-different-column* coincidence inside one source scope can only be disambiguated by the alias machinery at join sites; the bare trace has no FROM-clause resolution at all (acknowledged in the module docs). Fail-closed today; over-rejects.
4. **Nullability gate is join-blind.** `resolve_leaf_nullability` reads the source's declared/inferred column nullability; it cannot see that the leaf sits on the null-supplied side of an outer join (under-strict — the G-06 stranded-NULL hazard passes the gate if the column is declared `NOT NULL`), nor that a `WHERE ts IS NOT NULL` or an `ON`-equality de-nullifies (over-strict). Needs operator-level nullability propagation (§6).
5. **EXTRACT blanket disproof over-rejects monotone parts.** `EXTRACT(EPOCH …)` (strict) and `EXTRACT(YEAR …)` (non-strict, grid Year) are genuinely traceable but `Disproven` — and `Disproven` is declaration-proof, so there is no escape hatch either. Whitelist the monotone parts explicitly.
6. **Offset direction is not carried.** `Seconds` is an unsigned magnitude; `+1d` vs `−1d` verdicts are identical (documented judgment call). A signed consumer (asymmetric before/after reach; the widened scan's `− offset` term) must re-read the AST. Carry a sign.
7. **`Monotonicity` struct is degenerate.** `is_positive`/`is_always_monotonic` are always `true` and `is_monotonic` never `false` on a `Traceable` — decreasing chains are rejected rather than tracked. Harmless (the enum, not the struct, carries the decision) but the ClickHouse shape over-promises; either populate it (admit decreasing chains as `Traceable` with `is_positive = false` for consumers that can flip bounds) or shrink it.
8. **Smaller items.** `AT TIME ZONE` unparsed (fail-closed via unrecognised head — safe, documented); `GREATEST/LEAST` reason string says "plateau" where the sharper invariant is *unbounded offset* (§4) — cosmetic but the spec's §Semantics should state the bounded-offset conjunct explicitly; `COALESCE(col, const)` with a provably `NOT NULL` leaf is refused though semantically identity (unfixable under downgrade-only gating without letting schema facts *upgrade* — a deliberate architecture cost worth recording); `VALUES`-sourced clocks refuse via leaf-resolution rather than being named seed-like.

## 8. Open questions

1. **Where does the relational composition live?** §5.2's operator walk (π through CTEs, per-branch set-op vectors, join-side gating) could be a recursive extension of `trace_event_time` over a query tree, or a separate planner-side pass that calls the expression trace per layer. The former keeps one primitive; the latter respects the current "pure expression classifier below, orchestration above" split. Which layer owns the fixpoint, and does `SelectAnalysis` need to retain `Expr` nodes to support it (the 20260701 doc's last open question, still open)?
2. **What is the verdict *type* for aggregate event-time?** `MAX(ts)`-as-clock is refused by the trace (correctly) but is genuinely maintainable via the value-monotone discriminant + keyed folding. Should there be a fourth verdict arm (`AggregateClock{combiner, evolution}`) that routes to the keyed machinery, or is `Disproven` + the separate discriminant proof the permanent shape? (Gap 1's fix forces this decision.)
3. **Post-join nullability: propagate or refuse?** Close gap 4 by building operator-level nullability propagation feeding the gate, or by the cheaper rule "an anchor on any null-supplying join side is refused outright"? The cheap rule is sound and closes G-06 but rejects `LEFT JOIN dim` models whose anchor is on the *preserved* side-adjacent shape only accidentally.
4. **Should bounded-offset be explicit in the verdict, and should some `Disproven`s be `Undecidable`?** `GREATEST(ts, const)` (monotone, unbounded offset) and boundary-ordered `CASE` (piecewise but monotone) are both provably-sometimes-safe shapes currently classified `Disproven` — i.e. permanently beyond even the declaration. Is the right refinement a verdict that separates *monotone* from *constant-shift* (making the refusal reason precise), and a `Disproven`→`Undecidable` reclassification for shapes that are undecided-in-general rather than positively non-monotone?
5. **Set-op verdict shape.** Is the per-branch vector (§5.2) the surfaced verdict type — with `smelt explain` showing one trace row per branch — or do we keep a scalar model-level verdict and accept that mixed-source UNIONs report only "branch-wise licensed"? This decides the API before UNION-branch wrap-and-filter (unbuilt) is implemented against it.
