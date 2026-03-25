---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT segment, product_id, order_id
    FROM smelt.ref('categories')
    WHERE platform = 'web'
)
SELECT
    b.segment,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('categories') j ON b.user_id = j.user_id
GROUP BY b.segment
