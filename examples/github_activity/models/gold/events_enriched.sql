---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
merge_key: [id]
maintenance:
  scan_bounds:
    per_source:
      gold.repo_dim:
        allow_full_scan: true
---
-- Every deduped event, enriched with the repo's CURRENT name from
-- `gold.repo_dim` — the `LEFT JOIN`-against-a-`unique_key`-declaring-
-- dimension shape the outcome names as its `ColumnScopedMerge` instance
-- (`ValueEnrichedRecipe` in `crates/smelt-maintenance-testkit/src/
-- recipe.rs`). `current_repo_name` is a SELECTed payload column, not merely
-- read in the `LEFT JOIN`'s own `ON` predicate — the precondition that
-- shape's `{current_repo_name}` cell needs to be value-only rather than
-- row-admission-sensitive.
--
-- `gold.repo_dim` is an upstream MODEL, not a declared source, so whether
-- its mutation-sensitivity facts drive the same technique
-- `ValueEnrichedRecipe` proves over a declared source is not established
-- anywhere in the tree — measured via `smelt explain gold.events_enriched
-- --json`, not assumed: see `events_enriched_dimension_mutation_cell_
-- technique` in `crates/smelt-cli/tests/github_activity_replay.rs`. Measured
-- verdict, recorded rather than fixed (criterion 8 finding,
-- `docs/outcomes/20260906-bigquery-dogfood-spine/phases/04-summary.md`): NO
-- `UpstreamMutation(gold.repo_dim)` cell is derived at all —
-- `append_model_edge_cells`'s key-addressed route (the only route open to a
-- clockless upstream feeding a `grain: partition` downstream; the
-- clock-based route needs `edge.clock_col.is_some()`, which `gold.repo_dim`
-- never satisfies) requires the DOWNSTREAM's own declared `unique_key` to
-- scope the recompute, and a `grain: partition` model has none by
-- construction — so `admit_key_addressed_recompute` refuses with
-- `RepairKeysNotDiscoverable { source: "gold.repo_dim", why: "model has no
-- proven grain and no declared unique key" }`. Concretely: renaming a repo
-- in `silver.repo_naming` today does NOT re-derive `current_repo_name` on
-- this table's already-written rows through any tracked maintenance cell.
--
-- `merge_key: [id]` (not top-level `unique_key:`) is the write/dedup-only
-- spelling: a top-level `unique_key:` would flip the derived grain to
-- `Key`/`KeyPerPartition`, contradicting the `grain: partition` this model
-- asserts (`ValueEnrichedRecipe::model_file`'s own doc comment).
SELECT
    f.id,
    f.type,
    f.actor_id,
    f.actor_login,
    f.repo_id,
    f.repo_name,
    f.org_id,
    f.public,
    f.created_at,
    f.event_date,
    dim.current_repo_name
FROM smelt.silver.events_deduped f
LEFT JOIN smelt.gold.repo_dim dim ON f.repo_id = dim.repo_id
