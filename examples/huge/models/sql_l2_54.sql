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
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(*) AS agg_2
FROM smelt.models.sql_l1_17
GROUP BY profit

