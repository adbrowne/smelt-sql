---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT category, event_type, platform, 'source_0' AS source_tag FROM smelt.sql_l1_171
UNION ALL
SELECT category, event_type, platform, 'source_1' AS source_tag FROM smelt.sql_l1_77
