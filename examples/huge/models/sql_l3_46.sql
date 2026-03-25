---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    AVG(amount) AS agg_0,
    SUM(revenue) AS agg_1,
    SUM(amount) AS agg_2
FROM smelt.ref('py_l2_430')
GROUP BY order_id
