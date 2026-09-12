# Phase 4 summary — the payload-independent widening

**Shipped:**
- `examples/github_activity/models/gold/repo_dim.sql` — one row per repo (5,016), current
  name from `silver.repo_naming`'s `is_current` flag via a single `GROUP BY repo_id`
  aggregate.
- `examples/github_activity/models/gold/events_enriched.sql` — every deduped event
  (64,313) `LEFT JOIN`ed to `gold.repo_dim`, no NULL `current_repo_name`.
- `examples/github_activity/models/gold/repo_activity_daily.sql` — per-`(repo, day)`
  event/actor counts, `SUM(event_count)` matches `silver.events_deduped`'s row count.
- `examples/github_activity/models/marts/repo_leaderboard.sql`,
  `.../star_growth.sql` — full-refresh marts.
- 9 new tests + a `full_refresh_matches_incremental_replay` extension in
  `crates/smelt-cli/tests/github_activity_replay.rs` (16 tests total, all green).
- `examples/github_activity/README.md` — "Gold and marts" section.

**Decisions:**
- `merge_key: [id]` (frontmatter), not top-level `unique_key:`, on `events_enriched` —
  preserves `grain: partition` (see `ValueEnrichedRecipe::model_file`'s precedent).
- `gold.repo_dim`'s SQL is a plain `GROUP BY repo_id` aggregate, not a self-join of two
  CTEs — the latter shape resolved to no classifiable `OutputDelta` at all.
- Dated log entries added to `outcome.md` for both findings below.

**For the next planner (findings, not fixed here):**
- **No maintenance cell is derived for `gold.repo_dim`'s mutation sensitivity at all.**
  `smelt explain gold.events_enriched --json` shows a `RepairKeysNotDiscoverable` refusal,
  not an `UpstreamMutation(gold.repo_dim)` cell with some technique. Root cause: a
  `grain: partition` downstream reading a **clockless** upstream model has no reachable
  route in `append_model_edge_cells` — the key-addressed route needs the downstream's own
  declared `unique_key`, which `grain: partition` has none of by construction, and the
  clock-based route needs the upstream to declare `timeseries:`, which `gold.repo_dim`
  doesn't. Concretely: renaming a repo does not refresh `current_repo_name` on
  `gold.events_enriched`'s already-written rows through any tracked technique, and this is
  silent (only visible via `explain`, not `run`). Characterised by
  `events_enriched_dimension_mutation_cell_technique`. **Criterion-8 finding for
  `docs/outcomes/20260906-bigquery-correctness`** — likely needs either a new route in
  `append_model_edge_cells` for this combination, or a documented refusal surfaced at
  `run`/`build` time rather than only `explain`.
- The sample's measured skew moved since the 2026-09-07 log entry (527 → 1,750 event top
  repo); not load-bearing anywhere before this phase, corrected in `README.md` and the test.
- Everything else in the phase plan built cleanly with no other divergence — all four new
  models' incremental replay matches full-refresh exactly (unlike the succession models'
  known 139/145-row gap from phase 3).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-cli --test github_activity_replay` — 16 passed
- `cargo test -p smelt-cli --test example_diagnostics` — 125 passed, 1 ignored
- `cargo test -p smelt-lsp --test example_workspaces github_activity` — 1 passed
- `python3 examples/github_activity/run_incremental.py` — 30-day replay green, all 12
  models build
