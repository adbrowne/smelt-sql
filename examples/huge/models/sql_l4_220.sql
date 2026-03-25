---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT price, discount, amount
    FROM smelt.ref('sql_l3_19')
    WHERE event_type = 'purchase'
)
SELECT
    b.price,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_19') j ON b.user_id = j.user_id
GROUP BY b.price
