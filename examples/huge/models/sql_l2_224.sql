---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT duration_seconds, amount, transaction_id, 'source_0' AS source_tag FROM smelt.ref('py_l1_357')
UNION ALL
SELECT duration_seconds, amount, transaction_id, 'source_1' AS source_tag FROM smelt.ref('py_l1_370')
