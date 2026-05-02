---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    SUM(quantity) AS agg_0,
    SUM(revenue) AS agg_1,
    AVG(amount) AS agg_2
FROM smelt.models.sql_l1_87
GROUP BY created_at

