---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT price, referrer, duration_seconds
    FROM smelt.ref('sql_l1_96')
    WHERE platform = 'web'
)
SELECT
    b.price,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_96') j ON b.user_id = j.user_id
GROUP BY b.price
