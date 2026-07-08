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
    a.event_type,
    a.profit,
    b.is_verified
FROM smelt.page_views a
INNER JOIN smelt.page_views b ON a.user_id = b.user_id
