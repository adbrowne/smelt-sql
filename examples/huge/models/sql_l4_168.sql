---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT ip_address, order_id, platform, 'source_0' AS source_tag FROM smelt.ref('py_l3_481')
UNION ALL
SELECT ip_address, order_id, platform, 'source_1' AS source_tag FROM smelt.ref('py_l3_478')
UNION ALL
SELECT ip_address, order_id, platform, 'source_2' AS source_tag FROM smelt.ref('py_l3_402')
