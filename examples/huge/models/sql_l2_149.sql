---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT category, event_time, order_id
    FROM smelt.ref('py_l1_299')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.category,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_29') j ON b.user_id = j.user_id
GROUP BY b.category
