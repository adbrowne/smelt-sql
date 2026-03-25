---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT category, duration_seconds, user_id, 'source_0' AS source_tag FROM smelt.ref('logs')
UNION ALL
SELECT category, duration_seconds, user_id, 'source_1' AS source_tag FROM smelt.ref('logs')
