---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT is_active, duration_seconds, status
    FROM smelt.ref('py_l2_265')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.is_active,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_315') j ON b.user_id = j.user_id
GROUP BY b.is_active
