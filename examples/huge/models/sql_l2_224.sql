---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT duration_seconds, amount, transaction_id, 'source_0' AS source_tag FROM smelt.sql_l1_125
UNION ALL
SELECT duration_seconds, amount, transaction_id, 'source_1' AS source_tag FROM smelt.sql_l1_114
