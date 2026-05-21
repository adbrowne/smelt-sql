---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT channel, plan_type, event_time, 'source_0' AS source_tag FROM smelt.sql_l3_109
UNION ALL
SELECT channel, plan_type, event_time, 'source_1' AS source_tag FROM smelt.sql_l3_101

