---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT updated_at, product_id, user_id
    FROM smelt.ref('py_l2_373')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.updated_at,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_171') j ON b.user_id = j.user_id
GROUP BY b.updated_at
