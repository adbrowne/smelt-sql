---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT event_time, status, user_id, 'source_0' AS source_tag FROM smelt.ref('refunds')
UNION ALL
SELECT event_time, status, user_id, 'source_1' AS source_tag FROM smelt.ref('refunds')
