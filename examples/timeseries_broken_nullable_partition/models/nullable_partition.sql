---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: partition_date
  granularity: day
---
-- D-52 rule 7: partition_column must be NOT NULL.
-- A CASE without ELSE has an implicit ELSE NULL, so partition_date is nullable.
-- This must emit MalformedTimeseries.
SELECT CASE WHEN event_ts > '2020-01-01' THEN DATE '2020-01-01' END AS partition_date, event_ts FROM events
