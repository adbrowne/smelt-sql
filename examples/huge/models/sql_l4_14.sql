---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT cost, created_at, ip_address
    FROM smelt.sql_l3_241
    WHERE is_active = true
)
SELECT
    b.cost,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_131 j ON b.user_id = j.user_id
GROUP BY b.cost

