---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT product_id, rating, updated_at
    FROM smelt.ref('logs')
    WHERE quantity > 0
)
SELECT
    b.product_id,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('logs') j ON b.user_id = j.user_id
GROUP BY b.product_id
