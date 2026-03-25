---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT amount, transaction_id, device_type
    FROM smelt.ref('sql_l1_91')
    WHERE amount > 0
)
SELECT
    b.amount,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_120') j ON b.user_id = j.user_id
GROUP BY b.amount
