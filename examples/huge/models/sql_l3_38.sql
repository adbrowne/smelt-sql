---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT discount, session_id, page_path
    FROM smelt.ref('py_l2_350')
    WHERE status = 'active'
)
SELECT
    b.discount,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_351') j ON b.user_id = j.user_id
GROUP BY b.discount
