# Phase 24 summary

## Shipped

- `resolve_run_window(run: &PropagatedRun, now: &str) -> Result<PropagatedRun>`
  (`crates/smelt-runtime/src/propagation.rs`): resolves an open-ended
  `(start: Some, end: None)` propagated run to a closed `[start, today + 1
  day)` window against the caller-supplied `now`; a closed or whole-table
  run passes through unchanged; a `start` on/after the resolved end refuses
  fail-loud naming the model and both dates.
- Wired into `crates/smelt-cli/src/commands/run.rs::run_since_upstream`'s per-run
  loop: `ExecuteRequest.start/end` now come from `resolve_run_window(run, &now)`
  (the same `now` already built for `plan_since_upstream_with_observed_deltas`),
  so `parse_run_window`'s "Both start and end" guard is never reached for an
  open-ended self-edge frontier. The `--verbose` log line prints both the
  open-ended and resolved-closed forms (`[s, →) → [s, e)`).
- Spec paragraph in `docs/specs/incremental_models.md` §"Time-unrolled
  self-edges" stating the resolution rule (dirty set stays open-ended, run
  window is closed at scheduling time, an already-past frontier refuses) plus
  a mirroring sentence in `docs-site/docs/reference/cli.md`.
- Tests: 5 pure unit tests in `crates/smelt-runtime/tests/since_upstream_propagation.rs`
  and 2 CLI end-to-end tests in `crates/smelt-cli/tests/since_upstream.rs`
  (`web_analytics_whole_workspace_since_upstream_dry_run_completes`,
  `web_analytics_open_ended_run_logs_the_resolved_window`) — the flagship gate
  drives the real, unfiltered `examples/web_analytics` workspace (no
  `--select`) through `smelt run --since-upstream --dry-run` and asserts it
  completes with exit 0.

## Decisions

- The CLI flagship test points `--project-dir` directly at the real
  `examples/web_analytics` directory (read-only) with `--database` overridden
  to a tempdir file, rather than copying the whole workspace + running
  `smelt-datagen` (the pattern `since_upstream_composed_web_analytics.rs`
  uses) — `--dry-run` needs no real backend schema data
  (`rebuild_dry_run.rs`'s own doc comment already established "the run
  succeeds against a project whose `.duckdb` target need not exist"),
  confirmed live before writing the test.
- The `--verbose` log-line assertion avoids hardcoding today's date (the
  resolved end is `now + 1 day` against the real wall clock) — it checks for
  the `→) → [` transition marker instead, so the test doesn't rot day to day.
  Log output goes to **stdout** via `tracing`'s default writer, not stderr —
  corrected mid-implementation after the first version of the log-line test
  read the wrong stream and failed with an empty message.

## For the next planner

- Live-verified: the whole-workspace `--since-upstream --dry-run` run now
  produces 7 `RUN` lines and exits 0, including `silver.sessions_chained`
  (open-ended frontier, phase 22's self-edge) and `silver.device_user_edges`'s
  chain — device_user_edges itself is not in the propagated set for this
  `--landed sources.raw.events` delta shape (it isn't downstream of
  `raw.events` for this fixture), so phase 24b's `RepairKeysNotDiscoverable`
  gap for that model is untouched by this phase and stays exactly as
  planned.
- `window_independence`'s own `Ordered` verdict still doesn't check
  `before > 0` for a same-partition self-read — surfaced in phase 22's
  summary, still open, not this phase's concern (a narrower pre-existing gap
  in the ordered-backfill execution path, not the graph/scheduling layer).
- No new `unwrap`/`expect`/`println!` landed — `hardening_budget` untouched.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature
  sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-runtime --test since_upstream_propagation` — 34 passed.
- `cargo test -p smelt-cli --features duckdb --test since_upstream` — 19
  passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 119 passed, 1
  ignored.
