---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT channel, transaction_id, device_type, 'source_0' AS source_tag FROM smelt.sql_l1_180
UNION ALL
SELECT channel, transaction_id, device_type, 'source_1' AS source_tag FROM smelt.sql_l1_180

