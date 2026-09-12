# Phase 6 summary — trust the DuckDB numbers

**Status:** blocked (partial — infrastructure landed, centrepiece test ignored)

## Shipped

- `crates/smelt-cli/tests/github_activity_support/mod.rs` — the replay harness
  (`stage_workspace`, `duckdb_exec`/`duckdb_scalar_i64`, `create_empty_raw_table`,
  `load_day`, `smelt_run`, `FIXTURE_DAYS`, `day_after`, `replay_days`) extracted from
  `github_activity_replay.rs` so both test binaries share it (task 1 — also brought
  `github_activity_replay.rs` from 951 to 788 lines, under the 1000-line cap).
- `crates/smelt-cli/tests/github_activity_oracle.rs` — new test binary:
  - `discover_relations`: generic discovery via `information_schema.tables`, excluding
    `sources_*`/`_smelt_*`.
  - `compare_databases`/`relation_diff`: `ATTACH`-based `EXCEPT ALL` comparator in both
    directions, with JSON-rendered sample offending rows.
  - `DIVERGENCE_REGISTRY`: the two known succession entries (`silver_repo_naming`,
    `silver_actor_naming`), each bounded by `check_bound` — zero rows the incremental leg
    holds that the oracle lacks, and the oracle folds to exactly one row per `(key, clock)`
    group (not a magic row-count delta).
  - `full_replay_pair()`: the expensive 30-day full-refresh oracle, built at most once per
    test binary process via `OnceLock`, shared between the tests that need it.
  - 4 of 5 planned tests green: `oracle_comparison_covers_every_materialised_relation`,
    `an_unregistered_divergence_fails`, `succession_divergence_is_exactly_tied_row_
    multiplicity`, `registry_entries_are_all_live`.
  - `every_window_matches_the_full_refresh_oracle` (the centrepiece) is present but
    `#[ignore]`d — see Decisions.
- `examples/github_activity/README.md`: new "Trusting the numbers" section.
- `github_activity_replay.rs::full_refresh_matches_incremental_replay`'s original hardcoded
  row-count assertions were restored (not retired) after the centrepiece test was blocked —
  see Decisions.

## Decisions

- **Restored, not retired, the old row-count assertions.** The plan's task 7 asked to
  retire them in favor of the oracle binary; done and then reverted once the oracle's
  centrepiece test had to be `#[ignore]`d, so regression coverage isn't lost while it's
  blocked. Re-retire in the same change that unblocks the centrepiece test.
- **Shared the expensive full-30-day oracle build via `OnceLock`** across `succession_
  divergence_is_exactly_tied_row_multiplicity` and `registry_entries_are_all_live` (not in
  the plan, but cut wall time from ~2x37s to ~37s for those two tests combined; the
  centrepiece test reuses the same pair for its final checkpoint).
- **Blocked the centrepiece test rather than inventing a bound for the new divergence
  found.** See "For the next planner" and `outcome.md` "## Blocked" (phase 6) for the full
  writeup and candidate options.

## For the next planner

- `every_window_matches_the_full_refresh_oracle` found a real, previously-hidden divergence
  in `gold_events_enriched`: an early fact row's `current_repo_name` can be stale for at
  least one incremental window after a same-window rename, then self-heals by a later run
  (confirmed zero-diff at the full 30-day window via a manual rerun). This directly serves
  criterion 7 (it's exactly the kind of defect the outcome says per-window checking should
  surface) but needs root-causing before it can be bounded or fixed — not descoped, just
  not completable within this phase's budget. Full detail, root-cause candidates, and three
  options are in `outcome.md` "## Blocked" (phase 6).
- The oracle only checked the first 10 days + the final (30th) day, per the plan's own
  runtime-bounding allowance. This means an intermittent recurrence of the same divergence
  class in days 10-28 would not have been caught — worth widening to every day once the
  root cause is known and the fix (or bound) is in place.
- Once the finding above is resolved, `full_refresh_matches_incremental_replay`'s restored
  row-count assertions should be re-retired (they'd be fully superseded again).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  workspace test, example_diagnostics).
- `cargo test -p smelt-cli --test github_activity_oracle` — 4 passed, 1 ignored (documented).
- `cargo test -p smelt-cli --test github_activity_replay` — 16 passed (unchanged from before
  the extraction).
- `cargo test -p smelt-cli --test example_diagnostics` — 125 passed, 1 ignored (pre-existing).
- `bash .claude/scripts/large-file-check.sh` — OK (`github_activity_replay.rs` now 788
  lines, `github_activity_oracle.rs` 480, `github_activity_support/mod.rs` 190).

## Timing

- `github_activity_oracle.rs`'s green subset (tests 2-5): ~37s (dominated by the shared
  30-day full-replay pair).
- `every_window_matches_the_full_refresh_oracle` (ignored, run manually while
  investigating): failed at the second checkpoint (~3s), so full-suite timing for the
  centrepiece test with all 11 checkpoints is unmeasured — budget for it when it's
  unblocked.
