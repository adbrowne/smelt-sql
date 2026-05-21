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
    amount,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1
FROM smelt.sql_l2_206
GROUP BY amount

