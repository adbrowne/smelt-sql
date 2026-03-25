---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT category, is_active, session_id
    FROM smelt.ref('py_l3_485')
    WHERE event_type = 'purchase'
)
SELECT
    b.category,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_239') j ON b.user_id = j.user_id
GROUP BY b.category
