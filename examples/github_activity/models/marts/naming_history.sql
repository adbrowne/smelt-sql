---
materialization: table
refresh: full
---
-- The actual renames, derived downstream of the two per-event succession
-- models (`silver.repo_naming`, `silver.actor_naming`): the succession
-- classifier admits exactly one row-local pre-window filter, so "only the
-- rows where the name changed" — which needs `LAG` — cannot live in either
-- silver model itself
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/03-plan.md`).
--
-- Full refresh: this mart reads two already-maintained succession outputs
-- rather than a raw event stream, so there is no incremental window to
-- reason about here.
WITH entity_naming AS (
    SELECT 'repo' AS entity_kind, repo_id AS entity_id, repo_name AS name, created_at
    FROM smelt.silver.repo_naming
    UNION ALL
    SELECT 'actor' AS entity_kind, actor_id AS entity_id, actor_login AS name, created_at
    FROM smelt.silver.actor_naming
),
with_prior_name AS (
    SELECT
        entity_kind,
        entity_id,
        name,
        created_at,
        LAG(name) OVER (
            PARTITION BY entity_kind, entity_id ORDER BY created_at
        ) AS prior_name
    FROM entity_naming
)
SELECT
    entity_kind,
    entity_id,
    prior_name AS from_name,
    name AS to_name,
    created_at AS renamed_at
FROM with_prior_name
WHERE prior_name IS NOT NULL
  AND prior_name != name
