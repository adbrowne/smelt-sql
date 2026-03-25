---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT is_verified, plan_type, region
    FROM smelt.ref('py_l1_404')
    WHERE status = 'active'
)
SELECT
    b.is_verified,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_227') j ON b.user_id = j.user_id
GROUP BY b.is_verified
