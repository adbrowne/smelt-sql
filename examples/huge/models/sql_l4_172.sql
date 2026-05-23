---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    AVG(amount) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    COUNT(*) AS agg_2,
    MAX(created_at) AS agg_3
FROM smelt.sql_l3_63
GROUP BY profit
