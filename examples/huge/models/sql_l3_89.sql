---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    SUM(quantity) AS agg_0,
    AVG(price) AS agg_1,
    SUM(revenue) AS agg_2
FROM smelt.ref('py_l2_341')
GROUP BY amount
