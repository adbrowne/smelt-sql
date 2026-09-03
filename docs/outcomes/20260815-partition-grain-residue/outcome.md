# Outcome: Close the partition grain's stale-plan-tracked implementation residues

**Created:** 2026-08-15
**Status:** active
**Source:** `docs/specs/incremental_shapes.md` §"The partition grain" §Known Divergences;
`docs/plans/20260530-thread-fn-registry-classification.md`,
`docs/plans/20260616-smelt-feedback-fixes.md`, `docs/plans/20260509-meta-language-overall.md`,
`docs/plans/20260704-model-updates-l4-batched.md`;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`

## The outcome

Every partition-grain Known Divergences bullet that predates `docs/outcomes/` — most of them
citing a `docs/plans/*` tracker whose current status was never re-checked — either lands for real
or is confirmed already-landed by its cited plan. Function-registry-threaded classification means
the `NotDerivable` lookback-refusal gate and the window-function batch-safety check both read
through `smelt.define` function bodies, not only the outer SQL text. A CTE alias that fails to
project `event_time_column` is caught by the outer-visibility check before execution, not at
runtime. Generator-emitted models (`ModelDef`) gain the per-model overrides the spec's closed
field set currently omits. A monotone-integer `partition_column` gets a true end-to-end run path —
backfill chunking, scan-filter injection, and the `smelt explain` clamp rendering all handle a
non-date type, not only date-typed grids. Per-source clamp observability finishes: `smelt explain
--json` resolves the run-relative scan window when a concrete run window is supplied, and the
editor-hover readout ships. A `partition_column` rename gets a real refusal diagnostic with a
fixture.

## Success criteria (checkable)

1. Each of the four pre-outcome tracking plans (`20260530-thread-fn-registry-classification`,
   `20260616-smelt-feedback-fixes`, `20260509-meta-language-overall`,
   `20260704-model-updates-l4-batched`) is audited against the repo's current state before any
   re-implementation, so already-landed work isn't redone.
2. The `NotDerivable` lookback gate and the window-function batch-safety check both classify
   through `smelt.define` bodies, matching what expansion-then-analysis already promises
   elsewhere in the spec.
3. A CTE alias that fails to project `event_time_column` is caught by
   `EventTimeColumnNotVisibleAtOuterSelect` before execution.
4. Generator-emitted models (`ModelDef`) support the per-model overrides the spec's declared
   surface requires.
5. A monotone-integer `partition_column` model runs first-run, backfill, and steady-state
   end-to-end with correct chunking, scan-filter injection, and explain-clamp rendering.
6. `smelt explain --json`'s per-cell `source_bounds` resolves the run-relative scan window given a
   concrete run window; editor hover on a `smelt.<path>` reference shows the same clamp.
7. A `partition_column` rename gets a named diagnostic and a fixture exercising the refusal path.
8. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes; all
   standing gates green.

## Out of scope

- The `smelt.metric()` × time-filter-injection interaction is explicitly named "unspecified" (not
  merely unimplemented) by `incremental_shapes.md` — deciding what it should do is a design call,
  not an implementation gap against already-decided text. It stays in
  `docs/outcomes/20260815-definition-delta-migrate` §"Out of scope" pending sign-off.
- Otherwise none — this outcome exists specifically because these bullets had no other live owner
  (`docs/outcomes/20260815-definition-delta-migrate`'s scope statement). If the phase-1 audit
  finds a bullet is genuinely still owned by a live, actively-progressing plan outside
  `docs/outcomes/`, record that finding in the decision log rather than silently dropping the
  bullet from this outcome's phases.
- A window function nested inside a `CASE` arm is invisible to
  `analyze_expr_temporal` (`crates/smelt-logical/src/analysis/temporal.rs`), flagged by the phase-2
  summary. `temporal.rs` is advisory-only under the property-composition-walk rule — it feeds no
  admission gate — and the gap matches no partition-grain Known Divergences bullet with a
  pre-`docs/outcomes/` tracker, so it is not folded in here.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Audit the four cited pre-outcome tracking plans against current repo state; confirm what's already landed vs. still open | done |
| 2 | Function-registry-threaded classification: lookback gate + window-function batch-safety read through `smelt.define` bodies | done |
| 3 | CTE-only `event_time_column` detection in the outer-visibility check | done |
| 4 | Per-`ModelDef` overrides for generator-emitted models | done |
| 5a | Partition-axis domain: typed run window + backfill chunking over a monotone-integer axis | done |
| 5b | Integer-axis emission end-to-end: scan-filter/DELETE literals, explain clamp, first-run/backfill/steady-state proof | done |
| 6 | Per-source clamp observability: run-relative scan window in `explain --json`; editor hover | done |
| 7 | `partition_column` rename: refusal diagnostic + fixture | pending |
| 8 | Validate + close out: `/smelt:validate incremental_shapes` clean, standing gates green | pending |

## Decision log

- 2026-09-04 — Outcome activated. Phase 1 planned as an *evidence* audit: each
  partition-grain residue gets a characterization probe pinning today's behaviour, so phases
  2-7 start from a red test rather than re-investigating a stale plan file. No phase reshape
  made — this is the first phase, there is no prior summary, and reshaping on guesswork rather
  than the audit's findings is exactly what phase 1 exists to prevent.

- 2026-09-04 — Phase 1 audit complete (`audit.md`). All seven phase-mapped
  residues (phases 2–7, two residues folded into phase 2) confirmed still
  OPEN or only partially landed by a pinned probe — none closes early. Five
  other partition-grain Known Divergences bullets (per-column `data_latency`,
  non-deterministic row-set-membership, `PartitionGrainForbidsMetrics`,
  sub-`g_part` suggestion, `NOW()`/`CURRENT_*` pinning) cite no
  `docs/plans/*` tracker predating `docs/outcomes/` — decision records or
  by-design exclusions only — so none is folded into this outcome's phases;
  they stay unowned outside this outcome's scope. Correction to the spec
  itself: bullet #3 (per-source clamp observability) claims "specified ahead
  of a tracking plan," but it is tracked, by
  `docs/plans/20260704-model-updates-l4-batched.md` Phase BL8 (`pending`) —
  phase 8's close-out should fix this stale claim alongside the divergence
  removal.

- 2026-09-04 — Phase 2 planned. No phase row added, split, or reordered: the
  residual gap the phase-1 summary flagged (`analyze_one_select` never descends
  into subqueries, so even fully-expanded SQL hides an `OVER`/`LAG` inside the
  derived table that expansion produces) is absorbed *into* phase 2 rather than
  deferred — without it, expansion alone would leave a define-body `LAG`
  classified `FullyBatchSafe`, i.e. success criterion 2 unmet and the verdict
  unsafe rather than merely conservative.

- 2026-09-04 — Phase 2 implemented (`phases/02-summary.md`). Landed the AST
  descent into derived tables/CTEs in `temporal.rs`, threaded `FnBodyMap`
  through `safety::build_model_graph` (the shared classification call site),
  and fixed a root-cause bug in the standalone `expand_function_calls` helper
  itself: it never stripped frontmatter before parsing, so every production
  caller (including the *real execution windowing path*,
  `execute.rs`'s `compute_incremental_windows_ordered`) silently never
  expanded `smelt.define` bodies at all — a correctness gap wider than this
  phase's stated scope, now fixed for every caller. Consequence: `silver.sessions`
  in `examples/web_analytics` now genuinely needs
  `safety_overrides.allow_window_functions: true` (added, with rationale —
  a true positive per the plan's task 6 guidance, not a classifier bug) and
  its bound-based chunk size corrected from a stale 7-day/5-chunk split
  (computed against the always-broken expansion path) to the true 12-day/
  3-chunk split; `rebuild_dry_run.rs`'s golden values and one LSP goto-def
  line number were updated to match. No phase reshape.

- 2026-09-04 — Phase 3 planned. No phase row added, split, or reordered: the
  phase-2 summary's only forward-looking finding (a window function nested in a
  `CASE` arm is invisible to `temporal.rs`'s AST walk) is advisory-only — it
  gates nothing and matches no residue bullet this outcome owns — so it is
  recorded under §"Out of scope" rather than given a phase row. Phase 3 keeps
  its audited scope: the outer-visibility check's Case 2 matches only a bare
  parenthesized subquery in `FROM`, so the `WITH … FROM <cte>` form escapes.

- 2026-09-04 — Phase 3 implemented (`phases/03-summary.md`). Landed Case 3 in
  `check_event_time_injectable` (`crates/smelt-logical/src/rules/rule_diagnostics.rs`):
  a `FROM` naming a CTE that doesn't project `event_time_column` is rejected, resolved
  through a chain of CTEs with conservative fallback for wildcards, set-operation bodies,
  `WITH RECURSIVE`, and joined outer FROMs. Spec and diagnostics catalogue updated; probe
  inverted. Swept `examples/` — two batched models with CTE-shaped outer FROMs
  (`silver/sessions.sql`, `silver/sessions_chained.sql`) both already project the column;
  no example changes needed. No phase reshape.

- 2026-09-04 — Phase 4 planned. No phase row added, split, or reordered: the
  phase-3 summary surfaced no new work (its only forward-looking finding was
  the already-out-of-scope `CASE`-nested-window gap, and its `examples/` sweep
  found no over-reach). Phase 4 needed one design call the outcome text leaves
  open — *which* per-model overrides the closed `ModelDef` field set should
  admit. Made in the plan rather than escalated as a blocker: the concrete
  pressure `meta_language.md` §Design asks for is exactly the partition grain,
  so the set opens by two record-typed optional fields, `timeseries` and
  `safety_overrides`, spelt like the frontmatter keys they replace, with
  whole-block replacement (not key-level merge) and incremental-only
  applicability enforced by a new fail-loud diagnostic. Other frontmatter keys
  (`owner:`, `backend_hints:`, `target:`) stay closed — no residue bullet
  demands them.

- 2026-09-04 — Phase 4 implemented (`phases/04-summary.md`). Landed `timeseries` /
  `safety_overrides` as `MODEL_DEF_FIELDS` fields 6–7 (Record-typed, bespoke
  required/optional sub-field validation, whole-block replacement, incremental-only via
  the new `ModelDefOverrideRequiresIncremental` diagnostic). Both emission paths
  (array-literal and loader-driven lambda) extract and apply the override;
  `discovery::model_file_from_emitted_def` needed no changes since it already clones
  `EmittedModelDef`'s config fields verbatim. No phase reshape.

- 2026-09-04 — Phase 5 **split** into 5a (axis domain: typed run window,
  validation, backfill chunking) and 5b (emission: scan-filter/DELETE literal
  rendering, explain clamp, end-to-end first-run/backfill/steady-state proof).
  Reason: staging the probe by hand against the built binary showed the residue
  is two distinct changes stacked — the run window is `chrono::NaiveDate` in
  `windowing.rs`'s `IncrementalBatch` *and* the partition literal is hard-quoted
  (`format!("'{}'", …)`) at four `Region` construction sites in `execute.rs`, so
  a monotone-integer model dies at `DELETE … WHERE batch_id >= '2026-01-01'`
  (`Could not convert string '2026-01-01' to INT32`). Retyping the axis alone
  touches `windowing.rs`, ~32 format sites in `execute.rs` and four `smelt-cli`
  callers; folding the emission change and the e2e oracle into the same phase
  would make one commit that cannot be reviewed against a single red test.
  Nothing is deferred out — 5b carries every remaining part of success
  criterion 5. Later rows keep their numbers (the loop's row parser accepts the
  `5a`/`5b` form), so `audit.md`'s residue→phase mapping now reads residue #11 →
  phases 5a+5b.
- 2026-09-04 — Phase 5a planned. One design call the outcome text leaves open,
  made in the plan rather than escalated: *what a run window means on an integer
  partition axis*. `timeseries.md` rule 4 admits an integer `partition_column`
  and `monotonicity.rs` already derives `Offset::Integer` for it, but nothing
  says what a partition or a chunk is there. Chosen: a **unit-step integer
  grid** — one partition is one integer value, the run window's bounds are given
  as bare integers in the axis's own domain, `--batch-size N` counts N units, and
  `granularity` keeps its declared-propagation-grain role (grain alignment, graph
  edges) without being the chunk step. Rejected: (a) treating the integer as an
  epoch encoding of a calendar window, which needs an encoding smelt is never
  told; (b) probing the target for the batch-id range covering a date window,
  which makes chunk shape depend on live data. Day-typed widening inputs
  (`data_latency`, seconds-domain lookback/skew) have no conversion into the unit
  grid and are refused fail-closed rather than coerced.
- 2026-09-04 — Phase 5a implemented: `PartitionAxis`/`PartitionPoint` land in
  `smelt-logical`/`smelt-runtime::windowing`; `compute_incremental_windows{,_ordered}`
  take an axis and dispatch to a calendar branch (byte-identical to before) or a new
  unit-step integer branch with a fail-closed day-typed-widening refusal;
  `execute.rs` resolves each selected model's `partition_column` type via the same
  `resolved_model_schema` read `UpstreamSchemas::from_database` already performs,
  falling back to the axis implied by the run-window literal's form when the type is
  unresolvable. All standing gates (`statement_parity`, `rebuild_dry_run`,
  `maintenance_conformance`) stay green — calendar-axis output is unchanged.
  `probe_integer_partition_column_run` stays red as designed, but did **not** move
  off the DELETE-literal `INT32` error: the probe fixture's `batch_id` column
  resolves to `Unknown(Dynamic)` through `resolved_model_schema` (a `VALUES`-literal
  column threaded through one `smelt.ref()` hop with no metadata sidecar), so axis
  resolution falls back to the literal-implied axis, which happens to read as
  `Calendar` for the probe's `2026-01-01`-shaped bound. Verified the mechanism
  itself is correct by adding an explicit `CAST(... AS INTEGER)` to the fixture,
  which does trigger the intended fail-loud refusal
  (`docs/outcomes/20260815-partition-grain-residue/phases/05a-summary.md`).

- 2026-09-04 — Phase 5b planned. No phase row added, split, or reordered. Two
  findings from the 5a summary were absorbed into 5b rather than deferred:
  (i) `contract.frozen_horizon` on an integer axis is today *silently
  unclamped* (5a's `(Date, Date)` match arm just skips it) — a fail-loud-
  discipline gap in code this phase already touches, so 5b turns it into a
  refusal; (ii) the probe fixture's `batch_id` resolving to `Unknown(Dynamic)`
  is handled by option (b) of the 5a summary — the fixture gains an explicit
  `CAST(... AS INTEGER)` so axis resolution is real rather than literal-implied.
  Option (a), fixing the underlying inference of a `VALUES`-literal column
  crossing one `smelt.ref()` hop, is a pre-existing type-inference limitation
  that no success criterion depends on and is not folded in; the summary
  records it. `smelt explain` gains axis-awareness from the `--period`
  literal's own form (it has no Salsa handle to resolve a schema), which is the
  same fail-open posture `build_model_plans` already uses, hoisted into one
  shared helper rather than duplicated.

- 2026-09-04 — Phase 5b implemented (`phases/05b-summary.md`). Landed the
  single-owner axis renderer (`partition_literal`/`Region::for_axis`),
  threaded `axis` through `PartitionRange`/`TimeRange` to every emission
  site (backend crates' DELETE builders, `transformer.rs`'s clamp/pushdown
  injection, `smelt explain`'s derived window and region), refused
  `contract.frozen_horizon` on an integer axis, and inverted
  `probe_integer_partition_column_run` into a real first-run/backfill/
  steady-state-vs-full-refresh-oracle proof. Fixed a double-quoting bug in
  `wrap_source_ref_with_filter` surfaced by `statement_parity` going red
  mid-phase. Success criterion 5 complete; the matching Known Divergences
  bullet removed from `docs/specs/incremental_shapes.md`. No phase reshape.

- 2026-09-04 — Phase 6 planned. No phase row added, split, or reordered. The 5b summary's
  two forward items both land here rather than being deferred: the stale "specified ahead
  of a tracking plan" claim is fixed by *removing* the whole bullet (phase 6 closes both of
  its halves, so phase 8 has nothing left to correct), and the untouched editor-hover half
  is task 5–6 of this phase. Two design calls the outcome text leaves open, made in the
  plan rather than escalated: (i) *where the run window enters `explain --json`* — reuse the
  existing `--period` flag (already parses both calendar and integer forms since 5b) by
  dropping its `requires = "show_sql"`, rather than adding a second
  `--event-time-start`/`--end` pair that would then need its own axis parsing; (ii) *who
  owns the scan-window arithmetic* — the resolver is extracted out of
  `inject_source_filters` into `smelt-logical` beside `Offset`, so the window the report
  prints and the filter the run pushes down cannot drift (the alternative, a second
  offset→date arithmetic in `explain.rs`, is exactly the duplication the maintenance-plan
  purity rule forbids). A non-uniform (month/year) offset resolves to an explicit
  unresolved verdict naming the unit rather than a coerced day count, per fail-loud
  discipline. The LSP half needs no new crate dependency: `smelt-db` gains a thin Salsa
  wrapper over the pure `derive_model_bounds` and re-exports `BoundResult`/`Offset`.

- 2026-09-04 — Phase 6 implemented (`phases/06-summary.md`). Landed the shared
  `smelt_logical::resolve_scan_window` resolver, threaded through
  `inject_source_filters` (byte-identical calendar output) and `explain --json`'s
  `source_bounds` (`scan_start`/`scan_end`/`scan_unresolved`, gated on `--period`
  no longer requiring `--show-sql`), plus `smelt_db::model_source_clamps` and the
  LSP hover formatter for the editor-hover half. Spec updated; stale Known
  Divergences bullet removed; `probe_explain_json_run_relative_source_bounds`
  inverted. No phase reshape.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
