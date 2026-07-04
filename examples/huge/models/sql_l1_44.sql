---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    AVG(price) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(amount) AS agg_2,
    COUNT(*) AS agg_3
FROM smelt.categories
GROUP BY is_verified
