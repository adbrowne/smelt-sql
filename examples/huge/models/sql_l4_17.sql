---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT discount, user_id, platform, 'source_0' AS source_tag FROM smelt.ref('py_l3_393')
UNION ALL
SELECT discount, user_id, platform, 'source_1' AS source_tag FROM smelt.ref('py_l3_393')
