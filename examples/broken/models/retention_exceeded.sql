---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Reaches 30 days back into `retention_exceeded_events`, which retains only
-- 7 — refuses with `SourceRetentionExceeded`
-- (`docs/specs/model_properties.md` §"Reach versus retained history").
SELECT
    event_date,
    COUNT(*) AS n
FROM smelt.sources.retention_exceeded_events
WHERE event_date >= CURRENT_DATE - INTERVAL '30 days'
GROUP BY event_date
