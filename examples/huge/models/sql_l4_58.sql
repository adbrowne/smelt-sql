---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    AVG(amount) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    AVG(price) AS agg_2,
    SUM(revenue) AS agg_3,
    SUM(amount) AS agg_4
FROM smelt.sql_l3_79
GROUP BY profit

