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
    a.product_id,
    a.os_name,
    b.discount
FROM smelt.clicks a
INNER JOIN smelt.clicks b ON a.user_id = b.user_id
