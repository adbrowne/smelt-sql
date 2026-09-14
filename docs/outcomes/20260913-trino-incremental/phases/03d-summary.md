# Phase 3d summary — append family landed; whole-row `MERGE` upsert family blocked on gaps 4/5

**Status: blocked** (not `done` — the row's own acceptance criteria name the whole-row `MERGE`
family explicitly, and it does not run today). Full writeup in `outcome.md`'s Blocked log
(2026-09-15) and Decision log; this file is the condensed version.

## Shipped

- `append_family_matches_full_refresh_on_trino` (`crates/smelt-cli/tests/
  trino_incremental_families.rs`) — an insert-only `grain: partition` model, run as two disjoint
  windowed `smelt run --target trino` invocations, asserted multiset-equal to a `--full-refresh`
  rebuild in a second schema. Verified live (`scripts/trino-up.sh`).
- `trino_ci_wiring.rs`'s live-gated census is now **derived**: `discover_test_binaries`/
  `live_gated_census` scan `crates/{smelt-cli,smelt-backend-trino}/tests` for real call-site markers
  (`trino_env(`, `targets_to_run_with_trino(`, `live_env_or_skip(`, a raw `env::var("SMELT_TRINO_URL")`),
  not a hardcoded list or a filename pattern. New test
  `the_trino_job_runs_every_live_gated_trino_test_binary` asserts every derived `(crate, binary)`
  pair appears in the `trino-integration` job. `every_trino_gated_test_file_skips_through_the_shared_env_gate`
  rewritten to consume the same census, narrowed to single-purpose skip-gated binaries (a
  `_parity` suite driven only by `targets_to_run_with_trino()` has no per-Trino "Skipping" line of
  its own to check — a different, still-legitimate skip shape).
- `.github/workflows/compat.yml`'s smelt-cli step now also runs `trino_ddl_live`,
  `trino_state_residency`, `trino_lock_versioning`, `trino_incremental_families` — all previously
  unrun in CI despite being live-gated — verified passing live.
- Module doc on `trino_incremental_families.rs` records gaps 4 and 5 (below) for the next reader.

## Decisions

- Did **not** force the whole-row `MERGE` upsert family: live-measured (real `smelt run --target
  trino` against a live coordinator) that it hits two gaps this phase's task list has no fix for
  (gap 4: windowed-keyed driver's driving-source pushdown renders an untyped literal; gap 5:
  `Technique::KeyedFold`'s `Grade::Additive` hard-refuses on a fully-degraded backend with no
  plan-time downgrade, and even the idempotent grade needs its own downgrade path added). Per
  RECORD-AND-CONTINUE, recorded the measured errors and blocked rather than forcing a workaround
  outside scope.
- Reverted the `RecordingBackend`/`smelt-backend-trino` dev-dep/`trino.rs` scaffolding drafted for
  test 3 (`keyed_fold_parity_on_trino`) once the family it needs was confirmed unreachable — unused
  code left in place would violate "no half-finished implementations."
- Corrected a stale claim in the prior decision-log entry: `trino_posture_plan_invariance.rs` and
  `trino_broken_foreign_keys.rs` are fully offline (no live connection at all), not live-gated —
  the derived census correctly excludes them; no CI change needed for those two.

## For the next planner

- Gap 4 and gap 5 (full detail in `outcome.md`'s Blocked log) block the whole-row `MERGE` family and
  `statement_parity`'s Trino leg. Candidate next steps are listed there — fix gap 4 via the
  `resolve_partition_column_type` path 3b2 landed, then decide gap 5 (add a plan-time downgrade for
  `Technique::KeyedFold`, or declare it a permanent Trino gap like T3/T4's fully-degraded ruling).
- Once fixed, re-run this phase's originally-planned tests 2/3 — the design (SUM-combiner clocked
  keyed model, `emit_keyed_fold` byte-identity via a generalized `RecordingBackend`) is sound and
  cheap to redo.
- Row 3e (live-Trino test isolation) is unaffected and still `pending`.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature sets,
  shellcheck, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 45 passed (no live
  tier touched; unaffected by this phase since the Trino scaffolding was reverted).
- `cargo test -p smelt-cli --test trino_ci_wiring` — 6 passed.
- Live tier (`scripts/trino-up.sh` / `trino-env.sh` / `trino-down.sh`, serial per 3b2's workaround):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 4 passed
  (including the new `append_family_matches_full_refresh_on_trino`).
  `cargo test -p smelt-cli --test trino_ddl_live --test trino_state_residency --test
  trino_lock_versioning -- --test-threads=1` — 10 passed.
  `cargo test -p smelt-cli --test materialization_parity --features duckdb -- --test-threads=1` —
  2 passed.
