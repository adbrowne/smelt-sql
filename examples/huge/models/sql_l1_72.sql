---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT product_id, rating, updated_at
    FROM smelt.logs
    WHERE quantity > 0
)
SELECT
    b.product_id,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.logs j ON b.user_id = j.user_id
GROUP BY b.product_id

