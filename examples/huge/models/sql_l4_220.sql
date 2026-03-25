---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT price, discount, amount
    FROM smelt.ref('sql_l3_64')
    WHERE event_type = 'purchase'
)
SELECT
    b.price,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_203') j ON b.user_id = j.user_id
GROUP BY b.price
