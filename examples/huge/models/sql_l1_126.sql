---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT browser, rating, event_date, 'source_0' AS source_tag FROM smelt.notifications
UNION ALL
SELECT browser, rating, event_date, 'source_1' AS source_tag FROM smelt.notifications
