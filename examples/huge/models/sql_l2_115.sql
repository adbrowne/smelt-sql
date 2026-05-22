---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT page_path, region, cohort_date, 'source_0' AS source_tag FROM smelt.sql_l1_32
UNION ALL
SELECT page_path, region, cohort_date, 'source_1' AS source_tag FROM smelt.sql_l1_198
