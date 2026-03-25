---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT platform, rating, price
    FROM smelt.ref('py_l3_364')
    WHERE platform = 'web'
)
SELECT
    b.platform,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_364') j ON b.user_id = j.user_id
GROUP BY b.platform
