---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT browser, plan_type, amount, 'source_0' AS source_tag FROM smelt.sql_l3_66
UNION ALL
SELECT browser, plan_type, amount, 'source_1' AS source_tag FROM smelt.sql_l3_111

