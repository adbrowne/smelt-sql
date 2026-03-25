---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    order_id,
    AVG(amount) AS agg_0,
    SUM(revenue) AS agg_1,
    SUM(amount) AS agg_2
FROM smelt.ref('sql_l2_55')
GROUP BY order_id
