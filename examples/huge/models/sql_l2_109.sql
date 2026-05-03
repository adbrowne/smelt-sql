---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT segment, updated_at, is_verified, 'source_0' AS source_tag FROM smelt.sql_l1_57
UNION ALL
SELECT segment, updated_at, is_verified, 'source_1' AS source_tag FROM smelt.sql_l1_214

