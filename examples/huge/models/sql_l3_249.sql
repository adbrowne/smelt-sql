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
    SELECT updated_at, user_id, is_active
    FROM smelt.sql_l2_28
    WHERE quantity > 0
),
aggregated AS (
    SELECT updated_at, COUNT(*) AS cnt
    FROM filtered
    GROUP BY updated_at
)
SELECT
    a.updated_at,
    a.cnt,
    f.user_id
FROM aggregated a
INNER JOIN filtered f ON a.updated_at = f.updated_at
