---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    page_path,
    COUNT(DISTINCT user_id) AS agg_0,
    MAX(created_at) AS agg_1,
    MIN(created_at) AS agg_2
FROM smelt.sql_l1_180
GROUP BY page_path
