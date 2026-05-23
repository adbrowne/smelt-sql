---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT score, status, price, 'source_0' AS source_tag FROM smelt.sql_l1_73
UNION ALL
SELECT score, status, price, 'source_1' AS source_tag FROM smelt.sql_l1_237
UNION ALL
SELECT score, status, price, 'source_2' AS source_tag FROM smelt.sql_l1_131
