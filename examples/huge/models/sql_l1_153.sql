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
    a.updated_at,
    a.event_type,
    b.quantity
FROM smelt.shipments a
LEFT JOIN smelt.shipments b ON a.user_id = b.user_id
