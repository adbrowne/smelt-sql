---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT segment, is_active, transaction_id, 'source_0' AS source_tag FROM smelt.ref('sql_l2_231')
UNION ALL
SELECT segment, is_active, transaction_id, 'source_1' AS source_tag FROM smelt.ref('sql_l2_200')
