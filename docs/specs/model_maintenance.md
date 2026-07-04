---
feature: model_maintenance
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Model Maintenance

> **What this is.** The shared framework every non-`full` refresh mode composes: the correctness invariant they all uphold, the vocabulary for the analyses and declarations they share, the algebraic ladder that bounds what smelt can maintain itself, and the rules for where a shared capability lives versus where a mode owns its own. Out of scope: the `refresh:` enum, the three model axes, and the input-consumption axis surface, which stay in `models.md`; each mode's own surface and local machinery (`batched_models.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `materialized_view.md`); backend capability flags (`multi_backend.md`); the source clock and mutation declarations (`timeseries.md`, `sources.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the framework as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

This is a **system spec**. Its callers are the other refresh-mode specs and the planner/analysis layer. The surface is the vocabulary and contracts they depend on.

### The three buckets

Every capability in the maintained-model family is one of three kinds of thing. The buckets are **stages of one pipeline** — `declare → prove → transform` — not a partition of unrelated concepts; a single cross-cutting concern can occupy all three (see Semantics §"The input-consumption pipeline").

- **World-fact** — a property smelt **cannot** derive from SQL and must be **told**, via a source declaration or model frontmatter (e.g. a source's mutation profile, allowed lateness).
- **Proof** — a pure static analysis over a model's SQL (plus any declared world-facts) that returns a **verdict** (e.g. the event-time monotonicity trace, the combiner-algebra rung). Proofs must be **fail-closed**: an undecidable input rejects; a proof never assumes maintainability.
- **Transform** — a physical execution mechanism a proof (or world-fact) **licenses** (e.g. keyed `merge_into`, partition DELETE+INSERT, targeted column backfill).

### The composition contract

Each `refresh:` mode is a **composition** of the buckets. Every refresh-mode spec must present a **composition table** stating, for that mode: the world-facts it consumes, the proofs it requires, the transform it drives, and its output shape. A mode spec's normative content is exactly (a) that composition table, referencing **shared** fundamentals by name, plus (b) the mode's **local** fundamentals defined in full. It must not re-specify a shared fundamental (§"Shared-vs-local rule").

### The declaration law (three states)

Every fact the machinery uses falls into exactly one state. The `refresh:` mode is the **one thing the modeller declares to choose execution**; everything else is either derived or a consequence of that choice.

| State | Meaning | Members |
|---|---|---|
| **Declared** | stated by the modeller | the `refresh:` mode (the sole *selector*); plus *assertions* — source mutation profile (where not derivable), source-lateness, `nondeterministic_columns`, a bounded-domain space budget, a cost ceiling, the declared-monotonicity escape hatch |
| **Derived** | computed by a proof from the SQL (+ declared facts) | the algebraic rung, the lookback/horizon, ordering, partition alignment, input-delta discovery, and monotonicity *where statically decidable* (declared only as an escape hatch where undecidable) |
| **Implied by the mode** | a consequence of the selected mode, neither declared nor derived-from-SQL | the output shape; the **freshness owner** (pull for the smelt-driven modes, push for `materialized_view`) |

A declaration is either a **selector** (only the `refresh:` mode) or an **assertion** (every other declaration) — an assertion bounds or widens what the machinery may do but never itself picks the strategy.

### Shared declaration surfaces (world-facts)

These world-facts are declared once and consumed by many modes; they belong on the source or in frontmatter, never as a per-model `strategy:` knob:

| World-fact | Declared on | Consumed by |
|---|---|---|
| Source **mutation profile** (append-only / mutable / has-change-feed) | the source | every non-`full` mode; the one non-derivable input-consumption fact |
| Source **lateness** margin (allowed out-of-order arrival) | the source (`timeseries:`) | batched lookback, keyed-mode horizons |
| **Cost/ceiling assertion** (error if a derived cost-driver is exceeded) | the model | bounds the batched lookback, keyed-mode horizons, and keyed-state cardinality alike — never changes execution, only fails loud on breach |
| Source **functional dependency** (`key → column`) | the source | once-write milestones; 1:1-after-dedup joins |

## Semantics

### The equivalence invariant (parent contract)

Every non-`full` refresh mode upholds one invariant: **an incremental run produces the result a full refresh would, restricted to the inputs it has processed so far.** Two specialisations exist, one per output shape:

- **Per-partition equivalence** (partitioned output — `batched`): for every partition `p`, the incremental output sliced by `partition_column = p` equals a full refresh's slice for `p`.
- **End-state equivalence** (keyed output — `cumulative`, `latest_value`, `versioned`, `accumulating_snapshot`, `materialized_view`): for any set `S` of processed source partitions and any ordering π over `S`, the maintained state equals `full_refresh(model, source.where(partition ∈ S))`. The result depends only on the *set* processed, not the order.

Every proof exists to establish this invariant for its mode; every transform is licensed **because it preserves it**. The invariant is checked by the generative equivalence oracle (§References), which is the family's regression net.

### The algebraic maintenance ladder

What a keyed mode can maintain is fixed by the **algebra of its combiners**, not by any backend feature. The ladder has four rungs; the equivalence invariant holds unconditionally on every rung — what changes across rungs is the state representation and its size, never the fidelity of the user value.

1. **Direct monoid.** The stored column *is* the answer; the combiner is a commutative monoid (associative, commutative, identity = empty partition): `SUM`/`COUNT` (`+`, 0), `MIN`/`MAX` (±∞), `BOOL_*`, `BIT_*`.
2. **Decomposed monoid.** The user value is `π(state)` for a richer monoid element and a pure presentation map `π`: `AVG` = `(sum, count)` presented `sum/count`; variance = a Welford triple; approximate distinct = an HLL register vector. Kept in a state table, exposed through a presentation view. `π` must be a pure function of a single consistent state row.
3. **Group.** When inputs can change (corrections, reprocessing, deletes) the combiner must be **invertible** — a commutative group (`SUM`, `COUNT`, `BIT_XOR`). Monoids that are not groups (`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR`) cannot un-see a contribution and so cannot be reprocessed without a full refresh.
4. **Opt-in bounded-domain multiset.** Holistic aggregates needing all rows (exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`) are maintained by storing the per-key value→count multiset (a bounded-domain Z-set). **Opt-in and fail-loud**: state is `O(active domain)`, so the classifier default-refuses an unbounded-state aggregate (suggesting the approximate form or `refresh: full`) unless the modeller supplies a bounded-domain budget, and the runtime caps the multiset with a full-refresh fallback.

The ladder is the boundary: rungs 1–4 are what smelt maintains itself (a `merge_into` loop, optionally with a presentation view). Beyond it — general-operator retraction over joins, unbounded non-additive state — is **not** smelt-driven-maintainable and is delegated to the engine's native incremental-view maintenance via `refresh: materialized_view`.

### The needs-no-inverse discriminant

The real maintainability boundary within the smelt-driven modes is **needs-no-inverse (semilattice fold) vs needs-an-inverse (group)**: can a new row fold into state from `(current state, new row)` alone, or must a prior contribution be *un-seen*? A fold that needs no inverse never re-reads history and is `merge_into`-maintainable — this covers both **value-monotone** milestones (the value only moves one way: `MIN`/`MAX`/`COALESCE`/an `EXISTS` flag) and **order-monotone** folds (the merge state advances along a data-ordering key while the reported value may switch: `MAX_BY(v, ts)`). A contribution that must be un-seen needs the underlying multiset (the group rung) and, when unbounded, is delegated. "The value changes more than once" is *not* the boundary; "a folded element becomes wrong" is.

### The input-consumption pipeline

Which input rows are new since the last run is a **derived, cross-cutting axis**, orthogonal to what the stored relation means (canonical axis surface: `models.md` §"Input-consumption axis"). It is the clearest case of one concern occupying all three buckets:

`declare` the **source mutation profile** (world-fact) → `prove` **input-delta discovery** (window-forward / snapshot-diff / change-feed, from the source's shape) → `transform` via an **idempotent window re-scan or a delta-driven probe**.

Moving along this axis never changes the equivalence contract or output shape — only what is scanned. It is therefore surfaced (via `smelt explain`), never declared per model.

### The shared-vs-local rule

A **proof or transform** is **shared** (owned by this spec, built once, cited by mode specs) **iff ≥2 `refresh:` modes consume it** — where a consumer is a *mode*, not a sub-feature (a batched join-enrichment or UNION branch counts as `batched`, not an extra consumer). Otherwise it is **local** to the one mode, defined in that mode's spec.

**One exception, stated explicitly:** a single-mode *mechanism* may be owned here when a second mode consumer is *imminent and named* — the presentation-view mechanism and the windowed-keyed-maintenance driver (both `cumulative`-only today, with keyed-mode reuse pending). Such items are marked `shared*` and are **not built before their second consumer exists** (they are back-filled into the shared layer when the second mode lands).

**Scope of the rule:** it governs proofs and transforms. **World-facts are declaration surfaces** — they are shared by nature (declared once, consumed wherever) and are local only when a single mode interprets them (`nondeterministic_columns` is batched-only; a bounded-domain budget is cumulative-only).

### The validator, not chooser rule

The analysis machinery **validates** the declared `refresh:` mode against the derived facts and rejects (fail-loud) when the SQL cannot uphold the mode's contract. It **never chooses or silently switches** the mode. A full refresh is the honest fallback surfaced as a diagnostic, never an automatic downgrade.

### The litmus rule (when a combination earns a new mode)

For any proposed "can these combine?": a change to the **equivalence contract or output shape** earns a new **peer** `refresh:` value; a change only to **how deltas are discovered or how much is scanned** is **derived** (never a new mode); **two contracts at two grains** are expressed by **composing two models in the DAG**, not one mode with a sub-knob.

### Shared fundamentals catalogue

The shared (≥2-mode) proofs and transforms this spec owns. Per-fundamental detailed semantics live where noted; this table is the naming authority mode specs reference. (Verdict/role abbreviated; consumers by mode: ba=batched, cu=cumulative, lv=latest_value, ve=versioned, as=accumulating_snapshot, mv=materialized_view.)

| Fundamental | Bucket | Role / verdict | Consumers |
|---|---|---|---|
| Event-time monotonicity trace | proof | projected `event_time` traces monotonically to a real source column (`Traceable`/`StaticSeed`/`NotTraceable` + offset) | ba (+ cousin: as, join enrichment) |
| Column nullability gate | proof | downgrade `Traceable→NotTraceable` on a nullable/unknown leaf | ba, as (via the trace) |
| Unified bound/reach derivation | proof | finite backward/forward reach of a frame or interval band; splits derived computation-reach from declared source-lateness | ba, as |
| Combiner-algebra rung | proof | ladder rung of each combiner (§"algebraic ladder") | cu, lv, ve, as |
| Needs-no-inverse discriminant | proof | inverse-free fold vs needs-a-group (§"needs-no-inverse") | cu, lv, as, ba (enrichment) |
| Value/order-monotone classification | proof | value-monotone vs order-monotone fold | as, lv, ba (enrichment) |
| Join-contribution monotonicity | proof | a join's per-key contribution folds without an inverse and does not decrement | as, ba (enrichment) |
| Partition alignment (scoped) | proof | scope's `GROUP BY`/`DISTINCT` key ⊇ `partition_column`, per scope (opposite polarity: ba admits, cu/as reject) | ba, cu, as |
| Driving-fact / anchor resolution | proof | among joined inputs, exactly-one-`Traceable` is the anchor (alias-scoped) | ba (joins), cu, lv, ve, as |
| Window-independence / ordering | proof | window reads only sources (parallel) vs own prior output (ordered) | all (orchestration signal) |
| Additive-only model-diff | proof | a model edit only adds columns derivable from existing target + a monotone contribution → targeted backfill, not rebuild | all materialised |
| Input-delta discovery | proof | window-forward / snapshot-diff / change-feed, from the source's shape | all non-`full` |
| Keyed `merge_into` (target-as-replica) | transform | fold delta into keyed state, never re-read history | cu, lv, ve, as, ba (enrichment) |
| Source-filter pushdown | transform | window an input on its partition column | ba, cu, as |
| Targeted column backfill | transform | in-place UPDATE / dimension-merge for an additive diff | all materialised (excl. `view`/`mv`) |
| Dimension-driven horizon MERGE | transform | merge a dimension batch into the horizon-bounded target slice, never reading the fact | as, ba (enrichment) |
| Windowed-keyed-maintenance driver | transform (`shared*`) | the factored classify → step → pushdown → merge loop | cu today; lv/ve/as pending |
| Presentation view (hidden state) | transform (`shared*`) | store a richer monoid element, expose `π(state)` | cu today; lv/as pending |
| Delegate-to-native-IVM | transform | emit the backend's maintained object; hard error if unsupported | mv |
| DAG composition | transform | express a two-grain combination as two models, not a mode-combo | all |

## Design

**Why a fundamentals framework, not per-mode specs alone.** The refresh modes share a small set of proofs, declarations, and transforms, and the sharing is not hypothetical — the analysis code already carries six live duplications of shared logic (§Known Divergences), and the design research repeatedly asks for the same capability to be built once (`docs/research/20260703-model-updates.md` §§4.5, 9.5, 17.6, 18.1). Giving the spine one spec home is what lets `smelt:validate` catch drift: a shared classifier described twice is a flagged contradiction, not two blessed copies.

**Three declaration states, not two.** A strict "vertical-declared / horizontal-derived" dichotomy (`models.md` §Design) mis-sorts two items: freshness owner is neither declared nor derived-from-SQL — it is a consequence of the mode — and monotonicity is derived where decidable but declared as an escape hatch where not. Adding the *implied-by-the-mode* state removes the contradiction without weakening the core point that the mode is the only strategy-selecting declaration.

**The equivalence contract and algebraic ladder live here, not in cumulative.** The invariant and the ladder are cited as normative by every keyed mode; keeping them inside `cumulative_aggregate.md` forces the other modes to reach into a sibling spec for their own contract. This spec owns them so each mode cites one home. (`cumulative_aggregate.md` remains the reference implementation of the shared keyed-maintenance path; it does not shrink to a bare composition table.)

**The buckets are pipeline stages, not a taxonomy.** Treating world-facts/proofs/transforms as a clean partition breaks on input-consumption, which is one axis spread across a declaration, a proof, and a transform. Framing the buckets as `declare → prove → transform` keeps a single concept from being described three different ways across specs.

**The shared-vs-local rule is mode-counted to be mechanical.** Counting *modes* (not sub-features) makes "does this belong in the shared spec?" a check, not a judgement call — and doubles as the anti-over-engineering guard: a would-be-shared primitive with one live mode consumer is not built until a second mode needs it. The single `shared*` exception is named rather than left implicit precisely so it cannot be used to smuggle speculative generality in.

**Validator, never chooser.** Auto-selecting or silently downgrading a refresh mode was rejected: it reproduces dbt's `strategy:` footgun where the effective contract is invisible. The declared mode is authoritative; the machinery only proves or refuses it.

## Constraints & Invariants

- The **equivalence invariant** holds for every non-`full` mode and on every ladder rung; a transform that cannot preserve it for a given model is refused, never applied approximately.
- Each **shared fundamental has exactly one spec home** (this spec). A mode spec references it by name and must not re-specify its semantics.
- **Proofs are fail-closed**: an undecidable or unrecognised construct rejects with a diagnostic; it is never treated as maintainable. A **declared escape hatch may only widen** eligibility, never substitute for a proof's default reject.
- The **refresh mode is the only selector**. Input-consumption is derived from the source, never declared per model; there is no `strategy:` sub-knob.
- The machinery **validates, never chooses** the mode; a fallback to full refresh is a surfaced diagnostic, never an automatic switch.
- A **`shared*` primitive is not built before its second mode consumer exists**; it is back-filled when the second mode lands.

## Known Divergences / Open Questions

- **The framework is largely unbuilt / unconsolidated.** The shared classifiers named above are not yet extracted into one home, and the maintenance ladder plus the equivalence contract are today **owned by `cumulative_aggregate.md`** (§"The maintenance boundary", §"Cross-partition equivalence"), with the axes and litmus rule in `models.md` §Design. Lifting them here and rewriting the cross-references is tracked by `docs/plans/20260704-model-updates.md` (design: `docs/research/20260704-maintenance-fundamentals.md`); this spec claims the ownership those edits will realise.
- **Six live code duplications** the consolidation must remove: two interval-reach analyses (`analysis/temporal.rs` vs `analysis/source_bounds.rs`); three interval-literal parsers; the non-deterministic-function list copied across `rules/incremental.rs`, `rules/cumulative.rs`, and `analysis/monotonicity.rs`; two driving-fact resolvers (`cumulative.rs` ref-count vs `source_bounds.rs` alias-scoped); two bound-derivation orchestration sites; and aggregate-name extraction done twice.
- **Shared world-fact surfaces not yet declared.** The source **mutation profile**, **source-lateness**, and **cost/ceiling** assertions are described here but have no first-class declaration yet (the mutation profile is currently *inferred* from clock presence — `models.md` Known Divergences).
- **`models.md` drift.** `models.md` still states `accumulating_snapshot` is "not a recognized `refresh:` value and not accepted by the parser" while `accumulating_snapshot.md` exists; reconcile when the composition tables are authored.
- **One spec or split.** Whether the buckets warrant separate `maintenance_proofs.md` / `maintenance_transforms.md` files is open; kept as one spec because the buckets are stages of one pipeline bound by a single equivalence contract, so a split would fragment that contract.
- **Static vs declared boundaries.** How much of the source mutation profile and the additive-only model-diff is statically provable versus must be declared is open (see `docs/research/20260703-model-updates.md` §§17.6, 18.3 and `docs/research/20260704-monotone-join-maintenance.md` §§5, 8).

## References

- **Code**: `crates/smelt-logical/src/analysis/{monotonicity,source_bounds,temporal,mod}.rs`; `crates/smelt-logical/src/rules/{incremental,cumulative}.rs`; `crates/smelt-backend/src/lib.rs` (`merge_into`/DELETE+INSERT trait signatures; impls in `smelt-backend-duckdb`/`smelt-backend-spark`); `crates/smelt-runtime/src/compile.rs`.
- **Tests**: the cumulative and batched full-refresh-equivalence harnesses; the monotonicity-trace unit tests (`smelt-logical`); the generative soundness oracle.
- **User docs**: the per-mode refresh pages under `docs-site/docs/`.
- **Plans (history)**: `docs/plans/20260704-model-updates.md` (the mode-vertical master this framework re-cuts).
- **Related specs**: `models.md`, `batched_models.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `materialized_view.md`, `timeseries.md`, `sources.md`, `multi_backend.md`, `schema_evolution.md`.
