---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT session_id, browser, revenue
    FROM smelt.ref('py_l2_317')
    WHERE quantity > 0
)
SELECT
    b.session_id,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_251') j ON b.user_id = j.user_id
GROUP BY b.session_id
