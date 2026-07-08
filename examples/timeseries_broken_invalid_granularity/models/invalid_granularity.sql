---
materialization: table
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: fortnight
refresh: incremental
grain: partition
---
-- BUG-023 regression: `granularity: fortnight` must emit MalformedTimeseries,
-- not silently revert to VIEW with exit 0.
SELECT event_time, event_date FROM events
