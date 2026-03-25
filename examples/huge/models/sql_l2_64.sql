---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT category, product_id, rating, 'source_0' AS source_tag FROM smelt.ref('py_l1_351')
UNION ALL
SELECT category, product_id, rating, 'source_1' AS source_tag FROM smelt.ref('sql_l1_27')
