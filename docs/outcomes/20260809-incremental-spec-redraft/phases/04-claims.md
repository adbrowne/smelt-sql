# Phase 4 claim inventory — Shape profiles through Interactions (spec lines 1195–1851)

Diagnostic codes named in range (must survive unchanged): `EventTimeColumnNotVisibleAtOuterSelect`,
`KeyedMultipleDrivingSources`, `KeyedRecurrenceBoundViolated`, `KeyedReprocessedWindow`,
`KeyedRetractableContribution`, `KeyedSnapshotSourceUnsupportedColumn`, `KeyedUnknownCombiner`,
`MaintenanceNoAdmissibleTechnique`, `MaintenanceReachNotDerivable`, `MalformedTimeseries`,
`PartitionGrainNotSafe`.

Grading key: preserved / weakened / lost / strengthened, assigned by the adversarial-verify pass.

## Shape profiles (intro, 1195–1212)

1. A maintained model composes properties, transforms, world-facts, output shape, and scope maps
   owned across the spec set (not restated locally).
2. Every profile section (and `materialized_view.md`) opens with a composition table naming:
   required properties, consumed world-facts, default-plan transforms, invariant specialisation.
3. A profile's normative content is exactly its composition table plus its own local machinery;
   it never re-specifies a capability a capability spec or shared section already owns.

## The partition grain — intro + composition table (1214–1229)

4. The partition-addressed shape: a complete table with a monotone `partition_column`, kept
   current by partition DELETE+INSERT (the recompute-a-region corner) as default plan.
5. Declared surface is §Surface "Partition-grain declaration".
6. Composition table names: output shape/grain owner (`models.md` §"Refresh axis"); the full
   required-properties list (event-time monotonicity trace, column nullability gate, unified
   bound/reach derivation, frame-reach taxonomy, injection-point/pushdown-depth, scoped partition
   alignment, driving-fact/anchor resolution, determinism+nondeterminism predicate+taint,
   body-structure classifier, set-operation distribution, static-seed detection,
   window-independence/ordered-execution) — all in `model_properties.md`.
7. Consumed world-facts: timeseries clock (`event_time_column`/`partition_column`/`granularity`),
   source mutation profile + lateness margin, column-scoped equivalence contract
   (`columns.<c>.contract`) — owned by `timeseries.md`/`sources.md`/`models.md`.
8. Default plan transforms: source-filter pushdown, partition DELETE+INSERT, output-window
   derivation (partition-column skew inversion), outer output-clamp, two-layer widened-scan +
   exact output clamp, compile-time pinning — all in `model_transforms.md`.
9. Admission: every check is one instance of §"Per-cell admission" for the recompute-a-region
   corner over a partition-grain output (§"Safety checks").
10. Invariant upheld: per-partition equivalence, the partition-grain strengthening of the
    equivalence invariant and the plan's `S`-vector refinement.

## Execution model (DuckDB, current) (1231–1264)

11. For run window `[start, end)` the recompute corner drives four transforms: partition DELETE
    over the derived output window, outer output-clamp, source-filter pushdown, INSERT.
12. Derived output window = run window pushed through the declared partition-column relation:
    identity when `partition_column` tracks event time; skew-inverted (Form B relation) when
    `partition_column` is derived and skews from the driving date column.
13. Write-rebasing example: session keyed by `session_start_date`, `before = after = 1 day`, run
    `[D, D+1)` → output window `[D−1, D+2)`; DELETE must cover every partition the INSERT writes,
    including the skew-reached prior-day partition, else that partition strands stale forever.
14. Outer output-clamp injects `WHERE partition_column >= out_start AND partition_column <
    out_end` at the outermost SELECT, constraining output to the derived output window.
15. The outer clamp is dropped exactly for the transparent slice (one timeseries source,
    zero-margin `Bounded(_, 0, 0)`, no skew) because the per-source pushdown filter already is
    the output clamp.
16. A lookback margin, partition-column skew, or a second timeseries source keeps the outer
    clamp — scan window and output window are then distinct, load-bearing windows.
17. Each written partition's scan is sized from the derived output window's reach, never the run
    window's.
18. Source-filter pushdown injects a per-source `partition_column` filter on each `smelt.<path>`
    reference, derived from the model's SQL; sources without `timeseries:` are lookups read in
    full.
19. INSERT writes the resulting query's output into the output table.
20. DELETE range and output clamp derive from one window → idempotent for any write-window width:
    re-running the same `[start, end)` under fixed input converges to the same state.
21. The derived output window is a range to be covered, not a mandate for one statement; backfill
    chunking splits it into sequential DELETE+INSERT pairs, each chunk scanned by its own reach.

## Strategy enum (backend-internal) (1266–1285)

