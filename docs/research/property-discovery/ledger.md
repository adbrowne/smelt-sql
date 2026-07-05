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
