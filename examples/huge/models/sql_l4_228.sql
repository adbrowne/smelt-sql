---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    AVG(amount) AS agg_0,
    SUM(quantity) AS agg_1,
    SUM(amount) AS agg_2
FROM smelt.ref('py_l3_486')
GROUP BY amount
