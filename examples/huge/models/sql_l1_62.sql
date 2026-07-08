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
    category,
    AVG(duration_seconds) AS agg_0,
    AVG(amount) AS agg_1,
    MIN(created_at) AS agg_2,
    COUNT(*) AS agg_3
FROM smelt.users
GROUP BY category
