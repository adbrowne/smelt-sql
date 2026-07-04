---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT amount, profit, cohort_date, 'source_0' AS source_tag FROM smelt.errors
UNION ALL
SELECT amount, profit, cohort_date, 'source_1' AS source_tag FROM smelt.errors
