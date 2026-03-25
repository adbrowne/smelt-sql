---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT plan_type, score, is_verified
    FROM smelt.ref('sql_l2_2')
    WHERE event_type = 'purchase'
)
SELECT
    b.plan_type,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_315') j ON b.user_id = j.user_id
GROUP BY b.plan_type
