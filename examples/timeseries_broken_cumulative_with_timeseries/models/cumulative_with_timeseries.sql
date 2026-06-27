---
materialization: table
refresh: cumulative
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT device_id, COUNT(*) AS event_count
FROM some_events
GROUP BY device_id
