# Property-discovery ledger

Per-cell verdicts, appended by the property-discovery loop (one block per resolved catalog cell).
This is the deliverable: the empirical map of which `(construct × source × technique)` cells hold,
and — the headline — where **smelt's own analyzer/maintenance is unsound or over-conservative**.

Verdict vocabulary (design §2.4): **HOLDS** = "no counterexample found over N schedules" (never
"proven"); **REFUTED** = a witness schedule diverges (a mapped admission-matrix boundary or a smelt
bug); **CONDITIONAL** = holds only under a named traded guarantee; **BLOCKED** = a design fork or
missing infra.

Block schema:

```
### CELL <id> — <construct> × <source_property> × <technique>
- verdict: HOLDS | REFUTED | CONDITIONAL | BLOCKED
- P (Link 0): <property>          skeleton_cols (Link B): <set>
- Link B facts: combiner=<…> reach=<(b,a)|Unbounded|NotDerivable> footprint=<bounded|unbounded>
- smelt analyzer: sound | over-conservative | unsound | not-derivable      [← ACTION if not sound]
- Link C: no divergence over <N> schedules | WITNESS: <breaking schedule + EXCEPT ALL rows>
- condition (CONDITIONAL only): <named guarantee traded, paper §6>
- experimental smelt extensions (if any): <sites tagged EXPERIMENTAL(property-discovery)>
- evidence: <test path::name>, <schedule count>, <oracle mode>
```

---

<!-- The loop appends verdict blocks below this line. -->

### CELL P0-1 — (infra) × (infra) × in-process real-planner PBT harness
- verdict: HOLDS (infra deliverable — not a construct/source/technique cell; "HOLDS" here means the
  harness demonstrably drives the real path, not that a property held)
- P (Link 0): n/a          skeleton_cols (Link B): n/a
- Link B facts: n/a
- smelt analyzer: n/a
- Link C: n/a — this cell builds the Link-C harness itself, it does not yet run a property cell
  through it
- experimental smelt extensions (if any): none in production code. Added
  `crates/smelt-cli/tests/property_discovery/{main,link_c_harness,model_shapes,smoke}.rs`
  (`LinkCProject`, `SqlCapturingReporter`, `DuckDbBackendFactory`, `base_request`; `model_shapes` =
  the single model-SQL catalogue) — test-target-only, tagged
  `EXPERIMENTAL(property-discovery): disposable`, passes `property-experimental-gate.sh`. Uses
  `smelt-cli`'s existing `smelt-runtime`/`smelt-backend-duckdb`/`tokio`/`tempfile` deps under the
  `duckdb` feature — **no manifest change**. (Originally built in `smelt-db`'s tests + dev-deps;
  relocated to `smelt-cli` to avoid the `smelt-runtime → smelt-db` dev-dependency cycle.)
- evidence: `smelt-cli::tests::property_discovery::smoke::execute_project_derives_time_filter_no_hand_injected_where`
  (1 run, DuckDB oracle via `duckdb::Connection` read-back; run with `--features duckdb`). Stages a `refresh: batched` model with
  **no `WHERE` clause anywhere in its SQL**, runs it through `LinkCProject::run` →
  `smelt_runtime::execute_project` (the real bound-derivation + planner path, `execute_parity.rs`'s
  plumbing pattern generalised), and asserts the SQL captured via
  `RunReporter::model_compiled` contains a derived `WHERE` clause referencing the partition column —
  proof the filter came from `source_bounds`/`inject_time_filter`, not from the test. Also asserts
  the materialized table only contains rows inside the requested window, read back through a fresh
  DuckDB connection (not the same handle `execute_project` used).
  This harness (`LinkCProject`, `SqlCapturingReporter`) is the substrate every subsequent Link-C cell
  (`SC-1`, `SC-2`, the `G-*` grid) will build on; it does not itself carry a construct/source/technique
  verdict.

### CELL P0-2 — (infra) × (infra) × run-schedule generator + step-k snapshot driver
- verdict: HOLDS (infra deliverable — not a construct/source/technique cell; "HOLDS" here means the
  driver demonstrably captures between-run source mutation, not that a property held)
- P (Link 0): n/a          skeleton_cols (Link B): n/a
- Link B facts: n/a
- smelt analyzer: n/a
- Link C: n/a — this cell builds the run-schedule driver Link-C cells replay against, it does not
  itself run a construct through it
- experimental smelt extensions (if any): none in production code. Added
  `crates/smelt-cli/tests/property_discovery/{run_schedule,p0_2_run_schedule}.rs` — test-target-only,
  tagged `EXPERIMENTAL(property-discovery): disposable`, passes `property-experimental-gate.sh`.
  `run_schedule.rs` defines `ScheduleStep` (`AdvanceWindowAndRun` | `AppendLateRow` |
  `InPlaceUpdate` | `InPlaceDelete`), `RunSchedule` (a `Vec<ScheduleStep>`), a bounded proptest
  strategy `arb_schedule` (2-4 window-advance steps, each optionally followed by a late-row append
  landing back inside the window just processed — reserved for Link-A/Link-C cells to draw from;
  not yet exercised by a `proptest!` macro since no construct cell consumes it yet), and
  `RunScheduleDriver::execute`, which replays a `RunSchedule` against a staged `LinkCProject` +
  its seeded source table, snapshotting the source contents (`snapshot()`) after every step — the
  step-`k` full-refresh oracle baseline `P0-3`'s `EXCEPT ALL` oracle will consume. Added `proptest`
  as a `smelt-cli` dev-dependency (workspace version, no manifest widening beyond that). The `d`
  column is round-tripped via `CAST(... AS VARCHAR)` + `NaiveDate::parse_from_str` rather than a
  `chrono`-feature `FromSql` impl, since the workspace `duckdb` dep doesn't enable that feature and
  widening it isn't warranted for a disposable harness.
