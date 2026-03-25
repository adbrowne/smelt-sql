---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT amount, ip_address, segment
    FROM smelt.ref('sql_l3_147')
    WHERE platform = 'web'
)
SELECT
    b.amount,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_147') j ON b.user_id = j.user_id
GROUP BY b.amount
