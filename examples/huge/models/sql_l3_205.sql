---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT duration_seconds, os_name, category, 'source_0' AS source_tag FROM smelt.ref('sql_l2_246')
UNION ALL
SELECT duration_seconds, os_name, category, 'source_1' AS source_tag FROM smelt.ref('sql_l2_237')
