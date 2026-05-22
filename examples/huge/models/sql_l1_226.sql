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
    event_date,
    COUNT(*) AS agg_0,
    AVG(amount) AS agg_1,
    SUM(amount) AS agg_2,
    AVG(duration_seconds) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.page_views
GROUP BY event_date
