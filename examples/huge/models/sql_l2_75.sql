---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT is_verified, created_at, is_active
    FROM smelt.sql_l1_74
    WHERE platform = 'web'
),
aggregated AS (
    SELECT is_verified, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_verified
)
SELECT
    a.is_verified,
    a.cnt,
    f.created_at
FROM aggregated a
INNER JOIN filtered f ON a.is_verified = f.is_verified

