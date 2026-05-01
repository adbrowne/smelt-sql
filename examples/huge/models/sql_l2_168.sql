---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT duration_seconds, quantity, status, 'source_0' AS source_tag FROM smelt.models.sql_l1_21
UNION ALL
SELECT duration_seconds, quantity, status, 'source_1' AS source_tag FROM smelt.models.sql_l1_74

