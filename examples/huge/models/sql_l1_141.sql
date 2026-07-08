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
    a.referrer,
    b.user_id,
    c.is_verified,
    c.updated_at
FROM smelt.categories a
INNER JOIN smelt.categories b ON a.user_id = b.user_id
LEFT JOIN smelt.categories c ON a.user_id = c.user_id
