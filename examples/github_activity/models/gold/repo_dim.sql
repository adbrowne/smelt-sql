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
-- `gold/events_enriched.sql`'s header comment for the actual finding
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/04-summary.md`):
-- the refusal traces to `events_enriched`'s own `grain: partition`, not to
-- anything about how this model's shape is derived.
SELECT
    repo_id,
    MAX(repo_name) FILTER (WHERE is_current) AS current_repo_name,
    MIN(created_at) AS first_seen_at
FROM smelt.silver.repo_naming
GROUP BY repo_id
