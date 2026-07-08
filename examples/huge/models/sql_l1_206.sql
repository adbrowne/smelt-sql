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
    referrer,
    SUM(amount) AS agg_0,
    MAX(created_at) AS agg_1
FROM smelt.reviews
GROUP BY referrer
