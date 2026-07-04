---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    COUNT(DISTINCT user_id) AS agg_0,
    MAX(created_at) AS agg_1,
    SUM(revenue) AS agg_2
FROM smelt.reviews
GROUP BY category
