---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_time,
    SUM(revenue) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    SUM(quantity) AS agg_2,
    AVG(price) AS agg_3,
    AVG(amount) AS agg_4
FROM smelt.ref('sql_l1_204')
GROUP BY event_time
