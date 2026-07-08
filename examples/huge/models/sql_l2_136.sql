---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT user_id, status, updated_at
    FROM smelt.sql_l1_77
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.user_id,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_158 j ON b.user_id = j.user_id
GROUP BY b.user_id
