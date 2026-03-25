---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT country, created_at, order_id
    FROM smelt.ref('users')
    WHERE event_type = 'purchase'
)
SELECT
    b.country,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('users') j ON b.user_id = j.user_id
GROUP BY b.country
