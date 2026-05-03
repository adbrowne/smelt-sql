---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT tier, cost, created_at, 'source_0' AS source_tag FROM smelt.sql_l1_160
UNION ALL
SELECT tier, cost, created_at, 'source_1' AS source_tag FROM smelt.sql_l1_160

