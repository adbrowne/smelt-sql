---
feature: model_properties
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Model Properties

> **What this is.** The reusable **properties a model's SQL can have** — general, static capabilities of the query itself, each either a **derived proof** (a pure static analysis over the model's SQL that returns a verdict) or a **model-scoped declaration** (a world-fact the modeller states in the model's own SQL/frontmatter). These properties are consumed by incremental maintenance, backfills, schema evolution, and query optimisation, but they are stated **without naming any of those** — a property whose verdict is only meaningful inside one refresh mode does *not* live here. Out of scope, with their own homes: the algebraic *ladder* and the maintainable-vs-delegated cutoff, plus the equivalence invariant and composition contract (`model_maintenance.md` — this spec owns only the raw algebraic **discriminants** the ladder reads); the physical mechanisms a property licenses (`model_transforms.md`); the `refresh:` enum, the three-state declaration law, the input-consumption axis, and the litmus rule (`models.md`); the world-facts that live on the source, backend, or core — the timeseries clock (`timeseries.md`), the source mutation profile (`sources.md`), the backend capability flags (`multi_backend.md`) — which this spec only *catalogues by reference* as proof inputs; and each mode's local machinery (`batched_models.md`, `cumulative_aggregate.md`, and the other mode specs).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes these properties as if they have always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

The surface is the set of properties and their verdicts. Callers are the planner/analysis layer and the refresh-mode specs; the two verdict families a caller can depend on are **derived proofs** (established by static analysis) and **model-scoped declarations** (established by the modeller).

**Placement criterion (what belongs here).** A capability lives in this spec **iff its verdict is stateable without naming a refresh mode**. A capability meaningful only inside one mode lives in that mode's spec (see §Constraints).

### Derived proofs

| Property | Verdict / meaning | Maturity |
|---|---|---|
| Event-time monotonicity trace | `EventTimeTrace` = `Traceable{source, source_column, offset, monotonicity}` \| `StaticSeed{reason}` \| `NotTraceable{reason}`: does the projected `event_time` expression trace monotone-non-decreasing to exactly one source column, folding constant `INTERVAL` shifts (`offset` = `Seconds` \| `Symbolic`; `monotonicity` = ClickHouse-style `{is_monotonic, is_positive, is_always_monotonic, is_strict}`) | built |
| Column nullability gate | downgrades `Traceable → NotTraceable` when the traced leaf column is nullable or its nullability is unknown, so a pushed filter cannot silently drop `NULL` rows | built |
| Unified bound / reach derivation | `BoundResult` = `Bounded{source_partition_col, before, after}` \| `Unbounded` \| `NotDerivable`: the finite backward (`before`) / forward (`after`) reach in seconds a frame or interval band forces around the run window, per source; splits *computation-reach* (derived) from declared *source-lateness* | built, duplicated |
| Injection-point / pushdown-depth | `InjectionPoint` = `Source` (zero-margin transparent slice, push filter to the scan) \| `OuterClamp` (nonzero margin / `Unbounded` / `NotDerivable`): the deepest safe placement for the event-time filter | partial |
| Frame-reach taxonomy | `RANGE … INTERVAL` → derivable reach `k`; `ROWS`/`GROUPS`/bare `LAG`/`LEAD` → `NotDerivable`; `UNBOUNDED` → ∞ | built (RANGE/ROWS/LAG); `GROUPS` conservative |
| Algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone) | raw algebraic facts of the combiner: commutative-monoid membership (`SUM`/`COUNT`/`MIN`/`MAX`/`BOOL_*`/`BIT_*`), whether a contribution must be *un-seen* (needs an inverse), additive vs holistic (`SUM/COUNT/MIN/MAX/AVG` decomposable; `MEDIAN/MODE/exact-distinct` holistic), and value-monotone (`MIN`/`MAX`/`EXISTS`) vs order-monotone (`MAX_BY`). **Not the ladder** — see §Design | partial (monoid set built in `cumulative`; others not-yet) |
| Partition alignment (scoped) | `PartitionAlignment` = `Aligned` \| `NotAligned{reason}`, judged per scope (`GROUP BY`/`DISTINCT`/window `OVER`): does the scope's key set contain the `partition_column`. Consumed with *opposite polarity* by different modes — a raw containment fact, not a mode verdict | built (AST); window `OVER` text-scanned |
| Driving-fact / anchor resolution | among *joined* inputs, exactly-one-`Traceable` input is the anchor (alias-scoped leaf disambiguation); two or zero → fail-closed. A model may still independently window several sources — that multi-source bound derivation is distinct | built, duplicated |
| Fan-out / cardinality | a join multiplies rows / changes target cardinality vs enriches in place | not-yet |
| Join-contribution monotonicity | a semi-/dimension-join's per-key contribution folds without an inverse (value- or order-monotone) and does not fan into a decrementing aggregate — composed from the inverse-free discriminant + fan-out/cardinality; licenses the dimension-driven horizon MERGE | not-yet |
| Presentation-map purity | a hidden-state presentation map `π(state)` is a pure function of a single consistent state row — reads no other rows, tables, or windows; the soundness condition for a decomposed-state presentation view | not-yet |
| Interval / temporal-join detection | a bounded time band gives the second fact a finite lookback vs an unbounded equi-join | partial |
| Determinism (run vs row) + nondeterminism predicate | the predicate over function names (`RANDOM`/`RAND`/`UUID`/`GEN_RANDOM_UUID`/`SETSEED` are row-nondeterministic; `NOW`/`CURRENT_*` are run-deterministic and pinnable), plus the taint flow that no row-nondeterministic value reaches a skeleton position | built, duplicated |
| Body-structure classifier | `SelectItemKind` = `CountDistinct` \| `OtherAggregate` \| `GroupByKey`: is a subquery **or CTE** body transparent / group-aligned / order-sensitive (one parse-based check) | built |
| Set-operation distribution | whether a filter distributes over `UNION [ALL]` (branch-wise trace); `INTERSECT`/`EXCEPT` not yet classified | partial (UNION ALL) |
| Static-seed detection | a constant/`NULL` in the `event_time` slot (or `COALESCE(col, const)`) → `StaticSeed`, not a stream | built |
| Additive-only model-diff | a model edit only *adds* columns derivable from `{existing target} ∪ {monotone dim}` → in-place backfill is admissible, not a rebuild | not-yet |
| Window-independence / ordered-execution | a window reads only sources (parallelisable) vs its own prior output (ordered); self-edge detection | not-yet |
| Input-delta discovery | window-forward / snapshot-diff / change-feed, derived from the source's shape — the proof stage of the input-consumption axis (`models.md`); pairs with the mutation-profile world-fact | partial |

