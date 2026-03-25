---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT plan_type, created_at, transaction_id, 'source_0' AS source_tag FROM smelt.ref('py_l1_410')
UNION ALL
SELECT plan_type, created_at, transaction_id, 'source_1' AS source_tag FROM smelt.ref('py_l1_297')
