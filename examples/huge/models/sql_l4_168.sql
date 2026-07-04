---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT ip_address, order_id, platform, 'source_0' AS source_tag FROM smelt.sql_l3_111
UNION ALL
SELECT ip_address, order_id, platform, 'source_1' AS source_tag FROM smelt.sql_l3_111
