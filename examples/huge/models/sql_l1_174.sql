---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT price, cost, profit
    FROM smelt.ref('users')
    WHERE is_active = true
)
SELECT
    b.price,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('users') j ON b.user_id = j.user_id
GROUP BY b.price
