---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT event_time, status, user_id, 'source_0' AS source_tag FROM smelt.refunds
UNION ALL
SELECT event_time, status, user_id, 'source_1' AS source_tag FROM smelt.refunds
