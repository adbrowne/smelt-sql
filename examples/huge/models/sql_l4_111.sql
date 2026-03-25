---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT amount, segment, os_name
    FROM smelt.ref('py_l3_301')
    WHERE platform = 'web'
)
SELECT
    b.amount,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_433') j ON b.user_id = j.user_id
GROUP BY b.amount
