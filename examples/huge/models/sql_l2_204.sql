---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    price,
    AVG(amount) AS agg_0,
    COUNT(*) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.ref('py_l1_460')
GROUP BY price
