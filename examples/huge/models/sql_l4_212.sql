---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT profit, ip_address, event_date, 'source_0' AS source_tag FROM smelt.sql_l3_161
UNION ALL
SELECT profit, ip_address, event_date, 'source_1' AS source_tag FROM smelt.sql_l3_145
UNION ALL
SELECT profit, ip_address, event_date, 'source_2' AS source_tag FROM smelt.sql_l3_153
