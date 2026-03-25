---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT session_id, category, segment, 'source_0' AS source_tag FROM smelt.ref('sql_l3_18')
UNION ALL
SELECT session_id, category, segment, 'source_1' AS source_tag FROM smelt.ref('sql_l3_237')
UNION ALL
SELECT session_id, category, segment, 'source_2' AS source_tag FROM smelt.ref('sql_l3_81')
