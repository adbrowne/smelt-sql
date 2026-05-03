---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT category, status, os_name, 'source_0' AS source_tag FROM smelt.sql_l2_248
UNION ALL
SELECT category, status, os_name, 'source_1' AS source_tag FROM smelt.sql_l2_88

