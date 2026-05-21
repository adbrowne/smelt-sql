---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT os_name, price, is_verified, 'source_0' AS source_tag FROM smelt.sql_l2_90
UNION ALL
SELECT os_name, price, is_verified, 'source_1' AS source_tag FROM smelt.sql_l2_140

