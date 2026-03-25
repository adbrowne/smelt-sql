---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    amount,
    SUM(quantity) AS agg_0,
    AVG(price) AS agg_1,
    SUM(revenue) AS agg_2
FROM smelt.ref('sql_l2_47')
GROUP BY amount
