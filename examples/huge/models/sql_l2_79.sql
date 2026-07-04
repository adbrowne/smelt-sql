---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT os_name, revenue, event_time, 'source_0' AS source_tag FROM smelt.sql_l1_77
UNION ALL
SELECT os_name, revenue, event_time, 'source_1' AS source_tag FROM smelt.sql_l1_106
UNION ALL
SELECT os_name, revenue, event_time, 'source_2' AS source_tag FROM smelt.sql_l1_65