22. Strategy is not declared on the model — derived per cell; backends pick a physical strategy
    from config + capability for the recompute corner.
23. The strategy enum has exactly three named variants: `DeleteInsert`, `Append`,
    `InsertOverwrite`.
24. DuckDB always uses `DeleteInsert`.
25. A partition-shaped output's creation/backfill cells are region-addressed; a pure partition
    grain (no declared identity) has no keyed addressing at all.
26. Keyed `MERGE` is the addressing a dimension-change cell derives on a composed
    clock-and-identity output — per-cell, driven by what changed, not tied to a grain.
27. A backend may select only a strategy that preserves the declared shape's invariants.
28. `Append` is unreachable until gated on ledger-verified unwritten windows (§Known
    Divergences).

## Run window vs partition granularity (1287–1305)

29. The CLI `[--event-time-start, --event-time-end)` range declares a run window, not a
    per-partition invocation.
30. Run-window size and partition granularity are independent (within alignment rules): a
    daily-partitioned model run with a 30-day window is one engine query, one partition-aligned
    DELETE over the 30 partitions, one INSERT.
31. Backfilling 60 days is one `smelt run`, not 60 invocations; per-partition equivalence holds
    regardless of run-window size.
32. Declared `timeseries.granularity` (`g_run`) must be at least as coarse as the granularity
    implied by the `partition_column` projection's truncation/grid transform (`g_part`), derived
    independently from the SQL rather than trusted.
33. Example: `partition_column = DATE_TRUNC('day', event_time)` has `g_part = day`; declaring
    `granularity: hour` on it is rejected (misaligns the DELETE+INSERT contract).
34. `g_run >= g_part` checked under the closed coarseness ordering (`hour < day < week < month <
    quarter < year`, owned by `timeseries.md`).
35. When `g_part` cannot be derived (opaque projection), the comparison is skipped — undecided,
    not a positive disproof — and only the declared-granularity alignment check applies.
36. A sub-`g_part` run window is rejected with a message naming the minimum window, never
    silently widened.

## Batch safety classification (1307–1327)

37. The optimizer rolls the per-source bound map (`BoundResult`, from the unified bound/reach
    derivation) into a single class per model, meaningful only inside the recompute-a-region
    execution shape.
38. `FullyBatchSafe`: all timeseries sources `Bounded(_, 0, 0)`, no temporal dependencies →
    single query for any run window.
39. `BoundedSafe(n)`: all timeseries sources `Bounded` with `n = max(before + after) > 0` →
    auto-sized chunks (3× context, clamped 7–90 partitions).
40. `PerPartitionOnly`: any timeseries source `Unbounded` (cumulative-across-history) → one
    partition at a time, sequential.
41. `n` is rendered in the source's partition-column unit and is the same value the source-filter
    pushdown reads.
42. A model with any `NotDerivable` source is refused at planning time
    (`MaintenanceReachNotDerivable`), not assigned a class; diagnostic names the offending
    construct and its source-map points at the original SQL; no silent downgrade to full
    refresh.
43. Wide single-batch builds: when `FullyBatchSafe` yields a single batch spanning more than 30
    partition periods, smelt warns and recommends `--per-partition` or `--batch-size <n>`; either
    flag suppresses the warning.

## First-run and backfill (1329–1368)

44. A first run (no output table) and a backfill (re-run of a written range) follow the same
    DELETE+INSERT contract; the DELETE is a no-op when the partition is absent.
45. Chunking shape by batch-safety class: `FullyBatchSafe` → single DELETE+INSERT pair for any
    `[start, end)`; `BoundedSafe(n)` → auto-sized sub-ranges (3× context, clamped 7–90
    partitions), sequential temporal order; `PerPartitionOnly` → one partition per iteration,
    sequential temporal order.
46. Self-referential first-run bootstrap: a non-self-referential model's first run creates its
    target directly with `CREATE TABLE … AS SELECT` over the first batch.
47. A self-referential model cannot do that (its first batch reads the target via
    `smelt.<self>`, and no engine resolves a table to itself mid-creation); the runtime first
    materialises an empty target with the model's inferred output schema, then executes every
    batch — including the first — as ordinary partition DELETE+INSERT.
48. The self-read over the empty table yields no prior state for the first partition, so the
    resulting trajectory is identical to seeding the table by hand; the bootstrap is keyed only
    on whether the target exists yet.
49. Calendar alignment: when per-partition execution is forced (or `--per-partition` requested),
    `Month`/`Quarter`/`Year` batches advance by true calendar units landing on calendar
    boundaries regardless of month length; `Day`/`Week` use fixed 1-day/7-day steps.
50. Output grain may be finer than partition grain: a model whose `partition_column` holds
    monthly boundaries may emit daily/hourly rows within them; batch-splitting operates on the
    partition grain, reading/writing finer rows in their entirety within each batch.