### Model-scoped declarations

These are world-facts smelt cannot derive from the SQL, stated in the model's own SQL/frontmatter. Each may only **widen** what the proofs admit — never substitute for a proof's default reject on a construct it cannot decide.

| Declaration | What it asserts | Where declared |
|---|---|---|
| Declared monotonicity guarantee | the model is monotone where static proof is undecidable (a UDF, an opaque body). The static default is always reject-the-push; this escape hatch may only widen | model frontmatter |
| `nondeterministic_columns` | output columns exempt from the determinism requirement — audit stamps / surrogates the modeller accepts may vary. Admitted only when the nondeterministic value flows *exclusively* into a listed column; an `event_time`/`partition`/`unique_key` column may never be listed | model frontmatter |
| Functional dependency (`key → column`) | a column is a per-key constant, admitting once-write `COALESCE`/1:1-after-dedup enrichment | model frontmatter |
| Bounded-domain / space budget | a column's active domain is bounded, licensing an exact holistic aggregate via an explicit multiset. Fail-loud with a cap; never the default | model frontmatter |

### Catalogued inputs (owned elsewhere)

The proofs above consume world-facts that live on the source, backend, or core. This spec does **not** re-home them; it names them so the proof inputs are traceable: the **timeseries clock** (`event_time_column`/`partition_column`/`granularity`, `timeseries.md`); the **source mutation profile** (append-only / mutable / CDF, `sources.md`); the **source-lateness margin** (the declared term of the reach split); the **backend capability flags** (`multi_backend.md`); and the **refresh selector** itself (`models.md`).

