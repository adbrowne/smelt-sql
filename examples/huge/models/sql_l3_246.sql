---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT updated_at, product_id, user_id
    FROM smelt.sql_l2_12
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.updated_at,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_173 j ON b.user_id = j.user_id
GROUP BY b.updated_at

