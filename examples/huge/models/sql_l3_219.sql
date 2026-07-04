---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT price, segment, os_name, 'source_0' AS source_tag FROM smelt.sql_l2_155
UNION ALL
SELECT price, segment, os_name, 'source_1' AS source_tag FROM smelt.sql_l2_223
UNION ALL
SELECT price, segment, os_name, 'source_2' AS source_tag FROM smelt.sql_l2_8
