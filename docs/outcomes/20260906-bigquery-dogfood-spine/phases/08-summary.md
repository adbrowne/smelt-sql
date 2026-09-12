# Phase 8 summary — settle the DuckDB half of criterion 7

**Status:** done

## Shipped

- `crates/smelt-cli/tests/github_activity_oracle.rs`: `every_window_deep_sweep` (new,
  `#[ignore]`d measurement sweep, run once this phase) checked all 30 windows and found
  **two more previously-unknown, unregistered divergences** beyond `gold_events_enriched`:
  `silver_actor_sessions` and `marts_daily_active_contributors`.
- `DivergenceEntry`/`check_bound` generalised from one fold-equality shape to a `Bound` enum
  with three shapes: `FoldEquality` (unchanged, the two succession entries), `StaleButHistoricallyValid`
  (new — `gold_events_enriched`), `MonotoneDivergence` (new, parameterised by which side
  (`Side::Oracle`/`Side::Incremental`) is never allowed to lead — `silver_actor_sessions` and
  `marts_daily_active_contributors`, in **opposite** directions). Registry is now 5 entries.
- `every_window_matches_the_full_refresh_oracle` un-`#[ignore]`d and **promoted to check
  every one of the 30 windows** (not the first-10-plus-final sampling) — measured at 108s,
  well under the plan's 5-minute budget.
- Two new tests: `enrichment_staleness_is_confined_to_the_enriched_column`,
  `enrichment_staleness_is_never_a_fabricated_value` (replaces the plan's proposed
  "...converges_within_the_declared_bound" — see Decisions).
- `github_activity_replay.rs::full_refresh_matches_incremental_replay`'s hardcoded row-count
  loops (3-table, 5-table, and the 139/145 succession deltas) retired — superseded by
  `github_activity_oracle.rs`'s content-level, per-window checks. Kept: the distinct-id and
  64,313 business-count assertions (not oracle comparisons).
- `examples/github_activity/README.md` "Trusting the numbers" rewritten for 5 registry
  entries and to correct the wrong "self-heals" claim.

## Decisions

- **The plan's central assumption — that the staleness converges within N windows — is
  false, measured directly.** `every_window_deep_sweep`'s per-day stale-id list for
  `gold_events_enriched` is strictly non-decreasing across all 30 days (1, 1, 2, 5, 5, 8, 13,
  15, 16, 22, 22, 22, 28, 28, 28, 29, 29, 29, 31, 36, 36, 37, 37, 37, 37, 38, 38, 38, 39) —
  zero rows ever heal, including the specific row (`id=16854100084`) the phase 6 "manual
  rerun" claimed was byte-identical by the final window. That claim was wrong: it was a
  row-count check, not a content check. Root cause (confirmed by reading
  `crates/smelt-logical/src/maintenance/derive/model_edge.rs`, not inferred): no
  `UpstreamMutation(gold.repo_dim)` cell is ever derived, so a MERGEd row's
  `current_repo_name` is frozen forever — nothing revisits it. The registered bound is
  therefore non-fabrication (`StaleButHistoricallyValid`: every stale value is a name the
  repo genuinely held earlier, never invented), not convergence.
- **Two more divergence classes surfaced, unplanned but in-scope** (the plan's own task 2
  asked "is any relation other than gold_events_enriched involved" without assuming the
  answer). `silver_actor_sessions`: `compute_calendar_windows`
  (`crates/smelt-runtime/src/windowing.rs`) applies the Form-B forward-reach rebase only at
  the two outer edges of a single multi-day invocation, never at an interior chunk boundary —
  so **the full-refresh oracle itself under-counts** a cross-midnight session inside a wide
  `--full-refresh`; the incremental leg is correct (confirmed against a from-scratch raw-SQL
  recomputation). `marts_daily_active_contributors`: a *different*, fourth root cause — this
  Form-A downstream aggregate has no rebase of its own and never revisits an already-written
  partition, so it never learns when `actor_sessions`'s own (correct) rebase rewrites an
  earlier partition; its `total_events` is frozen at first-write time, a strict subset of the
  oracle. Both registered as `MonotoneDivergence` with opposite `behind_side`. All four root
  causes (naming-tie fold, enrichment freeze, oracle windowing gap, mart repair gap) are
  handed to `bigquery-correctness` as criterion-8 findings, not fixed here.
- **Renamed `enrichment_staleness_converges_within_the_declared_bound`** (the plan's proposed
  test name) to `enrichment_staleness_is_never_a_fabricated_value`, since convergence doesn't
  exist to test. Measured reality overriding a plan assumption, same posture as the outcome's
  other decision-log entries.
- **Re-retired, not deleted,** `full_refresh_matches_incremental_replay`'s row-count loops
  per the phase 6 plan's own instruction ("retire... leaving a pointer comment... distinct-id
  and business-count assertions stay").

## For the next planner

- Four criterion-8 findings now exist for `bigquery-correctness`, two of them (`silver_actor_sessions`
  oracle undercount, `marts_daily_active_contributors` repair gap) newly discovered this
  phase and not previously tracked anywhere. All four are named with file:line evidence in
  `DIVERGENCE_REGISTRY`'s doc comment (`github_activity_oracle.rs`) and `README.md`.
- The windowing bug (`compute_calendar_windows` losing forward reach at interior chunk
  boundaries) is **not specific to `--full-refresh`** — it affects any single invocation of a
  Form-B model spanning more than one partition chunk. Worth a standalone smelt-runtime bug
  report/fix independent of this outcome, since it means a very wide ordinary incremental
  backfill window could show the same undercount.
- The "no repair edge on self-rebase" gap (the mart's finding) is a third *instance* of the
  same missing-maintenance-cell shape as `gold_events_enriched`'s (a downstream that has no
  way to learn an upstream rewrote an already-materialised row) — worth checking whether
  `bigquery-correctness`'s fix for one naturally covers the other, or whether they need
  separate maintenance-cell work.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  workspace test, example_diagnostics).
- `cargo test -p smelt-cli --test github_activity_oracle` — 7 passed, 1 ignored
  (`every_window_deep_sweep`, measurement-only by design), 106s.
- `cargo test -p smelt-cli --test github_activity_replay` — 16 passed, 51s.
- `cargo test -p smelt-cli --test example_diagnostics` — 125 passed, 1 ignored (pre-existing).
- `bash .claude/scripts/large-file-check.sh` — OK (`github_activity_oracle.rs` 987 lines,
  `github_activity_replay.rs` 696 lines).

## Timing

- `every_window_deep_sweep` (measurement, run once manually): 108s for all 30 windows.
- `every_window_matches_the_full_refresh_oracle` (now unignored, per-PR): part of the 106s
  `github_activity_oracle.rs` full-file run above.
