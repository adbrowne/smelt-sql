---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    AVG(price) AS agg_0,
    MAX(created_at) AS agg_1,
    SUM(revenue) AS agg_2,
    MIN(created_at) AS agg_3,
    COUNT(DISTINCT user_id) AS agg_4
FROM smelt.ref('py_l3_467')
GROUP BY discount
