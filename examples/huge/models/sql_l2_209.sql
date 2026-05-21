---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT rating, score, amount, 'source_0' AS source_tag FROM smelt.sql_l1_135
UNION ALL
SELECT rating, score, amount, 'source_1' AS source_tag FROM smelt.sql_l1_135

