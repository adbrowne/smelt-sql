# Property composition under SQL constructs — overview and index

- **Date**: 2026-07-07
- **Status**: research (index / synthesis over nine per-property docs)
- **Motivation**: `model_properties.md` catalogues the proofs and states their verdicts
  per-construct in isolation; `docs/research/property-discovery/` tests specific
  `(construct × property × technique)` cells empirically. What was missing is the
  *analytical* layer between them: for each property, the full set of per-operator
  transfer rules and the algebra by which verdicts on sub-queries compose into a
  verdict on the whole query. This family of docs is that layer.
- **Related specs**: `model_properties.md`, `model_maintenance.md`, `model_transforms.md`,
  `models.md`, `batched_models.md`, `keyed_models.md`, `sources.md`.
- **Related research**: `20260705-refresh-as-maintenance-plan/` (the per-cell framework
  these rules feed), `property-discovery/catalog.md` (the empirical grid),
  `20260701-monotonicity-primitive-research/` (prior monotonicity deep-dive).

## The nine property docs

| Doc | Property | Scope | Verdict domain |
|---|---|---|---|
| [`20260707-property-event-time-monotonicity.md`](20260707-property-event-time-monotonicity.md) | event-time monotonicity / traceability | per-column | `Traceable{src,col,offset,mono}` \| `StaticSeed` \| `NotTraceable{Disproven\|Undecidable}` + nullability gate |
| [`20260707-property-bounded-reach.md`](20260707-property-bounded-reach.md) | bounded reach / temporal locality | per-(model, source) | `Bounded{before,after}` \| `Unbounded` \| `NotDerivable` |
| [`20260707-property-filter-distributivity.md`](20260707-property-filter-distributivity.md) | filter distributivity / pushdown depth | per-(model, source) | lattice `Source ≻ OuterClamp(m) ≻ Refused` |
| [`20260707-property-partition-alignment.md`](20260707-property-partition-alignment.md) | partition alignment | per-scope, meet-rolled per-model | `Aligned` \| `NotAligned{reason}` (aligned = scope keys *refine the partition grid*) |
| [`20260707-property-key-grain.md`](20260707-property-key-grain.md) | key grain / cardinality & fan-out | per-model (relation grain) + per-column lineage multiplicity | key sets; `OneToOne` \| `OneToMany` (fail-closed) |
| [`20260707-property-aggregate-algebra.md`](20260707-property-aggregate-algebra.md) | aggregate combiner algebra | per-column | `(is_monoid, needs_inverse, decomposable, monotone)` + proposed **idempotence** and multiset-function-ness |
| [`20260707-property-determinism.md`](20260707-property-determinism.md) | determinism (run vs row) + taint | per-column + per-relation membership bit | lattice `clean < run < row`; model roll-up = skeleton-clean |
| [`20260707-property-per-key-constancy.md`](20260707-property-per-key-constancy.md) | per-key constancy / functional dependencies | per-column (relative to a key) | `Constant` \| `NotProven` \| `Refused`, graded epistemically (axiom/declared/derived) × temporally (within-run / null-monotone / write-once) |
| [`20260707-property-delta-shape.md`](20260707-property-delta-shape.md) | delta shape propagation | per-model per input-edge, per-column refinement | lattice `none ⊑ insert-only ⊑ upsert-by-key(K,C) ⊑ general` |

