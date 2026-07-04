---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT is_active, platform, duration_seconds
    FROM smelt.sql_l1_159
    WHERE platform = 'web'
),
aggregated AS (
    SELECT is_active, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_active
)
SELECT
    a.is_active,
    a.cnt,
    f.platform
FROM aggregated a
INNER JOIN filtered f ON a.is_active = f.is_active
