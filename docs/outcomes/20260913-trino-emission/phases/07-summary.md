# Phase 7 summary — `Restructure`/`Rewrite` on a fourth dialect

## Shipped

- **Measured live (2026-09-14, Trino tier via `scripts/trino-up.sh`):** unlike BigQuery, Trino
  accepts `MAX_BY`/`MIN_BY`/`approx_distinct` as window functions in **every** position —
  aggregate, whole-partition (`OVER (PARTITION BY g)`), and running-frame
  (`OVER (PARTITION BY g ORDER BY t)`) — all execute and return correct per-row answers. No
  `Emission::Restructure`/`Emission::Unsupported` pair is needed for these on Trino.
  `percentile_cont`/`percentile_disc` remain unsupported on Trino, but for an unrelated reason:
  `FUNCTION_NOT_FOUND` — Trino has no function under either name at all (confirmed via
  `SHOW FUNCTIONS LIKE 'percentile%'`, empty result), not a `WITHIN GROUP` shape mismatch as the
  stale ledger comment implied. `LISTAGG ... WITHIN GROUP (ORDER BY ...)` also works fine as a
  window function on Trino (checked as a sanity probe, not part of the candidate list).
- `crates/smelt-types/src/signatures/builtins/extended_aggregates.rs`: `ARG_MAX`, `ARG_MIN`,
  `APPROX_COUNT_DISTINCT` each gain a single `(DialectId::Trino, Position::Any, Emission::Rename(...))`
  entry (`MAX_BY`, `MIN_BY`, `APPROX_DISTINCT`), same shape as their existing Spark verdicts.
- `crates/smelt-db/tests/dialect_audit/ledger.rs`: the three now-closed `#209` gap rows
  (`APPROX_COUNT_DISTINCT`, `ARG_MAX`, `ARG_MIN`) deleted.
- `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_trino` 58 → 55, with a dated sign-off entry.
- `docs/specs/multi_backend.md` §"Statement-level lowering": the aggregate-only-in-window-position
  paragraph now states Trino's negative finding explicitly (no `AnalyticToCte` candidate found;
  the coverage gate, not this enumeration, is what protects a future one), and the null-safe-join
  bullet adds Trino's `IS NOT DISTINCT FROM` spelling.
- Tests (all red-then-green):
  - `crates/smelt-dialect/tests/window_decorrelation.rs::trino_arg_max_window_prints_natively_no_restructure`
    — asserts `plan_restructure` returns an empty plan for Trino + `ARG_MAX` under a
    whole-partition window, and the printed call is a plain in-place rename with the `OVER`
    clause untouched, no `__smelt_` synthesis.
  - `...::trino_arg_max_running_window_prints_natively_no_refusal` — the running-frame
    (`ORDER BY`) form prints natively too, contrasting with BigQuery's refusal of the same
    built-in shape.
  - `crates/smelt-types/tests/registry_coverage/emission.rs::trino_restructure_pairs_with_a_window_refusal`
    — structural, forward-guarding gate: any future `(Trino, WholePartitionWindow, Restructure)`
    verdict must pair with a `(Trino, Window, Unsupported)` one. Vacuously green today (Trino has
    zero `Restructure` verdicts) by design — it exists so the first one can't land unpaired.
  - `crates/smelt-db/tests/dialect_audit/trino.rs::trino_arg_max_window_agrees_with_duckdb_native`
    — live: compiles smelt's `ARG_MAX` through the real registry lowering for both engines over a
    hand-built fixture whose grouping column includes a NULL row, asserts row-for-row and
    cell-for-cell agreement between Trino and DuckDB (both native window forms, no restructure/join
    involved on either side).
  - `schema_leg_trino`/`value_leg_trino` automatically gained the new `MAX_BY`/`MIN_BY`/
    `APPROX_DISTINCT` probes at all three positions (aggregate probing is universal for
    `ExprKind::Agg` entries) and pass live.
- `docs/reference/dialect-coverage.md` regenerated (`SMELT_REGEN_DOCS=1`).

## Decisions

- **`AnalyticToCte` does not apply to Trino among this phase's candidates.** Ruling recorded in
  the spec delta and the baseline sign-off entry: Trino's analytic-function support is broader
  than GoogleSQL's (closer to DuckDB/Postgres's "almost any aggregate works as a window function"
  shape), so none of `MAX_BY`/`MIN_BY`/`APPROX_DISTINCT` needed the restructure machinery this
  phase was written to exercise. The phase's own plan anticipated this outcome explicitly
  ("if the measurement finds no analytic-only built-in on Trino, say so in one sentence") — tests
  1–4 and 6 as originally specified (which assumed a live restructure/refusal candidate) were
  adapted to instead pin the *negative* result: native pass-through with no synthesis, no refusal,
  and a structural gate guarding the day a genuine candidate does appear.
- **No printer change was needed** — confirms `emission_ownership` is the correct oracle; only
  registry data changed.
- Kept `Emission::Rename` spellings uppercase (`MAX_BY`, not `max_by`) for consistency with the
  existing BigQuery/Spark convention in this file, even though Trino identifiers are
  case-insensitive.

## For the next planner

- **Phase 8** (seams: `dialect_seam`, `emission_ownership`, `projection_dialect_invariance`) is
  unaffected by this phase's negative finding — those gates don't assume a Restructure verdict
  exists, they assume `Unsupported` and native paths are exercised, which this phase's registry
  changes don't touch.
- **Phase 9** (type-oracle question) can proceed independently; nothing here informs it.
- Worth a follow-up (not scoped to this outcome): `percentile_cont`/`percentile_disc`'s ledger
  reason text ("requires a `WITHIN GROUP` clause; DuckDB's plain-call and `OVER` window forms do
  not parse") is misleading — the actual failure is `FUNCTION_NOT_FOUND`, not a shape mismatch.
  Low priority (the row's disposition — `Gap`, tracked under `#209` — is unaffected either way),
  but the next person reading `ledger.rs` for Trino context will be confused by it. Left
  unchanged here since fixing prose on an unrelated row was out of this phase's scope.
- If a genuinely analytic-only built-in on Trino is ever found (this phase's candidate list was
  not exhaustive over the whole registry, only `MAX_BY`/`MIN_BY`/`APPROX_DISTINCT`/
  `PERCENTILE_CONT`/`PERCENTILE_DISC`), `trino_restructure_pairs_with_a_window_refusal` will force
  its `Window` refusal to be stated, and `window_decorrelation.rs` has a live pattern
  (`bigquery_arg_max_under_window_to_cte_applies_rename_inside_cte`) to model the new test on.

## Gates

- Live (Trino tier exported): `cargo test -p smelt-db --test dialect_audit trino --quiet` (5
  passed, including the new live test); `cargo test -p smelt-backend-trino --quiet` (all green).
- Offline: `cargo test -p smelt-dialect --test window_decorrelation --test restructure_plan
  --test unsupported_emission --test emission_ownership --quiet` (all green); `cargo test -p
  smelt-types --test registry_coverage --quiet` (110 passed); `cargo test -p smelt-runtime --test
  restructure_multiplicity --quiet` (1 passed); `cargo test -p smelt-db --test dialect_audit
  --quiet` (70 passed, tier unexported).
- `bash .claude/scripts/verify-phase.sh` (Trino tier unexported): **ALL GREEN**
  (fmt, clippy both feature sets, shellcheck, full workspace `cargo test`, example_diagnostics).
- Ratchets: `dialect_gaps_trino` moved down only (58 → 55); `registry-migration-baseline.txt` and
  `parser-gaps-baseline.txt` untouched.
