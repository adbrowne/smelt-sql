---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT event_type, updated_at, event_time, 'source_0' AS source_tag FROM smelt.errors
UNION ALL
SELECT event_type, updated_at, event_time, 'source_1' AS source_tag FROM smelt.errors