## Semantics

Detailed below are the load-bearing proofs; the rest are as stated by their verdict in §Surface.

### Event-time monotonicity trace

`trace_event_time(expr, ctx)` walks the projected `event_time` expression's AST and returns `Traceable` **only if** the expression reduces to exactly one source column shifted by constant `INTERVAL`s that preserve monotone-non-decreasing order (`is_monotonic ∧ is_positive`). A constant or `NULL` leaf yields `StaticSeed`; anything undecidable (a non-monotone function, an unresolved reference, an opaque body) yields `NotTraceable{reason}`. The offset is retained exactly where seconds are exact (`Seconds`) and symbolically where a unit is calendar-variable (`Symbolic("1 month")`). The trace is **pure** and **fail-closed**: absence of a proof is `NotTraceable`, never an optimistic default.

### Column nullability gate

The trace alone does not certify that a filter pushed to the leaf is safe: a nullable leaf column can carry rows a `WHERE event_time ≥ …` would drop. `gate_nullable_leaf(trace, leaf_nullable)` therefore downgrades `Traceable → NotTraceable` when the leaf is nullable or its nullability is unknown. The gate resolves the leaf's nullability through the Salsa schema layer (`trace_event_time_checked`), keeping the pure trace independent of the catalogue.

### Unified bound / reach derivation

