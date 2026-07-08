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
    channel,
    SUM(amount) AS agg_0,
    AVG(price) AS agg_1,
    COUNT(*) AS agg_2
FROM smelt.page_views
GROUP BY channel
