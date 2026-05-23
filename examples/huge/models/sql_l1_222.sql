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
    referrer,
    SUM(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    MIN(created_at) AS agg_2,
    COUNT(*) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.orders
GROUP BY referrer
