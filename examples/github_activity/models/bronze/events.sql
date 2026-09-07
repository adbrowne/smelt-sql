---
materialization: table
timeseries:
  event_time_column: created_at
  partition_column: created_at
  granularity: day
---
-- Bronze passthrough — the source contract made concrete as a named model,
-- so a schema drift in `raw.github_events` (a column rename, a type change)
-- surfaces here as a compile refusal rather than downstream in a model that
-- also has dedup/sessionization logic to debug. No downstream model reads
-- this one: `silver.events_deduped` reads `smelt.sources.raw.github_events`
-- directly (see that model for why), the same shape `silver/events_parsed`
-- keeps in `examples/web_analytics/` for its own coverage after the
-- consuming models moved off it.
SELECT
    id,
    type,
    created_at,
    actor_id,
    actor_login,
    repo_id,
    repo_name,
    org_id,
    public
FROM smelt.sources.raw.github_events
