---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT browser, device_type, is_verified, 'source_0' AS source_tag FROM smelt.sql_l3_211
UNION ALL
SELECT browser, device_type, is_verified, 'source_1' AS source_tag FROM smelt.sql_l3_139
