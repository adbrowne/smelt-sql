---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT tier, quantity, order_id, 'source_0' AS source_tag FROM smelt.ref('sql_l3_17')
UNION ALL
SELECT tier, quantity, order_id, 'source_1' AS source_tag FROM smelt.ref('sql_l3_17')
