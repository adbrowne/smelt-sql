---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT quantity, channel, session_id, 'source_0' AS source_tag FROM smelt.ref('sql_l2_42')
UNION ALL
SELECT quantity, channel, session_id, 'source_1' AS source_tag FROM smelt.ref('sql_l2_175')
UNION ALL
SELECT quantity, channel, session_id, 'source_2' AS source_tag FROM smelt.ref('sql_l2_33')
