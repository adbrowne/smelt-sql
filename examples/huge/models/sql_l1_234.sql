---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT is_verified, created_at, user_id, 'source_0' AS source_tag FROM smelt.ref('events')
UNION ALL
SELECT is_verified, created_at, user_id, 'source_1' AS source_tag FROM smelt.ref('events')
