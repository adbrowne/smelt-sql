---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT event_type, transaction_id, updated_at, 'source_0' AS source_tag FROM smelt.ref('sql_l2_101')
UNION ALL
SELECT event_type, transaction_id, updated_at, 'source_1' AS source_tag FROM smelt.ref('py_l2_412')