- evidence: `smelt-cli::tests::property_discovery::p0_2_run_schedule::step_k_snapshot_differs_from_pre_populated_after_late_append`
  (1 deterministic schedule, DuckDB oracle via direct read-back). Stages `model_shapes::batched_passthrough`,
  seeds 2 rows, then runs a 3-step schedule: `AdvanceWindowAndRun(day1,day2)` →
  `AppendLateRow(d=day1, id=99)` → `AdvanceWindowAndRun(day2,day3)`. Asserts (a) the snapshot taken
  right after step 0 does NOT contain the late row (the driver isn't pre-populating), (b) the final
  snapshot does, (c) the step-0 snapshot and the final ("pre-populated equivalent") snapshot are
  `assert_ne!` — the exact gap a step-`k` oracle exists to close per design N3 — and (d) a live
  read-back of the source table matches the last recorded snapshot (`snapshot()` isn't stale).

### CELL P0-3 — (infra) × (infra) × EXCEPT ALL step-k oracle
- verdict: HOLDS (infra deliverable — not a construct/source/technique cell; "HOLDS" here means the
  oracle demonstrably distinguishes multiset from set equality, not that a construct property held)
- P (Link 0): n/a          skeleton_cols (Link B): n/a
- Link B facts: n/a
- smelt analyzer: n/a
- Link C: n/a — this cell builds the multiset-equality primitive Link-C cells diff against; it does
  not itself run a construct through `execute_project`
- experimental smelt extensions (if any): none in production code. Added
  `crates/smelt-cli/tests/property_discovery/oracle.rs` — test-target-only, tagged
  `EXPERIMENTAL(property-discovery): disposable`, passes `property-experimental-gate.sh`. Defines
  `except_all_row_count`/`except_row_count` (raw `EXCEPT ALL` / `EXCEPT` row counts between two SQL
  queries) and `multiset_equal` (the Link-C oracle: equal iff `EXCEPT ALL` is empty in both
  directions). Column scope (all-columns-minus-declared-payload, N2) is the caller's responsibility —
  this module is the mechanical multiset-equality primitive, not the per-cell scoping policy.
- evidence: `smelt-cli::tests::property_discovery::oracle::{duplicated_identical_row_is_visible_to_except_all_but_not_except,
  equal_multisets_compare_equal_regardless_of_row_order,
  unequal_multisets_of_distinct_rows_diverge_in_both_except_forms}` (3 deterministic DuckDB
  fixtures, in-memory). Proves the plan's acceptance (ii): a row duplicated on one side (`(1,10.0)`
  twice vs once) is invisible to plain `EXCEPT` (count 0) but surfaces to `EXCEPT ALL` (count 1) and
  `multiset_equal` correctly reports divergence — the exact additive double-counting shape cell
  `G-02` (re-delivered delta into a `SUM`/`COUNT` fold) will exercise through the real planner. Also
  proves row-order independence and that a genuine set difference (an extra distinct row) is caught
  by both forms.

### CELL P0-4 — (infra) × (infra) × generator MutationProfile self-check
- verdict: HOLDS (infra deliverable — not a construct/source/technique cell; "HOLDS" here means the
  self-check demonstrably distinguishes a schedule that matches its declared `MutationProfile` from
  one that doesn't, on both the positive and negative path)
- P (Link 0): n/a          skeleton_cols (Link B): n/a
- Link B facts: n/a
- smelt analyzer: n/a
- Link C: n/a — this cell builds the label-verification primitive later Link-C cells call before
  trusting a schedule's declared profile; it does not itself run a construct through
  `execute_project`
- experimental smelt extensions (if any): none in production code. Extended
  `crates/smelt-cli/tests/property_discovery/run_schedule.rs` (test-target-only, tagged
  `EXPERIMENTAL(property-discovery): disposable`, passes `property-experimental-gate.sh`) with:
  `MutationProfile` (`AppendOnly` | `Mutable`), `check_profile(&RunSchedule, MutationProfile) ->
  Result<(), (usize, ScheduleStep)>` (an `AppendOnly` schedule containing any `InPlaceUpdate`/
  `InPlaceDelete` step fails, returning the offending index + step; `Mutable` permits both), and
  `arb_mutable_schedule` — a `Mutable`-profile companion to `P0-2`'s `arb_schedule` that generates
  2-4 window-advance runs with a guaranteed in-place `UPDATE` of a previously-seeded row inserted
  after one of them (the SC-2 shape), so a `Mutable`-declared schedule provably exercises the
  mutation hazard rather than silently degenerating into an append-only run.
- evidence: `smelt-cli::tests::property_discovery::p0_4_mutation_profile_selfcheck::{
  self_check_detects_an_in_place_update_mislabeled_append_only,
  self_check_detects_an_in_place_delete_mislabeled_append_only,
  mutation_steps_are_permitted_under_the_mutable_profile,
  arb_schedule_output_always_matches_its_declared_append_only_profile (proptest, 256 cases),
  arb_mutable_schedule_output_matches_profile_and_actually_mutates (proptest, 256 cases)}`. Red-green:
  the two hand-constructed cases prove the self-check actually rejects an `InPlaceUpdate`/
  `InPlaceDelete` step mislabeled `AppendOnly` (not a vacuous always-`Ok` check) and that the same
  steps are accepted under `Mutable`; the two proptests are the F7 self-check proper — every
  schedule `arb_schedule` (declared `AppendOnly`) emits passes the `AppendOnly` check, and every
  schedule `arb_mutable_schedule` (declared `Mutable`) emits both passes its check AND contains at
  least one mutation step, so the `Mutable` label is never an unverified, unexercised claim.

### CELL P0-5 — (infra) × (infra) × Link-A abstract contract-safety scaffold
- verdict: HOLDS (infra deliverable — not a construct/source/technique cell; "HOLDS" here means
  the scaffold's own predictions match Link 0 on both arms: idempotent-monoid fold-delta holds
  under reorder/re-delivery/backfill, and MIN-under-retraction is a deterministic REFUTED witness)
- P (Link 0): additive monoid (non-idempotent) vs idempotent monoid (`MAX`/`MIN`)
  skeleton_cols (Link B): n/a — abstract scaffold, no concrete SQL construct/columns yet
- Link B facts: n/a (Link-A runs before any concrete construct is classified; that is `P0-6`)
- smelt analyzer: n/a — this cell never touches smelt's analyzer or `execute_project`; it is the
  abstract pre-filter Link C (real cells) replays against real smelt
- Link C: n/a for this cell itself; it *produces* the schedule kinds (partition/reorder/
  re-delivery/backfill/late-arrival/retraction) that `SC-1`/`G-02`/`G-04`/`G-08` will replay
  through `execute_project`
- experimental smelt extensions (if any): none — pure abstract Rust model, no smelt-internal
  extension, so the `EXPERIMENTAL(property-discovery)` tag/gate does not apply to this cell
- evidence: `smelt-db::tests::proptests::maintenance_link_a::{
  idempotent_monoid_fold_delta_matches_batch_over_reorder_redeliver_backfill (proptest, 256 cases:
  MAX/MIN × partition/reorder/re-delivery/backfill schedules all match the batch aggregate — HOLDS),
  additive_monoid_fold_delta_with_ledger_matches_batch_over_reorder_redeliver_backfill (proptest,
  256 cases: control proving the ledger, not idempotency, is what keeps SUM correct here),
  late_arrival_beyond_horizon_makes_fold_delta_diverge_from_batch (proptest, 256 cases: a delta
  landing beyond a technique's derived horizon is silently dropped by fold-delta but seen by batch
  — the abstract shape of the `SC-1` hazard),
  min_fold_delta_diverges_when_the_unique_minimum_is_retracted (deterministic unit test: retracting
  the unique minimum leaves fold-delta's memory stale while batch recomputes a new minimum — a
  REFUTED witness matching Link 0's "MIN over retractable? no (non-invertible)" prediction, the
  abstract shape `G-04` will replay through real smelt)}`. Red-green: the abstract model was built
  to make exactly these two predictions (idempotent-monoid HOLDS, MIN-under-retraction REFUTED)
  and both assertions pass as written — this cell is a scaffold, not a discovery about smelt.

### CELL P0-6 — (infra) × (infra) × Link-B classification-diagnostic scaffold
- verdict: HOLDS (infra deliverable — not a construct/source/technique cell; "HOLDS" here means the
  scaffold's independent DuckDB clamp-probe agrees with smelt's own analyzer reach for the fixed
  Form-B model, i.e. the analyzer is demonstrated **sound and tight** for this one construct — not
  that a Link-C property held)
- P (Link 0): `SUM` — commutative monoid, non-idempotent (additive; `discriminants.rs`)
  skeleton_cols (Link B): none for this construct — a bare group-less `SUM(payload)` has no
  grouping/dedup/ordering key; the skeleton floor is empty (payload-only aggregate)
- Link B facts: combiner=additive monoid (`is_monoid=true, needs_inverse=false`, matches Link 0's
  `SUM`/`COUNT` row) reach=`Bounded(event_date, before=1d, after=0)` via
  `source_bounds::derive_model_bounds` over the Form-B model
  `SELECT SUM(s.payload) FROM sessions s WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1
  day' AND m.partition_date` footprint=bounded
- smelt analyzer: sound — the independent DuckDB clamp-probe (restrict the *read* to a candidate
  margin, then apply the model's own filter on top) confirms 1 day is both **sufficient** (every
  generated dataset: margin=1d clamp always equals the true output) and **necessary** (margin=0d
  diverges on a row exactly at `partition_date - 1 day`) — the analyzer's derived reach is not just
  safe but tight for this construct
- Link C: n/a — this cell is a static classification diagnostic (analyzer facts vs an independent
  DuckDB probe); it does not run the model through `execute_project`. It is the diagnostic Link C
  cells (`G-01` etc.) reuse to localize *why* a reach fact was right or wrong
- condition (CONDITIONAL only): n/a
- experimental smelt extensions (if any): none — reuses smelt's existing analyzer APIs
  (`smelt_logical::analysis::{discriminants::combiner_discriminants, source_bounds::derive_model_bounds}`)
  read-only; no `EXPERIMENTAL(property-discovery)` tag applies
- evidence: `smelt-db::tests::proptests::maintenance_link_b::{
  clamp_probe_at_derived_margin_matches_true_output (proptest, 256 cases: 0-7 generated rows at
  offsets -3..=2 days from the partition date, random payloads — clamp-probe at the analyzer's
  derived 1-day margin always matches the model's true `SUM` output),
  clamp_probe_at_one_less_than_derived_margin_diverges_on_boundary_row (deterministic witness: a
  single row at exactly `partition_date - 1 day` is inside the model's filter but a 0-day-margin
  clamp misses it — proves the derived bound is *tight*, not merely conservative)}`. Red-green: the
  scaffold was built to make exactly these two predictions (sufficiency + tightness) and both
  assertions pass as written on the first construct classified — this cell validates the diagnostic
  harness itself works end-to-end, not yet a discovery about a construct the property grid (`G-*`)
  hasn't reached.

### CELL SC-1 — correlated EXISTS (7-day attribution) × append-only × recompute-region (Link C)
- verdict: HOLDS — hypothesis REFUTED (no divergence found; the predicted unsound acceptance did
  not reproduce for this construct's exact SQL shape)
- P (Link 0): n/a — correlated `EXISTS` has no combiner identity in the Link-0 table (it is a
  boolean membership test, not a fold); this cell is about reach derivation, not an algebraic
  property
  skeleton_cols (Link B): `user_id`, `event_date` (the `unique_key`; `converted` is the sole
  payload column)
- Link B facts: reach for `conversions` = `Bounded(conversion_date, before=0, after=7d)` via
  `source_bounds::derive_model_bounds` — **not** the hypothesized zero-margin fallback
- smelt analyzer: sound (for this shape) — but by an accident of implementation, not a reasoned
  derivation. `derive_bound_for_source`'s Form-B extractor (`extract_form_b_bounds`) scans the
  **whole model SQL as one text blob** for any `... BETWEEN <expr> AND <expr> + INTERVAL '...'`
  pattern; it takes a `_partition_col_upper` parameter but never uses it to check that the matched
  columns are actually the source's own partition column (`crates/smelt-logical/src/analysis/
  source_bounds.rs:589`, `_partition_col_upper` unused). The correlated EXISTS predicate
  `c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL '7 days'` happens to be the
  *only* BETWEEN+INTERVAL pattern in the model, so it gets attributed to `conversions` (correctly,
  here) — but the same derivation would attribute the *same* bound to `events` too (confirmed: the
  compiled SQL's `events` read filter is also widened to `< 2024-01-09`, a spurious 7-day
  over-read the outer `inject_time_filter` clamp happens to absorb harmlessly). The "no temporal
  dependency ⇒ Bounded(0,0)" fallback the hypothesis named is real code
  (`source_bounds.rs:406-411`) but is never reached for this shape, because the Form-B scan matches
  first, column-blind.
- Link C: no divergence over 1 seeded schedule (deterministic, not proptest-generated — see
  Coverage caveat below) — `run 1` processes `[2024-01-01, 2024-01-02)` with no conversion; a late
  conversion (`user_id=1, conversion_date=2024-01-03`) is appended directly to
  `main.sources_conversions` *between* runs (never pre-populated); `run 2` re-runs (backfills) the
  same window and correctly picks up the late row (`converted` flips to `true`, matching the
  full-refresh oracle). Compiled SQL for `conversions` on run 2:
  `WHERE conversion_date >= '2024-01-01' AND conversion_date < '2024-01-09'` — the 7-day forward
  margin is present.
- condition (CONDITIONAL only): n/a
- experimental smelt extensions (if any): none — the cell reads `RunReporter::model_compiled`
  output through the existing `SqlCapturingReporter` (already `EXPERIMENTAL(property-discovery)`
  from `P0-1`); no new production or analyzer code touched.
- evidence: `smelt-cli::tests::property_discovery::sc_1_correlated_exists::
  late_conversion_appended_between_runs_within_7_day_window` (deterministic 2-run schedule through
  `execute_project`, no hand-injected `WHERE`).
- Coverage caveat (design §2.1 N4): this is a single hand-authored schedule, not a proptest-shrunk
  family — appropriate for a seed-bug reproduction attempt (the goal was "does the hypothesized
  fallback ever fire for this shape", answered no), but it does not rule out the fallback firing
  for a *differently shaped* correlated-EXISTS query (e.g. one with the interval on the `BETWEEN`
  lower bound, or a query with a second unrelated `BETWEEN` earlier in the text that the scan
  matches instead — see the appended follow-on cell `SC-1b`).

### CELL SC-2 — pass-through + additive agg (SUM, batched unique_key=[d]) × clocked mutable-snapshot × recompute-region (Link C)
- verdict: HOLDS — hypothesis REFUTED (no divergence found for an explicit backfill of the
  mutated partition; the predicted unsound acceptance did not reproduce)
- P (Link 0): `SUM` — commutative monoid, non-idempotent (additive; matches the Link-0 table row;
  `discriminants.rs`)
  skeleton_cols (Link B): `d` (the `unique_key` / partition column; `total` is the sole payload
  column)
- Link B facts: `input_delta.rs:88-93` (`input_delta_discovery`) classifies this source
  `WindowForward` for `has_clock=true` **independent of `mutation_profile`** — the match arm order
  is `Some(ChangeFeed) => ChangeFeed`, then `_ if has_clock => WindowForward`, so a clocked
  `Mutable` source takes the identical branch a clocked `AppendOnly`/undeclared source would. This
  is the hypothesis's premise and is confirmed as written.
  **However**: a repo-wide grep (`rg -n "input_delta_discovery|InputDeltaKind"`, excluding its own
  module and `#[cfg(test)]`) finds **zero call sites outside `input_delta.rs`'s own unit tests** —
  neither `smelt-runtime::maintenance_driver` (the actual batched-partition INSERT/MERGE driver,
  which has no `mutation_profile`-conditioned branch at all) nor any other production path reads
  this function's verdict. It is a **proof-only artifact, not yet wired to any consuming mode**
  (matches its own doc comment: "the re-scan/probe transform this verdict licenses is wired per
  consuming mode (L4), not here"). smelt's actual emitted maintenance for this cell does not
  consult `MutationProfile`/`InputDeltaKind` at all — what actually governs re-processing a
  partition is simply whether that partition falls inside the run's requested
  `[start, end)` window, exactly as for an append-only source.
- smelt analyzer: **not-derivable** (for the runtime-behaviour question) rather than unsound —
  `input_delta_discovery`'s `WindowForward` verdict for `Mutable` is a real, confirmed
  classification, but it has no observable effect on the emitted maintenance SQL because nothing
  in the execution path consumes it yet. The Link-C divergence the hypothesis predicted (an
  explicit backfill of the mutated partition still missing the mutation) does not reproduce,
  because batched partition maintenance is unconditionally recompute-region: re-running a window
  always re-derives `SUM` fresh from current source contents for the requested partition,
  regardless of any mutation-profile classification.
- Link C: no divergence over 1 seeded schedule (deterministic, not proptest-generated — see
  Coverage caveat below). Seeded row `(d=2024-01-01, id=1, val=10.0)`; run 1 processes
  `[2024-01-01, 2024-01-02)`, materializing `total=10.0`. Between runs, `id=1`'s `val` is updated
  in place to `999.0` (the already-processed partition, mutated between runs — never
  pre-populated). Run 2 advances FORWARD to `[2024-01-02, 2024-01-03)` — as expected (not a bug),
  the never-re-requested `2024-01-01` partition stays stale at `10.0`, establishing the baseline.
  Run 3 explicitly re-runs (backfills) the SAME `[2024-01-01, 2024-01-02)` window run 1 processed:
  the maintained `total` becomes `999.0`, matching the full-refresh oracle. Compiled SQL for the
  backfill run: `SELECT d, SUM(val) AS total FROM main.sources_events WHERE d >= '2024-01-01' AND
  d < '2024-01-02' GROUP BY d` — a fresh, unconditional re-read of current source contents for the
  requested partition (recompute-region), not a fold over a remembered delta.
- condition (CONDITIONAL only): n/a
- experimental smelt extensions (if any): none — reuses the existing `SqlCapturingReporter`
  (`EXPERIMENTAL(property-discovery)` from `P0-1`) and the `run_schedule`/`link_c_harness`
  infrastructure; no new production or analyzer code touched. Added `mutation_profile: mutable` to
  the staged source YAML (a pre-existing, non-experimental declaration surface).
- evidence: `smelt-cli::tests::property_discovery::sc_2_clocked_mutable_window_forward::
  in_place_update_of_already_processed_partition_is_missed_on_forward_only_advance` (deterministic
  3-run schedule through `execute_project`, no hand-injected `WHERE`).
- Coverage caveat (design §2.1 N4): single hand-authored schedule, not proptest-shrunk — the goal
  was "does an explicit backfill of a mutated partition still miss the mutation", answered no.
  This does NOT establish that a plain forward-only advance ever revisits a mutated partition
  without an explicit backfill request (it provably doesn't — see the run-2 assertion above, which
  is the expected/documented limitation, not the hazard this cell hunts). It also does not rule out
  a divergence for a technique other than the simple per-partition `SUM`/`unique_key` batched form
  tested here (e.g. a stateful fold-delta technique that *does* consult `mutation_profile` once one
  is wired) — this cell is scoped to the one technique smelt's batched materialization currently
  emits for this construct.

### CELL G-01 — additive agg (SUM/COUNT) group-by × append-only × fold-delta
- verdict: HOLDS
- P (Link 0): commutative monoid (additive, non-idempotent; Link 0 table §2.0)
  skeleton_cols (Link B): `{d}` (the `unique_key`/`GROUP BY` column — determines row
  existence/grouping; `total` is payload)
- Link B facts: combiner=additive-monoid(SUM) reach=n/a (no correlated/cross-source read; disjoint
  per-partition aggregation) footprint=bounded (one partition's rows only)
- smelt analyzer: sound — batched per-partition materialization (`unique_key: [d]`) recomputes
  `SUM(val)` fresh from current source contents for exactly the requested `[start, end)` window on
  every run; a disjoint append-only schedule never asks it to fold a delta against remembered
  state, so there is no ledger/dedup obligation to violate.
- Link C: no divergence over 8 proptest cases (2-4 disjoint one-day windows, 1-3 rows/window,
  values in `[-50, 50]`). Every window's rows are inserted before that window is ever requested
  (no lateness, no re-delivery, no revisit) — the disjoint-delta control shape design §4 predicts
  HOLDS unconditionally for an additive combiner. Confirmed: `maintained_total(d) ==
  full_refresh_total(d)` for every processed partition, every case.
- condition (CONDITIONAL only): n/a
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request`; adds
  `model_shapes::additive_agg_append_only` (a plain `ModelShape`, not experimental analyzer code)
  and a source YAML declaring `mutation_profile: append_only`.
- evidence: `smelt-cli::tests::property_discovery::g_01_additive_agg_append_only::
  additive_sum_fold_over_disjoint_append_only_windows_matches_full_refresh` (8 proptest cases
  through `execute_project`, no hand-injected `WHERE`).
- **Related finding (out of this cell's scope, recorded for follow-up — NOT this cell's verdict
  driver):** the proptest generator originally drew arbitrary fractional `f64` values and found a
  real, reproducible divergence — `maintained_total` truncated to an integer (e.g. `11.0` instead
  of `11.094738641060989`). Root-caused (via subagent investigation) to
  `crates/smelt-db/src/queries/schema.rs::add_source_info_to_type_context` (~line 1356): it derives
  `(schema, table)` from a source's `address_segments` and requires `segs.len() >= 2`, silently
  `continue`-ing (dropping ALL of that source's declared columns from the `TypeContext`) for a
  source file at scan-root with a single-segment address (e.g. `sources/events.yml` →
  `["events"]`, as every `model_shapes` fixture in this loop declares it). With `val`'s `DOUBLE`
  type unresolved, `SqlFunction::Sum` (`crates/smelt-db/src/type_inference/function_call.rs`
  ~435-471) falls through to its historical `BigInt` default, and
  `crates/smelt-dialect/src/type_conformance.rs::wrap_with_type_casts` faithfully emits
  `CAST(total AS BIGINT)` — silently truncating any fractional aggregate. **This is NOT a
  fold-delta/schedule-safety bug** (this cell's target): it reproduces identically on a single
  non-incremental run with no schedule at all, so it says nothing about append-only maintenance
  correctness. It is, however, a real, general smelt correctness bug (silently corrupts financial
  aggregates over any scan-root-declared source with a non-integer combiner) already
  partially documented (`crates/smelt-db/tests/proptests/aggregate_widening.rs`'s header references
  the same failure class for a different trigger — an empty `TypeContext` — this is an
  **uncovered variant**: a populated-but-arity-mismatched `TypeContext`). Worked around in this
  cell by constraining generated values to whole numbers (`arb_disjoint_windows`, see its doc
  comment) so the wrong `BigInt` cast is a no-op; **not fixed** (this is a research loop, not an
  implementation loop — design §8/§9). Flagged here rather than filed as its own catalog cell
  because it is a type-inference defect, not a `(construct × source-property × technique)` cell in
  this catalog's schema; a human should triage it against
  `docs/research/20260417-0.3-regression-triage.md` bug #3 and `aggregate_widening.rs`.

### CELL G-02 — additive agg (SUM/COUNT) group-by × append-only (delta RE-DELIVERED) × fold-delta
- verdict: HOLDS
- P (Link 0): commutative monoid (additive, non-idempotent; Link 0 table §2.0) — the property under
  test is whether smelt's *execution mechanism* upholds re-delivery safety despite `SUM` itself
  being a non-idempotent combiner (re-delivery is unsafe for a technique that blindly appends a
  delta onto remembered state; the question is whether smelt's batched materialization is such a
  technique).
  skeleton_cols (Link B): `{d}` (the `unique_key`/`GROUP BY` column; `total` is payload)
- Link B facts: combiner=additive-monoid(SUM) reach=n/a footprint=bounded (one partition's rows
  only, resolved fresh from current source contents every run)
- smelt analyzer: sound — but the reason is not a ledger/dedup mechanism at all. A dedicated
  Explore pass (this cell) confirmed `refresh: batched` never resolves to a `unique_key`-scoped
  MERGE/upsert: `crates/smelt-backend/src/lib.rs::resolve_strategy` always returns
  `IncrementalStrategy::DeleteInsert` for a batched model (`unique_key` on `BatchedConfig` is
  reserved for diagnostics only — `let _ = unique_key;` at `lib.rs:195` — MERGE/`merge_into` is
  dead code on this path, it backs `cumulative_aggregate` instead). `delete_and_insert_transactional`
  (`crates/smelt-backend-duckdb/src/lib.rs:618-659`) runs, in one transaction,
  `DELETE FROM table WHERE col >= start AND col < end` then `INSERT INTO table {sql}`, where the
  DELETE range is exactly the run's `[start, end)` write window (`crates/smelt-runtime/src/
  execute.rs:970-973, 1028-1042` — "the DELETE range must equal exactly what the INSERT writes").
  Re-delivering an identical window is therefore a **full partition replace**, not a fold onto
  remembered state — there is no ledger obligation to violate because there is nothing folded onto;
  each run recomputes `SUM(val)` fresh from the source's CURRENT contents for that window alone.
- Link C: no divergence over 8 proptest cases (1-3 rows in a single one-day window, values in
  `[-50, 50]`, re-delivered 1-3 times with no new rows landing between re-runs — Link A kind (2),
  design §2.1). `maintained_total(d) == full_refresh_total(d)` after every re-delivery count tried,
  every case — re-delivery is a no-op vs a single run, never a double-count.
- condition (CONDITIONAL only): n/a
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request` and
  `model_shapes::additive_agg_append_only` (already added for `G-01`); adds no new model shape.
- evidence: `smelt-cli::tests::property_discovery::g_02_additive_agg_redelivery::
  redelivering_the_same_window_does_not_double_count_the_fold` (8 proptest cases through
  `execute_project`, no hand-injected `WHERE`).
- **Note on generality (not this cell's verdict driver):** this HOLDS is a mechanism-level fact
  about `refresh: batched`'s DELETE+INSERT-by-window-range strategy, not an algebra-level fact
  about `SUM` — the same non-idempotent combiner folded by a *different* smelt materialization
  path that DOES consult `unique_key` for a scoped upsert (`cumulative_aggregate`, mentioned above
  but out of this cell's scope) would need its own cell, since a real MERGE/upsert path is exactly
  where a re-delivery ledger obligation could actually be violated.

### CELL G-03 — idempotent agg (MAX/BOOL_OR) group-by × append-only × fold-delta
- verdict: HOLDS
- P (Link 0): idempotent monoid (`MAX`; Link 0 table §2.0) — folding the same delta twice is the
  identity, so this combiner has no ledger/dedup obligation even under a hypothetical
  fold-onto-remembered-state technique.
  skeleton_cols (Link B): `{d}` (the `unique_key`/`GROUP BY` column; `max_val` is payload)
- Link B facts: combiner=idempotent-monoid(MAX) reach=n/a (no correlated/cross-source read;
  disjoint per-partition aggregation) footprint=bounded (one partition's rows only)
- smelt analyzer: sound — same mechanism as `G-01`/`G-02`: batched refresh always emits
  `DELETE FROM table WHERE col IN [start,end)` then `INSERT INTO table {sql}` in one transaction
  (`crates/smelt-backend-duckdb/src/lib.rs::delete_and_insert_transactional`), a full partition
  *replace* recomputing `MAX(val)` fresh from current source contents every run — not a fold onto
  remembered state. There is no ledger obligation to violate for ANY combiner on this path
  (established generically by `G-02`); this cell confirms smelt does not do anything additionally
  unsound for the idempotent case, and exercises both adversarial dimensions (`G-01`'s disjoint
  windows, `G-02`'s re-delivery) together in one schedule.
- Link C: no divergence over 8 proptest cases (2-4 disjoint one-day windows, 1-3 rows/window,
  values in `[-50, 50]`, each window re-delivered 0-2 extra times after its first run with no new
  rows landing between re-runs). `maintained_max(d) == full_refresh_max(d)` for every processed
  partition after every re-delivery count tried, every case.
- condition (CONDITIONAL only): n/a
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request`; adds
  `model_shapes::idempotent_agg_append_only` (a plain `ModelShape`, not experimental analyzer code)
  over the same `events(d, id, val)` source shape as `G-01`/`G-02`.
- evidence: `smelt-cli::tests::property_discovery::g_03_idempotent_agg_append_only::
  idempotent_max_fold_over_disjoint_append_only_windows_with_redelivery_matches_full_refresh`
  (8 proptest cases through `execute_project`, no hand-injected `WHERE`).

### CELL FIX-1 — correlated EXISTS / Form-B reach derivation × multi-source × column-aware bound attribution (production fix)
- verdict: HOLDS (production fix landed — test-backed, no-regression gated; not a construct/source/
  technique discovery cell in the usual sense, but the follow-on production change `SC-1` recorded
  as a finding)
- P (Link 0): n/a — this is a reach-derivation fix (Form-B pattern attribution), not an algebraic
  combiner property
  skeleton_cols (Link B): `user_id`, `event_date` (unchanged from `SC-1`)
- Link B facts (before fix): `extract_form_b_bounds` scanned the whole model SQL text for any
  `BETWEEN … INTERVAL …` pattern and attributed the match to every source in `BoundContext`
  regardless of which column the pattern actually constrained — `_partition_col_upper` was accepted
  but unused (`crates/smelt-logical/src/analysis/source_bounds.rs:589`, pre-fix). For `SC-1`'s
  model, the correlated-`EXISTS` predicate `c.conversion_date BETWEEN e.event_date AND
  e.event_date + INTERVAL '7 days'` was correctly attributed to `conversions` but *also*
  spuriously attributed to `events` (confirmed: compiled `events` read widened to
  `< 2024-01-09` even though `event_date` never appears left of `BETWEEN` in the model).
- smelt analyzer (after fix): sound and now reasoned rather than accidental. Made
  `extract_form_b_bounds`, `extract_gte_lt_interval_bounds`, and
  `extract_gte_lt_bare_integer_bounds` column-aware: a new `lhs_column_is_partition_col` helper
  checks that the identifier immediately to the left of the matched `BETWEEN`/`>=`/`<` operator
  (bare or table-qualified, e.g. `E.EVENT_DATE`) is the source's own partition column before a
  match contributes to that source's bound; a match whose LHS column belongs to a different source
  is skipped for this source. Cross-column rebase (`WHERE b.event_ts_utc BETWEEN
  m.event_date_local - INTERVAL … AND m.event_date_local + INTERVAL …`, `test_cross_column_tz_rebase`)
  is preserved — only the LHS column is checked, not the RHS anchor expression.
- Link C: re-ran `sc_1_correlated_exists::late_conversion_appended_between_runs_within_7_day_window`
  after the fix — still passes (HOLDS), and manually inspected the compiled run-2 SQL: `events`'
  read is now the tight `event_date >= '2024-01-01' AND event_date < '2024-01-02'` (no more
  spurious 7-day widen) while `conversions`' read correctly stays widened to
  `< '2024-01-09'`. No divergence introduced.
- condition (CONDITIONAL only): n/a
- production files/functions changed: `crates/smelt-logical/src/analysis/source_bounds.rs` —
  `extract_form_b_bounds` (now uses `partition_col_upper`, renamed from `_partition_col_upper`),
  `extract_gte_lt_interval_bounds`, `extract_gte_lt_bare_integer_bounds` (both gained a
  `partition_col_upper` parameter), new `lhs_column_is_partition_col` helper. Untagged — this is a
  permanent production change, not disposable test scaffolding.
- red→green: added `test_form_b_does_not_leak_bound_to_unrelated_source` to
  `source_bounds.rs`'s `#[cfg(test)] mod tests`. Verified red first — ran the new test against the
  pre-fix code (temporarily restored from `git show HEAD:…`): FAILED, `events` derived
  `Bounded(event_date, before=0, after=604800)` (the spurious 7-day leak) instead of the expected
  `Bounded(event_date, 0, 0)`. Restored the fix: PASSED, along with all 30 pre-existing
  `source_bounds` tests (including `test_form_b_forward_only`, `test_explicit_between_filter`,
  `test_cross_column_tz_rebase`, `test_aggregation_max`, `test_integer_key_bare_constant_offset_form_b`
  — all still green, confirming the column-aware scoping does not regress any existing Form-B shape).
- no-regression gate: `cargo test -p smelt-logical --quiet` → 296 passed, 0 failed.
  `cargo test -p smelt-planner --quiet` → 38 passed, 0 failed (consumes `source_bounds` via
  `rules::incremental::derive_model_source_bounds`). `cargo test -p smelt-runtime --quiet` → all
  suites passed (consumes via `compile::build_source_bound_map`). `cargo test -p smelt-cli --test
  property_discovery --quiet` → 15 passed, 0 failed (all prior Link-C cells, including `SC-1`
  itself, still HOLD). `cargo fmt --all` clean. `cargo clippy --all-targets -p smelt-logical -p
  smelt-planner -p smelt-runtime -p smelt-cli --quiet` clean.
- experimental smelt extensions (if any): none beyond the production fix above; the new test is a
  normal (non-`EXPERIMENTAL(property-discovery)`) unit test in `source_bounds.rs`'s existing test
  module — the tag is reserved for disposable Link-A/B/C harness scaffolding, not permanent
  production-adjacent tests.
- evidence: `smelt-logical::analysis::source_bounds::tests::
  test_form_b_does_not_leak_bound_to_unrelated_source`.

### CELL FIX-2 — input_delta_discovery dormant classifier × clocked mutable × wire-or-fence
- verdict: BLOCKED (mechanical tripwire applied; the wiring decision itself is deferred to human review)
- P (Link 0): n/a (this cell audits a classifier's *consumption*, not an algebraic combiner property)
  skeleton_cols (Link B): n/a
- Link B facts: `input_delta_discovery` (`crates/smelt-logical/src/analysis/input_delta.rs:88`)
  classifies a clocked `Mutable` source as `InputDeltaKind::WindowForward` — confirmed by SC-2
  (this ledger) that a forward-only consumer of that verdict misses an in-place UPDATE of an
  already-processed partition. Confirmed by direct grep
  (`rg input_delta_discovery crates --include='*.rs'`, run this iteration) that the function has
  **zero production call sites** — every match is its own definition or its own `#[cfg(test)]`
  unit tests in the same file. It is proof-stage-only, dead code from the maintenance planner's
  point of view.
- smelt analyzer: not-derivable (the function is unconsumed; there is no execution path to audit)
- Link C: not applicable — nothing calls this classifier today, so there is no emitted maintenance
  SQL to run an adversarial schedule against. (SC-2 already exercises the *effective* forward-only
  hazard this classifier would reproduce if wired, via a hand-simulated forward-only consumer —
  see that cell.)
- condition: the finding this cell records — wiring `input_delta_discovery`'s `WindowForward`
  verdict to any consuming maintenance mode for a `Mutable`-profiled source is a
  **behaviour-defining design decision** (it would newly licence "read only the next window
  forward" as a real refresh technique for sources that can be updated in place — new maintenance
  semantics, design §8(4)), not a mechanical bug fix. Per policy this loop must record and BLOCK,
  not decide it. The mechanical, in-scope action taken instead: a **permanent tripwire test**
  (`crates/smelt-logical/tests/input_delta_discovery_dead_code_tripwire.rs`) asserting the
  call-site set stays empty (whitelisted only to the function's own definition file); the moment a
  future change adds a production caller, this test fails and its message points the author at
  SC-2 + this cell before they can silently ship the wiring.
- production files/functions changed: none (no analyzer/planner/runtime behaviour changed). Added
  `crates/smelt-logical/tests/input_delta_discovery_dead_code_tripwire.rs` — a permanent guard
  test, not disposable scaffolding, so left **untagged** (no
  `EXPERIMENTAL(property-discovery)` marker; that tag is reserved for throwaway harness code per
  design §8).
- red→green: ran the new tripwire test against the current tree first — PASSED immediately (zero
  callers today is the expected, already-true state, so there is no pre-existing divergence to
  reproduce). To verify the tripwire actually tripped, temporarily added a scratch test file with a
  fake call to `input_delta_discovery` outside the allowed file: the tripwire FAILED with the
  expected message naming the new caller. Removed the scratch file; re-ran: PASSED again. This
  establishes the red→green pair for the *guard*, since there is no divergence in current behaviour
  to fix.
- no-regression gate: `cargo test -p smelt-logical --test input_delta_discovery_dead_code_tripwire
  --quiet` → 1 passed. `cargo test -p smelt-logical --quiet` → 296 passed, 0 failed (unchanged).
  `cargo fmt --all` clean. `cargo clippy -p smelt-logical --all-targets --quiet` clean.
- experimental smelt extensions (if any): none — the tripwire is a permanent regression guard
  living in normal test surface (`crates/smelt-logical/tests/`), not
  `EXPERIMENTAL(property-discovery)` scaffolding.
- evidence: `smelt-logical::tests::input_delta_discovery_dead_code_tripwire::
  input_delta_discovery_has_no_production_call_sites`.

### CELL G-04 — idempotent agg (MIN) group-by × mutable-snapshot (in-place update lowering then raising the min) × fold-delta
- verdict: HOLDS — hypothesis REFUTED (no divergence found for an explicit backfill after a
  non-invertible mutation; the predicted "fold gets stuck at the lowest value ever observed" did
  not reproduce)
- P (Link 0): `MIN` — idempotent commutative monoid, but **non-invertible** under retraction/mutation
  (Link 0 table §2.0: "no" for retractable/mutable — a stateful fold `state = MIN(state, delta)`
  can only ever lower, never recover, once the min-holding row is mutated upward)
  skeleton_cols (Link B): `d` (the `unique_key`/`GROUP BY` column; `min_val` is the sole payload
  column)
- Link B facts: combiner=idempotent-monoid(MIN) (Link 0 table, `discriminants.rs`) reach=n/a (no
  correlated/cross-source read; disjoint per-partition aggregation, same shape as G-01/G-03/SC-2)
  footprint=bounded (one partition's rows only)
- smelt analyzer: sound — same finding as `SC-2`: batched per-partition materialization
  (`unique_key: [d]`) is unconditionally recompute-region (`DELETE [start,end)` + fresh `INSERT`
  from current source contents for the requested `[start, end)` window), never a stateful fold onto
  remembered state, for ANY combiner. Because it is a genuine recompute over the current snapshot
  rather than `state = MIN(state, delta)`, the non-invertibility hazard in the Link-0 table (which
  applies to a hypothetical *fold-delta* technique) has no purchase here — there is no remembered
  state to get stuck at the wrong value. `input_delta.rs`/`MutationProfile` are not consulted for
  this path either (same as SC-2); what governs recompute is solely whether the partition falls
  inside the run's requested window.
- Link C: no divergence over 1 seeded schedule (deterministic, not proptest-generated — mirrors
  SC-2's Coverage caveat). Seeded rows `(d=2024-01-01, id=1, val=10.0)`, `(id=2, val=5.0)`; run 1
  processes `[2024-01-01, 2024-01-02)`, materializing `min_val=5.0`. Between runs, `id=2`'s `val` is
  first LOWERED to `1.0` (a stateful fold would now track `1.0` as its running state), then RAISED
  to `999.0` — the non-invertible case: the true current minimum is now `10.0` (`id=1`), a value a
  `state = MIN(state, delta)` fold could never recover once it had latched onto `1.0`. Run 2
  advances FORWARD to `[2024-01-02, 2024-01-03)` — as expected (not a bug, matches SC-2's finding),
  the never-re-requested `2024-01-01` partition stays stale at `5.0`. Run 3 explicitly backfills the
  SAME `[2024-01-01, 2024-01-02)` window: the maintained `min_val` becomes `10.0`, matching the
  full-refresh oracle exactly — a fresh, unconditional recompute of `MIN(val)` over current source
  contents for the requested partition, not a fold over remembered state.
- condition (CONDITIONAL only): n/a
- production files/functions changed: none — pure test scaffolding, no analyzer/planner/runtime
  change (this cell confirms the same "batched materialization is recompute-region, not fold-delta"
  fact SC-1/SC-2 already established production-side; no new production behaviour was exercised).
- experimental smelt extensions (if any): none — reuses `model_shapes::idempotent_agg_mutable_source`
  (new fixture, tagged `EXPERIMENTAL(property-discovery): disposable` per the file's existing
  header) and the pre-existing `link_c_harness`/`SqlCapturingReporter` infrastructure.
- evidence: `smelt-cli::tests::property_discovery::g_04_idempotent_min_mutable_snapshot::
  in_place_update_lowering_then_raising_the_min_is_recovered_on_backfill` (deterministic 3-run
  schedule through `execute_project`, no hand-injected `WHERE`;
  `cargo test -p smelt-cli --features smelt-cli/duckdb --test property_discovery
  in_place_update_lowering_then_raising_the_min_is_recovered_on_backfill` → 1 passed).
- Coverage caveat (design §2.1 N4): single hand-authored schedule targeting the specific
  non-invertibility hazard (lower-then-raise), not proptest-shrunk over arbitrary MIN schedules —
  scoped the same way as SC-2, to "does an explicit backfill after this mutation shape still miss
  it", answered no. Does not establish behaviour for a technique other than the simple
  per-partition batched form every G-*/SC-* cell in this shape has tested.

### CELL G-05 — inner-join enrichment (fact × dim) × mutable dimension (in-place update between runs) × column-scoped re-derivation
- verdict: HOLDS (CONDITIONAL on an explicit backfill for the already-processed partition — see
  condition below); hypothesis's "does smelt miss the dimension change" arm REFUTED for the
  backfill technique, CONFIRMED (expected, not a bug) for a plain forward-only advance
- P (Link 0): n/a — inner-join enrichment has no combiner identity; this cell is about whether the
  dimension side of a join is read fresh or from some cached/bounded snapshot
  skeleton_cols (Link B): `d`, `user_id` (the `unique_key`; `val`/`tier` are payload — `tier` is
  the enrichment column under test)
- Link B facts: the dimension source (`users`) carries no `timeseries:` block, so it is never
  added to `BoundContext`/`dep_timeseries` at all (`crates/smelt-logical/src/analysis/
  source_bounds.rs::derive_model_bounds` only iterates `ctx.source_partition_cols`, populated from
  `crates/smelt-runtime/src/compile.rs::build_source_bound_map`'s walk over `dep_timeseries`).
  reach for `users` = **absent** (not `Unbounded` as a computed value — it never enters the bound
  map, so `compile.rs`'s `SourceBound`-emission loop simply has nothing to `continue` past for it)
  footprint=unbounded (whole table, every run)
- smelt analyzer: sound (no filter is needed or emitted for the dim side, and none is) — but this
  is a structural consequence of non-timeseries sources being outside `source_bounds`'s domain
  entirely, not a reasoned "read the dimension fully" derivation. There is no per-run snapshot,
  cache, or point-in-time pin of a joined lookup source anywhere in this path: the compiled
  `INSERT`'s `SELECT` plainly references `main.sources_users`, so a same-window backfill re-reads
  whatever the dimension table currently contains at execution time.
- Link C: no divergence over 1 seeded schedule (deterministic, not proptest-generated — mirrors
  SC-2/G-04's Coverage caveat). Seeded fact row `(d=2024-01-01, user_id=1, val=10.0)`, seeded
  dimension row `(user_id=1, tier='bronze')`; run 1 processes `[2024-01-01, 2024-01-02)`,
  materializing `tier='bronze'`. Between runs, the dimension row is updated in place to
  `tier='gold'` (the already-processed partition's enrichment now points at stale dimension data —
  never pre-populated). Run 2 advances FORWARD to `[2024-01-02, 2024-01-03)` — as expected (not a
  bug, matches SC-2/G-04's finding for the *fact* side), the never-re-requested `2024-01-01`
  partition's enrichment stays stale at `'bronze'`. Run 3 explicitly backfills the SAME
  `[2024-01-01, 2024-01-02)` window: the maintained `tier` becomes `'gold'`, matching the
  full-refresh oracle (`multiset_equal` over all columns) exactly — the recompute-region re-reads
  the dimension table's current contents, broadcasting the update to the fact row that references
  it (paper §10's "breaks invariant A, keeps B" shape, confirmed empirically: a *repeated* backfill
  of the same fact window is exactly what recovers a dimension change; a fact-side-only advance
  never does).
- condition (CONDITIONAL only): the guarantee holds only for a partition that is **explicitly
  re-run** after the dimension changes (a backfill/re-materialization of that window) — a
  forward-only advance that never revisits the partition leaves its enrichment permanently stale
  relative to the current dimension, with no automatic re-derivation triggered by the dimension
  mutation itself (there is no dependency tracking from dimension source → previously-materialized
  fact partitions). This is the same traded guarantee SC-2/G-04 already named for mutation of the
  *fact* source; this cell confirms it holds symmetrically for mutation of a *joined dimension*.
- production files/functions changed: none — this cell found no divergence to fix; the "absent
  from the bound map" behaviour for non-timeseries sources is correct as-is for a plain
  inner-join enrichment (no filter is needed on an always-fully-read dimension).
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request`/
  `oracle::multiset_equal`; adds `model_shapes::join_enrichment_mutable_dimension` (a new
  `MultiSourceModelShape` fixture, not experimental analyzer code) and a cell-local
  `stage_project` that declares `mutation_profile: mutable` on the `users` source only.
- evidence: `smelt-cli::tests::property_discovery::g_05_join_enrichment_mutable_dimension::
  dimension_update_between_runs_is_recovered_on_backfill_but_not_forward_advance` (deterministic
  3-run schedule through `execute_project`, no hand-injected `WHERE`;
  `cargo test -p smelt-cli --test property_discovery dimension_update_between_runs --quiet` →
  1 passed; full suite `cargo test -p smelt-cli --test property_discovery --quiet` → 17 passed).
- Coverage caveat (design §2.1 N4): single hand-authored schedule, not proptest-shrunk over
  arbitrary dimension-mutation shapes (e.g. a dimension row deleted rather than updated, or a
  composite dimension key) — scoped to "does an explicit backfill after a simple scalar dimension
  update recover the current value", answered yes. Does not establish behaviour for a
  materialization strategy other than the `unique_key`-scoped batched DELETE+INSERT every prior
  cell in this shape has tested, nor for a dimension source that itself carries a `timeseries:`
  block (which would enter `source_bounds`'s domain and could in principle be filtered).

### CELL G-06 — left-join null-preservation (fact × late-arriving right side) × append-only both sides × recompute-region
- verdict: HOLDS (no divergence over the seeded schedule); hypothesis's "does smelt strand the
  left-join NULL" arm REFUTED for the backfill/recompute-region technique, CONFIRMED (expected, not
  a bug) that a plain forward-only advance never revisits the already-processed partition.
- P (Link 0): n/a — a left-join has no combiner identity; this cell is about whether the batched
  recompute-region (`DELETE [start,end)` + fresh `INSERT`) re-reads the right-side source's CURRENT
  contents when the partition is explicitly re-run, or whether the unmatched-row NULL is stranded.
  skeleton_cols (Link B): `d`, `user_id` (the `unique_key`; `val`/`refund_amt` are payload —
  `refund_amt` is the recovered column under test)
- Link B facts: `refunds` is a genuine `timeseries:` source (unlike G-05's non-timeseries `users`
  dimension), so it DOES enter `BoundContext`/`source_bounds::derive_model_bounds` via its own
  `refund_date` partition column — but the model never applies a `WHERE` on `refund_date` (the join
  predicate `e.d = r.refund_date` is an equality across two different source columns, not a
  same-source temporal filter smelt's Form-A/B derivation recognizes), so no bound clips it. reach
  for `refunds` = whatever `derive_model_bounds` derives (unbounded/no-op for this join shape, same
  observed effect as G-05's absent case) footprint=unbounded (whole table read fresh on
  recompute-region)
- smelt analyzer: sound (no over-read, no under-read observed on backfill; the recompute-region's
  DELETE+INSERT simply re-reads both sources' current contents for the reprocessed window,
  independent of any per-source bound on `refunds`)
- Link C: no divergence over 1 seeded schedule (deterministic, not proptest-generated — mirrors
  SC-2/G-04/G-05's coverage caveat). Seeded fact row `(d=2024-01-01, user_id=1, val=10.0)`, empty
  `refunds` table; run 1 processes `[2024-01-01, 2024-01-02)`, materializing `refund_amt=NULL` (no
  matching refund exists yet). Between runs, a refund row `(refund_date=2024-01-01, user_id=1,
  refund_amt=3.5)` is APPENDED (never pre-populated, never mutated in place — the `SC-1` late-append
  shape, not `G-04`/`G-05`'s in-place-update shape) into the already-processed `2024-01-01`
  partition. Run 2 advances FORWARD to `[2024-01-02, 2024-01-03)` — as expected (not a bug, matches
  every prior forward-advance finding in this catalog), the never-re-requested `2024-01-01`
  partition's `refund_amt` stays `NULL`. Run 3 explicitly backfills the SAME
  `[2024-01-01, 2024-01-02)` window: the maintained `refund_amt` becomes `3.5`, matching the
  full-refresh oracle (`multiset_equal` over all columns) exactly — the recompute-region re-reads
  `refunds`'s current contents and recovers the previously-unmatched row, the same "explicit
  backfill recovers a late/updated right-side fact" shape SC-1/G-04/G-05 already established for
  their respective constructs, now confirmed for LEFT JOIN null-preservation specifically.
- condition (CONDITIONAL only): n/a — recorded as unconditional HOLDS (the guarantee is identical
  in shape to G-05's traded condition — recovery requires an EXPLICIT backfill of the affected
  window, a forward-only advance never revisits it — but that same condition already applies
  uniformly to every batched cell in this catalog via the shared recompute-region technique, so it
  is not re-declared as a cell-specific trade here; see G-04/G-05 for the general statement).
- production files/functions changed: none — this cell found no divergence to fix.
- Note (separate, out-of-scope finding surfaced while authoring this cell): giving both `events`
  and `refunds` the SAME partition column name (`d`) makes smelt's derived bare `WHERE d >= ... AND
  d < ...` filter genuinely ambiguous across two FROM-clause sources — a DuckDB `Binder Error:
  Ambiguous reference to column name "d"` at execution time, not a silent wrong-answer. This is a
  real filter-emission gap (the derived predicate is not qualified by source alias/table), but it
  is orthogonal to G-06's late-arrival hypothesis and was sidestepped here by naming `refunds`'s
  column `refund_date` instead. Not filed as a new catalog cell per the ≤2-adjacent-append cap
  discipline (§4) — flagged here for whoever next touches `source_bounds`/filter emission for
  multi-timeseries-source models.
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request`/
  `oracle::multiset_equal`; adds `model_shapes::left_join_late_right_side` (a new
  `MultiSourceModelShape` fixture, not experimental analyzer code) and a cell-local `stage_project`
  that keeps both sources at the append-only default (no `mutation_profile:` block).
- evidence: `smelt-cli::tests::property_discovery::g_06_left_join_null_preservation::
  late_refund_appended_between_runs_is_recovered_on_backfill_but_not_forward_advance`
  (deterministic 3-run schedule through `execute_project`, no hand-injected `WHERE`;
  `cargo test -p smelt-cli --test property_discovery g_06 --quiet` → 1 passed; full suite
  `cargo test -p smelt-cli --test property_discovery --quiet` → 18 passed).
- Coverage caveat (design §2.1 N4): single hand-authored schedule, not proptest-shrunk over
  arbitrary late-arrival timing (e.g. a right-side row landing beyond the derived horizon, or
  multiple late rows across several partitions). Scoped to "does an explicit backfill after one
  late right-side append recover the true left-join result", answered yes. Does not establish
  behaviour for join fan-out (a right side with multiple matching rows) or for a right-side source
  declared `mutation_profile: mutable` (that shape is G-05's territory, not this cell's).

### CELL G-07 — holistic agg (MEDIAN / COUNT DISTINCT) × append-only × recompute-region
- verdict: HOLDS (no divergence over 8 proptest cases × 2-4 disjoint windows each, including 0-2
  re-deliveries per window); catalog hypothesis REVISED before Link C ran (research below), then
  the revised prediction was CONFIRMED.
- P (Link 0): holistic / non-monoid — `MEDIAN` and exact `COUNT(DISTINCT ...)` have no bounded
  combiner state (Link 0 table §2.0: "no bounded state" row). skeleton_cols (Link B): `{d}` (the
  `unique_key`; `id`/`val` are payload feeding the holistic aggregates under test)
- Link B facts: `combiner_discriminants` (`crates/smelt-logical/src/analysis/discriminants.rs:77-134`)
  DOES classify both correctly — exact `DISTINCT` always routes to `holistic_or_unknown()`
  (lines 80-84) regardless of underlying function, and unmatched functions (`MEDIAN`, `MODE`,
  `PERCENTILE_CONT/DISC`) fail-closed to the same bucket via the `_` arm (lines 130-132). But this
  classification is a **dead input** for the construct under test: `combiner_discriminants`/
  `Discriminants` is consumed only by the cumulative/running-total rule
  (`crates/smelt-logical/src/rules/cumulative.rs:92`, refusing `COUNT(DISTINCT)` for THAT feature at
  lines 294-300) and `join_shape`'s fan-out analysis — `crates/smelt-logical/src/rules/incremental.rs`
  (the rule governing `refresh: batched` GROUP BY models under test here) never imports or consults
  `discriminants`/`Discriminants` at all; its only refusals are keyed on time-bound derivability
  (`NotDerivable`) and window/ordering shape, not combiner algebra. reach for `events` =
  `derive_model_bounds`'s ordinary timeseries-column reach (unaffected by the aggregate's identity)
  footprint=bounded (one partition's worth of source rows, same as every other batched-GROUP-BY
  cell in this catalog)
- smelt analyzer: sound (not-derivable-by-design is not exercised here — `rules/incremental.rs`
  simply has no combiner-sensitive branch to be unsound or over-conservative about; the technique it
  actually applies, recompute-region, is combiner-identity-agnostic by construction)
- Link C: no divergence over 8 proptest cases (`ProptestConfig::with_cases(8)`), each 2-4 disjoint
  one-day windows with 2-5 `(id, val)` rows per window (id drawn from `1..=3` so duplicate ids
  within a window are common — exercising `COUNT(DISTINCT id)`'s de-duplication, which a
  globally-unique-id scheme like `G-01`/`G-03`'s would never touch) and 0-2 re-deliveries of the
  identical window with no new rows landing between re-deliveries (the seeded hazard: a genuinely
  holistic combiner is the shape most likely to expose a hidden fold-onto-remembered-state
  optimization, since re-delivering a delta into partial holistic state has no well-defined
  semantics at all — smelt's actual `DELETE [start,end)` + fresh-`INSERT` recompute-region sidesteps
  the question entirely by never retaining partial state to re-deliver into).
- condition (CONDITIONAL only): n/a — recorded as unconditional HOLDS.
- production files/functions changed: none — this cell found no divergence to fix. The catalog's
  original hypothesis (CONDITIONAL/REFUTED, predicting a "no bounded state to fold" failure mode)
  was falsified by research *before* Link C ran: smelt's batched materialization
  (`crates/smelt-backend-duckdb/src/lib.rs::delete_and_insert_transactional`, `DELETE` the window +
  `INSERT` a freshly-recomputed `SELECT ... GROUP BY d` for it, one DuckDB transaction) is
  recompute-region, not fold-delta, for every `refresh: batched` cell in this catalog (`G-01`
  through `G-06` already established this as an aside; this cell is the first to make it the
  *headline* finding by choosing combiners for which the distinction is load-bearing) — there is no
  partial aggregate state anywhere in the maintained table for a holistic combiner to corrupt.
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request`; adds
  `model_shapes::holistic_agg_append_only` (a new single-source `ModelShape` fixture, not
  experimental analyzer code) and a cell-local `stage_project`/oracle mirroring `G-03`'s pattern.
- evidence: `smelt-cli::tests::property_discovery::g_07_holistic_agg_append_only::
  holistic_median_and_count_distinct_fold_over_disjoint_append_only_windows_with_redelivery_matches_full_refresh`
  (`cargo test -p smelt-cli --test property_discovery --features duckdb g_07 --quiet` → 1 passed;
  full suite `cargo test -p smelt-cli --test property_discovery --features duckdb --quiet` →
  19 passed).
- Coverage caveat (design §2.1 N4): proptest-generated over disjoint-window shape + re-delivery
  count only — does not cover backfill/recompute of an ALREADY-emitted partition after a NEW row is
  appended late into it (that shape is `G-06`'s territory, established there for a different
  construct and expected to generalize here since the underlying technique, recompute-region, is
  identical), nor a holistic combiner over a `mutable-snapshot` source (a distinct, not-yet-catalogued
  cell — mutable-snapshot's non-invertibility concern, per `G-04`, is orthogonal to holistic's
  no-bounded-state concern and would need its own cell).

### CELL G-08 — self-referential batched model (running-total self-join) × append-only (late transaction into an already-processed partition) × recompute-region (local) / no-cascade (trajectory)
- verdict: CONDITIONAL
- P (Link 0): additive over a prefix; **unbounded-forward footprint** — a change to one partition's
  stored value invalidates every LATER partition's own stored trajectory value, not just its own
  (paper §7). Not a monoid-foldable delta: the "delta" for a stored trajectory is really "recompute
  every downstream partition", never a local fold.
  skeleton_cols (Link B): `{d}`
- Link B facts: combiner=additive SUM (trivially idempotent-under-recompute-region, same as every
  other batched cell); reach=`Bounded(1 day, 0)` for the self-edge (`bal.d >= t.d - INTERVAL '1 day'
  AND bal.d < t.d`, the exact form `window_independence`'s own `backward_bounded_self_edge_is_ordered`
  unit test proves `Ordered`); `window_independence` verdict = `Ordered` (confirmed: forces
  strictly-sequential single-partition batches via `compute_incremental_windows_ordered`); footprint
  = bounded PER PARTITION (one partition's own source rows + its immediately-prior stored balance)
  but **unbounded across partitions** (every downstream partition's stored value depends
  transitively on it).
- smelt analyzer: sound for what it actually claims (`Ordered` only asserts "this self-edge
  converges partition-by-partition under strictly sequential execution" — it does NOT claim that an
  out-of-order backfill of a single stale partition repairs downstream partitions too, and smelt's
  execution never claims that either). Not unsound: no analyzer fact says "backfilling day1 alone
  is sufficient to repair the whole trajectory."
- Link C: no divergence over the seeded hazard schedule (a late transaction appended into an
  ALREADY-PROCESSED partition, day1, after the initial 3-day trajectory was built sequentially) —
  but the schedule surfaces the CONDITIONAL boundary directly: after backfilling ONLY the mutated
  partition (day1), day1 itself self-corrects (110, matching full-refresh) but day2/day3 remain
  STALE (15/16, diverging from the true 115/116) until they too are explicitly re-run, in temporal
  order, downstream of the mutation. Once that cascade is performed, all three partitions match
  full-refresh exactly.
- condition (CONDITIONAL): the maintained trajectory equals full-refresh **only when every
  backfill of a partition `p` is followed by a backfill of every partition `> p`, in strict temporal
  order** (the same ordering discipline the self-edge's own `Ordered` verdict already requires
  within a single run, extended here across separate runs/backfills). smelt neither enforces nor
  automates this cascade: nothing detects that day1's stored value changed and nothing schedules
  day2/day3 for re-derivation. This is a real, silent staleness trap for an operator who backfills
  a single day of a running-balance model expecting the trajectory to "just be correct" downstream —
  worth flagging as a **known limitation to document**, not a divergence bug (no analyzer claims
  otherwise; recompute-region did exactly what it was asked to do for the partition it was asked to
  rebuild).
- production files/functions changed: none — this cell's finding is a scoping/documentation gap in
  the CONDITIONAL sense, not a code defect: no analyzer fact claims backfill-of-one-partition
  repairs a self-referential trajectory, so there is nothing "unsound" to fix. A genuine EXECUTION
  bug was found in constructing this cell's own model (below) and is spun out as its own pending
  cell (`G-11`) rather than decided here.
- ancillary finding (spun out, not fixed in this cell): the spec's own documented self-referential
  form — a DIRECT join to `smelt.<self>` where both the driving source and the self-reference expose
  the partition column under its own bare name (`t.d`/`bal.d`, exactly `window_independence`'s own
  unit-test SQL shape) — fails at EXECUTION time with a DuckDB `Binder Error: Ambiguous reference to
  column name "d"`, because `crates/smelt-runtime/src/transformer.rs::inject_time_filter` injects the
  outer output-clamp as a bare, unqualified `event_time_column` whenever the model is not
  `is_transparent_single_source` (true here — a self-referential model with any nonzero self-margin
  always has ≥2 bound sources). This cell's own test could only proceed by wrapping the self-join in
  a subquery (`model_shapes::running_balance_self_ref`) so the outer clamp's FROM scope exposes only
  one `d`-named column — a workaround the documented pattern does not mention. Recorded as `G-11`
  (appended below) rather than fixed here: the correct fix requires resolving WHICH FROM-item
  legitimately owns the output `event_time_column` when several expose the same bare name, which is
  itself judgment-bearing (per policy §8(4), not folded into this cell's mechanical scope).
- experimental smelt extensions (if any): `model_shapes::running_balance_self_ref` (a new
  self-referential `ModelShape` fixture, subquery-wrapped per the `G-11` finding above, not
  experimental analyzer code); a cell-local `stage_project`/`seed_sources`/oracle mirroring `G-05`'s
  deterministic (non-proptest) pattern, since the self-edge's own `Ordered` requirement (strictly
  sequential per-partition execution) makes an arbitrary proptest-generated schedule redundant with
  the specific sequential-then-mutate-then-selectively-backfill scenario this cell needs to isolate.
- evidence: `smelt-cli::tests::property_discovery::g_08_running_total_self_ref::
  late_transaction_into_an_already_processed_partition_requires_a_downstream_cascade`
  (`cargo test -p smelt-cli --test property_discovery --features duckdb g_08 --quiet` → 1 passed;
  full suite `cargo test -p smelt-cli --test property_discovery --features duckdb --quiet` →
  20 passed; `cargo fmt --all`; `cargo clippy -p smelt-cli --all-targets` clean).
- Coverage caveat (design §2.1 N4): a single deterministic scenario (one late-append hazard, one
  local-backfill-then-cascade sequence), not proptest-generated — the `Ordered` self-edge's forced
  strictly-sequential execution collapses the useful schedule space to essentially this shape; a
  future cell could still vary the NUMBER of stale downstream partitions or interleave a SECOND
  independent mutation, but the qualitative finding (local recompute-region has no cross-partition
  cascade) would not change.

### CELL G-09 — UNION ALL of two sources × append-only both arms × recompute-region
- verdict: HOLDS (no divergence over the seeded hazard schedule: a late row appended into EACH arm
  independently, between runs, into an already-processed partition).
- P (Link 0): a multiset union is combiner-identity-agnostic — no combiner runs at all, so §2.0's
  table does not apply; the only live question is reach/footprint of smelt's chosen technique.
  skeleton_cols (Link B): `{d, id, src}` (the declared `unique_key`; `val` is payload). `src` is a
  literal discriminator added so the two arms' `(d, id)` domains can overlap without colliding under
  the union's shared grain.
- Link B facts: `combiner_discriminants` is not consulted (no aggregate in this model — a plain
  `UNION ALL` of two projections). reach for `events_a`/`events_b` = `derive_model_bounds`'s ordinary
  per-source timeseries-column reach, derived independently for each `FROM`/arm of the `UNION ALL`
  (both arms are their own top-level `FROM` clause, not one nested inside the other the way `SC-1`'s
  correlated subquery was) — footprint=bounded (one partition's worth of rows from EACH arm).
- smelt analyzer: sound — `rules/incremental.rs`'s batched technique (recompute-region: `DELETE
  [start,end)` + `INSERT` a freshly-recomputed `SELECT` for that window,
  `crates/smelt-backend-duckdb/src/lib.rs::delete_and_insert_transactional`) re-executes the model's
  ENTIRE `SELECT` — both `UNION ALL` arms — against their CURRENT source contents on every run,
  including an explicit backfill of an already-processed window. There is no per-arm bound that
  could under-cover one arm while covering the other: the derived `WHERE` clamp is applied to the
  OUTER window, not threaded separately into each arm, so both arms see the same backfilled range.
- Link C: no divergence over the deterministic 3-run scenario (run 1: process day 1 with one row per
  arm; between runs: append a late row into EACH arm independently, into the SAME already-processed
  day-1 partition; run 2: forward-only advance to day 2 — confirmed to leave day 1 stale at 2 rows,
  not a bug; run 3: explicit backfill of day 1 — confirmed to recover BOTH late rows, 4 rows total,
  multiset-equal to the full-refresh oracle over both arms' current contents).
- condition (CONDITIONAL only): n/a — recorded as unconditional HOLDS.
- production files/functions changed: none — no divergence found, nothing to fix.
- experimental smelt extensions (if any): none — reuses `link_c_harness`/`base_request`/`oracle::
  multiset_equal`; adds `model_shapes::union_all_two_append_only` (a new `MultiSourceModelShape`
  fixture, not experimental analyzer code) and a cell-local `stage_project`/`seed_sources` mirroring
  `G-06`'s deterministic (non-proptest) late-append-then-backfill pattern — a single scenario
  suffices here since the question (does recompute-region read BOTH arms on backfill) does not
  depend on window count or row volume the way a combiner-fold cell's re-delivery count does.
- evidence: `smelt-cli::tests::property_discovery::g_09_union_all_append_only::
  late_rows_appended_into_both_union_arms_between_runs_are_recovered_on_backfill_but_not_forward_advance`
  (`cargo test -p smelt-cli --test property_discovery --features duckdb g_09 --quiet` → 1 passed;
  full suite `cargo test -p smelt-cli --test property_discovery --features duckdb --quiet` →
  21 passed).
- Coverage caveat (design §2.1 N4): a single deterministic scenario (one late-append hazard per arm,
  one backfill), not proptest-generated — mirrors `G-06`'s rationale: the qualitative question (does
  the whole `SELECT` re-execute on backfill) has no combiner-fold dimension to vary. Not covered: a
  THIRD arm, an arm declared `mutable-snapshot` instead of append-only (a distinct, not-yet-catalogued
  cell mixing `G-09`'s multi-arm shape with `G-04`/`G-05`'s in-place-mutation hazard), or a `UNION ALL`
  feeding a downstream `GROUP BY` (this cell's model is a bare pass-through union, no aggregate above
  it).

### CELL G-10 — join fan-out on COMPOSITE unique key × append-only × column-scoped re-derivation
- verdict: CONDITIONAL (Link-B classification finding: over-conservative, not unsound — recorded
  here because Link C's execution gate does not apply to this cell at all, see below).
- P (Link 0): n/a — this cell concerns `join_shape::fan_out`'s cardinality proof itself, not a
  combiner-algebra property of a maintained aggregate.
- skeleton_cols (Link B): `{user_id, dt}` — the composite equi-join key columns
  (`ON f.user_id = d.user_id AND f.dt = d.dt`); `dim_payload` is enrichment payload.
- Link B facts: `join_shape::JoinContext::with_unique_key` declares uniqueness **per single
  column** ("alone uniquely identifies a row" — `join_shape.rs:29-35`); there is no API to declare
  a composite (multi-column) unique key. Ground-truth proptest
  (`composite_key_equality_join_is_truly_one_to_one_in_ground_truth`, 100+ generated composite
  dim/fact datasets where no single column is unique but the `(user_id, dt)` pair is) confirms the
  join is genuinely one-to-one. `fan_out`'s own `equality_columns_for_table` walk correctly
  extracts BOTH equality columns from the `AND`-ed `ON` clause, but `JoinContext` has no declared
  key for either column alone (since neither is individually unique) — so `is_unique` is `false`
  and `fan_out` returns `OneToMany`: a **false negative**, not a false positive. Also noted:
  `fan_out`/`JoinContext` and their sole would-be production consumer
  (`dimension_horizon_merge`, `crates/smelt-runtime/src/dimension_horizon_merge.rs`) have **zero
  production call sites** today (`rg -n "JoinContext|dimension_horizon_merge\("` outside tests
  finds none) — this classifier is dormant, same class as `FIX-2`'s `input_delta_discovery`.
- smelt analyzer verdict: over-conservative (fail-closed correctly) — never unsound. The gap is a
  missing expressiveness feature (composite unique keys), not an incorrect classification of what
  it CAN express.
- Link C: not run — there is no production execution path that consumes `fan_out`'s cardinality
  verdict (dormant classifier, no caller), so there is nothing for an adversarial run-schedule to
  exercise; the finding is fully contained in Link B.
- condition (CONDITIONAL only): the over-conservative gap only matters IF/WHEN `fan_out` or
  `dimension_horizon_merge` is ever wired to a live maintenance path — at that point a composite
  natural key (a common real-world shape: e.g. `(user_id, dt)` slowly-changing dimensions) would be
  refused a horizon-bounded MERGE it could safely take, falling back to a full rebuild. Wiring
  either function to a consumer, or extending `JoinContext` to accept declared composite keys, is a
  behaviour-affecting design decision — BLOCKed for human review per policy 8(d), same as `FIX-2`.
- production files/functions changed: none — the cell establishes a classification gap in currently
  dormant code, not a live analyzer/planner bug; per policy 8(d) this is a design fork (extending
  the `JoinContext` surface, or wiring a consumer), not a mechanical fix.
- experimental smelt extensions (if any): none — reuses `prop_helpers::duckdb_oracle::DuckDbOracle`
  directly (no new harness), calls `smelt_logical::analysis::join_shape::fan_out` unmodified.
- evidence: `smelt-db::tests::proptests::maintenance_link_b_composite_key_fan_out::
  {composite_key_equality_join_is_truly_one_to_one_in_ground_truth,
  fan_out_cannot_express_composite_unique_key_and_conservatively_classifies_one_to_many}`
  (`cargo test -p smelt-db --test proptests maintenance_link_b_composite_key_fan_out --quiet` →
  2 passed; full suite `cargo test -p smelt-db --test proptests --quiet` → 169 passed; `cargo fmt
  --all`; `cargo clippy -p smelt-db --tests --quiet` clean;
  `bash .claude/scripts/property-experimental-gate.sh` → clean).
- Coverage caveat (design §2.1 N4): only a 2-column composite key with small cardinalities (0..4 per
  column) was generated — not a 3+-column composite key, and not the case where `JoinContext` is
  MISUSED (a caller wrongly declares one column of a composite key as individually unique, which
  this cell's analysis suggests would happen to still be safe when the `ON` clause ANDs the other
  composite column, but was not itself proptested here).

### CELL SC-1b — correlated EXISTS (same-named-column collision across sources) × append-only × recompute-region (Link C)
- verdict: HOLDS — hypothesis (as literally stated: a wrong-source misattribution clamps away the
  late row, REFUTED-as-unsound) REFUTED; the mechanism the cell actually reaches is an
  over-conservative spurious widen, not an unsound narrowing
- P (Link 0): n/a — reach derivation, not an algebraic combiner property
  skeleton_cols (Link B): `user_id`, `d` (the `unique_key`; `reset_flag` is the sole payload column)
- Link B facts: `derive_bound_for_source` (`source_bounds.rs`) is invoked once per source with only
  that source's own partition-*column-name*; `FIX-1`'s `lhs_column_is_partition_col` scopes a
  Form-B match to the LHS column *name*, but has no notion of *which FROM/JOIN alias belongs to
  which source*. `model_shapes::column_name_collision_across_sources` stages `logins` (partition
  col `d`, no Form-B pattern of its own) alongside `resets` (partition col also `d`, the only
  source the correlated `EXISTS` predicate `r.d BETWEEN l.d AND l.d + INTERVAL '3 days'` actually
  constrains). Confirmed via captured compiled SQL: `logins`'s own read is spuriously widened to
  `d >= '2024-01-01' AND d < '2024-01-05'` (the resets-only 3-day margin), even though `logins` has
  no textual Form-B pattern of its own — the exact cross-source leak `FIX-1`'s column-name check
  cannot close because it never resolves alias→source identity.
- smelt analyzer: over-conservative for this shape (not unsound) — and provably so by construction,
  not merely by this one probe: `BoundResult::merge` (`source_bounds.rs`) takes `before.max`/
  `after.max` when folding multiple matches into one source's bound, so a spurious cross-source
  match can only ever ADD margin, never remove it. A same-named-partition-column collision can
  widen a source's read (wasted work) but cannot narrow it (cannot clamp away a row full-refresh
  would include) — the class of bug `SC-1`/`SC-1b`'s hypothesis chain was hunting for is therefore
  not reachable via this mechanism, by the same algebraic argument each cell's own analysis
  predicted before running it.
- Link C: no divergence over 1 deterministic schedule — run 1 processes `[2024-01-01, 2024-01-02)`
  with no reset row; a late reset (`user_id=1, d=2024-01-03`) is appended directly to
  `main.sources_resets` *between* runs (never pre-populated); run 2 re-runs (backfills) the same
  window and correctly picks up the late row (`reset_flag` flips to `true`, matching the
  full-refresh oracle over the step-2 source snapshot).
- condition (CONDITIONAL only): n/a
- production files/functions changed: none — this cell establishes an over-conservative
  (safety-preserving) gap, not a correctness bug; per design §8 the test gate applies to
  correctness fixes, and there is no wrong-answer divergence here to drive a red→green fix against.
  Making `derive_bound_for_source` alias/source-scoped (not just column-name-scoped) would be a
  legitimate future efficiency improvement but is out of scope for this cell — recorded as a
  finding, not actioned, since it changes no observable behaviour.
- experimental smelt extensions (if any): none — the cell reads `RunReporter::model_compiled`
  output through the existing `SqlCapturingReporter` (already `EXPERIMENTAL(property-discovery)`
  from `P0-1`); no new production or analyzer code touched. Added
  `model_shapes::column_name_collision_across_sources` (tagged, disposable) and
  `sc_1b_column_name_collision.rs` (tagged, disposable).
- evidence: `smelt-cli::tests::property_discovery::sc_1b_column_name_collision::
  same_named_partition_column_collision_only_widens_never_narrows` (deterministic 2-run schedule
  through `execute_project`, no hand-injected `WHERE`; asserts both the maintained-vs-full-refresh
  equality and the spurious widen visible in the captured compiled SQL).
  `cargo test -p smelt-cli --test property_discovery --features smelt-cli/bundled-duckdb --quiet` →
  22 passed, 0 failed (all prior Link-C cells unaffected). `cargo fmt --all`;
  `bash .claude/scripts/property-experimental-gate.sh` → clean.
- Coverage caveat (design §2.1 N4): a single hand-authored deterministic schedule, not a
  proptest-shrunk family — appropriate for chasing a specific seeded hazard (same-named-column
  collision) to its algebraic conclusion (widen-only, proven safe by `merge`'s max semantics), not a
  general sweep over arbitrary schedules for this construct.

### CELL G-11 — self-referential batched model direct-join × execution layer × outer clamp qualification (Link C)
- verdict: BLOCKED — root cause confirmed and reproduced red→green as a test; the PRODUCTION FIX
  requires a judgment call between two non-equivalent repair strategies, which is a design fork per
  policy §8(d), not a mechanical change
- P (Link 0): n/a — execution-layer bug (SQL binder ambiguity), not an algebraic combiner property
  skeleton_cols (Link B): n/a — the model never executes far enough to materialize a table to diff
- Link B facts: `crates/smelt-runtime/src/transformer.rs::inject_time_filter` injects the outer
  output clamp as a BARE, unqualified `{event_time_column} >= .. AND {event_time_column} < ..`
  whenever `is_transparent_single_source` returns false — true for any self-referential batched
  model, since the self-edge (`smelt.<self>`) counts as a second bounded source alongside the
  driving fact. `docs/specs/batched_models.md`'s own documented self-referential-model pattern, and
  `window_independence`'s own unit tests (`crates/smelt-logical/src/analysis/window_independence.rs`
  lines ~113-119), use a DIRECT join — `bal.partition_date`/`t.partition_date`, no subquery wrap —
  where BOTH the driving source and the self-reference expose the model's own output/partition
  column under its own bare name. `G-08`'s own test could only proceed by discovering it needed to
  wrap the self-join in a subquery (`SELECT d, balance FROM (...) inner_balance`) — a workaround the
  spec's documented pattern does not itself describe.
- smelt analyzer verdict: n/a (no analyzer classification is wrong; this is a code-generation /
  SQL-injection bug in `smelt-runtime`, downstream of the analyzer's bound derivation, which is
  itself correct — the *value* of the derived filter is right, only its unqualified textual
  placement is unsafe against a FROM scope with a repeated bare column name).
- Link C: reproduced deterministically on the FIRST run (no adversarial schedule needed — this is a
  hard SQL compile failure, not a data-dependent divergence). `running_balance_self_ref_direct_join`
  (the exact `window_independence`-documented direct-join shape, no subquery wrap) fails
  `execute_project`'s first run with DuckDB `Binder Error: Ambiguous reference to column name "d"`,
  because the compiled query's FROM scope exposes `d` from both `t` (driving source alias) and
  `bal` (self-reference alias) and the injected clamp does not qualify which one it means.
- condition (CONDITIONAL only): n/a
- production files/functions changed: none — establishing WHICH fix is correct requires choosing
  between two non-equivalent repair strategies, each carrying its own behaviour/contract
  implications, so it is BLOCKed rather than applied:
  1. **Qualify the injected clamp** to the specific FROM item that legitimately owns the model's
     output `event_time_column` — resolved the same way `crates/smelt-logical/src/rules/
     cumulative.rs`'s `resolve_single_anchor` / `crates/smelt-logical/src/analysis/
     source_bounds.rs::resolve_join_driving_fact` already pick a driving fact for other analyses.
     This requires `smelt-runtime::transformer` (a pure, alias-unaware text/AST transform today) to
     either gain the same alias-resolution knowledge `smelt-logical` already encodes (new
     cross-crate dependency/duplication question — `smelt-runtime` does not depend on
     `smelt-logical` today) or have that resolution threaded through from the analyzer at compile
     time. Also requires deciding what happens when resolution is ambiguous (two FROM items besides
     the self-edge, e.g. a 3-way join) — fail loudly with a smelt diagnostic, or fall back?
  2. **Always wrap the *whole* original query in an outer `SELECT * FROM (...) AS __clamp`** before
     applying the non-transparent-slice clamp, so the outer WHERE only ever sees the query's own
     SELECT-list column names (never an inner FROM scope's aliases) — mechanically closes the
     ambiguity for ANY multi-source model, not just self-referential ones, and appears
     result-equivalent for the currently-passing `inject_time_filter` test suite (the outer clamp
     already only ever needs to see the model's own declared output/event-time column). But it is a
     structural change to what `docs/specs/model_transforms.md` calls the "two-layer clamp"
     mechanism, and one existing unit test
     (`crates/smelt-runtime/src/transformer.rs::tests::test_with_join`) exercises passing an
     ALREADY-QUALIFIED column (`orders.created_at`) into `inject_time_filter` directly — a real
     calling convention this fix must either preserve (qualify-aware wrap) or deliberately drop,
     which is itself a contract decision, not derivable from this cell alone.
  Per policy §8(d) (same precedent as `FIX-2`/`G-10`), choosing between these — or inventing a
  third — is a behaviour/contract-affecting design decision, not a mechanical red→green fix, so it
  is recorded here for human review rather than applied.
- experimental smelt extensions (if any): added `model_shapes::running_balance_self_ref_direct_join`
  (tagged, disposable) and `g_11_self_ref_ambiguous_column.rs` (tagged, disposable) — the RED
  reproduction only; no production code touched.
- evidence: `smelt-cli::tests::property_discovery::g_11_self_ref_ambiguous_column::
  direct_self_join_output_clamp_is_ambiguous_without_subquery_wrap` (asserts `execute_project`
  returns an `Err` whose message contains a DuckDB ambiguous-column binder error).
  `cargo test -p smelt-cli --test property_discovery --features duckdb --quiet` → 23 passed, 0
  failed (all prior Link-C cells unaffected). `cargo fmt --all`;
  `bash .claude/scripts/property-experimental-gate.sh` → clean.
- Coverage caveat (design §2.1 N4): a single deterministic first-run reproduction — appropriate here
  since the failure is a hard SQL compile error independent of any run schedule, not a data-dependent
  divergence Link A's generic schedule kinds would help enumerate.

---

## G-12 — `cumulative_aggregate` × keyed additive fold × `merge_into` (the live targeted-write path) — 2026-07-07, closed 2026-07-10 (MP12)

- construct: keyed additive fold (`refresh: keyed`, `COUNT(*) GROUP BY device_id`) over an
  append-only driving source, dispatched by `execute_project` through
  `crates/smelt-runtime/src/cumulative.rs::execute_cumulative_aggregate` →
  `maintenance_driver::run_windowed_keyed_maintenance` → `Backend::fold_ledger_delta` — the only
  live path where a generalized-ledger obligation can actually be violated
  (`09-spec-readiness.md` §3 item 1; previously entirely unprobed).
- verdict, arm 1 (frontier advance): **HOLDS** — disjoint windows folded in temporal order
  through the real run path equal a full refresh (Jan-1 fold 2 + Jan-2 fold 1 = 3).
- verdict, arm 2 (reprocessed window): **originally CONFIRMED VIOLATION (live)** — re-running the
  already-merged Jan-1 window used to double-fold (3 → 5); the never-fold-a-delta-twice
  obligation (`01-framework.md` §4; `keyed_models.md` §Reprocessing's `KeyedReprocessedWindow`
  refusal) was unenforced on the run path. **Now ENFORCED**: MP12
  (`docs/plans/20260707-maintenance-plan-impl.md`) wired a warehouse-resident per-delta ledger
  table (`smelt_state::ddl_duckdb::generate_ledger_table_ddl`/`generate_ledger_insert_sql`,
  transactional with the fold via `Backend::fold_ledger_delta`) into
  `run_windowed_keyed_maintenance`'s create-or-merge step; a repeat of Jan-1's delta identity
  violates the ledger table's own `PRIMARY KEY` and refuses the run before any double count can
  land — device 1's count stays at 3.
- production files/functions changed: `crates/smelt-state/src/ddl_duckdb.rs`
  (`generate_ledger_table_ddl`/`generate_ledger_insert_sql`/`generate_ledger_exists_sql`),
  `crates/smelt-backend/src/{lib.rs,error.rs}` (`Backend::fold_ledger_delta` default,
  `BackendError::AlreadyReflected`), `crates/smelt-backend-duckdb/src/lib.rs` (transactional
  override), `crates/smelt-runtime/src/{maintenance_driver.rs,cumulative.rs}`
  (`WindowedKeyedRule::ledger_grade`/`ledger_input`, the ledger-guarded step loop).
- evidence: `smelt-cli::tests::property_discovery::g_12_keyed_merge_reprocessed_window::
  keyed_merge_frontier_holds_and_reprocessed_window_is_refused` (both arms asserted through
  `execute_project`; arm 2 now asserts the refusal and the unchanged count).
  `crates/smelt-state/tests/reconciliation.rs::per_delta_grade_lives_in_warehouse` and
  `crates/smelt-backend-duckdb/src/lib.rs`'s `test_fold_ledger_delta_*` tests cover the
  transactional guarantee directly against a real DuckDB connection.
- Coverage caveat (design §2.1 N4): deterministic 3-run schedule (fold, fold, re-fold) — the
  fix is mechanism-level (a ledger table + a transactional trait method), not data-dependent;
  adversarial value schedules add nothing new here.

---

## SC-7 — cross-partition `DISTINCT`/`HAVING` inside a CTE body × batched admission + partition rewrite — 2026-07-07

- construct: `refresh: batched` model whose CTE body holds a cross-partition scope
  (`SELECT DISTINCT user_id, tier FROM events` — no partition column in the dedup key set),
  consumed by an outer aligned per-row query. The outer query is clean; only the CTE body is
  hazardous.
- verdict: **REFUTED = fail-OPEN admission hole, confirmed and FIXED.** RED: the model was
  ADMITTED (the HAVING/DISTINCT admission walks covered only the outer UNION chain and CTE
  bodies are exempt from the subquery gate — research doc §6 gap 2); a late row appended into
  the NEXT partition changed the CTE's dedup output for a key whose fact rows live in the
  already-written 2024-01-01 partition, which batched maintenance never rewrites: maintained
  1 row vs full-refresh oracle 2 rows, permanently. GREEN: batched admission now judges
  **every scope the composition walk enumerates** (CTE bodies, derived tables, set-operation
  arms) with the same AST-pure per-scope classifiers (`scope_group_by_alignment` /
  `scope_distinct_alignment` / `window_over_alignment`), and refuses the model.
- production files/functions changed: `crates/smelt-logical/src/analysis/walk.rs`
  (`batched_admission_violations` — the admission transfer function over the Phase-1 walk;
  per-scope region collection for OVER/LIMIT), `crates/smelt-logical/src/analysis/mod.rs`
  (`window_over_alignment` + `window_has_bounded_range_interval_frame` leaf classifiers),
  `crates/smelt-logical/src/rules/incremental.rs` (gates 2a/2b/2c/2f rewired onto the walk;
  the uppercase-substring `find_inadmissible_over` scanner and the textual LIMIT keyword scan
  deleted; new gate 2g refuses fail-closed on any construct the walk cannot normalize).
  Bounded-`RANGE BETWEEN INTERVAL` frames stay exempt (reach obligation, not alignment),
  now via the frame's AST.
- evidence: `smelt-cli::tests::property_discovery::sc_7_cte_body_admission::
  cte_internal_cross_partition_distinct_is_refused` (owning test; red-then-green through
  `execute_project` with `enforce_safety`), plus unit mirrors
  `smelt-logical::rules::incremental::tests::cte_body_{having,distinct,over,limit}_gated_same_as_outer`
  (same construct judged identically at top level and inside a CTE; aligned CTE-internal
  scopes stay admitted — no blanket refusal). Full workspace suite, `example_diagnostics`
  (115 passed) and `example_workspaces` (34 passed) green — no example model newly refused.
- Coverage caveat (design §2.1 N4): deterministic 2-run schedule — the divergence is
  mechanism-level (an unjudged cross-partition scope), not data-dependent.

## SC-4 — stacked bounded `RANGE` frames across CTE layers × widened scan + exact clamp — 2026-07-07

- construct: `refresh: batched` model with a 7-day `RANGE` frame inside a CTE and a 3-day
  `RANGE` frame over the CTE's output. True backward reach is the SERIES SUM (10 days): an
  output row reads 3d of inner values, each of which reads 7d of source rows.
- verdict: **REFUTED = under-widened scan, confirmed and FIXED.** RED: the whole-text bound
  derivation (`derive_bound_for_source`) max-merged every frame it found → 7d; run 2's source
  scan `[D−7d, D+1)` excluded a row 10 days back, truncating the inner running sum near the
  scan edge: maintained `m3(2024-01-11)` = 2 vs full-refresh oracle = 101, silently (the
  model is admitted — bounded `RANGE INTERVAL` frames are the Form-A exemption). GREEN:
  `derive_model_bounds` now runs the composition walk — each query-tree node derives reach
  from its OWN region only, children compose in parallel (`BoundResult::merge`, max) across
  set-op arms and join inputs, and a node's own reach composes in series (`BoundResult::then`,
  add) onto every source beneath it. Chained interval-join bands add along the path via a
  conservative sibling-slack carry (may over-widen a scan, never under-widen). An absorbing
  region verdict (`Unbounded`/`NotDerivable`) still rejects every context source — the
  whole-text conservatism is kept for rejections; only the additive arithmetic is per-region.
- coverage caveats (recorded honestly): (a) a tree the walk cannot fully normalize falls back
  to the legacy whole-text derivation for every source — the known case is the redundantly-
  parenthesized derived table function expansion emits (`FROM ((SELECT …)) AS t`), which the
  parser does not nest (`QueryNode::has_unsupported` gates the fallback); series-add does not
  apply there, exactly matching pre-fix behaviour. (b) Same-scope chained bands (a→b→c all in
  ONE FROM clause) still max-merge within the region; chains split across CTE/derived-table
  layers compose correctly. Both residues tracked in `model_properties.md` §Known Divergences.
- production files/functions changed: `crates/smelt-logical/src/analysis/source_bounds.rs`
  (`BoundResult::then`, `ReachTransfer`, `derive_region_reach`, `derive_model_bounds`
  rerouted through the walk with the fallback above; context keys resolved in both planner
  (`sources.x`) and runtime (`smelt.sources.x`) forms), `crates/smelt-logical/src/analysis/walk.rs`
  (`own_region_text` — a node's region minus child walk-node subtrees, expression-position
  subqueries kept; `QueryNode::has_unsupported`).
- evidence: `smelt-cli::tests::property_discovery::sc_4_stacked_frames::
  late_row_inside_summed_reach_is_folded` (owning test; red-then-green through
  `execute_project`), plus unit mirrors `smelt-logical::analysis::walk::tests::
  {reach_series_adds_parallel_maxes, chained_join_bands_add_along_path}` (series-add,
  parallel-max, symbolic-offset absorption, chained-band carry). Full workspace suite,
  `example_diagnostics` (115), `example_workspaces` (34), `property_discovery` (38) green.

## SC-5 — window frame + declared lateness folded with max × widened scan — 2026-07-07

- construct: a bounded frame (computation-reach) plus declared lateness (`data_latency` on
  the event-time column), combined by `compute_effective_window` with `max` where the sound
  composition of two independent quantities is `+`.
- verdict: **CONFIRMED AT THE SITE, NOT REPRODUCIBLE AT linkC — fixed as a unit-level
  correction, with a larger finding recorded.** The max-vs-sum site is real
  (`temporal.rs::compute_effective_window`), but the value it feeds —
  `IncrementalBatch::{filter_start, filter_end}` — is consumed by **no live execute path**:
  the runtime's actual scan widening is `inject_source_filters` ← `build_source_bound_map`
  ← `derive_model_bounds` (per-source `BoundResult`), and the declared lateness never enters
  that map. In the current write-window model (write window = the requested range only, never
  extended backward by lateness) lateness scan-widening is not needed for the write window's
  own correctness: any input row affecting an output in the write window lies within the
  computation-reach of that window regardless of when it arrived. Lateness only becomes a
  scan obligation once a lateness-extended write window exists — the unbuilt horizon
  settled-delay / tail-rewrite transform (`model_transforms.md`). Fixing max→sum at the site
  keeps the advisory number sound for that future consumer.
- production files/functions changed: `crates/smelt-logical/src/analysis/temporal.rs`
  (`compute_effective_window`: `ast_days.max(latency)` → `ast_days.saturating_add(latency)`).
  Tests that encoded the max (`test_effective_window_ast_wins_over_latency` →
  `..._ast_plus_latency`, `..._latency_wins_over_ast` → `..._latency_plus_ast`,
  `windowing_parity::test_multi_source_bound_aware_windows`) updated with the rationale
  in-place.
- evidence: `smelt-logical::analysis::temporal::tests::effective_window_sums_lateness_and_reach`
  (owning unit test, red-then-green). No linkC harness cell lands (the divergence is not
  reachable through `execute_project` today); the dead-consumer finding
  (`batch.filter_start/filter_end` unused outside tests) is the actionable residue, tracked
  with the tail-rewrite transform.

## SC-6 — declared FD over a `UNION ALL` body × once-write / FD-widened admission — 2026-07-08

- construct: a model whose body is `SELECT customer_id, region FROM crm_a UNION ALL SELECT
  customer_id, region FROM crm_b`, with a declared `functional_dependencies: [{key:
  [customer_id], determines: region}]`. Each arm may hold `customer_id → region` as a
  world-fact, but the union does not — the same `customer_id` can appear in both arms with a
  different `region` (concretely `(c1,'EU')` in arm A, `(c1,'US')` in arm B).
- verdict: **CONFIRMED = analyzer-fact bug (linkB), FIXED.** RED: `functional_dependency_verdict`
  took only `declared: bool` and never read `FunctionalDependency.key`, and no union analysis
  existed; a `determines` column with no traceable join origin resolved `determines_fan_out =
  None`, so a declared FD over the union widened `None → Constant` — exactly the unsound
  widening §3.8 shows destroys the FD. GREEN: the composition walk now derives a per-model
  `PropertyVector` (grain from the `GROUP BY`/`DISTINCT` factory and the discriminated-union
  rule; a set-operation FD barrier; per-column determinism; per-column aggregate
  discriminants), and the new key-aware `functional_dependency_verdict_over_vector(key,
  determines, vector, declared)` reads the declared `key`: it refuses a `determines` column
  crossing an undiscriminated `UNION ALL` (`has_set_op_barrier`), refuses a fan-out join
  (`has_fan_out_join`), returns `Constant` when a proven grain key subsumes the declared key
  (query-derived, no declaration needed), and widens only a genuinely-undecidable
  single-branch origin. A declared key column the model does not project is not widened
  (the "parsed but never read" gap closed).
- smelt analyzer: was **unsound** (widened a union FD it could not prove) → now **sound**
  (fail-closed refusal, over-refusal permitted). No once-write consumer is wired, so this is a
  proof-layer classification fix only (no transform emitted; wiring stays with the
  model-updates master).
- interaction with G-10 (composite unique key inexpressible in `JoinContext`): not worsened.
  The vector's grain/fan-out facts still flow through `join_shape::fan_out`, which fails closed
  to `OneToMany` on a composite key — a declared FD backed by a composite unique key therefore
  falls to the undecidable-single-branch arm and is widened by the declaration (unchanged), or
  refused if it crosses a union (correct). G-10's over-conservatism is inherited, not deepened.
- production files/functions changed: `crates/smelt-logical/src/analysis/walk.rs`
  (`PropertyVector`, `Grain`, `DerivedFd`, `Determinism`/`ColumnDeterminism`,
  `ColumnDiscriminant`, `PropertyTransfer`, `model_property_vector`; the union
  discriminated-grain and columnar-determinism folds), `crates/smelt-logical/src/analysis/
  functional_dependency.rs` (`functional_dependency_verdict_over_vector` — the key-aware
  verdict; existing `functional_dependency_verdict` preserved and still used for the pairwise
  join case), `crates/smelt-logical/src/analysis/mod.rs` (re-exports).
- evidence: `smelt-cli::tests::property_discovery::sc_6_fd_over_union::
  declared_fd_over_union_all_is_refused` (owning test; the over-narrowing guard
  `declared_fd_over_single_branch_still_widens` alongside it), plus unit mirrors
  `smelt-logical::analysis::functional_dependency::tests::{fd_key_field_is_consulted,
  declared_fd_over_union_all_does_not_widen}` and `smelt-logical::analysis::walk::tests::
  {group_by_establishes_grain_and_fds, union_all_drops_grain_and_fds_unless_discriminated,
  determinism_predicate_registered_as_leaf}`. Full `smelt-logical` lib (323 passed),
  `property_discovery` (40 passed), `example_diagnostics` (115 passed) green.
- Coverage caveat: the SC-6 cell is linkB — it asserts the analyzer fact directly (no once-write
  consumer exists to drive a linkC divergence). The narrative divergence (a frozen once-write
  value for a colliding key) is described, not executed.
- (Phase 7 review, 2026-07-08) `has_set_op_barrier` grades a *bare* `UNION ALL` (no branch-level
  discriminant or declared disjointness) as `Refused`, where `docs/research/20260707-property-
  per-key-constancy.md` §5 prescribes `NotProven` — a bare union's branches might be key-disjoint
  as a world-fact, so a future branch-disjointness declaration could widen it; `Refused`
  forecloses that route entirely. This is strictly more conservative, not unsound, and stays
  as-is behaviourally (docs-only note); a widening-in-place fix, if wanted, is future work.