Per the user question that motivated the sweep: monotonicity, determinism, per-key
constancy, and the aggregate discriminants are **per-column** facts; reach, pushdown
depth, grain, and delta shape are **per-relation** facts (reach and pushdown additionally
parameterised by source, delta shape by input edge); partition alignment is **per-scope**
with a per-model roll-up. Several per-relation facts carry a per-column refinement that
is load-bearing (delta shape's mutable-column set `C`, grain's lineage multiplicity,
taint's per-column sets) — collapsing them to a relation-level bit loses real admissions.

## Cross-property composition matrix

Compact verdicts; each cell is normative only in its own doc (side-conditions and
counterexamples live there). ✓ = property transfers unconditionally; cond = transfers
under the doc's side-condition; ✗ = breaks (fail-closed).

| Operator | Monotonicity | Reach | Pushdown σ | Alignment | Grain | Agg algebra | Determinism | FD | Δ-shape |
|---|---|---|---|---|---|---|---|---|---|
| Projection / scalar expr | cond (monotone fn, const offset) | ✓ (neutral) | cond (monotone rewrite) | ✓ (carriage obligation) | cond (key cols projected bare) | — | ✓ (taint union; drop clears) | cond (key+col projected; congruence) | ✓ (drops to `none`/`general` per cols kept) |
| WHERE | ✓ (value-neutral) | + band on shifted compares | ✓ | ✓ | ✓ (never establishes) | — | ✓ (tainted pred → membership) | ✓ | ✓ unless pred reads mutable col; run-varying pred → `general` |
| INNER JOIN | cond (exactly one anchor) | **adds** along join band; ∞ if unbounded | anchor side ✓; other side margin(band) | join on p pins both sides | probe grain iff other side unique on join cols | fan-out corrupts non-idempotent aggs downstream | per-side per-column import | both sides + transitive import iff OneToOne | lub of sides; new rows pair with old address-space |
| LEFT JOIN | cond + post-join nullability of anchor | as inner | preserved side ✓; null-supplying margin(band) | as inner | left grain iff right unique | as inner | as inner | imports as null-monotone (weaker) | insert-only inputs → **upsert-by-left-key** (stranded NULL) |
| SEMI / ANTI (EXISTS) | ✓ (row-filter) | + correlation band | outer ✓; inner margin(band) | correlation must pin p | **✓ always** (the safe enrichment) | — | membership taint from inner | ✓ | ANTI is **shape-inverting** (insert→delete) |
| GROUP BY + aggs | ✗ as trace (`MIN/MAX(ts)` is a different property — evolution-monotonicity) | identity if ts is a key; +bucket width if truncated | cond (aligned, bucket-rounded endpoints) | aligned iff keys refine grid | **establishes** {group keys} | discriminants of the combiners | collapses row→value taint (never escalates) | **factory**: key → every output (within-run grade) | insert-only → upsert-by-group-key(aggs) |
| Window fn | ✗ (relation-dependent value) | + frame reach; ROWS/bare LAG ⊥; UNBOUNDED ∞ | aligned PARTITION ✓; RANGE k margin(k); else refused | every OVER must contain p | preserves; `ROW_NUMBER…=1` establishes | same algebra, different addressing | + tie-ambiguity taint (no volatile fn needed) | `PARTITION BY k` whole-frame → FD on k | insert-only → upsert over frame neighbourhood |
| DISTINCT | ✓ | parallel-max | ✓ (δ∘σ) | aligned iff p projected | establishes whole-row key | identity for idempotent combiners; group→holistic otherwise (as modifier) | any tainted projected col → membership | projected set becomes a key | survives w/ membership probe |
| UNION ALL | cond (branch-wise vector; no single reduction across sources) | parallel-max | branch-wise ✓ | branches transparent + cross-branch grain consistency | **✗** unless discriminated (literal tag in key) | monoid folds across branches unconditionally | columnar union (clean ∪ clean = clean) | **✗** unless disjoint keys / discriminator / shared derivation | branchwise lub |
| UNION / INTERSECT / EXCEPT | cond (branch-wise; EXCEPT left arm only) | parallel-max, time col must be compared | σ provably distributes (EXCEPT minuend only); refused today (unclassified) | global whole-row scopes; p at exact grain in every branch | UNION ✗ / INTERSECT-EXCEPT preserve left | idempotent survive dedup; SUM/COUNT corrupted | membership taint if any col tainted | INTERSECT/EXCEPT preserve; UNION ✗ | INTERSECT probe; EXCEPT anti-monotone right |
| ORDER BY / LIMIT | LIMIT ✗ (non-local) | LIMIT ∞ | LIMIT refused | LIMIT global scope, unaligned | LIMIT preserves | — | tie-ambiguous membership taint | ✓ | top-k evicts: insert-only → general |
| CTE / subquery stacking | cond (each layer transparent; offsets add, strictness meets) | **series-add** | margins **add**; meet of verdicts | carriage through every SELECT list | grain chains through operator rules | re-agg rule: outer = inner's state-merge partner | fixed-point over DAG | Armstrong closure over transfer rules | fold of transfer rules along the path |
| Self-reference | — (own clock) | (k,0) if backward-bounded, else refused | not a pushdown — ordered execution | — | — | fold pattern | — | — | per-run `none` under ordered execution |

## Cross-cutting findings

1. **Two algebraic shapes recur.** Reach and pushdown margins compose as a *tropical
   semiring*: **add in series** (stacked CTEs, chained join bands), **max in parallel**
   (set-op branches), with `NotDerivable`/`Refused` as absorbing ⊥. Taint, delta shape,
   and grain compose as *monotone lattice folds*. Once stated this way, each property's
   whole-tree verdict is a bottom-up fold — the analyses are compositional almost
   everywhere, with `LIMIT` (non-local membership) as the recurring exception.

2. **Idempotence is the missing discriminant.** UNION DISTINCT (dedup) and fan-out joins
   (duplication) are twins: both are harmless exactly for idempotent combiners
   (`MIN`/`MAX`/`BOOL_*`) and corrupting for `SUM`/`COUNT`. It also makes the `DISTINCT`
   modifier a no-op instead of a holistic demotion. The current
   `(is_monoid, needs_inverse, decomposable, monotone)` tuple cannot express it.

3. **UNION ALL splits the property space in half** — the user's motivating question has a
   crisp answer. Value-algebra properties survive it unconditionally (monoid folds,
   reach-max, branch-wise traces, clean taint). Identity properties are destroyed by it
   even when both branches hold them (key uniqueness, functional dependencies,
   single-column traceability across different sources). The shared repair is the
   **discriminated union**: a literal tag column added to the key/trace makes branches
   provably disjoint and restores FD, grain, and per-branch injection.

4. **Shape vs addressing.** A delta's *shape* (insert-only) does not bound *where* it
   lands: an insert-only join delta pairs new rows with old address-space; a keyed write
   reaches outside any time window (the SCD close-out). Several properties exist
   precisely to re-bound addressing (reach, key temporal locality, alignment).

5. **Per-column precision pays.** Dropping a tainted column clears taint; projecting away
   the mutable columns collapses an upsert delta to `none`; aggregation absorbs `general`
   input back to key-grain upserts. Relation-level bits would refuse all three.

6. **Aggregate-of-the-clock is a category shift.** `MIN(ts)`/`MAX(ts)` as event-time is
   not a monotone trace and not merely undecidable — it is *evolution monotonicity*, an
   aggregate discriminant belonging to the keyed-fold machinery. Filing it under
   `Undecidable` is what makes finding B1 below a soundness hole.

## Candidate soundness issues surfaced (to verify, then ledger)

Each was found analytically; the next step for each is a red harness case in the
property-discovery loop, not a direct fix.

- **B1 — `assert_monotonic` can widen `MAX(ts)`.** Aggregate clocks fall to the
  `Undecidable` arm of the trace (unrecognised function), the only arm the declaration
  may widen — so a declared model with an aggregate event-time is admitted unsoundly.
  Should be `Disproven` or a distinct verdict arm. (monotonicity doc §7)
- **B2 — `BoundResult::merge` uses max where series composition needs add.** A 7d window
  in a CTE under a 3d outer window derives 7d, true reach 10d; chained join bands
  likewise don't add. Corroborated independently by the reach and pushdown docs.
  (bounded-reach §7, filter-distributivity §7)
- **B3 — lateness folded with `max` not `+`.** `compute_effective_window` combines
  source-lateness and computation-reach with `max`; a late row needing lookback gets the
  larger of the two, not the sum. (bounded-reach §7)
- **B4 — `FunctionalDependency.key` is parsed but never read**, and no union analysis
  exists — a declared FD over a UNION ALL body widens unsoundly today.
  (per-key-constancy §7)
- **B5 — CTE-internal HAVING/DISTINCT judged by nobody.** The admission walks cover the
  outer UNION chain only while CTE bodies are exempt from the subquery gate — an
  optimistic hole in batched admission. (partition-alignment §7)
- **B6 — `BIT_XOR` classified `needs_inverse: true`** in `discriminants.rs`,
  contradicting `model_maintenance.md` rung 3 (self-inverse group) — spec/code drift.
  (aggregate-algebra §7)
- **B7 — UDFs fail-open in taint positions.** The determinism predicate defaults an
  unknown function to deterministic everywhere except the event-time trace.
  (determinism §6)
- Known/already-catalogued, re-confirmed analytically: **SC-1** (correlated-EXISTS band
  falls back to `(0,0)`), **SC-2** (clocked `Mutable` → `WindowForward` misses back-dated
  updates), **G-10** (composite `unique_key` inexpressible in `JoinContext` — blocks the
  grain, FD, and once-write consumers alike).

## Consolidated open design questions

1. **Where does the relational composition walk live?** Every doc independently needs a
   per-operator fold over the query tree (trace, reach, alignment carriage, grain, FD
   closure, taint, shape). Today each analysis is a flat or per-clause scan. One shared
   bottom-up pass over `LogicalNode` carrying a property vector per node — with each
   property contributing its transfer function — is the obvious architecture question.
2. **Extend the discriminant tuple** with idempotence (and multiset-function-ness), and
   decide its owner (field on `Discriminants` vs proof-site table).
3. **Discriminated-union repair as a recognised idiom**: derive branch disjointness from
   literal tag columns (restores FD/grain/injection over UNION ALL) vs a declaration vs
   fail-closed only.
4. **Lattice gaps**: a keyed-delete point for anti-monotone edges (delta shape); a fourth
   `AggregateClock` verdict arm (monotonicity); temporal grades on `Constant` (FD);
   a conditionally-deterministic class keyed on order-key uniqueness (determinism).
5. **Set-op admissions**: σ provably distributes over UNION/INTERSECT/EXCEPT (minuend
   side) — what bar admits the proven rules currently refused as unclassified?
