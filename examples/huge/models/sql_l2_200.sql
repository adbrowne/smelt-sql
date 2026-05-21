---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT os_name, amount, segment, 'source_0' AS source_tag FROM smelt.sql_l1_174
UNION ALL
SELECT os_name, amount, segment, 'source_1' AS source_tag FROM smelt.sql_l1_3
UNION ALL
SELECT os_name, amount, segment, 'source_2' AS source_tag FROM smelt.sql_l1_144

