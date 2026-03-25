---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT event_type, updated_at, event_time, 'source_0' AS source_tag FROM smelt.ref('errors')
UNION ALL
SELECT event_type, updated_at, event_time, 'source_1' AS source_tag FROM smelt.ref('errors')
