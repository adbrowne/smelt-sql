---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT category, duration_seconds, user_id, 'source_0' AS source_tag FROM smelt.logs
UNION ALL
SELECT category, duration_seconds, user_id, 'source_1' AS source_tag FROM smelt.logs
