---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT os_name, revenue, is_active, 'source_0' AS source_tag FROM smelt.sessions
UNION ALL
SELECT os_name, revenue, is_active, 'source_1' AS source_tag FROM smelt.sessions

