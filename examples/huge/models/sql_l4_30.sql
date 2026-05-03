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
    COUNT(DISTINCT user_id) AS agg_1,
    MAX(created_at) AS agg_2,
    SUM(amount) AS agg_3
FROM smelt.sql_l3_43
GROUP BY created_at

