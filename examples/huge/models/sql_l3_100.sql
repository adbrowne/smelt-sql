---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT browser, session_id, quantity, 'source_0' AS source_tag FROM smelt.sql_l2_232
UNION ALL
SELECT browser, session_id, quantity, 'source_1' AS source_tag FROM smelt.sql_l2_88

