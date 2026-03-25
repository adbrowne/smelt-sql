---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT channel, transaction_id, device_type, 'source_0' AS source_tag FROM smelt.ref('py_l1_410')
UNION ALL
SELECT channel, transaction_id, device_type, 'source_1' AS source_tag FROM smelt.ref('py_l1_313')
UNION ALL
SELECT channel, transaction_id, device_type, 'source_2' AS source_tag FROM smelt.ref('py_l1_287')
