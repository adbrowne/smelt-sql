---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT quantity, channel, session_id, 'source_0' AS source_tag FROM smelt.ref('sql_l2_223')
UNION ALL
SELECT quantity, channel, session_id, 'source_1' AS source_tag FROM smelt.ref('sql_l2_4')
UNION ALL
SELECT quantity, channel, session_id, 'source_2' AS source_tag FROM smelt.ref('sql_l2_125')
