---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT updated_at, transaction_id, revenue, 'source_0' AS source_tag FROM smelt.ref('sql_l3_225')
UNION ALL
SELECT updated_at, transaction_id, revenue, 'source_1' AS source_tag FROM smelt.ref('py_l3_454')
UNION ALL
SELECT updated_at, transaction_id, revenue, 'source_2' AS source_tag FROM smelt.ref('py_l3_451')
