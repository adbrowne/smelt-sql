---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT is_active, channel, platform, 'source_0' AS source_tag FROM smelt.sql_l3_131
UNION ALL
SELECT is_active, channel, platform, 'source_1' AS source_tag FROM smelt.sql_l3_131
