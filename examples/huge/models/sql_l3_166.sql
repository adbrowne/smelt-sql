---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT browser, rating, updated_at, 'source_0' AS source_tag FROM smelt.ref('py_l2_474')
UNION ALL
SELECT browser, rating, updated_at, 'source_1' AS source_tag FROM smelt.ref('py_l2_404')
UNION ALL
SELECT browser, rating, updated_at, 'source_2' AS source_tag FROM smelt.ref('py_l2_300')
