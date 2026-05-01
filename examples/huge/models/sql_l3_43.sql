---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT cost, order_id, browser
    FROM smelt.models.sql_l2_159
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.cost,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l2_79 j ON b.user_id = j.user_id
GROUP BY b.cost