For each source ref, `derive_model_bounds(sql, ctx)` computes how far outside the run window that source must be read. The verdict separates two independently-sourced quantities: the **computation-reach** (derived from the SQL's frames, `LAG`/`LEAD` offsets, interval join bands, and `WHERE` interval shifts) and the **source-lateness** margin (declared on the source; default 0). A `Bounded{before, after}` result licenses a widened scan; `Unbounded` and `NotDerivable` forbid a pushed filter and force an outer clamp. The `before`/`after` split names backward and forward reach separately because forward reach interacts with watermark settling, which backward reach does not.

### Algebraic discriminants (the raw facts, not the ladder)

This spec owns the **discriminants** — is-monoid, needs-inverse, decomposable, value-vs-order-monotone — as static properties of the combiner algebra in the SQL. They are raw facts: `SUM`/`COUNT` are commutative monoids that are also groups (invertible); `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` are monoids that are **not** groups (a contribution cannot be un-seen); `AVG`/variance/approx-distinct are decomposable into a richer monoid element; `MEDIAN`/`MODE`/exact-`COUNT(DISTINCT)` are holistic. `MIN`/`MAX`/`EXISTS` are value-monotone (the value moves one way); `MAX_BY` is order-monotone (a semilattice fold whose presented value may switch). The **ordering** of these facts into a ladder, and the maintainable-vs-delegated cutoff, are a maintenance consequence and live in `model_maintenance.md` — this spec states only which discriminant each combiner has.

### Driving-fact / anchor resolution

`resolve_join_driving_fact(event_time_expr, alias_sources, ctx)` disambiguates, among the joined inputs of a scope, which single input is the anchor by re-running the monotonicity trace against each alias-scoped source. Exactly one `Traceable` input is required; zero or two or more is fail-closed (the model must be disambiguated by the modeller or is rejected).

### Determinism (run vs row) and the nondeterminism predicate

The nondeterminism predicate classifies a function name as **run-deterministic** (`NOW`/`CURRENT_TIMESTAMP`/`CURRENT_DATE` — one value per run, pinnable at compile time) or **row-nondeterministic** (`RANDOM`/`RAND`/`UUID`/`GEN_RANDOM_UUID`/`SETSEED` — a fresh value per row, unpinnable). The predicate is the shared, mode-agnostic fact; a *taint flow* built atop it verifies that no row-nondeterministic value reaches a skeleton position (event-time, partition, unique-key, or row-membership). The `nondeterministic_columns` declaration widens the taint flow to tolerate a row-nondeterministic value that flows exclusively into a listed payload column.

### Interactions

- **Input-consumption axis** (`models.md`): input-delta discovery is the proof stage of that cross-cutting axis; the mutation-profile world-fact and the re-scan/probe transform are its other two stages. This proof derives *which* rows are new; it never changes what the stored relation means.
- **The algebraic ladder** (`model_maintenance.md`): consumes the discriminants above as its ordering criterion. The ladder is not defined here.

## Design

**Properties are named for what they are, not for maintenance.** A monotonicity trace, an algebraic discriminant, a partition-alignment signal, an additive-only diff are each useful well beyond the refresh modes — backfills, schema evolution, query optimisation — so they live in a capability spec keyed on the SQL property, not filed under any one consumer. This is what lets a single proof serve several consumers without a private copy per mode.

**Placement is definitional, not consumer-counted.** A capability belongs here iff its verdict is stateable without naming a refresh mode. Pushdown-depth, used only by `batched` today, is a SQL property and lives here; backfill chunking, meaningless outside batched execution, stays in `batched_models.md`. This gives every capability exactly one home — what lets `smelt:validate` catch a spec that silently re-describes it — without a mechanical ≥N-consumer rule. Because these properties are broadly useful and cheap to name, building one before a second consumer exists is fine.

**Discriminants here, ladder in maintenance.** The cut between this spec and `model_maintenance.md` is exactly the cut between a *raw algebraic fact* and its *maintenance consequence*. Is-monoid / needs-inverse / decomposable / value-vs-order-monotone are facts of the SQL; the ordering `monoid ⊂ decomposed ⊂ group ⊂ multiset` and the maintainable/delegated cutoff are what those facts *imply for maintenance*, so they live with the equivalence invariant, not here. Splitting it this way keeps the discriminants reusable by query optimisation and schema evolution, which do not care about the ladder.

**Derive where decidable, declare where not.** A property is a **derived** proof where it is statically decidable and a **declared** world-fact otherwise (the three-state law, `models.md`). This holds generally: upstream source changes are outside smelt's control, so when smelt cannot derive a world-fact about a source, declaring it is the honest fallback rather than guessing. Event-time monotonicity is derivable in the common case and declared only as an escape hatch; source append-only-ness is derivable only narrowly (an immutable clock, no delete path) and otherwise declared on the source; additive-only model-diff is derivable from the column/dependency diff, but "did an existing column's semantics change" is not and falls to a declared migration intent.

**Proofs are validators, never choosers.** A proof returns a verdict; it never picks a refresh mode or silently switches strategy. The declared mode is authoritative and the machinery only proves or refuses it (`model_maintenance.md` §"Validator, not chooser").

## Constraints & Invariants

- **Proofs are fail-closed.** An undecidable construct yields the reject verdict (`NotTraceable` / `Unbounded` / `NotDerivable` / `NotAligned`), never an optimistic default. Absence of a proof is a rejection, not a pass.
- **Declared escape hatches may only widen.** A model-scoped declaration (declared monotonicity, `nondeterministic_columns`, functional dependency, bounded-domain budget) may only *widen* the set a proof admits; it may never substitute for a proof's default reject on a construct the proof itself cannot decide, and never narrow eligibility.
- **One home per property.** Each property's normative verdict is defined once, here. A mode spec references it by name and must not re-specify it. The algebraic *ladder* (as opposed to the discriminants) is **not** defined here — it is owned by `model_maintenance.md`.
- **Placement criterion.** A capability whose verdict names a refresh mode does not belong in this spec. Mode-only capabilities stay in the mode spec: batch-safety roll-up, column-locality, event-time outer-visibility, backfill chunking, run/partition granularity alignment (`batched_models.md`); reprocessing detection (`cumulative_aggregate.md`); once-write verification, hot-key/settled-key GC (`accumulating_snapshot.md`); engine-incrementalizability (`materialized_view.md`). (Presentation-map purity is *not* mode-only — its verdict is stateable without naming a mode, so it is a derived proof above, not an exclusion.)
- **Catalogued inputs are not re-homed.** The timeseries clock, source mutation profile, source-lateness margin, backend capability flags, and refresh selector are declared in their existing homes; this spec only references them.

## Known Divergences / Open Questions

- **The property layer is unbuilt as a consolidated whole**, and several proofs carry known gaps captured in §Surface as `partial`/`not-yet`. The consolidation, the new classifiers (algebraic discriminants beyond the monoid set, fan-out/cardinality, additive-diff, window-independence/ordered-execution), and the model-scoped declaration surfaces are tracked by `docs/plans/20260704-model-updates.md` (design: `docs/research/20260704-maintenance-fundamentals.md`).
- **Six live code duplications** the consolidation must remove — every one of these is a proof that currently exists in more than one copy: (1) two interval-reach analyses (`analysis/temporal.rs` `EffectiveWindow`, day-granular, vs `analysis/source_bounds.rs` `BoundResult`, second-granular); (2) three interval-literal parsers with divergent unit handling (month = symbolic vs ≈30d); (3) the nondeterminism-function list copied across `rules/incremental.rs`, `rules/cumulative.rs`, and `analysis/monotonicity.rs`; (4) two driving-fact resolvers (`cumulative.rs` ref-count vs `source_bounds.rs` alias-scoped trace); (5) two bound-derivation orchestration sites; (6) aggregate-name extraction done twice (typed `SqlFunction::is_aggregate` vs a string re-parse in `cumulative.rs`). Until consolidated, the second-granular AST core and the day-granular text-scanning layer can disagree at the margins.
- **Heuristic text-scanning layer.** The mature core is AST-pure (`EventTimeTrace`, `Monotonicity`, `SelectItemKind`, `PartitionAlignment`, the monoid combiner set); an older layer (`source_bounds.rs` bound extraction, `temporal.rs`, the window-`OVER`/`LIMIT` scans in `incremental.rs`) is uppercase-substring based and fail-closed. Set-operation distribution covers `UNION ALL` only; `INTERSECT`/`EXCEPT` are unclassified.
- **Additive-only model-diff vs semantic change.** The column/dependency-set diff is derivable, but "did an existing column's semantics change" is not; that residue falls to a declared migration intent whose exact surface is open (`models.md` §Known Divergences).

## References

- **Code**: `crates/smelt-logical/src/analysis/{monotonicity,source_bounds,temporal,mod}.rs` (the trace, bound/reach, frame-reach, partition-alignment, body-structure, driving-fact proofs); `crates/smelt-logical/src/rules/{incremental,cumulative}.rs` (injection-point, nondeterminism taint, algebraic combiner set); `crates/smelt-db/src/queries/monotonicity.rs` (the nullability gate + Salsa wrapper).
- **Tests**: the monotonicity-trace unit tests (`smelt-logical`); the batched per-source bound tests; the cumulative classifier tests.
- **User docs**: the per-mode refresh pages under `docs-site/docs/` consume these properties; no standalone user page (the properties are internal to the analysis layer).
- **Plans (history)**: `docs/plans/20260704-model-updates.md` (the mode-vertical master whose capabilities this spec re-homes).
- **Related specs**: `model_maintenance.md`, `model_transforms.md`, `models.md`, `batched_models.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `materialized_view.md`, `timeseries.md`, `sources.md`, `multi_backend.md`, `schema_evolution.md`.
