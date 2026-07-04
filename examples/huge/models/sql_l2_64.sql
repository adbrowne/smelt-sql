---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT category, product_id, rating, 'source_0' AS source_tag FROM smelt.sql_l1_110
UNION ALL
SELECT category, product_id, rating, 'source_1' AS source_tag FROM smelt.sql_l1_120
UNION ALL
SELECT category, product_id, rating, 'source_2' AS source_tag FROM smelt.sql_l1_147
