---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    SUM(revenue) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    SUM(quantity) AS agg_2,
    AVG(price) AS agg_3,
    AVG(amount) AS agg_4
FROM smelt.ref('py_l1_458')
GROUP BY event_time
