---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT transaction_id, device_type, discount, 'source_0' AS source_tag FROM smelt.events
UNION ALL
SELECT transaction_id, device_type, discount, 'source_1' AS source_tag FROM smelt.events

