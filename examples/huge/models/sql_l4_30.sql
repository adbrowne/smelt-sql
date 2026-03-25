---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    SUM(quantity) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    MAX(created_at) AS agg_2,
    SUM(amount) AS agg_3
FROM smelt.ref('py_l3_291')
GROUP BY created_at
