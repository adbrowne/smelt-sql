---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_date, revenue, rating
    FROM smelt.ref('products')
    WHERE category IS NOT NULL
)
SELECT
    b.event_date,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('products') j ON b.user_id = j.user_id
GROUP BY b.event_date
