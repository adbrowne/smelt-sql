---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT browser, transaction_id, event_type, 'source_0' AS source_tag FROM smelt.ref('sql_l1_209')
UNION ALL
SELECT browser, transaction_id, event_type, 'source_1' AS source_tag FROM smelt.ref('sql_l1_209')
