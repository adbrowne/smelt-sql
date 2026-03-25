---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT region, user_id, referrer
    FROM smelt.ref('sql_l2_220')
    WHERE is_active = true
)
SELECT
    b.region,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_429') j ON b.user_id = j.user_id
GROUP BY b.region
