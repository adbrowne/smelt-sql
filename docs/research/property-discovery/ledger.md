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
