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
    a.tier,
    a.revenue,
    b.channel
FROM smelt.subscriptions a
INNER JOIN smelt.subscriptions b ON a.user_id = b.user_id
