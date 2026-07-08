---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT session_id, tier, created_at
    FROM smelt.sql_l2_208
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT session_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY session_id
)
SELECT
    a.session_id,
    a.cnt,
    f.tier
FROM aggregated a
INNER JOIN filtered f ON a.session_id = f.session_id