51. Per-chunk transaction boundary: each chunk's DELETE+INSERT is one backend transaction; INSERT
    failure rolls back the chunk's DELETE; earlier committed chunks do not roll back (partial
    progress is intentional — each chunk is idempotent).
52. Failure mode: a run halts at the first failed chunk and exits non-zero; re-running the same
    `[start, end)` resumes correctly because every committed chunk is idempotent.
53. Late-arriving data (interim guidance): smelt does not auto-re-run partitions when data
    arrives late; interim mitigations are trailing `--event-time-end` behind known latency, or
    overlapping re-process ranges.
54. A planned automated mechanism is per-column `data_latency:` (§Known Divergences).
55. Contract-level statement: the derived horizon (§"Windowed maintenance and the horizon") — a
    late arrival past the derived clamp is silently excluded, surfacing it is a
    model-author/data-quality concern, and the mitigations only widen the window a late row can
    still land in.

## Per-partition equivalence (1370–1392)

56. For every partition `p` in `[run_start, run_end)`: `partition_grain_run(...).where(partition
    = p) == full_refresh(model).where(partition = p)` — the partition-grain strengthening of the
    equivalence invariant, independent of run-window size.
57. Column-locality: the equality holds for local columns only — those depending only on source
    rows visible within the model's source-filter ranges.
58. A column depending on history outside those ranges (cumulative aggregation,
    connected-components, backward-fill) is not equivalent — its per-partition value reflects
    run-time state, not final cumulative state; such a column forces its source to `Unbounded`
    and the model to `PerPartitionOnly`; the run is correct as-of-the-run, not equal to a full
    refresh with final input.
59. Equivalence is up to full-refresh non-determinism: bit-identical on deterministic columns; a
    `contract: plausible` column need only be a plausible full-refresh value; this never extends
    to a column governing which rows exist, their partitioning, or dedup.

## Safety checks (per-cell admission for the recompute corner) (1394–1429)

60. The optimizer rejects (`PartitionGrainNotSafe`) a partition-grain model whose SQL uses
    constructs breaking the partition-DELETE-then-INSERT contract.
61. Each check applies a shared `model_properties.md` proof to discharge one §"Per-cell
    admission" obligation for the recompute-a-region corner over this output shape; each is
    individually disabled via `safety_overrides.allow_<check>: true` (opt-in, recorded).
62. Window functions: admitted when `PARTITION BY <keys> ⊇ partition_column` (partition alignment
    scoped over the window), OR `PARTITION BY` omits `partition_column` but the frame is bounded
    `RANGE BETWEEN INTERVAL '…' PRECEDING` with no `UNBOUNDED` bound; `UNBOUNDED PRECEDING` or no
    `PARTITION BY` at all is never admitted. Escape hatch `safety_overrides.allow_window_functions`.
63. `HAVING`: admitted when the enclosing `GROUP BY` key ⊇ `partition_column`.
64. `DISTINCT`: admitted when `partition_column` is projected in the same scope.
65. `LIMIT`: never admitted — a row-count cap never commutes with the partition filter (survival
    depends on which other rows are present, which differs between a run and a full refresh).
66. Subqueries (`SELECT … FROM (SELECT …)`): rejected unless overridden; a `WITH`-clause CTE is
    not gated by this check (CTE bodies flow through bound derivation via the body-structure
    classifier; only a subquery nested in FROM/JOIN is).
67. Non-deterministic functions: confined to a payload column with `contract: plausible`.
68. All partition-alignment checks are evaluated per scope: a `UNION` branch's own
    `HAVING`/`DISTINCT`/window is judged against that branch's own key set, never inheriting
    alignment from a sibling or the outer query (set-operation distribution governs the
    branches).
69. Non-determinism/payload rule: a non-deterministic value is admitted only when it flows
    exclusively into a `contract: plausible` column — written once per window, never read back to
    place/filter/group/dedup a row.
