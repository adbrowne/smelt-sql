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
    region,
    session_id,
    is_verified,
    quantity
FROM smelt.shipments
WHERE score >= 50

