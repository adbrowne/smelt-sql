# Phase 4 plan — the payload-independent widening

## Objective

Widen `examples/github_activity/` from the spine plus succession to the gold and mart
layers that make it a pipeline: a keyed repo dimension, the `LEFT JOIN`-against-a-
`unique_key`-declaring-dimension enrichment the outcome names as its `ColumnScopedMerge`
instance, a per-repo daily aggregate, and the two remaining marts from the research doc's
full sketch. Advances criterion 4 (every model builds, zero diagnostics, per-PR CI) and
supplies criterion 8 with its first offline finding if the enrichment's derived technique
is not the one the shape predicts. The typed fan-out is **not** here — it needs `payload`,
which needs a human-minted token (phase 5, blocked).

## Spec delta

None. This phase adds example models only; no user-visible feature behaviour changes.

## Tests

All in `crates/smelt-cli/tests/github_activity_replay.rs` unless noted.

- `repo_dim_is_one_row_per_repo` — `gold.repo_dim` has exactly one row per `repo_id`
  (5,016 in the fixture) and its `current_repo_name` matches the `is_current` row of
  `silver.repo_naming` for a spot-checked renamed repo.
- `events_enriched_preserves_every_fact_row` — the `LEFT JOIN` is row-preserving:
  `gold.events_enriched` has exactly `silver.events_deduped`'s row count, and no row has a
  NULL `current_repo_name` (every fact repo is in the dimension).
- `events_enriched_renamed_repo_carries_current_name` — for a repo the fixture renames, the
  enriched row for an *early* event carries the **current** name, not the name at the time —
  the property that makes the dimension join worth having.
- `events_enriched_dimension_mutation_cell_technique` — parses `smelt explain
  gold.events_enriched --json` and asserts the `UpstreamMutation(gold.repo_dim)` cell's
  resolved technique **equals whatever the derivation produces**, pinned as a literal in the
  test with a comment naming it. This is a characterisation test: it records the verdict so
  a later change moves it deliberately. If the verdict is not `column_scoped_merge`, the
  test still asserts the actual value and the summary records the gap as a criterion-8
  finding — do not change derivation code to force it.
- `repo_activity_daily_totals_match_events` — `SUM(event_count)` over
  `gold.repo_activity_daily` equals `silver.events_deduped`'s row count, and per-day totals
  match `marts.daily_active_contributors`'s `total_events` where both are defined.
- `star_growth_counts_the_fixtures_watch_events` — `marts.star_growth`'s final cumulative
  value is exactly 47 (measured `WatchEvent` count) and the series is non-decreasing.
- `repo_leaderboard_top_repo_is_the_bot_repo` — the leaderboard's top row is the known
  527-event repo, i.e. the mart reproduces the sample's documented skew rather than hiding it.
- `full_refresh_matches_incremental_replay` (existing) — extended to cover the four new
  incremental/full models. Any new divergence is recorded and asserted explicitly, in the
  style phase 3 used for the succession row-count gap; it is not silently tolerated.
- `github_activity_no_diagnostics` (existing, `smelt-cli --test example_diagnostics`) and
  `smelt-lsp --test example_workspaces` — must stay green with the new models.

## Tasks

1. Add `examples/github_activity/models/gold/repo_dim.sql` — one row per repo:
   `unique_key: [repo_id]`, no `timeseries:`, `refresh: incremental`; `current_repo_name`
   from `silver.repo_naming`'s `is_current` row, plus `first_seen_at`. Header comment states
   it exists to be the enrichment's keyed dimension.
2. Add `examples/github_activity/models/gold/events_enriched.sql` — `silver.events_deduped`
   `LEFT JOIN gold.repo_dim ON repo_id`, SELECTing the dimension's `current_repo_name` as a
   payload column (not merely reading it in `ON`). Frontmatter follows
   `ValueEnrichedRecipe`'s proven preconditions: `refresh: incremental`, `grain: partition`,
   `timeseries:` on `event_date`, `maintenance.scan_bounds.per_source` giving the dimension
   `allow_full_scan: true`, and the merge key spelled `merge_key:` — **not** top-level
   `unique_key:`, which would flip the derived grain to `Key` and contradict
   `grain: partition` (see `crates/smelt-maintenance-testkit/src/recipe.rs`'s
   `ValueEnrichedRecipe::model_file` doc comment).
3. Run `smelt explain gold.events_enriched --json`, read the dimension-mutation cell's
   technique, and write it into the characterisation test and a header comment on the model.
4. Add `examples/github_activity/models/gold/repo_activity_daily.sql` — per
   `(repo_id, event_date)` event/actor counts from `silver.events_deduped`;
   `refresh: incremental`, `grain: partition`, partitioned on `event_date`.
5. Add `examples/github_activity/models/marts/repo_leaderboard.sql` and
   `.../star_growth.sql` — `refresh: full`; leaderboard over `gold.repo_activity_daily`,
   star growth as a cumulative daily `WatchEvent` count over `gold.events_enriched`.
   Both carry a header comment pointing at the sample-skew finding in `README.md`.
6. Extend `setup_sources.sql` / `run_incremental.py` / the `github_activity_replay.rs`
   harness so the new models are staged, replayed and full-refresh-compared alongside the
   existing ones.
7. Write the tests above red, then make them green.
8. Update `examples/github_activity/README.md`: a "Gold and marts" section, the model list,
   and one sentence stating that the typed silver fan-out awaits the `payload` re-pin
   (phase 5) so a reader does not conclude it was forgotten.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_replay`
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces github_activity`
- `python3 examples/github_activity/run_incremental.py` — full 30-day replay + `smelt test`

## Commit message

`feat(examples): gold and marts for github_activity, payload-free`