70. Taint check hard-excludes, regardless of opt-in, naming the offending position: the
    `event_time_column`/`partition_column` expression; any `unique_key` column; any
    row-set-membership or grouping position (`WHERE`, `HAVING`, `JOIN … ON`, `DISTINCT`,
    `GROUP BY`, a window's `PARTITION BY`/`ORDER BY`/frame).
71. Run-nondeterministic class (`NOW()`/`CURRENT_*`) is additionally admitted as a direct
    SELECT-list projection even without `contract: plausible`, because compile-time pinning
    freezes it once per run.
72. Row-nondeterministic class (`RANDOM()`/`UUID()`) always requires the target column declared
    `plausible`.
73. Declaring an excluded column `plausible` is a configuration error.
74. `safety_overrides.allow_nondeterministic` drops the guardrail wholesale and is discouraged.

## Event-time outer-visibility (1431–1444)

75. The outer output-clamp injects `WHERE event_time_column >= start AND event_time_column <
    end` at the outermost SELECT, so `event_time_column` must be accessible there.
76. A plain `UNION`/`INTERSECT`/`EXCEPT`, a `UNION ALL` whose branches cannot be proven traceable,
    or a subquery FROM not projecting `event_time_column`, is rejected
    (`EventTimeColumnNotVisibleAtOuterSelect`) before execution.
77. A `UNION ALL` is exempt when every branch's projection of `event_time_column` traces
    `Traceable` back to a real source's own partition column — per-source pushdown then narrows
    each branch independently and outer-clamp placement is immaterial.
78. A `StaticSeed` branch is named and rejected; a `NotTraceable` branch conservatively keeps the
    whole-model outer clamp.

## Observing the per-source clamp (1446–1467)

79. Because lookback is derived from SQL rather than declared, the derived clamp is surfaced so
    the author can confirm the analyzer read their SQL as intended.
80. `smelt explain --json`'s per-cell `source_bounds` map reports, per source, `source_
    partition_col` and derived `(before, after)` offsets; with a concrete run window it also
    resolves the scan window.
81. Editor hover (LSP) on a `smelt.<path>` reference in a partition-grain model shows that
    reference's clamp alongside the schema/column readout.
82. Readout table: `Bounded(c, 0, 0)` → partition-by-partition, no lookback/lookforward;
    `Bounded(c, before, after)` → window `c ∈ [run_start − before, run_end + after)` shown;
    `Unbounded` → read across all history, forces `PerPartitionOnly`; lookup (no `timeseries:`) →
    read in full, not a pushdown candidate.
83. A `NotDerivable` source surfaces the planning-time refusal diagnostic instead of a window.

## Functions inside partition-grain bodies (1469–1478)

84. Function expansion runs before every analysis stage here — bound derivation, source-filter
    pushdown, batch-safety sub-checks see the expanded CST, so an inlined vs `smelt.define`-body
    `LAG()` are indistinguishable.
85. The outer output-clamp is injected at the outermost expanded query; pushdown reaches
    `smelt.<path>` references originating inside function bodies.
86. Opaque calls remain black boxes: bound derivation cannot read through
    `smelt.extern`/built-ins, so a model whose time-dependence hides behind an opaque call is
    `NotDerivable` and refused unless a bound is provable from surrounding SQL.

## Window independence and self-referential models (1480–1508)

87. Whether windows build in parallel or must build sequentially in temporal order is the
    window-independence/ordered-execution property, derived from the dependency graph, never
    declared.
88. Window-independent (default): every window is a pure function of source rows in its own scan
    range, lookback reaches only into sources never the model's own earlier partitions; a
    backfill may split into sub-ranges built in any order including parallel.
89. Window-dependent → ordered: a self-referential model (reading its own prior partitions via
    `smelt.<self>`) is in scope and still executes as partition DELETE+INSERT — it stays
    partition-addressed, does not become key-grain — but its windows build sequentially in strict
    temporal order and its backfill may not be parallelised or reordered.
90. A self-edge the planner cannot prove converges partition-by-partition (reading forward or
    across all history) is refused at planning time.
91. This is the same stateless/stateful spine separating partition grain from key grain: a
    self-referential partition-grain model is stateful-ordered in execution yet keeps the
    partition-grain output shape (partitioned, per-partition-equivalent within each window's own
    input).
92. Ordered execution composes with the derived output window: a Form B skew relation anchored on
    a non-self source rebases an `Ordered` model's write window exactly as it would a
    window-independent model's; ordering then applies over the rebased partitions, every one
    building strictly sequentially.
93. The self-edge itself is never a skew anchor: its own bounding relation (the backward-bounded
    read proving the `Ordered` verdict) is a distinct convergence mechanism, not a
    partition-column skew declaration, even when the self-referenced column shares the model's
    `partition_column` name.

## State ownership (1510–1517)

94. smelt does not track watermarks, offsets, or run history for partition-grain models — the
    backend owns computational state (DuckDB: table state + transactions; Delta/Spark:
    transaction log + MERGE; Flink: checkpoints).
95. Optional run-state tracking with gap detection is opt-in via `state.mode: intervals`
    (`virtual_environments.md`); on-disk layout owned by `run_state.md`.
96. The one deliberate exception across the family is the key grain's transactional merge ledger
    (§"The transactional frontier write (merge ledger)" and §"Key-grain design").

## `partition_column` validation (1519–1524)

97. Partition-column projection is owned by `timeseries.md` §"Constraints & Invariants" rule 1:
    `partition_column` must appear in the model's output `SELECT` (and `GROUP BY` when grouping
    is present), else `MalformedTimeseries`; this profile consumes that guarantee rather than
    re-checking it.

## The key grain — intro + composition table (1526–1541)

98. The key-addressed shape: keyed state, one row per `unique_key`, kept current by the
    fold-a-delta corner (keyed `merge_into`) as default plan.
99. Declared surface is §Surface "Key-grain declaration".
100. Composition table: output shape/grain owner (`models.md` §"Refresh axis"); required
     properties (algebraic discriminants defining column families, driving-fact/anchor
     resolution, event-time monotonicity trace of the driving source's clock, once-write
     provenance, join-contribution monotonicity, input-delta discovery, key temporal locality for
     a time-partitioned output) — `model_properties.md`.
101. World-facts consumed: timeseries clock of a clocked driving source, source mutation profile,
     declared key-recurrence bound where the recurrence route is declared rather than derived —
     `timeseries.md`/`sources.md`.
102. Default plan (fold corner): keyed `merge_into` (target-as-replica) sequenced by the
     windowed-keyed-maintenance driver with source-filter pushdown on the driving source; the
     transactional merge ledger; dimension-driven horizon-bounded MERGE for enrichment shapes;
     slice-pruned merge target under established key temporal locality — `model_transforms.md`.
103. Admission: every check is one instance of §"Per-cell admission" for the fold-a-delta corner
     over a key-grain output (§"Admission matrix").
104. Invariant upheld: end-state equivalence, the end-state specialisation of the equivalence
     invariant, oracle is the model's own SQL.

## The two run shapes (derived, never declared) (1543–1563)

105. The run shape is the keyed application of the input-consumption axis, derived from the
     driving source.
106. Window-forward: FROM clause contains exactly one source whose resolved target declares
     `timeseries:` (the driving source, resolved by the driving-fact/anchor proof); zero clocked
     sources means snapshot-reconcile; two or more is `KeyedMultipleDrivingSources`.
107. Window-forward run steps over source partitions covered by `[run_start, run_end)` in
     temporal order: per partition, source-filter pushdown injects the window onto the driving
     source, the per-partition delta SELECT executes, `merge_into` folds the delta with the
     per-column combiner map.
108. Non-timeseries sources (lookups/dimensions) are read in full each step; if the target does
     not exist at the first step it is created from that step's delta (`CREATE TABLE AS SELECT`).
109. Snapshot-reconcile (no clocked source): the run re-scans the source whole, computes the
     per-key aggregation, `merge_into`s the result — matched keys overwritten, unmatched
     inserted.
110. A key present in the store but absent from the incoming scan is retained unchanged;
     deletion requires an explicit mechanism (out of scope, §Known Divergences).
111. Out-of-order, parallel, or sliced-backfill window application is admitted iff the model is
     order-independent; otherwise windows apply sequentially in temporal order.

## Derived execution postures (1565–1581)

112. Three model-level properties fold from the column families, each derived, surfaced by
     `smelt explain`, never declared.
113. Re-run tolerance: may an already-merged window be blindly re-merged over unchanged input?
     Holds iff every column is idempotent (no additive-fold column); example convergence
     `GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`; an additive model double-counts and must be
     refused (the ledger).
114. Order-independence: may windows apply out of order or in parallel? Holds iff every column's
     combiner is order-independent — extremal/lattice, decomposed-fold, and proven once-write
     families qualify; order-monotone overwrite does not (order-independence holds only up to
     ordering-key ties), so any model with an overwrite column executes windows sequentially in
     temporal order.
115. Reprocessing refusal: a window whose input changed since it was merged must not be re-merged
     for any family — an irreversible fold cannot un-see a removed contribution, an overwrite
     cannot retract a superseded-by-nothing value.

## The transactional frontier write (merge ledger) (1583–1600)

116. Every window-forward keyed model maintains a per-model frontier — a small backend table
     recording each merged window — written in the same backend transaction as that window's
     `merge_into`.
117. Additive-fold models (not re-run tolerant): a run whose window is already recorded is
     refused (`KeyedReprocessedWindow`) exactly, not best-effort; crash resume merges only
     unrecorded windows; a run interrupted at window k of n resumes correctly by re-running the
     same range.
118. Re-run-tolerant models: a recorded window may be re-merged (a no-op on unchanged input); the
     frontier serves reprocessing detection and `--auto` bookkeeping, not refusal.
119. Snapshot-reconcile models keep no frontier — each run is a self-contained reconciliation.
120. This realization is backend-resident and transactional with the write it describes — a
     correctness structure, distinct from the opt-in run-state observability surface
     (`run_state.md`); rationale for not violating state-ownership doctrine: §"Partition-grain
     design".

## Admission matrix (column family × source shape) (1602–1636)

121. The key-grain instance of §"Per-cell admission": each cell discharges obligations 2
     ("faithful fold") and 3 ("combiner algebra class") for one (column family × run shape) pair.
122. Fold families consume events (each row contributes exactly once — replayable,
     retraction-free feed required); overwrite families consume observations (each row supersedes
     — current-snapshot semantics required); checked per column.
123. Matrix cells (window-forward / snapshot-reconcile): additive fold ✓(ledger-enforced)/✗;
     extremal-lattice fold ✓/✗; order-monotone overwrite ✓/✗; once-write ✓(provenance proof)/✗;
     decomposed fold ✓(ledger-enforced, graded additive)/✗; plain overwrite
     ✗(`KeyedUnknownCombiner` names the `MAX_BY` fix)/✓(current-snapshot semantics).
124. The three snapshot ✗ cells (extremal/lattice, order-monotone overwrite, once-write) are not
     double-count hazards (those families re-merge safely) — they are equivalence failures:
     `MIN(price)` folded over successive snapshots computes min-ever-observed vs current min;
     `MAX_BY(attr, updated_at)` retains a stale incumbent if a mutation regresses the ordering
     value; `COALESCE`-once-write captures first-observed, unrecoverable from the current
     snapshot; each refused (`KeyedSnapshotSourceUnsupportedColumn`) rather than admitted
     silently.
125. Additive/decomposed-fold snapshot ✗: re-folding state double-counts (decomposed fold "same as
     additive fold").
126. Plain-overwrite window-forward ✗: order-dependent over events, `KeyedUnknownCombiner` names
     the `MAX_BY` fix.
127. Scope: the replayable-feed obligation binds each fold-contributing source (one whose columns
     feed an aggregate the cumulative combiner folds), not every referenced source.
128. A mutable source consumed only through a covered enrichment cell
     (`UpstreamMutation`-triggered column-scoped `MERGE`) is admitted regardless of its own
     mutation profile — its post-creation mutations are maintained by that separate cell.
129. A source that is both a fold input and a mutable enrichment stays refused
     (`MaintenanceNoAdmissibleTechnique`) — admission fails closed rather than approximating which
     columns are "safe".

## End-state equivalence: the SQL is the oracle (1638–1651)

130. The key grain upholds the end-state specialisation of the equivalence invariant; because the
     body is required to be the aggregation itself, the oracle is executable for every admitted
     model — it is the model's own SQL.
131. Window-forward: for any set `S` of processed driving-source partitions and any admitted
     ordering over `S`, stored state equals the model SQL evaluated over `source.where(partition
     ∈ S)`; for overwrite columns the equality holds up to ordering-key ties.
132. Snapshot-reconcile: the stored row for every key present in the current snapshot equals the
     model SQL evaluated over that snapshot; keys absent from the snapshot are retained — the
     stored table is the oracle's rows plus retained departed keys.

## No write-eligibility clamp (1653–1658)

133. A run merges every delta row it scans, into whatever key it names, however old that key is.
134. A derivable forward reach is computed and reported (`smelt explain`) but never gates
     admission and never bounds which keys a run may touch — no scanned input is ever silently
     dropped.

## Key temporal locality (the time-partitioned output) (1660–1721)

135. A keyed model may time-partition its output with a `timeseries:` block; named columns must
     be projections of the model, and `event_time_column` may name the partition column itself.
136. Admission requires key temporal locality: a guarantee that every stored row a run's deltas
     can touch lies within a computable slice of the output's time axis; locality lets the
     `merge_into` target scan be pruned to the slice, and lets downstream consumers window over
     the output.
137. Structural preconditions checked before the routes: run shape is window-forward
     (snapshot-reconcile establishes no locality); `partition_column` names either a `unique_key`
     column or a non-key projection in the extremal-fold/order-monotone-overwrite/once-write
     family, provably NOT NULL from a key's first stored row; block `granularity` equals the
     driving source's granularity.
138. Route 1, key-embedded: `partition_column` is a `unique_key` column; a stored row's partition
     value is its key's own, a delta touches exactly its own partition values; slice = run's scan
     window widened by derived lateness/skew margins.
139. Route 2, key-determined: partition projection is a per-key constant under once-write
     provenance (key-derived expression, or a declared FD over a column present non-null on
     every input row); every delta row carries its key's fixed partition value, slice = delta's
     own partition values, exact regardless of key age.
140. Route 3, recurrence-bounded: a key-recurrence bound `r` holds — every pair of input rows
     sharing a key lies within `r` of each other on the event-time axis; `r` derived from the
     model's SQL where statically decidable, else declared on the driving source
     (`sources.md`, `key_recurrence`); slice = scan window widened backward by `r` plus derived
     margins.
141. A declared `r` is admitted only checked: the run verifies at merge time that no delta row
     matched (or would duplicate) a stored key outside the slice, any violation fails the run
     transactionally (`KeyedRecurrenceBoundViolated`); a declaration can bound work, never
     silently drop data.
142. Pruning is not a write clamp: slice pruning is no-op elimination on the merge's target scan
     only — rows outside the slice provably cannot match a delta key (routes 1–2) or are checked
     not to (route 3); every scanned delta row still merges, §"No write-eligibility clamp"
     unchanged.
143. Governing principle: §"Windowed maintenance and the horizon" — only proofs prune; a declared
     bound is admitted only checked; no unproven bound ever refuses a write.
144. Row movement: under routes 1–2 a key's partition value never changes; under route 3 it may
     move (an extremal/overwrite partition projection superseded by a late row) — the merge
     updates the stored row in place, partition value included, both old and new values lie
     within the slice by the bound; movement does not change derived postures (an overwrite
     column still forces sequential temporal order).
145. Per-slice equivalence: with locality established, the invariant is additionally checkable
     slice-by-slice — for any output slice, stored rows equal the model SQL evaluated over source
     rows within the slice's derived reach (the keyed analogue of per-partition equivalence).
146. The output as a clocked source: an admitted block makes the output a clocked,
     time-partitioned table — downstream partition-grain models receive source-filter pushdown
     against it, and a downstream keyed model may take it as its clocked driving source (the
     clock propagates through the DAG instead of stopping at the keyed stage).
147. The output's settle bound (how long a written slice may still change) is derived and
     surfaced by `smelt explain`: route 1 settles with the source's lateness margin; route 3
     settles after `r` plus margins; route 2 never settles (a late delta may touch an arbitrarily
     old slice).
148. A re-written slice is changed input to downstream consumers, handled by the ordinary
     staleness machinery (§"Interaction with `--auto` / staleness").

## What the composed shape enables (1723–1751)

149. The composed shape — key-addressed and time-partitioned — is not "keyed with an
     optimisation": several capabilities hold only in that form, which is why the two declared
     facts must never be read as exclusive alternatives.
150. Propagation admissibility: a bare keyed node refuses in the graph layer (no partition axis to
     carry interval dirt); a locality-admitted keyed output participates in forward propagation
     and backward resolution as a clocked node — the composed shape is the only way a keyed
     stage sits inside a propagation chain rather than terminating it.
151. Exact key→partition dirt projection: under routes 1–2 a stored row's partition value is a
     per-key constant, so a key-level change set projects to exact partition intervals (no
     widening); under route 3 the projection widens backward by `r` plus margins (widen-never-
     narrow); a composed node hands precise interval dirt downstream without key-level dirt
     representation in the graph.
152. Slice-bounded no-op write elimination: the conditional write (category 2, §"Windowed
     maintenance and the horizon") must read stored rows to compare against candidates; on a bare
     keyed output that read is the whole key space, on a composed output it is bounded by the
     pruned target slice — compare cost proportional to the slice, making suppression affordable
     at volume.
153. Settle-bound × observed-delta composition: the settle bound (static) composes with the
     observed output delta (dynamic, §"The graph layer") — consumers skip settled slices
     unconditionally and skip unsettled slices whose observed delta is empty; a stable upstream
     chain degenerates to empty-delta no-ops with a provable horizon behind it.
154. The first two bullets bind at the graph layer, the third at statement emission, the fourth
     across both; implementation status recorded in §Known Divergences.

## The maintenance boundary (1753–1765)

155. On the algebraic ladder the keyed families sit on rungs 1 and 2: every catalogued combiner
     folds `(state, delta)` with no inverse and no history re-read.
156. The additive and decomposed-fold families sit on rungs 1 and 2 respectively and are
     additionally groups (invertible) — what a future subtract-then-add reprocessing path would
     exploit.
157. The extremal/lattice, order-monotone-overwrite, and once-write families (the latter two rung
     2 for the state-widened spellings) are monoids but not groups (a folded contribution cannot
     be un-seen), which is why reprocessing is refused for them.
158. Rungs 3–4 (group-rung retraction; opt-in bounded-domain multiset) grow this shape further
     without changing its contract; transforms catalogued in `model_transforms.md`, the
     `bounded_domain:` budget declaration in `model_properties.md`.
159. Beyond the ladder is delegated to `refresh: materialized_view`.

## Reprocessing (1767–1778)

160. If a merged window's source data changes, re-running the ordinary reprocessing path does not
     produce correct state for any family.
161. Before that refusal fires, the change routes to the repair family first: a retraction or
     mutation whose affected keys are discoverable and whose per-group slice is bounded
     recomputes just those groups, and no reprocessing refusal is raised — a plan-level route,
     not a new mode or user flag.
162. The rule refuses at planning time only when a repair obligation fails (the ledger says the
     window was merged; `--auto` staleness says the input changed), `KeyedReprocessedWindow`
     naming the failing repair obligation and pointing at the two mitigations: `--full-refresh`
     (truncate-and-rebuild), or a manual cascade rebuild.
163. Subtract-then-add for all-invertible models is a future path (§Known Divergences).

## Ordering ties (order-monotone overwrite) (1780–1787)

164. The pairwise combiner for `MAX_BY(value, ordering)`: the delta wins iff `delta.ordering >
     target.ordering` (strict); on equality the incumbent wins.
165. This is deterministic given the processing history but not order-independent when ties occur
     across windows — why overwrite columns force sequential execution.
166. Recommended modelling practice: a composite, provably tie-free ordering expression (e.g.
     `(updated_at, source_seq)`); the classifier cannot verify uniqueness and does not claim to.

## Enrichment joins (1789–1804)

167. A fact-to-dimension join bringing an enriching event as a separately-arriving relation is
     admitted when its per-key contribution is provably monotone (join-contribution monotonicity
     proof): the contribution feeds only extremal/order-monotone/once-write columns and does not
     fan into a decrementing aggregate.
168. The maintainability line is monotone-vs-retractable semantics, not join-vs-union spelling —
     the join form is normalised to the same keyed-monoid merge as the union form; only a
     genuinely retractable contribution is refused (`KeyedRetractableContribution`).
169. A re-scanned existence flag additionally requires the dimension source declared
     `append_only`; extremal milestones are safe regardless.
170. Where a dimension batch's forward reach `H` is derivable from the model's SQL, the
     dimension-driven horizon-bounded MERGE may clamp the enrichment recompute to `[event_ts,
     event_ts + H]` — a scan-side bound that cannot under-cover because it is derived; where `H`
     is not derivable, the transform is not licensed and the enrichment evaluates through the
     ordinary widened scan; no declared value ever truncates a recompute or a write.

## Key-grain output shape (1806–1813)

171. One row per `unique_key`; column names are the projection's `AS` aliases (or source column
     names).
172. By default there is no `partition_column`, no `event_time_column`, no `timeseries:` on the
     model; downstream consumers see the output as a lookup table read in full each run,
     identical to any non-timeseries source.
173. With an admitted `timeseries:` block the output is instead a clocked, time-partitioned keyed
     table — still one row per key — that downstream consumers window over like any clocked
     source.

## Functions inside keyed bodies (1815–1822)

174. Function expansion runs before the classifier: projection reading, GROUP-BY inspection,
     FROM-clause walking, family classification, and pushdown operate on the expanded CST.
175. A `smelt.define`-resolved call is admitted iff its expanded body produces a catalogued
     aggregator at the outermost expression position — pattern functions are admitted exactly
     this way, with no privileged treatment.
176. Opaque calls (`smelt.extern`, non-inlinable built-ins) in the projection list are rejected
     via `KeyedUnknownCombiner`.

## Interaction with `--auto` / staleness (1824–1830)

177. Window-forward: stale driving-source windows are re-processed subject to posture —
     re-run-tolerant models re-step exactly the stale windows (safe by idempotence); additive
     models refuse re-processing of ledgered windows (`KeyedReprocessedWindow`) and steer to
     `--full-refresh`.
178. Snapshot-reconcile: the model is treated as always-stale; every `--auto` run reconciles.

## Interactions (1832–1851)

179. The invariant, ladder, horizon, and validator-not-chooser are owned above; the plan's
     per-cell theorem is the `S`-vector refinement of the invariant, and per-cell choice operates
     strictly inside validator-not-chooser.
180. Output shape/grain declaration and the refresh trichotomy are owned by `models.md`; the plan
     validates against them.
181. The declaration law and litmus rule (`models.md` §Design) — whether a fact is declared,
     derived, or implied, and whether a proposed combination earns a new peer shape — are owned
     there; this spec consumes them.
182. Input consumption (`models.md` §"Input-consumption axis") is a derived, cross-cutting axis
     (mutation-profile world-fact → input-delta-discovery proof → re-scan/probe transform); moving
     along it never changes the equivalence contract, only what is scanned; default is windowed,
     full scan is the surfaced fallback.
183. Source postures (`mutation_profile`, lateness, retention, delta identity, unique keys) are
     declared in `sources.md` and consumed by admission; their runtime tripwires live there.
184. The technique primitives (`merge_into`, DELETE+INSERT, column-scoped merge, targeted
     backfill) are catalogued in `model_transforms.md`; the outer output clamp is the subquery
     wrap over the model's output schema defined there.
