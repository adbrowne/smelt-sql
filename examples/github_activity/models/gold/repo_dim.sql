---
materialization: table
refresh: incremental
unique_key: [repo_id]
maintenance:
  scan_bounds:
    per_source:
      silver.repo_naming:
        allow_full_scan: true
---
-- One row per repo, current name from `silver.repo_naming`'s `is_current`
-- flag. Exists to be `events_enriched`'s keyed dimension — the payload-
-- independent widening's `LEFT JOIN`-against-a-`unique_key`-declaring-
-- dimension shape (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/
-- 04-plan.md`). `manual.repo_watchlist` (the research doc's original,
-- hand-authored motivation for this shape) lost its reason to exist once
-- `docs/TODO.md` recorded `Technique::ColumnScopedMerge` as reachable via
-- `ValueEnrichedRecipe`; this is the same shape occurring on a real
-- pipeline instead.
--
-- No `timeseries:` and no clock: `unique_key` alone derives `grain: key`
-- (`smelt_core::config::derive_grain`), so `silver.repo_naming` — itself
-- keyed on `(repo_id, created_at)`, not time-partitioned in the sense this
-- model would key off of — is read in full each run rather than windowed.
--
-- A single `GROUP BY repo_id` aggregate, not a self-join of two CTEs — this
-- model's OWN `delta_signature` (`smelt explain gold.repo_dim --json`) is
-- `keyed_upsert` over `["repo_id"]` either way, but the plain aggregate
-- form is kept for legibility. This model's own classification is not the
-- one that matters for `events_enriched`'s enrichment cell, though — see
-- `gold/events_enriched.sql`'s header comment for how the enrichment-keyed
-- route (`docs/specs/incremental_models.md` §"Upstream model edges")
-- addresses that cell by this model's own declared `unique_key`
-- (`repo_id`), not by its `delta_signature`.
-- `MAX(CASE WHEN … END)` rather than `MAX(…) FILTER (WHERE is_current)`: the
-- two are exactly equivalent for a NULL-ignoring aggregate, and GoogleSQL has
-- no aggregate `FILTER` clause at all, so the `FILTER` spelling is refused at
-- compile time on the `bigquery` target
-- (`SqlDialect::supports_aggregate_filter_clause`).
SELECT
    repo_id,
    MAX(CASE WHEN is_current THEN repo_name END) AS current_repo_name,
    MIN(created_at) AS first_seen_at
FROM smelt.silver.repo_naming
GROUP BY repo_id
