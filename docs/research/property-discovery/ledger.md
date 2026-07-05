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
