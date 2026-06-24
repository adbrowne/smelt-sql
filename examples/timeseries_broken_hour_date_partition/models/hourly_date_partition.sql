---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: partition_date
  granularity: hour
---
-- D-52 rule 8: granularity=hour requires a TIMESTAMP partition column.
-- CAST(event_ts AS DATE) produces a DATE, which cannot represent hour boundaries.
-- This must emit MalformedTimeseries.
SELECT CAST(event_ts AS DATE) AS partition_date, event_ts FROM events
