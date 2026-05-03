---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT product_id, platform, device_type, 'source_0' AS source_tag FROM smelt.sql_l2_92
UNION ALL
SELECT product_id, platform, device_type, 'source_1' AS source_tag FROM smelt.sql_l2_147
UNION ALL
SELECT product_id, platform, device_type, 'source_2' AS source_tag FROM smelt.sql_l2_176

