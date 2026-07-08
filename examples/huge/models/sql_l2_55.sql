---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT updated_at, cost, os_name, 'source_0' AS source_tag FROM smelt.sql_l1_121
UNION ALL
SELECT updated_at, cost, os_name, 'source_1' AS source_tag FROM smelt.sql_l1_231
