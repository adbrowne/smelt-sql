---
feature: model_properties
status: experimental
last_reviewed: 2026-09-05
owners: [andrew]
---

# Model Properties

> **What this is.** The reusable **properties a model's SQL can have** — general, static capabilities of the query itself, each either a **derived proof** (a pure static analysis over the model's SQL that returns a verdict) or a **model-scoped declaration** (a world-fact the modeller states in the model's own SQL/frontmatter). These properties are consumed by incremental maintenance, backfills, schema evolution, and query optimisation, but they are stated **without naming any of those** — a property whose verdict is only meaningful inside one refresh mode does *not* live here. This spec is the **complete catalogue**: a composition consumed by only one feature today still belongs here when its verdict is a provable fact of a model's SQL plus its declared inputs (mutation-sensitivity, skeleton-role, footprint, partition-locality, faithful-fold, grain-alignment, and definition-change classification are exactly this — see §Surface); what stays out is policy, surface, and state machinery. Out of scope, with their own homes: the algebraic *ladder* and the maintainable-vs-delegated cutoff, plus the equivalence invariant and composition contract (`incremental_models.md` — this spec owns only the raw algebraic **discriminants** the ladder reads); the physical mechanisms a property licenses (`model_transforms.md`); the `refresh:` enum, the three-state declaration law, the input-consumption axis, and the litmus rule (`models.md`); the world-facts that live on the source, backend, or core — the timeseries clock (`timeseries.md`), the source mutation profile (`sources.md`), the backend capability flags (`multi_backend.md`) — which this spec only *catalogues by reference* as proof inputs; and each mode's local machinery (`incremental_shapes.md`, and the other mode specs); the plan-level policy that consumes these proofs — which cells demand which of them, the refusal policy, and the diagnostics (`incremental_models.md`).
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
| Event-time monotonicity trace | `EventTimeTrace` = `Traceable{source, source_column, offset, monotonicity}` \| `StaticSeed{reason}` \| `NotTraceable{reason, kind}`: does the projected `event_time` expression trace monotone-non-decreasing to exactly one source column, folding constant `INTERVAL` shifts and constant integer shifts over a monotone, non-temporal partition key (`offset` = `Seconds` \| `Symbolic` \| `Integer`; `monotonicity` = ClickHouse-style `{is_monotonic, is_positive, is_always_monotonic, is_strict}`). `kind` = `Disproven` (the classifier positively knows the shape is not monotone — never widened) \| `Undecidable` (no rule for the shape, e.g. an opaque function — the only kind the declared-monotonicity guarantee below may widen) | built |
| Column nullability gate | downgrades `Traceable → NotTraceable` when the traced leaf column is nullable or its nullability is unknown, so a pushed filter cannot silently drop `NULL` rows | built |
| Unified bound / reach derivation | `BoundResult` = `Bounded{source_partition_col, before, after}` \| `Unbounded` \| `NotDerivable`: the finite backward (`before`) / forward (`after`) reach in seconds a frame or interval band forces around the run window, per source; splits *computation-reach* (derived) from declared *source-lateness* | built |
| Maintained-window / horizon derivation | the clamp bound — the far edge of the maintained window, past which inputs are no longer folded in — composed from the reach (`before`/`after`) and any join contribution, per source. **Derived**, never trusted from a declaration: a declared horizon *ceiling* only warns when the derived value would exceed it and never relaxes the clamp, because an under-estimate would silently drop in-reach rows (`incremental_models.md` §"Windowed maintenance and the horizon"). The horizon-ceiling warning compares against the per-source reach (the row above) today; composing every source's reach plus join contribution into one model-wide horizon number remains the `not-yet` gap | not-yet |
| Injection-point / pushdown-depth | `InjectionPoint` = `Source` (zero-margin transparent slice, push filter to the scan) \| `OuterClamp` (nonzero margin / `Unbounded` / `NotDerivable`): the deepest safe placement for the event-time filter | partial |
| Frame-reach taxonomy | `RANGE … INTERVAL` → derivable reach `k`; `ROWS`/`GROUPS`/bare `LAG`/`LEAD` → `NotDerivable`; `UNBOUNDED` → ∞ | built (RANGE/ROWS/LAG); `GROUPS` conservative |
| Algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone) | raw algebraic facts of the combiner: commutative-monoid membership (`SUM`/`COUNT`/`MIN`/`MAX`/`BOOL_*`/`BIT_*`), whether a contribution must be *un-seen* (needs an inverse), additive vs holistic (`AVG`/variance/approx-distinct decomposable; `MEDIAN`/`MODE`/exact-distinct holistic), and value-monotone (`MIN`/`MAX`/`EXISTS`) vs order-monotone (`MAX_BY`). **Not the ladder** — see §Design | built |
| Partition alignment (scoped) | `PartitionAlignment` = `Aligned` \| `NotAligned{reason}`, judged per scope (`GROUP BY`/`DISTINCT`/window `OVER`): does the scope's key set contain the `partition_column`. Consumed with *opposite polarity* by different modes — a raw containment fact, not a mode verdict | built |
| Driving-fact / anchor resolution | among *joined* inputs, exactly-one-`Traceable` input is the anchor (alias-scoped leaf disambiguation); two or zero → fail-closed. A model may still independently window several sources — that multi-source bound derivation is distinct | built |
| Fan-out / cardinality | `Cardinality` = `OneToOne` \| `OneToMany`: does a join multiply rows / change target cardinality vs enrich in place. Proven from a declared unique key — **composite-valued** (a single column is the one-element list, `sources.md`) — matched against the join's `ON`/`USING` equality as a set; a `CROSS JOIN`, a missing condition, or an equality that matches no declared key (in full, not column-by-column) fails closed to `OneToMany` | built |
| Join-contribution monotonicity | `ContributionVerdict` = `Monotone` \| `Refused{reason}`: a semi-/dimension-join's per-key contribution folds without an inverse (value- or order-monotone) and does not fan into a decrementing aggregate — composed from the inverse-free discriminant + fan-out/cardinality; licenses the dimension-driven horizon MERGE | built |
| Presentation-map purity | a hidden-state presentation map `π(state)` is a pure function of a single consistent state row — reads no other rows, tables, or windows; the soundness condition for a decomposed-state presentation view | built |
| Interval / temporal-join detection | a bounded time band gives the second fact a finite lookback vs an unbounded equi-join | partial |
| Determinism (run vs row) + nondeterminism predicate | the predicate over function names (`RANDOM`/`RAND`/`UUID`/`GEN_RANDOM_UUID`/`SETSEED` are row-nondeterministic; `NOW`/`CURRENT_*` are run-deterministic and pinnable), plus the taint flow that no row-nondeterministic value reaches a skeleton position | built |
| Change comparability | `Comparability` = `Comparable` ⊑ `Incomparable`: is a projected column's value a pure function of the *processed inputs*, and therefore safe to diff against a prior run's stored value. Fail-closed — an unrecognised construct, a `contract: plausible` payload column, and a volatile clock (`NOW`/`CURRENT_*`, whose value tracks run time, not inputs) are all `Incomparable`; nothing defaults to `Comparable` | built |
| Region row identity | `RowIdentity` = `Key{cols}` \| `WholeRow`: what a conditional write joins stored rows to candidate rows on. Precedence is a declared `unique_key`, else the walk's own proven grain key, else the identity-free `WholeRow` fallback (a multiset diff — delete-the-disappeared, insert-the-appeared — expresses no targeted update). Fail-closed: a proven key that does not cover the output (an input join the walk cannot prove row-preserving) is never trusted, not even as a partial key — `WholeRow` is used instead. A declared key and a differing proven key may both be present; the declared key wins, and the disagreement is carried alongside the verdict rather than dropped | built |
| Body-structure classifier | `SelectItemKind` = `CountDistinct` \| `OtherAggregate` \| `GroupByKey`: is a subquery **or CTE** body transparent / group-aligned / order-sensitive (one parse-based check) | built |
| Set-operation distribution | whether a filter distributes over `UNION [ALL]` (branch-wise trace); `INTERSECT`/`EXCEPT` not yet classified | partial (UNION ALL) |
| Static-seed detection | a constant/`NULL` in the `event_time` slot (or `COALESCE(col, const)`) → `StaticSeed`, not a stream | built |
| Additive-only model-diff | a model edit only *adds* columns derivable from `{existing target} ∪ {monotone dim}` → in-place backfill is admissible, not a rebuild | built |
| Window-independence / ordered-execution | a window reads only sources (parallelisable) vs its own prior output (ordered); self-edge detection | built |
| Input-delta discovery | window-forward / snapshot-diff / change-feed, derived from the source's shape — the proof stage of the input-consumption axis (`models.md`); pairs with the mutation-profile world-fact | built |
| Per-column mutation-sensitivity / column provenance | `MutationSensitivity` = two independent kinds per column group: **value** sensitivity (the set of sources whose *post-creation* deltas can change a column's stored value; a reference to the row's own immutable-at-creation skeleton value contributes none) and **membership** sensitivity (the set of mutable sources read in row-admission position — join `ON`, `WHERE`/`HAVING` conjuncts — whose deltas can add or remove already-materialized rows; subqueries in admission position fail closed to collapse). Columns with identical sensitivity sets share a **column group**; a column sensitive to two sources merges their groups (fail-closed — degenerate collapse widens to the whole table, never silently) | partial (both passes are walk-composed — a simple rename or an embedded aggregate/computed-expression reference chases through arbitrarily nested single-use CTEs and derived tables to its base relation for value sensitivity; membership sensitivity scans every scope's own `JOIN`/`WHERE`/`HAVING` conjuncts — the outermost scope and every CTE/derived-table body reachable from it — resolving each conjunct's references against that scope's own alias table, so a mutable dimension joined only inside a CTE still attaches. Composition is uniform attachment: a contribution found in any scope joins the model-level union governing every payload group, not a per-scope-refined subset. Closure-pruning — a proven `Closed` skeleton-source closure (row preservation from the join's own shape, never a declared `referential_integrity` world-fact) removing an enrichment join's own membership contribution — is wired for the model's top-level `SELECT` scope; a nested CTE/derived-table scope's own enrichment join is not yet checked, so its membership contribution always attaches) |
| Affected-key discovery | `AffectedKeys` = `Keys{cols}` \| `NotDiscoverable{reason}`: from a changed input's delta rows, the finite set of output group keys the delta can affect — the model's grain expression evaluated over the delta rows. A **sound over-approximation** (a superset of the true affected keys) is admissible; an under-approximation never is. `NotDiscoverable` when the delta shape cannot be resolved to a key set: an unkeyed retraction, or a grain expression reading a column absent from the delta's own row shape. Consumed by `incremental_models.md` §"The repair family" | partial (proof derived; not yet consumed by plan derivation) |
| Skeleton-role extraction | `SkeletonRole` = `Identity` \| `Grouping` \| `Dedup` \| `Ordering` \| `Payload` per column: does the column occupy a row-membership/identity position (unique key, `GROUP BY`/`DISTINCT` key, dedup/ordering key) or a payload position. Promoted out of the determinism taint-flow's informal "skeleton position" vocabulary into its own reusable classifier | partial (single top-level `SELECT` scope) |
| Footprint reflection / bounded write footprint | `FootprintResult` = `Bounded{output_partition_col, before, after}` \| `Unbounded` \| `NotDerivable`: the write-scope dual of the reach derivation above — does an input delta map to a bounded set of output addresses. A stored trajectory column under late data is the canonical `Unbounded` case | built |
| Partition-locality projection | `LocalityVerdict` = `Local` \| `NotLocal{reason}` per `(cell × source)`: does a derived scan bound (and the reflected write footprint) each project onto a bounded interval of, respectively, the source's and the output's partition column. A cross-axis source (partition column not the output's) is local only via an explicit, derivable predicate relating the two — never an inferred guess | built (partition-addressed outputs; a keyed-grain output with a declared event-time axis poses the same question against that axis; a *bare* keyed output with no declared axis has no output axis to project onto, and gets no footprint claim) |
| Faithful-fold conditions | `FaithfulFold` = a composition of two independent conditions: the delta stream *partitions* the input (an append-only or retraction-free source posture — declared and verified, never derived) **and** the combiner's fold over any sub-multiset of that partition equals the batch aggregate (derived from the algebraic discriminants above). A retraction-carrying feed fails the first condition even when the combiner is invertible; a holistic combiner fails the second even over an append-only feed | built |
| Grain-alignment check | `GrainAlignment` = `Aligned` \| `NotAligned{reason}`: does the declared `timeseries.granularity` (`timeseries.md`) match the SQL's own `GROUP BY`/`date_trunc` grouping. Check-only — the grain itself stays declared; this proof only validates it, never derives or substitutes one | partial (single top-level `SELECT` scope; widen-never-narrow) |
| Definition-change column classification | `DefinitionChangeClass` = `SkeletonAdd{reason}` (refused — a grain change) \| `PureBackfill` (a pure function of already-stored columns, no upstream read) \| `UpstreamRederive` (re-derives from upstream, keyed where the source is keyed): composes skeleton-role extraction (is the new field in a skeleton position), the additive-only model-diff (is the change pure addition), and per-column provenance (does the new field's expression read only stored columns or reach upstream) | built |
| Fingerprint projection | `Projection` = `Columns{cols}` \| `FullRow{reason}`: for a consuming model reading an external source, which of the source's columns feed the model — the projection a row-content fingerprint sidecar digests (`sources.md` §"The fingerprint sidecar"). Fail-closed: `SELECT *`, an opaque function call over the source, or a provenance path the walk cannot resolve yields `FullRow{reason}`, never a guessed subset. Per-consumer — two consumers of the same source derive independent projections, never unioned | partial (simple-rename lineage composes through CTEs/derived tables via the walk; an embedded reference inside a computed expression does not chase further) |
| Skeleton-source closure | `SkeletonSourceClosure` = `Closed` \| `Open{reason}`: does an enrichment join's row skeleton (row membership, count, and multiplicity) trace to the driving source **alone**, so the enrichment side can never add, drop, or duplicate a row. Composes five independently-sourced conjuncts — skeleton-role extraction (no skeleton column originates on the enrichment side), per-column provenance (every enrichment column's mutation-sensitivity is confined to its own source, never blended with the driving fact's), one-to-one join contribution (fan-out/cardinality proves the join `OneToOne`, never `OneToMany`), row preservation (every driving row survives the join — a `LEFT JOIN` always qualifies; an inner/equi-join qualifies only under a source's declared `referential_integrity` world-fact, `sources.md`), and no membership predicate on an enrichment column (a `WHERE`/`HAVING` clause testing an enrichment-side column would make output row-membership depend on the enrichment source, breaking the closure). Fail-closed to `Open` on any unproven conjunct. **v1 restriction**: the enrichment join must sit below any aggregation in the scope — a join feeding a `GROUP BY`/window changes the skeleton question (which rows survive the fold, not which rows survive the join), so join-below-aggregation is `Open` regardless of the five conjuncts | built (v1: non-aggregating enrichment scopes only) |
| Output-delta shape | `OutputDelta` = `AppendOnlyWindow{axis}` ⊑ `KeyedUpsert{keys}` ⊑ `General{reason}`, derived **per column group** (§"Per-column mutation-sensitivity / column provenance"), never per model — what the model *emits* when its inputs change, not what its inputs are. See §"Output-delta shape" | derived, typed onto propagation edges, and acted on by dirt propagation (keyed dirt-sets for an admitted `KeyedUpsert` component) |
| Keyed-succession classification | `Succession` = `Recognized{source, key_cols, clock_col, lead_cols, lag_cols, delete_flag}` \| `NotSuccession{reason}`: is the `FROM` exactly one `append_only` source reference, does every window function in the projection reduce to `LEAD(t)`/`LAG(t)` over the clock (or a scalar expression over one) partitioned by one consistent key and ordered by a strictly-`Traceable` clock, with every other projected column row-local, and at most one trailing `WHERE NOT <row-local boolean>` filter. The one classifier whose vocabulary is window-function/`ORDER BY` shape rather than aggregation — see §"Keyed-succession classification" | not-yet |

### Model-scoped declarations

These are world-facts smelt cannot derive from the SQL, stated in the model's own SQL/frontmatter. Each may only **widen** what the proofs admit — never substitute for a proof's default reject on a construct it cannot decide.

| Declaration | What it asserts | Where declared |
|---|---|---|
| Declared monotonicity guarantee (`timeseries.assert_monotonic`) | the model is monotone where static proof is undecidable (a UDF, an opaque body). The static default is always reject-the-push; this escape hatch widens only an `Undecidable` verdict — a `StaticSeed`, or a `Disproven` verdict (a row-nondeterministic function, a periodic/piecewise construct, an ambiguous or unresolvable leaf) is refused regardless of the declaration | `timeseries:` block (model frontmatter) |
| Functional dependency (`key → column`, `functional_dependencies:`) | a column is a per-key constant, admitting the once-write `COALESCE`/1:1-after-dedup family (`incremental_shapes.md` §"The column-family catalogue" — the once-write provenance proof) for every one of its admitted spellings, including the fallback-bearing and multi-candidate forms whose NULL-preservation is discharged by decomposed state (`incremental_shapes.md` §"Decomposed state (rung 2) in keyed models") rather than by restricting which spellings are provable. Widens only the *undecidable* per-key-constancy verdict — a `determines` column the fan-out/cardinality proof positively proves multi-valued per key (a row-multiplying join into it) is refused regardless of the declaration | model frontmatter |
| Bounded-domain / space budget (`bounded_domain:`) | a column's active domain is bounded by an explicit `max_cardinality`, licensing an exact holistic aggregate (`MEDIAN`/`MODE`/exact `COUNT(DISTINCT)`) via an explicit per-key multiset. **Fail-loud with a cap, never the default**: `max_cardinality` is a required field — an absent cap is a configuration error, not a permissive default — and the declaration widens only the holistic-aggregate case; applied to a monoid/decomposable combiner (which needs no multiset licence) it is refused | model frontmatter |
| Horizon ceiling (`horizon_ceiling:`) | a *warning-only* ceiling on the maintained window's derived horizon — the only declaration in this table that cannot widen anything: the clamp always uses the reach `derive_model_bounds` (below) computes, and a declared ceiling only licenses a compile-time warning when the derived reach would exceed it. Normative home is `incremental_models.md` §"Windowed maintenance and the horizon"; listed here because it is a model-scoped frontmatter declaration like its table-mates | model frontmatter |

### Catalogued inputs (owned elsewhere)

The proofs above consume world-facts that live on the source, backend, or core. This spec does **not** re-home them; it names them so the proof inputs are traceable: the **timeseries clock** (`event_time_column`/`partition_column`/`granularity`, `timeseries.md`); the **source mutation profile** — the structured `mutation_profile:` block (`kind: append_only | mutable_snapshot | change_feed`, plus the `lateness`/`redelivery`/`retractions`/`delta_identity`/`key_recurrence` sub-facts inside it; the bare-string form is shorthand for `{ kind: <value> }`; `sources.md` §"`mutation_profile` — the structured block"); the **source-lateness margin** (`mutation_profile.lateness`, or its standalone-key alias `source_lateness:` — declaring both is an error — the declared term of the reach split, `sources.md` §"Source YAML shape"); the **watermark** (`watermark.complete_through`, or the derived `max(partition_column)` fallback, `sources.md` §"Source YAML shape"); the **backend capability flags** (`multi_backend.md`); and the **refresh selector** itself (`models.md`).

## Semantics

Detailed below are the load-bearing proofs; the rest are as stated by their verdict in §Surface.

### Probe obligation

`sources.md` §Semantics "The trust rule" states, for source-declared world-facts, that a declaration which only *widens* a scan is safe to trust as declared, while a declaration that *narrows* what maintenance reads or licenses a cheaper technique is admitted only paired with a verification mechanism. The same classification governs a **model-scoped** declaration (§"Model-scoped declarations"): a declaration is admissible only if a **probe** exists that can falsify it at run time — no probe, no declaration. A declaration whose widening cannot be revoked by any runtime check (§Constraints "Declared escape hatches may only widen") does not evade this rule by being harmless; it is exempt from it, and the registry below names the exemption and its reason rather than leaving the declaration unaccounted for.

A probe's answer is always the same one-row shape: `violation_count` (the number of offending rows/keys/partitions found, `0` when the declaration holds) and `sample_keys` (up to 5 comma-joined offending identifiers, `NULL`/empty when `violation_count` is `0`) — every emitter in the registry below returns exactly this shape, so the run driver reads one contract regardless of which declaration a probe verifies.

A **probe** is a read-only query the maintenance layer's emitter authors and the run driver dispatches **before** any write the run would otherwise commit — the same before-the-write placement `emit_count_preservation_probe` and `emit_recurrence_bound_probe` already use — so that a disproved declaration rolls back trivially: nothing has been written yet for the probe to have to unwind. A firing probe is always a **named error diagnostic** carrying the violated fact and its remedy (repair or refresh the affected cells); it is never downgraded to a warning and never a silent continue, because the declaration was load-bearing for the technique the run just used — a violation the run failed to report would leave wrong output looking correct. A probe is a runtime check on top of a proof, never a substitute for one: a proof's default reject on an undecidable construct (§Constraints "Proofs are fail-closed") is unaffected by whether a probe exists: the probe only re-verifies, at run time, a fact the declaration already stated and a proof already admitted on the strength of.

The **probe registry** below lists one row per declaration named in §"Model-scoped declarations" (this spec) and per source-side narrowing declaration this spec's proofs consume by name (`sources.md`): its probe, what the probe queries, when it fires, the diagnostic it raises, the operator's remedy, and its default cadence (§"Probe cadence", `smelt_yml.md` §"Top-level keys" `probes:`). A row's **Status** is `built` when the named probe emitter exists in the maintenance layer and is dispatched by a live run; `built (unwired)` when the emitter exists but no live run yet calls it; `not-yet` when no emitter exists. A `not-yet`/`built (unwired)` row is still an admissible declaration — the admissibility rule is that a probe is *specified*, not that every specified probe already runs — but the fact it verifies is unverified in practice until its row reads `built`, exactly as recorded per-row in §Known Divergences.

| Declaration | Probe | What it queries | Fires when | Diagnostic | Remedy | Default cadence | Status |
|---|---|---|---|---|---|---|---|
| `timeseries.assert_monotonic` | Monotonicity probe (`emit_monotonicity_probe`) | re-derives the traced event-time ordering over the run's processed rows, per partition | a processed row's event-time value is out of non-decreasing order relative to its partition predecessor | `DeclaredMonotonicityViolated` | disable `assert_monotonic`, fix the upstream ordering, or `smelt repair` the affected partition | per_run | built |
| `functional_dependencies:` | Functional-dependency probe (`emit_functional_dependency_probe`) | re-aggregates the declared `key` over the run's processed rows and counts distinct `determines` values per key | more than one distinct `determines` value is found for the same key | `DeclaredFunctionalDependencyViolated` | drop the declaration, correct the source data, or `smelt repair` the affected keys | per_run | built |
| `bounded_domain:` | Bounded-domain probe (`emit_bounded_domain_probe`) | counts the distinct values of the declared column within the run's processed region | the count exceeds the declared `max_cardinality` | `DeclaredBoundedDomainExceeded` | raise `max_cardinality`, narrow the domain upstream, or `smelt repair` the affected keys | per_run | built |
| Source `mutation_profile.kind: append_only` posture | Append-only posture probe (`emit_append_only_posture_probe`, watermark-monotonicity + frontier-checksum, `sources.md` §Semantics 4) | the source's recorded per-partition baseline (`source_postures.json`, one row per partition: row count and skeleton-column fingerprint from the last held probe) against the source's current per-partition state; the fingerprint leg is gated to partitions strictly below the recorded maximum partition value — the still-open partition legitimately gains appends, so only its count is checked | a partition's row count decreases, or a closed partition's fingerprint changes under a re-check | `SourceMutationProfileViolated` | correct the source (undo the delete/reload), or `smelt repair` the affected partitions | per_run | built |
| Source `referential_integrity` | Count-preservation probe (`emit_count_preservation_probe`/`emit_count_preservation_probe_from_body`) | the touched region's enrichment-join row count against the driving side's row count | the enrichment join returns fewer rows than the driving side over the touched region | `SourceCountPreservationViolated` | correct or backfill the dimension's missing key, or drop the declaration | per_run | built |
| Source `key_recurrence` | Recurrence-bound probe (`emit_recurrence_bound_probe`) | a merged delta row's key against the run's derived out-of-slice window | a key recurs outside the declared recurrence window | `KeyedRecurrenceBoundViolated` | widen `key_recurrence.window`, or `--full-refresh` the affected model | per_run | built |
| Source `unique_key` / `delta_identity` | Uniqueness probe | duplicate values of the declared key within the consuming run's scan window (full-table via `smelt verify`) | a duplicate key value is found | `SourceUniqueKeyViolated` | correct the source's key generation, or narrow the declared key | per_run default; full-table on demand via `smelt verify` | not-yet |
| `horizon_ceiling:` | exempt | — | — | — | a declared ceiling only ever licenses a compile-time *warning* (`incremental_models.md` §"Windowed maintenance and the horizon"); it never revokes a technique a proof already chose, so there is nothing a runtime probe could falsify | — | exempt |
| `columns.<c>.contract: plausible` | exempt | — | — | — | a column-scoped equivalence contract, not a narrowing licence: it widens what non-determinism the payload-column taint flow admits, refused outright on any skeleton column (`models.md` §"Constraint violations"), so there is nothing a runtime probe could falsify | — | exempt |

### Probe cadence

Probe dispatch is governed by the project-level `probes:` cadence policy (`smelt_yml.md` §"Top-level keys"): `per_run` (default) dispatches every `built` probe on every consuming run; `periodic` dispatches once every `every_n_runs` runs (a model's first consuming run always dispatches); `off` skips dispatch entirely and is recorded on the run so `smelt explain` can report the declaration as unverified this run rather than presenting it as checked. Probe cost — the extra query each dispatched probe adds to a run — is visible in `smelt explain`'s plan rendering alongside the declaration and technique it verifies. Per-declaration cadence override is open (`smelt_yml.md` §Known Divergences).

Two distinct non-dispatch cases exist and are never collapsed into each other. A **policy skip** (cadence `off`, or a `periodic` run that is not the nth) trusts the declaration for that run and records it unverified — the technique it licenses still runs. A probe that **cannot be built** (the rule has no probe SQL to offer, or a declared closure cannot construct one from the model's own body) stays fail-closed exactly as it does without cadence at all: the recurrence-bound route-3 checked merge refuses the run, and the declared-`referential_integrity` route drops its narrowing and falls back to the widened scan. Cadence governs only whether a *buildable* probe runs; it never weakens the probe-unavailable path.

A firing probe's diagnostic carries the violated fact, the maintenance cell the probe was licensing, and the remedy text from the registry row above — never a bare violation count with no route back to what to fix.

### Event-time monotonicity trace

`trace_event_time(expr, ctx)` walks the projected `event_time` expression's AST and returns `Traceable` **only if** the expression reduces to exactly one source column shifted by constant `INTERVAL`s that preserve monotone-non-decreasing order (`is_monotonic ∧ is_positive`). A constant or `NULL` leaf yields `StaticSeed`; anything else yields `NotTraceable{reason, kind}`, where `kind` is `Disproven` for a shape the classifier positively knows is not monotone (a periodic function, a piecewise `CASE`, a run- or row-nondeterministic function, two-column arithmetic, an ambiguous/unresolvable leaf, a non-temporal cast) and `Undecidable` for a shape the classifier has no rule for (an unrecognised function call — a UDF or opaque body). The offset is retained exactly where seconds are exact (`Seconds`) and symbolically where a unit is calendar-variable (`Symbolic("1 month")`). A constant integer shift over a monotone, non-temporal partition key (a sequence id, offset, or watermark column — e.g. the `+ 5` in `batch_id + 5`) folds to `Integer(i64)` instead, signed because such a key has no inherent "always forward" direction the way a temporal interval literal does (`incremental_models.md` §Surface "`partition_column` must be monotone" owns the monotone-integer-partition-key feature this offset serves). The trace is **pure** and **fail-closed**: absence of a proof is `NotTraceable`, never an optimistic default.

`trace_event_time_declared(expr, ctx, declared_monotonic)` is the same trace with the declared-monotonicity guarantee (`timeseries.assert_monotonic`) threaded in: when `declared_monotonic` is `true` and the *only* obstacle to a `Traceable` verdict is an `Undecidable` `NotTraceable` (the unrecognised-function case), it recurses into that function's single column-bearing argument and admits the pushdown (weakening `is_strict`, since the opaque function's own shape is unproven). A `Disproven` verdict or a `StaticSeed` is returned unchanged regardless of `declared_monotonic` — the declaration widens only the one verdict a proof could not decide, never a verdict a proof positively rejected. `trace_event_time(expr, ctx)` is `trace_event_time_declared(expr, ctx, false)`.

The expression-level trace above is lifted through the model's relational operators by the composition walk (§"The composition walk"): `trace_event_time_composed(sql, target_col, ctx, declared_monotonic)` folds `ComposedTrace::Single` (a rename/monotone re-projection through a CTE or derived table) by reducing the outer projection layer onto the inner column's own trace — offsets add, strictness meets, and the outermost grid-truncating layer wins — so a target column that only reaches its source through one or more layers of renaming still traces, not merely a column that names the source's partition column directly. A set-operation root yields `ComposedTrace::Branches`, one verdict per arm in source order, because branches may anchor to different sources with different offsets and no single-verdict reduction exists in general; a `StaticSeed`/`NotTraceable` arm keeps its own refusal in the vector rather than being averaged away. The relational lift only ever widens what the trace can *reach* through composition — it never changes what counts as `Traceable` at a leaf, so a shape the expression-level trace disproves stays disproven at every layer it appears in.

### Column nullability gate

The trace alone does not certify that a filter pushed to the leaf is safe: a nullable leaf column can carry rows a `WHERE event_time ≥ …` would drop. `gate_nullable_leaf(trace, leaf_nullable)` therefore downgrades `Traceable → NotTraceable` when the leaf is nullable or its nullability is unknown. The gate resolves the leaf's nullability through the Salsa schema layer (`trace_event_time_checked`), keeping the pure trace independent of the catalogue.

### Unified bound / reach derivation

For each source ref, `derive_model_bounds(sql, ctx)` computes how far outside the run window that source must be read. The verdict separates two independently-sourced quantities: the **computation-reach** (derived from the SQL's frames, `LAG`/`LEAD` offsets, interval join bands, and `WHERE` interval shifts) and the **source-lateness** margin (declared on the source; default 0). A `Bounded{before, after}` result licenses a widened scan; `Unbounded` and `NotDerivable` forbid a pushed filter and force an outer clamp. The `before`/`after` split names backward and forward reach separately because forward reach interacts with watermark settling, which backward reach does not.

Every `INTERVAL '<value>'` literal this walk encounters is parsed by one shared parser into `Offset::Seconds` (seconds/minutes/hours/days/weeks — uniform durations) or `Offset::Symbolic` (month/year — non-uniform: a month is 28-31 days, a year 365-366). A symbolic literal in a bound-relevant position cannot populate a `Bounded{before, after}` value, so per the fail-closed constraint below it forces `NotDerivable` for that source rather than an approximate fixed-day guess. The same parser backs the driving-fact/anchor trace's constant-shift folding (below), so a `col ± INTERVAL '<n> month'` shift and a `RANGE BETWEEN INTERVAL '<n> month' PRECEDING` frame are classified identically.

Both consumers that need "SQL + source list → bound + injection point" — the planner's pushdown-eligibility rule and the runtime's SQL compiler — call the same entry point (`derive_and_classify_bounds`) rather than re-deriving the bound independently; the planner additionally narrows the source list for UNION/JOIN/derived-table constructs before calling it.

### Algebraic discriminants (the raw facts, not the ladder)

This spec owns the **discriminants** — is-monoid, needs-inverse, decomposable, value-vs-order-monotone — as static properties of the combiner algebra in the SQL. They are raw facts: `SUM`/`COUNT` are commutative monoids that are also groups (invertible); `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` are monoids that are **not** groups (a contribution cannot be un-seen); `AVG`/variance/approx-distinct are decomposable into a richer monoid element; `MEDIAN`/`MODE`/exact-`COUNT(DISTINCT)` are holistic. `MIN`/`MAX`/`EXISTS` are value-monotone (the value moves one way); `MAX_BY` is order-monotone (a semilattice fold whose presented value may switch). The **ordering** of these facts into a ladder, and the maintainable-vs-delegated cutoff, are a maintenance consequence and live in `incremental_models.md` — this spec states only which discriminant each combiner has. The concrete state shape a `decomposable` combiner decomposes into, and how the key-addressed profile stores and hides it, is catalogued in `incremental_shapes.md` §"Decomposed state (rung 2) in keyed models" — this spec proves only the discriminant, never a state layout.

### Driving-fact / anchor resolution

`resolve_join_driving_fact(event_time_expr, alias_sources, ctx)` disambiguates, among the joined inputs of a scope, which single input is the anchor by re-running the monotonicity trace against each alias-scoped source. Exactly one `Traceable` input is required; zero or two or more is fail-closed (the model must be disambiguated by the modeller or is rejected).

### Determinism (run vs row) and the nondeterminism predicate

The nondeterminism predicate classifies a function name as **run-deterministic** (`NOW`/`CURRENT_TIMESTAMP`/`CURRENT_DATE` — one value per run, pinnable at compile time) or **row-nondeterministic** (`RANDOM`/`RAND`/`UUID`/`GEN_RANDOM_UUID`/`SETSEED` — a fresh value per row, unpinnable). The predicate is the shared, mode-agnostic fact; a *taint flow* built atop it verifies that no row-nondeterministic value reaches a skeleton position (event-time, partition, unique-key, or row-membership). A `columns.<c>.contract: plausible` declaration widens the taint flow to tolerate a row-nondeterministic value that flows into the declared payload column `<c>`; declaring it on a skeleton column is refused (`models.md` §"Constraint violations").

### Change comparability

Change comparability asks a narrower question than determinism: not "does this column's value vary?" but "can this run's value be safely diffed against a prior run's stored value for the same processed inputs?" The two questions coincide for most columns but diverge for a clock — `NOW()`'s value is fixed by when the run executes, not by the inputs (`Determinism::Run`; safe to project, and the columns it feeds carry no equivalence promise — `incremental_models.md` §"The equivalence invariant"), yet it is still `Incomparable`, because the value legitimately differs from the value the same row carried last run even though nothing about the processed inputs changed; a compare that trusted it would either manufacture a false change or, symmetrically, mask a real one. `Comparable ⊑ Incomparable` is a two-point lattice, folded per column bottom-up by the same composition walk as every other per-column proof: a column comparable in one set-operation arm and incomparable in another folds `Incomparable`; a column read straight through a CTE carries its defining scope's verdict forward. The leaf rule composes two independently-sourced facts of the projecting expression — first, the determinism verdict below: `Clean` contributes `Comparable`, `Run`/`Row` contributes `Incomparable`; second, a recognised-function check: a call to a function smelt's registry does not recognise (an opaque body, a user-defined function) contributes `Incomparable` even though it is not a known nondeterministic function by name, since smelt has no basis to claim it is a pure function of its arguments — determinism's permissive default for an unrecognised function name (assumed clean, since taint-flow soundness only needs to catch *known* hazards) is exactly the default change comparability cannot inherit. A `contract: plausible` column (`models.md` §"`columns:` — column metadata") is `Incomparable` regardless of what the walk proves about its SQL — the modeller has already stated the column's value need not agree bit-for-bit across an equivalent recomputation, which subsumes not agreeing across runs; this override is applied where the derived verdict meets the model's declared metadata, since the walk itself has no visibility into frontmatter declarations. No consumer reads the verdict yet — a future write-suppression compare is the intended consumer (`incremental_models.md` §Future Extensions "Conditional maintenance without a change feed").

### Region row identity

A conditional write must address the rows it touches — including under the partition grain, where no row identity is otherwise required for an unconditional region overwrite. `row_identity(declared_unique_key, sql)` derives `RowIdentity::Key{cols}` or `RowIdentity::WholeRow` by a fixed precedence: a declared `unique_key` (`models.md` §"Refresh axis") wins outright, since a modeller's own stated identity needs no proof; failing that, the walk's own proven grain key (`PropertyVector.grain`, the same `GROUP BY`/`DISTINCT`-factory key the grain-alignment check and functional-dependency derivation already establish) is used; failing that, `WholeRow` — the identity-free fallback where a conditional write degenerates to a **multiset** diff (delete-the-disappeared, insert-the-appeared) rather than a targeted per-row update. `WholeRow` still suppresses a write for an unchanged row; it just cannot express an update to one, only a delete+insert pair.

The proven-key branch is fail-closed against fan-out: a key the `GROUP BY` factory would otherwise prove is only trusted when the walk also proves no input join fans the output out (`PropertyVector.has_fan_out_join`) — a key that does not uniquely cover every output row is never used, not even as a partial key, since joining stored to candidate rows on a non-unique key would silently corrupt whichever rows share it. A declared key carries no such caveat: it is the modeller's own world-fact, not a derived proof, so it is trusted through a fan-out join exactly as it is trusted anywhere else.

A declared key and a differing proven key may both be present for the same output at once — the declared key always wins the precedence, but the disagreement is never silently dropped: the proven key it overrode is carried alongside the verdict, so a caller (`smelt explain`, a future admission audit) can see that the two facts disagree rather than only ever seeing the winner.

### Keyed-succession classification

`classify_keyed_succession(sql, ctx)` is the leaf classifier the composition walk invokes to
decide whether a model's SQL is the keyed-succession shape — the sole vocabulary this spec has
for `ORDER BY`-carrying window functions rather than aggregation, since every other classifier
above reasons about `GROUP BY`/`DISTINCT` scopes or frame reach, never window-function
succession semantics. It returns `Recognized{source, key_cols, clock_col, lead_cols, lag_cols,
delete_flag}` when every condition below holds, and fail-closed `NotSuccession{reason}`
otherwise — absence of a proof is a refusal, never an approximate or partial admission:

1. **The `FROM` clause is exactly one source reference** (`source`) — no join, CTE, subquery,
   or set operation — and that source declares `mutation_profile.kind: append_only`
   (`sources.md`). A join refuses because nothing today proves an enrichment side cannot add,
   drop, or duplicate a version row (§"Skeleton-source closure" rules a join feeding a window
   `Open`); a non-append-only source refuses because the succession technique folds each event
   exactly once and cannot un-see a retracted one (`incremental_shapes.md` §"Run shape and late
   events").
2. **Every window function in the outermost projection is a succession function.** Each
   `OVER (...)` clause is `LEAD(t)`/`LAG(t)` **over the clock column itself, with the default
   offset of 1 and no default-value argument**, or a scalar expression over exactly one such
   call (an `IS NULL`, a `CASE`, an arithmetic expression), never a plain aggregate window, a
   running total, a ranking function, a `LEAD`/`LAG` over any other column, or one with an
   explicit offset (`LEAD(t, 2)` reaches past the immediate neighbour the maintenance theorem
   patches). Columns derived from a `LEAD` call are collected as `lead_cols`, from a `LAG`
   call as `lag_cols`; the technique patches a new event's predecessor for the former and its
   successor for the latter.
3. **Every succession function shares one `PARTITION BY` key set** (`key_cols`) and **one
   ascending `ORDER BY` column** (`clock_col`). Every `key_cols` member is proven `NOT NULL`
   (a NULL key belongs to no key's sequence). `clock_col` must trace `Traceable` **with
   `is_strict`** under the event-time monotonicity trace (§"Event-time monotonicity trace")
   against the driving source — an `ORDER BY` column the trace cannot certify monotone refuses
   the whole model, since a non-monotone order makes "immediate predecessor" undefined, and
   one it certifies monotone but not strict refuses too, since a clock that can collapse two
   distinct source times onto one value makes the derived `(k, t)` identity collide. A
   descending order or a second sort key refuses: the former swaps which neighbour `LEAD` and
   `LAG` name, the latter makes the order depend on more than the clock.
4. **The key columns and the clock column are each projected row-locally** (possibly under an
   alias), so the derived `(k, t)` identity is recoverable from the presented table and the
   succession `MERGE` can address rows by it. A projection that drops `k` or `t` refuses
   (`SuccessionIdentityNotProjected`).
5. **Every other projected column is row-local** — a function of the current row's own columns
   alone, containing no aggregate and no further window function. A column that mixes
   aggregation into the same projection as a succession window (a would-be
   `SUM(...) OVER (...)` sibling) refuses; that shape is not this pattern
   (`incremental_shapes.md` §"What stays out of this grain").
6. **At most one trailing filter is admitted**, and only in the exact shape `WHERE NOT
   <row-local boolean column>`, applied *after* the window functions rather than filtering the
   FROM-clause input before them — the delete-flag admission (`incremental_shapes.md`
   §"Delete events"). The column must be proven `NOT NULL`: under `WHERE NOT`, a NULL flag
   drops the row from the output exactly as `TRUE` does, so a nullable flag would let a row
   vanish without being recorded as a tombstone. The filtered column becomes `delete_flag`;
   its absence leaves `delete_flag = None`. Any other filter shape, position, or column
   refuses.

The classifier is a **leaf** the shared bottom-up walk invokes over one already-bounded scope's
own projection (the Property composition walk rule, `architecture.md`) — it does not itself
compose through CTEs or set operations; a succession-shaped projection nested inside a CTE or
a `UNION` arm is future work (`incremental_shapes.md` §Future Extensions), and today refuses at
the outer scope rather than being silently missed.

### Per-column mutation-sensitivity / column provenance

For each output column, the provenance walk asks which sources' *post-creation* deltas can change that column's value — a reference that only ever reads a source's row *at creation time* (never revisited) contributes no sensitivity, so an immutable-at-creation join input does not drag its whole column group along. Columns whose sensitivity sets are identical share a column group; a column reachable from two different sensitivity sets merges those groups. This is a raw provenance fact of the SQL plus the sources' declared mutation profiles (`sources.md`) — it names no trigger, no technique, and no plan cell. `incremental_models.md` §"The plan matrix" consumes it as the column-group factoring the rest of the plan indexes by.

Sensitivity has two distinct kinds, and the walk derives both. **Value sensitivity** is the per-column question above: which sources' deltas can change this column's stored value. **Membership sensitivity** is row-scoped: a mutable source read in a row-admission position — an inner-join predicate, a semi-join, any read that decides whether an output row exists at all — can retroactively add or remove rows the model already materialized, even when no select-list expression ever reads that source. Membership sensitivity therefore attaches to *every* column of the rows the admission read governs, and it marks the affected groups as requiring a membership-capable repair: a technique that can create and delete rows, never one that only rewrites column values in place. A mutable join partner whose columns appear only in the `ON` predicate has empty value sensitivity and full membership sensitivity — the two kinds are independent facts, and deriving only the first silently un-maintains the second.

One proof prunes membership sensitivity, per the only-proofs-prune principle (`incremental_models.md` §"Windowed maintenance and the horizon"). An enrichment join whose skeleton-source closure (§"Skeleton-source closure") is proven `Closed` **with row preservation established by the join shape itself** (a provably outer join, never the declared `referential_integrity` world-fact) cannot add, drop, or duplicate an output row, so the enrichment source's `ON` read contributes no membership sensitivity — its deltas are pure value changes, and value sensitivity carries them. The declared-`referential_integrity` route is excluded here: this pruning pass never attempts the declared route in the first place (`RowPreservation::DeclaredReferentialIntegrity` never arises from a `None` referential-integrity input), so a declaration alone can never delete a membership fact — widening this pass to consult the declared route (paired with its own probe dispatch) is a separate narrowing-widening decision, tracked alongside the delta-restriction consumer's own widening (§"Skeleton-source closure" §Known Divergences). An `Open` closure, or any admission read that is not the enrichment join's own equality (a `WHERE`/`HAVING` conjunct, a semi-join, a subquery), attaches membership sensitivity exactly as above — fail-closed.

**Across set-operation arms**, a model whose outermost query is a chain of one repeated set
operator over arms `a0..an` gets a real per-arm verdict instead of collapsing whole-model: output
column names and positions come from `a0`, and each subsequent arm's own select items are matched
to `a0`'s by position, never by name. Value provenance combines positionally per output position
across the arms the operator makes *value-contributing*, and every arm's referenced sources fold
into membership sensitivity wherever the operator couples arms for row existence: `UNION ALL`
takes every arm's own provenance per position, contributing no cross-arm membership coupling (each
arm's rows are independent); `UNION` (distinct) does the same for value provenance but also adds
every arm's referenced sources to membership sensitivity, because deduplication makes one arm's row
able to suppress or admit another arm's matching row; `INTERSECT[ ALL]` takes every arm's provenance
per position and adds every arm's referenced sources to membership sensitivity, because any arm can
decide an output row's existence; `EXCEPT[ ALL]` takes only the first arm's provenance per position
(the subtrahend arms contribute no value) but still adds every arm's referenced sources to
membership sensitivity, because a later arm's row can delete a first-arm row from the output.
"Referenced sources" here is each arm's pre-`contributes` source set — an append-only insert into
an `EXCEPT` right arm still deletes an output row, so the append-only/aggregated-read filter that
governs value sensitivity does not filter the membership leg. A chain whose operators are not all
the same, an arm that is not itself a single `SELECT` (a nested compound arm), an arity mismatch
between arms, or an arm with its own unresolvable provenance collapses the whole model exactly as
today, carrying that arm's own degenerate reasons.

### Affected-key discovery

`derive_affected_keys(delta, sql, ctx)` computes, from a changed input's delta rows, the finite
set of output group keys the repair family (`incremental_models.md` §"The repair family") must
recompute: it evaluates the model's grain expression — the same `GROUP BY`/key-derivation the
walk already proves for region row identity and column-group factoring — over the delta rows and
unions the resulting key values. The result is `Keys{cols}`, a finite key set, or
`NotDiscoverable{reason}` when the delta shape cannot be resolved to one: an unkeyed retraction
(no row identity to project through the grain expression), or a grain expression that reads a
column absent from the delta's own row shape. A **sound over-approximation** — a superset of the
true affected keys — is admissible: recomputing an unaffected key alongside the affected ones
costs extra work, never correctness, since a per-group recompute is idempotent
(`incremental_models.md` §"The repair family"). An **under-approximation is never admissible**: a
missed key would leave stale state for a group the retraction actually touched, silently breaking
the equivalence invariant. Fail-closed, matching every other proof in this spec: absence of a
derivation is `NotDiscoverable`, never an optimistic default. A grain column with no dependency
on the delta's own source contributes no key requirement — the same "no requirement" verdict any
column independent of the changed source gets. When *every* grain column is independent of the
delta's source, the delta names no finite key set at all: the verdict is `NotDiscoverable`, never
an unconstrained (whole-table) key set, because the repair family never widens to a whole-table
repair (`incremental_models.md` §"The repair family").

### Output-delta shape

`output_delta_shape(node, child verdicts, ctx)` derives, per column group (§"Per-column
mutation-sensitivity / column provenance"), the **shape of change a model emits** when one of its
inputs changes — a distinct question from *which* rows are new (input-delta discovery, above) or
*which* keys a delta touches (affected-key discovery, above). The verdict is a three-level lattice,
`AppendOnlyWindow{axis} ⊑ KeyedUpsert{keys} ⊑ General{reason}`: `AppendOnlyWindow` means every
emitted change lands as new rows within a bounded window on the named output axis, never revising
an already-emitted row; `KeyedUpsert` means a change instead revises the row identified by `keys`,
addressable by that key set rather than by position; `General` means neither addressing holds — the
consumer can only treat the column group as arbitrarily rewritten. The lattice is ordered by
addressability, not information content: `AppendOnlyWindow` is the most narrowly addressable shape,
`General` the least. An addressing component travels alongside the shape — a window on the output
partition axis for `AppendOnlyWindow`, a key set for `KeyedUpsert`, whole-table for `General` — and
is what a consuming edge projects (§"The graph layer", `incremental_models.md`).

The verdict is produced by the shared composition walk (§"The composition walk"), never a text
scan, via a table of **transfer rules** keyed on operator family:

| Operator family | Input-shape condition | Output shape |
|---|---|---|
| Base relation (leaf: source or table reference) | the source's declared mutation profile (`sources.md`) | `append_only` with a declared clock ⇒ `AppendOnlyWindow{axis = partition column}`; `change_feed` with a `delta_identity` ⇒ `KeyedUpsert{identity}`; everything else (`mutable_snapshot`, an undeclared profile, a clockless `append_only`, a `change_feed` without `delta_identity`, an unresolved reference) ⇒ `General{reason}` — fail-closed, mirroring `input_delta_discovery`'s fail-closed default: an undeclared profile is never optimistic |
| Selection (`WHERE`/`HAVING` filter) | any | the input shape, unchanged (`AppendOnlyWindow`/`KeyedUpsert`/`General`, whichever the input already is) |
| Projection (column list, computed expressions) | any | the input shape, unchanged (`AppendOnlyWindow`/`KeyedUpsert`/`General`, whichever the input already is) |
| `UNION ALL` | every arm | the meet of the arms' shapes — `AppendOnlyWindow` only if every arm is; `KeyedUpsert` if every arm is `AppendOnlyWindow` or `KeyedUpsert`; `General` if any arm is `General` |
| Keyed aggregation (`GROUP BY k`, `DISTINCT k`) | `AppendOnlyWindow` | `KeyedUpsert{k}` |
| Keyed aggregation (`GROUP BY k`, `DISTINCT k`) | `KeyedUpsert`/`General` | the input shape, unchanged — already keyed or worse, so re-aggregating cannot narrow it back to `AppendOnlyWindow` |
| Join | both sides | the meet of the inputs' shapes (`AppendOnlyWindow` ⊑ `KeyedUpsert` ⊑ `General`, same meet as `UNION ALL`), degraded further to `General` on a proven `OneToMany` fan-out (§Surface "Fan-out / cardinality") |
| Window function | any | `General{reason}` — an emitted row's value can depend on sibling rows outside the addressed window/key |
| Unregistered or unnormalizable operator | any | `General{reason}` naming the operator — the fail-closed default |

A **model reference** leaf takes the referenced model's own derived output-delta verdict where
available, otherwise `General` — the hook a consuming model's fold over an upstream verdict reads
(`incremental_models.md` §"The graph layer"). This resolution holds regardless of whether the
reference is spelled with the `models.` breadcrumb (`smelt.models.<addr>` and `smelt.<addr>` name
the same model); for a bare (unprefixed) name, a declared source of that name takes precedence over
a same-named model verdict, and a name matching neither is `General{reason}` naming both misses.

Fail-closure matches every other proof in this spec: an unregistered or unnormalizable operator
yields `General`, naming that operator, rather than an optimistic default. Widening is never
permitted: a shape may only **degrade** upward through the lattice
(`AppendOnlyWindow → KeyedUpsert → General`) as the tree composes, never recover a narrower
addressing than its inputs prove. One mutable column group therefore never degrades a sibling
group's shape — the per-column-group scoping (§"Per-column mutation-sensitivity / column
provenance") is what keeps a model with one `General` group and one `AppendOnlyWindow` group from
collapsing the whole model to `General`.

### Skeleton-role extraction

`skeleton_role(column, ctx)` classifies each output column into exactly one of `Identity` (a declared `unique_key` member), `Grouping`/`Dedup` (a `GROUP BY`/`DISTINCT`/window-partition key), `Ordering` (an `ORDER BY`/window-order key), or `Payload` (none of the above). This is the same "skeleton position" vocabulary the determinism taint flow (above) and the additive-only model-diff already reason about informally; extracting it as its own classifier gives both — and the definition-change classification below — one shared, reusable notion of "position that decides which rows exist" rather than a private copy each.

### Footprint reflection / bounded write footprint

`reflect_footprint(delta, sql, ctx)` is the write-scope dual of `derive_model_bounds` (above): instead of asking how far outside the run window a *read* must reach, it asks how far a *write* triggered by an input delta must spread across the output's own partition column. The two are structurally the same reach computation run over the model's write-side derivation rather than its read-side one, and share the same three-way verdict shape (`Bounded{before, after}` / `Unbounded` / `NotDerivable`). A stored trajectory column — one whose value can still change arbitrarily far downstream under late-arriving input — is the canonical case where the reflected footprint is `Unbounded`.

A clamp carries the **derived** footprint verdict, never an assumed mirror of its own read reach: a clamp is only ever built once `reflect_footprint` has answered `Bounded` for the axis in question, and the value it stores is that answer, not a value re-derived from the clamp's own `(before, after)` read margins. A clamp whose footprint was never posed — no axis to pose it against — carries no write-scope claim at all, and a consumer reading an absent footprint always widens (`incremental_models.md` §"The graph layer") rather than silently reusing the read-side numbers as a stand-in.

For a **partition-addressed** output the axis is always the output's own partition column, exactly as above. For a **keyed-grain** output the question becomes posable when the model declares a `timeseries:` block: the declared `timeseries.partition_column` is the axis `reflect_footprint` poses the question against, using the same derivation as the partition-addressed case — a bounded reach on that axis derives a footprint, a stored trajectory column over that axis still reflects `Unbounded`, and a source's read bound that fails to derive still yields `NotDerivable`. A *bare* keyed output (no declared `timeseries:`) has no axis to pose the question against at all; its clamps still build off the read bound alone, exactly as before, but carry no footprint claim.

### Partition-locality projection

`locality_verdict(cell, source, ctx)` composes the read-side bound (`derive_model_bounds`) and the write-side footprint (footprint reflection, above): a cell is local in a source when both project onto a bounded interval of that source's, respectively the output's, partition column. A source whose partition column is not the output's own axis (a cross-axis source) is local only through an explicit, derivable predicate connecting the two columns — smelt does not infer a relationship between two differently-named timestamp columns, so the absence of such a predicate is the absence of a link, never a zero-cost default. This is the *proof* consumed by `incremental_models.md` §"Partition-local maintenance" (the K8 guardrail); the policy layer — defaults, `allow_full_scan`, `max_lookback`, the `MaintenanceScanUnbounded` diagnostic — stays there.

### Faithful-fold conditions

`faithful_fold(combiner, source_posture, ctx)` composes two independently-sourced conditions into one verdict: (1) the delta stream **partitions** the input — every row lands in exactly one delta, none are retracted-then-relanded without a matching retraction in the stream — a fact of the source's declared and verified mutation posture (`sources.md`), never derived from the model's SQL; and (2) the combiner's fold over any sub-multiset of a partition equals the fold over the whole partition — a fact of the algebraic discriminants above (a commutative monoid needs no inverse to satisfy this; a holistic aggregate never does). Both conditions must hold for a fold technique to be admissible, and either alone refuses: a retraction-carrying feed fails (1) even when the combiner is invertible, and a holistic combiner fails (2) even over an append-only feed. `incremental_models.md` §"Per-cell admission" obligation 2 cites this row rather than restating the two conditions.

### Grain-alignment check

`grain_alignment(declared_granularity, sql, ctx)` validates the declared `timeseries.granularity` (`timeseries.md`) against the model's own `GROUP BY`/`date_trunc` grouping, when the body aggregates. This is check-only, mirroring the horizon-ceiling declaration's posture: the propagation grain always stays the declared value (deriving it would let a projection refactor silently change downstream scheduling — `incremental_models.md` §Design "Grain is declared"), and this proof exists only to catch a declaration that disagrees with what the SQL actually groups by.

### Definition-change column classification

`classify_definition_change(added_column, sql, ctx)` composes three of the proofs above into the verdict `definition_deltas.md` §"The verdict per column group" needs: skeleton-role extraction decides whether the added column lands in a skeleton position (`SkeletonAdd` — refused, a grain change, never a column backfill); if not, the additive-only model-diff confirms the change is a pure addition; and per-column provenance decides whether the new column's expression is a pure function of already-stored columns (`PureBackfill`, admits an in-place `UPDATE`, no upstream read) or reaches upstream (`UpstreamRederive`, admits a column-scoped `MERGE`, keyed where the source is keyed). The plan-level policy — which technique each verdict maps to, group convergence, the `MaintenanceSkeletonChanged` diagnostic — stays in `definition_deltas.md`; this row is the classification itself.

### Skeleton-source closure

`skeleton_source_closure(scope, ctx)` decides whether an enrichment join's output rows are entirely accounted for by the driving (fact) side — the licence a delta-restricted recompute needs to skip rows whose enrichment inputs did not change (`model_transforms.md` — delta-restricted enrichment join) without also having to prove the driving side's own delta separately covers every row the enrichment side might have touched. The five conjuncts are checked independently and each names its own owning proof: skeleton-role extraction (§"Skeleton-role extraction") for the no-skeleton-on-enrichment-side leg; per-column mutation-sensitivity (§"Per-column mutation-sensitivity / column provenance") for the confined-provenance leg; fan-out/cardinality (§Surface) for the one-to-one leg; a declared `referential_integrity` world-fact (`sources.md`) or a provably outer join for the row-preservation leg; and a syntactic scan of the scope's `WHERE`/`HAVING` predicates against the enrichment alias's columns for the no-membership-predicate leg. Any conjunct that cannot be proven — not just one that is positively disproven — yields `Open`: the closure is an *and* of five provable facts, not a default with exceptions. The v1 aggregation restriction (join-below-aggregation ⇒ `Open`) is a scope restriction, not a sixth conjunct — a later widening would extend the proof to reason about the fold's own row-preservation instead of ruling the shape out entirely.

A `Closed` verdict names **which route** proved conjunct 4 (row preservation): `JoinShape` for a provably outer join, needing no further runtime check, or `DeclaredReferentialIntegrity { source }` for a declaration-licensed inner/equi-join. The route is not cosmetic — it carries an obligation. A `Closed { DeclaredReferentialIntegrity { .. } }` verdict may license a narrowing consumer (a delta-restricted recompute) **only** paired with that consumer dispatching the count-preservation probe (§"Probe obligation") over the touched region *before* trusting the narrowing; an unbuildable probe drops the narrowing back to the widened scan rather than proceeding on an unverified declaration. A `Closed { JoinShape }` verdict carries no such obligation — the join's own shape, not a declared world-fact, is what proves the row-preservation leg.

A model-edge creation cell's own P1 verdict is not one join's closure but an **AND over every enrichment relation actually joined in the scope** — each upstream model edge *and* each external source joined there, every one judged with its own declared facts (a model edge's declared `unique_key` for conjunct 3; an external source's declared `unique_key` for conjunct 3 and its declared `referential_integrity` for conjunct 4). The shared verdict is `Closed` only when every joined relation's own closure is `Closed`; an unproven join against any one of them — a model edge or an external source — fails the whole AND to `Open`, naming that relation, exactly as an unproven model edge already did before external sources joined the same AND.

### Fingerprint projection

`fingerprint_projection(model_sql, source_ref, ctx)` derives, for a consuming model reading an
external source, exactly which of the source's columns feed the model's output — the column set a
row-content fingerprint sidecar (`sources.md` §"The fingerprint sidecar") digests instead of the
row's full content, so a source-side edit outside the projection never dirties the consumer. The
projection composes per-column provenance (§"Per-column mutation-sensitivity / column provenance")
restricted to the one source ref in question. Fail-closed to `FullRow{reason}`: a `SELECT *` read
of the source, an opaque function call (`smelt.extern`, a UDF) applied over the source's row, or a
provenance path the walk cannot resolve to a concrete column set all yield the full-row digest
rather than a guessed subset — an under-projection would let a real content change go undetected,
the exact hazard the sidecar exists to avoid. The projection is derived **per consuming model**,
never per source: two models reading the same source with different column sets get independently
derived projections; a wider projection at one consumer never widens the digest computed for
another (`sources.md`'s projection-identity namespacing is what stores this correctly).

### Interactions

- **Input-consumption axis** (`models.md`): input-delta discovery is the proof stage of that cross-cutting axis; the mutation-profile world-fact and the re-scan/probe transform are its other two stages. This proof derives *which* rows are new; it never changes what the stored relation means.
- **The algebraic ladder** (`incremental_models.md`): consumes the discriminants above as its ordering criterion. The ladder is not defined here.
- **The maintenance plan** (`incremental_models.md`): the plan matrix's column-group factoring, per-cell admission obligations 2/4/5/6/7, the K8 partition-locality guardrail, the repair family (obligation 7, affected-key discovery), and the definition-delta trigger's classification (`definition_deltas.md`) all consume the eight proofs above by name; the plan owns only which cells demand which proof and what a verdict costs (technique choice, refusal diagnostics), never the proof's own logic.

### The composition walk

Every **composition-relevant** verdict — any verdict that can differ between a construct in isolation and the same construct nested under another operator — is computed by a **single shared bottom-up fold over the model's logical operator tree**. Each property contributes a *transfer function* `(operator, child verdicts) → verdict`; the walk applies every registered transfer function in one pass, carrying a per-node **property vector** (per-column facts keyed by the node's output columns, per-relation facts on the node itself, per-source facts keyed by source ref). The composition forms are not per-property ad-hoc rules; they fall into two shared shapes:

- **Series/parallel (tropical) composition** — reach and pushdown margins **add along a sequential path** (a stacked window frame, a chained join band) and take the **max across parallel branches** (set-op arms), with the reject verdicts (`NotDerivable`/`Unbounded`/`Refused`) as absorbing elements.
- **Monotone lattice folds** — taint (`clean < run < row`), output-delta shape (§"Output-delta
  shape": `AppendOnlyWindow ⊑ KeyedUpsert ⊑ General`), grain/key sets, alignment carriage, and
  functional-dependency sets compose as monotone transfer rules whose whole-tree verdict is the
  fold's fixed point.

A property implementation must not re-derive composition by scanning the query text or a single clause in isolation: a flat scan cannot express series composition (stacked frames must *add* reach — merging with max under-derives the scan) and cannot see scope nesting (a scope inside a CTE body must be judged by the same rule as the same scope at the top level). Per-clause and substring classifiers remain admissible only as **leaf-level classifiers invoked by the walk** (e.g. the interval parser, the function-name determinism predicate) or as **advisory heuristics that never feed admission** (e.g. batch-size estimation). Fail-closure is preserved by construction: an operator with no registered transfer rule for a property yields that property's reject verdict for the subtree above it.

A node of the operator tree is: a CTE body, a set-operation arm, a derived table (including a redundantly-parenthesized one, `FROM ((SELECT …)) AS t`), the members of a parenthesised join group (`FROM (a JOIN b ON …)`, flattened into the enclosing scope's join inputs), and the body of an expression-position subquery — a scalar subquery, an `EXISTS (…)`, an `IN (…)`, or a quantified `ANY`/`ALL`/`SOME (…)` — appearing in the enclosing scope's own select list, `WHERE`, `HAVING`, `QUALIFY`, or `ORDER BY`. An expression-position scope composes as a scope of its own: its verdict is folded and made available to every transfer function like any other child, with the per-property consumption rule (what a property does with that verdict) specified in that property's own section.

Two properties' consumption rules are stated here since they share the same shape rather than
belonging to either property's own section:

- **Bound/reach.** An expression-position scope is a *read* of its sources: its verdict merges into
  the enclosing node's per-source map exactly like an input's (parallel max-merge), so a source
  read only through a scalar subquery or an `EXISTS`/`IN`/quantified filter is reachable, and a
  window frame inside the subquery body contributes reach the same way a frame in the outer scope
  would. It does **not**, however, participate in the enclosing node's join-sibling slack
  computation (the chained-band carry between two relations tied by *this scope's own FROM join
  graph*): an expression-position scope is not joined into that graph at this node — a correlated
  one is a per-outer-row computation, not a join partner sharing row cardinality with the FROM
  inputs — so folding it in would spread its own reach onto an unrelated FROM input's margin. This
  makes the verdict equal by construction to the same subquery rewritten as an *uncorrelated*
  cross-joined derived table (the shape criterion 1's oracle test uses); a correlated subquery's
  only valid literal rewrite is a `LATERAL` join, which the walk does not model as a distinct
  input shape, so equality with that form is not claimed.
- **Grain (key set, per-column determinism, per-column change-comparability).** An expression-position
  scope contributes **no key, no output columns, and no fan-out** to the enclosing scope — a filter
  subquery cannot establish a key, and a scalar subquery yields one value, not a joined row. It
  contributes two things: its `has_set_op_barrier` bit (a `UNION`-bodied subquery body propagates
  the barrier, matching the inlined derived-table form), and, for the select item whose expression
  embeds it, per-column determinism/comparability: that item's verdict is the max of its own
  syntactic verdict — computed excluding the subquery's own subtree, since that subtree is a walk
  node judged on its own — and the worst per-column verdict of the matching expression scope's own
  property vector.
- **Partition skew and footprint-trajectory.** An expression-position scope composes into both
  exactly as any other child does. Skew folds the scope's verdict into the enclosing node's fold by
  `Skew::union` (max before, max after) — a Form B relation living inside a scalar/`EXISTS`/`IN`/
  quantified subquery body can push rows into a neighbouring partition just as one in a
  `FROM`-position derived table can, so there is no carve-out: unlike bound/reach this transfer has
  no join-sibling slack computation to protect from an unrelated scope's contribution. Footprint
  trajectory folds by parallel OR, but only for a scope whose value **flows into a stored output
  column** — a running fold over the output axis inside a select-list scalar subquery still makes
  the model a trajectory, but the same fold inside a `WHERE`/`HAVING`/`QUALIFY`/`ORDER BY` scalar,
  `EXISTS`, `IN`, or quantified subquery never becomes a stored column of the enclosing scope, so it
  contributes no trajectory (only filters rows or orders them). Both folds are widening — the
  conservative direction is *more* skew / *more* trajectory — so admitting either only over-approximates,
  never under-derives.
- **Keyed-admission presence verdicts (window-function presence, non-deterministic-call
  presence).** The two `grain: key` admission checks (`incremental_shapes.md` §"Key-grain
  codes" `KeyedForbidsWindowFunctions`/`KeyedForbidsNondeterministic`) compose as **parallel OR
  over the whole children slice** (`ctes ++ inputs ++ expr_scopes`) — an `OVER` clause or a
  non-deterministic call anywhere in the model's scope tree refuses, since the keyed state is
  the window and row identity must not be time-of-run-dependent regardless of which scope the
  construct sits in. Each node is classified over its own region only — the leaf classifier
  never descends into a child that is itself a walk node — so a scope's contribution is counted
  exactly once.

## Design

**Properties are named for what they are, not for maintenance.** A monotonicity trace, an algebraic discriminant, a partition-alignment signal, an additive-only diff are each useful well beyond the refresh modes — backfills, schema evolution, query optimisation — so they live in a capability spec keyed on the SQL property, not filed under any one consumer. This is what lets a single proof serve several consumers without a private copy per mode.

**Placement is definitional, not consumer-counted.** A capability belongs here iff its verdict is stateable without naming a refresh mode. Pushdown-depth, used only by `batched` today, is a SQL property and lives here; backfill chunking, meaningless outside batched execution, stays in `incremental_shapes.md`. This gives every capability exactly one home — what lets `smelt:validate` catch a spec that silently re-describes it — without a mechanical ≥N-consumer rule. Because these properties are broadly useful and cheap to name, building one before a second consumer exists is fine.

**Discriminants here, ladder in maintenance.** The cut between this spec and `incremental_models.md` is exactly the cut between a *raw algebraic fact* and its *maintenance consequence*. Is-monoid / needs-inverse / decomposable / value-vs-order-monotone are facts of the SQL; the ordering `monoid ⊂ decomposed ⊂ group ⊂ multiset` and the maintainable/delegated cutoff are what those facts *imply for maintenance*, so they live with the equivalence invariant, not here. Splitting it this way keeps the discriminants reusable by query optimisation and schema evolution, which do not care about the ladder.

**Derive where decidable, declare where not.** A property is a **derived** proof where it is statically decidable and a **declared** world-fact otherwise (the three-state law, `models.md`). This holds generally: upstream source changes are outside smelt's control, so when smelt cannot derive a world-fact about a source, declaring it is the honest fallback rather than guessing. Event-time monotonicity is derivable in the common case and declared only as an escape hatch; source append-only-ness is derivable only narrowly (an immutable clock, no delete path) and otherwise declared on the source; additive-only model-diff is derivable from the column/dependency diff, but "did an existing column's semantics change" is not and falls to a declared migration intent.

**Proofs are validators, never choosers.** A proof returns a verdict; it never picks a refresh mode or silently switches strategy. The declared mode is authoritative and the machinery only proves or refuses it (`incremental_models.md` §"Validator, not chooser").

**Two temporal walks by design.** `EffectiveWindow` (day-granular, batch sizing) and `BoundResult` (second-granular, pushdown) answer different questions with deliberately different fail-closure: the former treats a bare `LAG`/`LEAD` as a bounded estimate, the latter refuses it (`NotDerivable`). Collapsing them would lose one of the two properties, so they stay separate; a per-consumer strictness parameter was considered and rejected as a merge that hides which question is being asked (decided 2026-09-04, `docs/research/20260904-decision-track.md`).

## Constraints & Invariants

- **Proofs are fail-closed.** An undecidable construct yields the reject verdict (`NotTraceable` / `Unbounded` / `NotDerivable` / `NotAligned`), never an optimistic default. Absence of a proof is a rejection, not a pass.
- **Declared escape hatches may only widen.** A model-scoped declaration (declared monotonicity, `columns.<c>.contract: plausible`, functional dependency, bounded-domain budget) may only *widen* the set a proof admits; it may never substitute for a proof's default reject on a construct the proof itself cannot decide, and never narrow eligibility.
- **No narrowing declaration without its probe.** A model-scoped declaration that licenses a cheaper technique than the undeclared default is admitted only paired with a named probe that can falsify it at run time (§"Probe obligation"); a declaration with no probe row (or exempt row, with reason) in that section's registry is inadmissible. The model-scoped twin of `sources.md` Constraint 8 ("No narrowing declaration is consumed without its verification mechanism").
- **One home per property.** Each property's normative verdict is defined once, here. A mode spec references it by name and must not re-specify it. The algebraic *ladder* (as opposed to the discriminants) is **not** defined here — it is owned by `incremental_models.md`.
- **This spec is the complete catalogue.** Every derived-proof verdict any other spec consumes has a row here, including a composition consumed by only one feature today (§"Surface"). A new proof introduced by any spec must land its row here first; consumer specs (`incremental_models.md` above all) reference rows by name and never restate verdict logic inline — a consumer spec may state which cells demand a proof, the refusal policy, and the diagnostics, but not the proof's own derivation.
- **Placement criterion.** A capability whose verdict names a refresh mode does not belong in this spec. Mode-only capabilities stay in the mode spec: batch-safety roll-up, column-locality, event-time outer-visibility, backfill chunking, run/partition granularity alignment (`incremental_shapes.md`); reprocessing detection, once-write verification (`incremental_shapes.md`, whose single classifier now consumes these discriminants for every keyed column family — running-aggregate, latest-value, and milestone alike, in place of what would once have been three per-mode classifiers); engine-incrementalizability (`materialized_view.md`). (Presentation-map purity is *not* mode-only — its verdict is stateable without naming a mode, so it is a derived proof above, not an exclusion.)
- **Catalogued inputs are not re-homed.** The timeseries clock, source mutation profile, backend capability flags, and refresh selector are declared in their existing homes; this spec only references them.
- **Declared lateness is orchestration-only.** `mutation_profile.lateness` is never an input to any proof or to plan derivation: it never widens a scan window or an effective window, never gates a probe, never licenses or refuses a technique, and never changes emitted SQL. It is consumed by scheduling and staleness (`--auto`, when a window is treated as final) and printed by `smelt explain` as a world-fact. Late rows are caught by what actually landed — observed deltas, the ledger frontier and the probes — so the append-only posture probe classifies a row-count *increase* in a closed partition as a late arrival to re-process, never as a posture violation, without consulting lateness; only a decrease or a fingerprint change is a violation. Decision record: `docs/research/20260904-decision-track.md`.
- **Composition happens in the walk, not in scans.** A composition-relevant verdict is produced by the shared bottom-up property walk (§"The composition walk"). Per-clause or substring scans are admissible only as leaf-level classifiers invoked by the walk, or as advisory heuristics that never feed admission.

## Known Divergences / Open Questions

Live gaps between this spec and the implementation, as of `last_reviewed`. Completed work is not
recorded here — history lives in git and §References → Plans.

- **Several declared proofs have no consumer wired yet.** `functional_dependency_verdict_over_vector`
  and the once-write *enrichment transform* remain unconsumed; `bounded_domain:` has no consumer
  (the multiset maintenance transform); `horizon_ceiling:` is checked but never narrows the bound
  used for the clamp; fan-out/join-contribution-monotonicity has no consumer (F15's
  dimension-driven horizon MERGE); window-independence has no consumer (the ordered-backfill
  chunker); change comparability has no consumer (a future write-suppression compare); region row
  identity has no write emitter or admission rule consuming it; the whole-model property vector
  (`model_property_vector`) has no consumer. Tracked: `docs/plans/20260704-model-updates-l3-declarations.md`,
  `docs/plans/20260707-property-composition-walk.md`.
- **The join driving-fact/anchor resolution the event-time monotonicity trace consumes is not
  migrated onto the composition walk.** `resolve_join_driving_fact` runs its own single-level
  FROM-clause traversal rather than the shared walk, and its verdict feeds admission (unlike the
  deliberately separate, advisory-only `EffectiveWindow` walk in `analysis/temporal.rs`,
  `architecture.md` §"Property composition walk rule"). Expression-position subquery bodies,
  by contrast, are enumerated as walk nodes and bound/reach, grain (key set, determinism,
  comparability), partition skew, and footprint-trajectory all consume their verdicts
  (§"The composition walk"). Tracked: `docs/plans/20260707-property-composition-walk.md`.
- **`compute_effective_window` still sums declared lateness into the lookback** — §Constraints
  "Declared lateness is orchestration-only" forbids any proof from reading it; the summation
  survives (its output feeds batch fields no execute path consumes, so it is inert today) and
  must be removed rather than wired further. Scheduled:
  `docs/outcomes/20260904-decision-residue/outcome.md`.
- **`INTERSECT`/`EXCEPT` are unclassified for filter distribution** — their arm scopes are judged
  by the admission walk only; this is independent of the per-arm mutation-sensitivity combination
  rule (§"Per-column mutation-sensitivity / column provenance" "Across set-operation arms"), which
  is classified. A mixed-operator chain, a nested compound arm, or an arity mismatch across arms
  still collapses whole-model for mutation-sensitivity too.
- **Additive-only model-diff can't detect a semantic change under an unchanged expression (Open Question)**
  — whether an existing column's meaning changed is not derivable from the column/dependency-set
  diff alone; falls to a declared migration intent whose exact surface is open. Cross-ref
  `models.md` §Known Divergences.
- **Two FD-backed readers build their `JoinContext` empty because their callers hold no declared
  source facts to populate it with.** `rules/cumulative.rs`'s once-write route and
  `maintenance/locality.rs`'s key-temporal-locality route 2 each call
  `model_property_vector`/`functional_dependency_verdict_over_vector` with a literal
  `JoinContext::new()`; this is a correct fail-closed default today (neither is a
  model-edge/repair admission route), not a live admission gap, but widening either needs its
  caller to gain declared-fact access first. Tracked:
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Fingerprint projection (P4) has no consumer yet** — the sidecar build and digest compare are a
  later phase's scope. Tracked: `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **The append-only posture probe does not yet distinguish a late append from a violation** — a
  row-count increase in an already-closed partition can fire the frontier gate as if it were a
  reload. §Constraints "Declared lateness is orchestration-only" requires an increase to be
  classified as a late arrival to re-process (an observed delta), with only decreases and
  fingerprint changes counting as violations; lateness is not consulted either way. Scheduled:
  `docs/outcomes/20260904-decision-residue/outcome.md`.
- **`SourceUniqueKeyViolated` remains the one probe-registry row with no emitter at all (Open Question)**
  — whether it needs its own emitter or should fold into a general uniqueness probe family is
  undecided.
- **Output-delta shape is derived, typed onto propagation edges, and acted on by dirt
  propagation, but the keyed dirt-set remains symbolic.** Verdicts fold across model references in
  the real workspace graph (`build_forward_graph`), and `classify_keyed_edges` reads
  `Edge.components` to route an admitted `KeyedUpsert` component through the keyed dirt-set
  channel; the residual gap is that the keyed dirt-set is a symbolic key-addressed channel (key
  columns + provenance), not a materialised key-value set — value-level affected-key discovery
  stays with the run-time mechanism. Cross-ref `incremental_models.md` §Known Divergences "The
  contract, plan, and graph layer".
- **The grammar boundary between `columns.<c>.contract` and a future column `tests:` block is
  deliberately deferred (Open Question)** — cross-ref `models.md` §Known Divergences decision 8.
- **A source read only as an argument to a table-valued function call is invisible to the
  composition walk.** `smelt.functions.<fn>(source => smelt.silver.x, …)` normalizes to a single
  opaque leaf named after the call itself; the walk never descends into the call's argument list,
  so a source passed that way (the real shape `examples/web_analytics/models/silver/sessions.sql`
  reads its upstream through) never becomes a walk leaf under any tree shape. Bound derivation
  (`derive_model_bounds`) covers the gap with a per-source whole-text fallback classified as a leaf
  classifier (`docs/specs/architecture.md` §"Property composition walk rule"); every other walk
  consumer (grain, footprint, cross-axis links, the driving-fact/anchor resolution) does not.
  Tracked: `docs/outcomes/20260904-walk-migration-residue/outcome.md`.

## Future Extensions

Ideas that are **not decided**; nothing here is surface or may be implemented against until it
graduates into §Surface/§Semantics via its own spec diff.

- **Skeleton-source closure through a fold.** Closure v1 is deliberately restricted to
  non-aggregating enrichment scopes: a join feeding a `GROUP BY`/window is `Open` regardless of
  the five conjuncts, because a fold's own row-preservation is separate, harder reasoning. The
  restriction is the decided v1 boundary (2026-09-04, `docs/research/20260904-decision-track.md`).
  Trigger for revisiting: a real model refused for exactly this reason.

## References

- **Code**: `crates/smelt-logical/src/analysis/{monotonicity,source_bounds,temporal,mod}.rs` (the trace, bound/reach, frame-reach, partition-alignment, body-structure, driving-fact proofs); `crates/smelt-logical/src/analysis/model_diff.rs` (additive-only model-diff); `crates/smelt-logical/src/analysis/{faithful_fold,footprint,locality_projection,definition_change}.rs` (the faithful-fold conditions, footprint reflection, partition-locality projection, and definition-change classification proofs); `crates/smelt-logical/src/rules/{incremental,cumulative}.rs` (injection-point, nondeterminism taint, algebraic combiner set); `crates/smelt-db/src/queries/monotonicity.rs` (the nullability gate + Salsa wrapper).
- **Tests**: the monotonicity-trace unit tests (`smelt-logical`); the batched per-source bound tests; the cumulative classifier tests; the §"Probe obligation" registry gate (`crates/smelt-logical/tests/probe_obligation.rs`); the fact-violation conformance pool (`crates/smelt-cli/tests/maintenance_conformance/fact_violations.rs`) — one recipe per `built` registry row, driven through conforming and violating feeds and, where end-state observable, a `probes: {cadence: off}` leg proving the probe is load-bearing.
- **User docs**: the per-mode refresh pages under `docs-site/docs/` consume these properties; no standalone user page (the properties are internal to the analysis layer).
- **Plans (history)**: `docs/plans/20260704-model-updates.md` (the mode-vertical master whose capabilities this spec re-homes); `docs/plans/20260707-maintenance-plan-impl.md` (the maintenance-plan tracer and its first classifiers); `docs/plans/20260808-derived-maintenance-proofs.md` (the derived faithful-fold, footprint, locality, and definition-change proofs).
- **Design research**: `docs/research/20260707-property-composition-overview.md` (the per-operator transfer rules and composition algebra behind §"The composition walk", with nine per-property companion docs); `docs/research/20260723-scd2-succession-pattern.md` (§"Keyed-succession classification").
- **Related specs**: `incremental_models.md`, `model_transforms.md`, `models.md`, `incremental_models.md`, `incremental_models.md`, `incremental_models.md`, `materialized_view.md`, `timeseries.md`, `sources.md`, `multi_backend.md`, `schema_evolution.md`.
