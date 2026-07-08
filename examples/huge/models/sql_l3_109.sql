---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT amount, category, rating, 'source_0' AS source_tag FROM smelt.sql_l2_22
UNION ALL
SELECT amount, category, rating, 'source_1' AS source_tag FROM smelt.sql_l2_14
UNION ALL
SELECT amount, category, rating, 'source_2' AS source_tag FROM smelt.sql_l2_149
