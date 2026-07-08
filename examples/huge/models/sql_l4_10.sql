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
    SELECT user_id, status, page_path
    FROM smelt.sql_l3_54
    WHERE quantity > 0
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.status
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
