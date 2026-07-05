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
  `crates/smelt-db/tests/prop_helpers/link_c_harness.rs`
  (`LinkCProject`, `SqlCapturingReporter`, `DuckDbBackendFactory`, `base_request`) — test-target-only
  (`crates/smelt-db/tests/`), tagged `EXPERIMENTAL(property-discovery): disposable`, passes
  `property-experimental-gate.sh`. Added `smelt-backend`, `smelt-backend-duckdb`, `tokio`,
  `tokio-util` to `crates/smelt-db/Cargo.toml` `[dev-dependencies]` (test-only deps, no production
  dependency change).
- evidence: `smelt-db::tests::link_c_harness_smoke::execute_project_derives_time_filter_no_hand_injected_where`
  (1 run, DuckDB oracle via `duckdb::Connection` read-back). Stages a `refresh: batched` model with
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
