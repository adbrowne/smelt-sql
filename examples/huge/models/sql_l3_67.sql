---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_type, transaction_id, referrer
    FROM smelt.ref('py_l2_319')
    WHERE quantity > 0
)
SELECT
    b.event_type,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_188') j ON b.user_id = j.user_id
GROUP BY b.event_type
