---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_time, transaction_id, product_id
    FROM smelt.ref('sql_l3_210')
    WHERE amount > 0
)
SELECT
    b.event_time,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_210') j ON b.user_id = j.user_id
GROUP BY b.event_time
