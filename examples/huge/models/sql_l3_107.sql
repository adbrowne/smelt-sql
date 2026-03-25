---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT product_id, platform, device_type, 'source_0' AS source_tag FROM smelt.ref('sql_l2_21')
UNION ALL
SELECT product_id, platform, device_type, 'source_1' AS source_tag FROM smelt.ref('py_l2_333')
UNION ALL
SELECT product_id, platform, device_type, 'source_2' AS source_tag FROM smelt.ref('py_l2_443')
