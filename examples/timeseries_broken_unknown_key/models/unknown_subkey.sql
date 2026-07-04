---
materialization: table
timeseries:
  event_time_column: event_time
  partition_columm: event_date
  granularity: day
refresh: batched
---
-- BUG-025 regression: typo'd `partition_columm` (double m) is an unknown
-- timeseries sub-key and must emit MalformedTimeseries, not silently accept it.
SELECT event_time, event_date FROM events
