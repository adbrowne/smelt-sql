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
    page_path,
    MAX(created_at) AS agg_0,
    SUM(amount) AS agg_1
FROM smelt.sql_l3_53
GROUP BY page_path
