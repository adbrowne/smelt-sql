---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT segment, is_active, transaction_id, 'source_0' AS source_tag FROM smelt.sql_l2_232
UNION ALL
SELECT segment, is_active, transaction_id, 'source_1' AS source_tag FROM smelt.sql_l2_77
