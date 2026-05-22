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
    category,
    SUM(quantity) AS agg_0,
    COUNT(*) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    AVG(duration_seconds) AS agg_3
FROM smelt.events
GROUP BY category
