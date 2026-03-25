---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT user_id, amount, channel, 'source_0' AS source_tag FROM smelt.ref('py_l1_369')
UNION ALL
SELECT user_id, amount, channel, 'source_1' AS source_tag FROM smelt.ref('py_l1_380')
