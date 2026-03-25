---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT user_id, status, updated_at
    FROM smelt.ref('sql_l1_84')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.user_id,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l1_306') j ON b.user_id = j.user_id
GROUP BY b.user_id
