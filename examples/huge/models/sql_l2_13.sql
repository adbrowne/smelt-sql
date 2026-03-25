---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT referrer, amount, os_name
    FROM smelt.ref('py_l1_417')
    WHERE is_active = true
)
SELECT
    b.referrer,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_204') j ON b.user_id = j.user_id
GROUP BY b.referrer
