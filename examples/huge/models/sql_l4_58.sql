---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    AVG(amount) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    AVG(price) AS agg_2,
    SUM(revenue) AS agg_3,
    SUM(amount) AS agg_4
FROM smelt.ref('py_l3_265')
GROUP BY profit
