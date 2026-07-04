---
feature: model_maintenance
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Model Maintenance

> **What this is.** The framework for *keeping a model current*: the correctness invariant every non-`full` refresh mode upholds, the algebraic ladder that bounds what smelt can maintain itself, and the contract by which a `refresh:` mode is composed from a model's properties and transforms. It **names** capabilities defined elsewhere and describes their **combination** into refresh modes. Out of scope, with their own homes: the properties a model's SQL can have (`model_properties.md`); the physical transforms those properties license (`model_transforms.md`); the `refresh:` enum, the three model axes, the input-consumption axis, the three-state declaration law, and the litmus rule (`models.md`); each mode's own surface and local machinery (`batched_models.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `materialized_view.md`); backend capability flags (`multi_backend.md`); the source clock and mutation declarations (`timeseries.md`, `sources.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the framework as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

This is a **system spec**. Its callers are the refresh-mode specs and the planner/analysis layer. The surface is the invariant, the ladder, and the composition contract they depend on.

### The composition contract

A `refresh:` mode is a **composition** of three kinds of thing:

- **Properties** — what a model's SQL can be proven (or declared) to be: the monotonicity trace, the algebraic discriminants, partition alignment, and the rest (`model_properties.md`).
- **Transforms** — the physical mechanisms a property licenses: keyed `merge_into`, source-filter pushdown, partition DELETE+INSERT, and the rest (`model_transforms.md`).
- **Output shape** — partitioned (a complete table with a `partition_column`) or keyed (one row per key), implied by the mode (`models.md` §"Refresh axis").

Every refresh-mode spec must present a **composition table** stating, for that mode: the properties it requires, the world-facts it consumes, the transform it drives, and its output shape. A mode spec's normative content is exactly (a) that composition table, referencing shared capabilities **by name**, plus (b) the mode's own **local** machinery, defined in full. It must not re-specify a capability that a capability spec owns.

## Semantics

### The equivalence invariant (parent contract)

Every non-`full` refresh mode upholds one invariant: **an incremental run produces the result a full refresh would, restricted to the inputs it has processed so far.** Two specialisations exist, one per output shape:

- **Per-partition equivalence** (partitioned output — `batched`): for every partition `p`, the incremental output sliced by `partition_column = p` equals a full refresh's slice for `p`.
- **End-state equivalence** (keyed output — `cumulative`, `latest_value`, `versioned`, `accumulating_snapshot`, `materialized_view`): for any set `S` of processed source partitions and any ordering π over `S`, the maintained state equals `full_refresh(model, source.where(partition ∈ S))`. The result depends only on the *set* processed, not the order.

Every property is proven in service of this invariant; every transform is licensed **because it preserves it**. For the smelt-driven modes the invariant is discharged by the generative equivalence oracle (§References), the family's regression net; for `materialized_view` it is discharged by the **engine's** native IVM, not the smelt oracle (smelt runs no combiner for that mode — §"Validator, not chooser").

### The algebraic maintenance ladder

What a keyed mode can maintain is fixed by the **algebra of its combiners**, not by any backend feature. The ladder is a partial order whose ordering criterion **is** invertibility → maintainability — which is why it lives here (with the invariant) and not in `model_properties.md`: the *discriminants* it reads (is-monoid, needs-inverse, decomposable, value-vs-order-monotone) are raw properties of the SQL and are owned by `model_properties.md`; the ladder — the ordering *and* the maintainable-vs-delegated cutoff — is the maintenance consequence and is owned here. The equivalence invariant holds unconditionally on every rung; only the state representation and its size change across rungs, never the fidelity of the user value.

