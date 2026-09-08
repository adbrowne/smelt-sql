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
-- `gold.repo_dim` is an upstream MODEL, not a declared source. Its own
-- key-addressed route (`append_model_edge_cells`'s two discovery legs) still
-- cannot admit here — both need the DOWNSTREAM's own declared `unique_key`,
-- which a `grain: partition` output has none of by construction — but the
-- **enrichment-keyed** route does: the `LEFT JOIN` above matches
-- `gold.repo_dim`'s own declared `unique_key` (`repo_id`), `current_repo_name`
-- is read only as a SELECTed payload column (never in the `ON` predicate, so
-- it is pure value-enrichment, not row-admission-sensitive), and the
-- `allow_full_scan: true` above accepts the resulting full-table merge
-- (`docs/specs/incremental_models.md` §"Upstream model edges"). The derived
-- `{current_repo_name}` `UpstreamMutation(gold.repo_dim)` cell resolves to
-- `Technique::ColumnScopedMerge`, addressed by `repo_id` — characterised via
-- `smelt explain gold.events_enriched --json` in
-- `events_enriched_dimension_mutation_cell_technique`
-- (`crates/smelt-cli/tests/github_activity_replay.rs`). Making that cell
-- live on the run path (so a rename in `silver.repo_naming` actually heals
-- this table's already-written rows) is phase 5's own scope
-- (`docs/outcomes/20260906-bigquery-correctness/phases/04-plan.md`).
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
