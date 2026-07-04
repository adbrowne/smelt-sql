---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT category, event_time, order_id
    FROM smelt.sql_l1_101
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.category,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_101 j ON b.user_id = j.user_id
GROUP BY b.category