1. **Direct monoid.** The stored column *is* the answer; the combiner is a commutative monoid (associative, commutative, identity = empty partition): `SUM`/`COUNT` (`+`, 0), `MIN`/`MAX` (±∞), `BOOL_*`, `BIT_*`.
2. **Decomposed monoid.** The user value is `π(state)` for a richer monoid element and a pure presentation map `π`: `AVG` = `(sum, count)` presented `sum/count`; variance = a Welford triple; approximate distinct = an HLL register vector. Kept in a state table, exposed through a presentation view.
3. **Group.** When inputs can change (corrections, reprocessing, deletes) the combiner must be **invertible** — a commutative group (`SUM`, `COUNT`, `BIT_XOR`). Monoids that are not groups (`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR`) cannot un-see a contribution and so cannot be reprocessed without a full refresh.
4. **Opt-in bounded-domain multiset.** Holistic aggregates needing all rows (exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`) are maintained by storing the per-key value→count multiset (a bounded-domain Z-set). **Opt-in and fail-loud**: state is `O(active domain)`, so an unbounded-state aggregate is default-refused (suggesting the approximate form or `refresh: full`) unless the modeller supplies a bounded-domain budget, and the runtime caps the multiset with a full-refresh fallback.

The ladder is the boundary: rungs 1–4 are what smelt maintains itself (a `merge_into` loop, optionally with a presentation view). Beyond it — general-operator retraction over joins, unbounded non-additive state — is **not** smelt-driven-maintainable and is delegated to the engine's native incremental-view maintenance via `refresh: materialized_view`.

### Validator, not chooser

The machinery **validates** the declared `refresh:` mode against the derived properties and rejects (fail-loud) when the SQL cannot uphold the mode's contract. It **never chooses or silently switches** the mode. A full refresh is the honest fallback surfaced as a diagnostic, never an automatic downgrade.

### Interactions

- **Declaration law and litmus rule** (`models.md` §Design): whether a fact is declared, derived, or implied by the mode, and whether a proposed combination earns a new peer mode, is derived, or composes in the DAG — both owned by `models.md`; this framework consumes them.
- **Input-consumption** (`models.md` §"Input-consumption axis"): which input rows are new is a derived, cross-cutting axis (mutation-profile world-fact → input-delta-discovery proof in `model_properties.md` → re-scan/probe transform in `model_transforms.md`). Moving along it never changes the equivalence contract, only what is scanned.

## Design

**The invariant and ladder live here, lifted from `cumulative_aggregate.md`.** They are cited as normative by every keyed mode; keeping them inside one mode's spec forces the others to reach into a sibling for their own contract. This spec owns them so each mode cites one home. `cumulative_aggregate.md` remains the reference implementation of the keyed-maintenance path (retraction, reprocessing, presentation-purity), not a bare composition table.

**Properties and transforms are separate specs, not filed under maintenance.** A monotonicity trace, a combiner-algebra classification, a keyed `merge_into`, a targeted backfill are general model capabilities — useful for backfills, schema evolution, and query optimisation, not only the refresh modes — so they live in `model_properties.md` / `model_transforms.md`. This spec names and combines them.

**Placement is definitional, not consumer-counted.** A capability whose verdict is stateable **without naming a refresh mode** lives in a capability spec; a capability meaningful **only inside a mode** lives in that mode's spec. (So pushdown-depth, used only by `batched` today, is a SQL property and lives in `model_properties.md`; backfill chunking, meaningless outside batched execution, stays in `batched_models.md`.) This gives every capability exactly one home — what lets `smelt:validate` catch drift — without a mechanical ≥N-consumer rule; because these capabilities are broadly useful, building one before a second consumer exists is fine.

**The declaration law and litmus rule stay in `models.md`.** They are refresh-axis reasoning, and `models.md` already owns the refresh axis and a first cut of both. Duplicating them here would split refresh-axis reasoning across two specs; this framework links to them instead.

**Validator, never chooser.** Auto-selecting or silently downgrading a refresh mode was rejected: it reproduces dbt's `strategy:` footgun where the effective contract is invisible. The declared mode is authoritative; the machinery only proves or refuses it.

## Constraints & Invariants

- The **equivalence invariant** holds for every non-`full` mode and on every ladder rung; a transform that cannot preserve it for a given model is refused, never applied approximately.
- **One home per capability and per rule.** The invariant, ladder, and composition contract are owned here; properties in `model_properties.md`, transforms in `model_transforms.md`, the declaration law and litmus rule in `models.md`. No spec re-specifies another's.
- **Proofs are fail-closed** (owned in `model_properties.md`, relied on here): an undecidable construct rejects; a declared escape hatch may only *widen* eligibility, never substitute for a proof's default reject.
- The **refresh mode is the only strategy selector**; input-consumption is derived from the source, never declared per model. No `strategy:` sub-knob.
- The machinery **validates, never chooses** the mode; a fallback to full refresh is a surfaced diagnostic, never an automatic switch.

## Known Divergences / Open Questions

- **The framework is unbuilt / unconsolidated**, and the equivalence invariant plus the algebraic ladder are today **owned by `cumulative_aggregate.md`** (§"Cross-partition equivalence", §"The maintenance boundary"). Lifting them here, authoring `model_properties.md` and `model_transforms.md`, and refining `models.md` (the three-state declaration law, the litmus rule as single home, and reconciling the stale claim that `accumulating_snapshot` is not a recognized `refresh:` value) are tracked by `docs/plans/20260704-model-updates.md` (design: `docs/research/20260704-maintenance-fundamentals.md`).
- **Six live code duplications** the consolidation must remove: two interval-reach analyses (`analysis/temporal.rs` vs `analysis/source_bounds.rs`); three interval-literal parsers; the non-deterministic-function list copied across `rules/incremental.rs`, `rules/cumulative.rs`, and `analysis/monotonicity.rs`; two driving-fact resolvers; two bound-derivation orchestration sites; aggregate-name extraction done twice.
- **Derive-else-declare** is the settled rule for facts smelt cannot compute: a property is a derived proof where statically decidable and a declared world-fact otherwise (the three-state law). Upstream source changes are outside smelt's control, so declaring a source world-fact smelt cannot derive is the honest fallback rather than guessing. The precise static/declared split for the source mutation profile and additive-only model-diff is settled per this rule; where each lands is recorded in `model_properties.md` as those proofs are authored.

## References

- **Code**: `crates/smelt-logical/src/analysis/{monotonicity,source_bounds,temporal,mod}.rs`; `crates/smelt-logical/src/rules/{incremental,cumulative}.rs`; `crates/smelt-backend/src/lib.rs` (`merge_into`/DELETE+INSERT trait signatures; impls in `smelt-backend-duckdb`/`smelt-backend-spark`); `crates/smelt-runtime/src/compile.rs`.
- **Tests**: the cumulative and batched full-refresh-equivalence harnesses; the monotonicity-trace unit tests (`smelt-logical`); the generative soundness oracle.
- **User docs**: the per-mode refresh pages under `docs-site/docs/`.
- **Plans (history)**: `docs/plans/20260704-model-updates.md` (the mode-vertical master this framework re-cuts).
- **Related specs**: `model_properties.md`, `model_transforms.md`, `models.md`, `batched_models.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `materialized_view.md`, `timeseries.md`, `sources.md`, `multi_backend.md`, `schema_evolution.md`.
