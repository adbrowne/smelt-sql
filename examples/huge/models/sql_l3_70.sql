---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    SUM(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1
FROM smelt.ref('py_l2_274')
GROUP BY updated_at
