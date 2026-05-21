---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT profit, revenue, user_id, 'source_0' AS source_tag FROM smelt.sql_l1_160
UNION ALL
SELECT profit, revenue, user_id, 'source_1' AS source_tag FROM smelt.sql_l1_160

