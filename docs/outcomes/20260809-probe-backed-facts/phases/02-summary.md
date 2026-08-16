# Phase 2 summary — Probe emitters for FD, `bounded_domain`, append-only posture, `assert_monotonic`

**Shipped:**
- Four new pure emitters in `crates/smelt-logical/src/maintenance/emit.rs`:
  `emit_functional_dependency_probe`, `emit_bounded_domain_probe`,
  `emit_monotonicity_probe`, `emit_append_only_posture_probe` (plus the
  `AppendOnlyBaselinePartition` carrier struct). All pure string construction,
  no dispatch, no `DiagnosticCode`.
- Extracted the dialect-keyed wrapper out of `emit_recurrence_bound_probe`
  into `probe_dialect_string_type`/`probe_dialect_sample_agg`/
  `wrap_violation_probe`/`probe_key_display_expr`, shared by all five
  key-membership-shaped probes; `emit_recurrence_bound_probe`'s own SQL is
  pinned byte-identical by a golden test.
- Added `row_fingerprint_expr` (whole-row `sha256`) and threaded a cast-type
  parameter through `column_fingerprint_expr`/`concat_varchar_expr_typed` so
  the append-only probe's fingerprint is Spark-safe (`STRING`, not
  `VARCHAR`); `emit_fingerprint_digest_select` now calls the same helper,
  unchanged output (verified by its own existing tests).
- `docs/specs/model_properties.md` §"Probe obligation": the four rows move
  `not-yet` → `built (unwired)`, name their emitter, and a new sentence
  states the shared `violation_count`/`sample_keys` result shape. Known
  Divergences rewritten (six of seven rows now emitter-built, unique_key
  the one remaining `not-yet`). `docs/specs/diagnostics.md`'s matching entry
  updated the same way.
- New tests: 26 unit tests in `emit_statements.rs` (shape + golden +
  panic-on-empty-key), 8 DuckDB-executability tests in the new
  `crates/smelt-logical/tests/probe_execution.rs`, and 1 new
  `probe_obligation.rs` gate test pinning the four rows to their exact
  emitter names.

**Decisions:**
- The monotonicity probe's `LAG` is ordered by a `ROW_NUMBER() OVER ()`
  processed-row ordinal, **not** by the event-time column itself — ordering
  the window by the very column being checked makes every partition
  trivially sorted, so no violation could ever be detected. The plan's own
  test-1 phrasing ("ordered by event time") described the *trace*, not the
  literal `ORDER BY` clause; the executability test caught this immediately
  (violation_count came back 0 on genuinely out-of-order data).
- `row_fingerprint_expr` double-hashes a single-column digest (matching
  `emit_fingerprint_digest_select`'s pre-existing, unchanged convention:
  `concat_varchar_expr`'s single-column branch already returns a `sha256`
  digest, and the wrapper adds one more layer unconditionally). Kept as-is
  rather than special-cased, since it's pre-existing behavior this phase
  didn't introduce and every consumer only ever compares the digest for
  equality, never reads it as a literal.
- `bounded_domain`'s probe departs from the shared `wrap_violation_probe`
  shape (it needs a cap comparison the other four don't) but still reuses
  the shared dialect helpers (`probe_dialect_string_type`/
  `probe_dialect_sample_agg`) for its cast type and sample aggregate.

**For the next planner:**
- Phase 3/4 (referential-integrity wiring, runtime dispatch): all four new
  emitters are proven executable and discriminating but **nothing calls
  them yet** — no `DiagnosticCode` variants, no cadence, no cell-remedy
  marking. `docs/specs/diagnostics.md` and `model_properties.md` Known
  Divergences both point here now.
- `AppendOnlyBaselinePartition` currently has no persistence — phase 3/4's
  run driver needs to decide where the recorded baseline (row count +
  fingerprint per partition) is stored and refreshed between runs; this
  phase deliberately left that as the caller's problem (maintenance-plan
  purity).
- Out of scope, not touched: `unique_key`/`delta_identity` uniqueness probe
  (still `not-yet`, tracked separately per the outcome's success criterion
  2 wording, which named exactly these four).

**Gates:**
- `cargo test -p smelt-logical --test emit_statements --test probe_execution --test probe_obligation` — 56 passed.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` — 48 passed (no perturbation).
- `cargo test -p smelt-logical` (full crate) — all green.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, workspace tests, example_diagnostics).
