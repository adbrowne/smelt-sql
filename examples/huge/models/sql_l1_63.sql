---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT updated_at, country, plan_type, 'source_0' AS source_tag FROM smelt.ref('events')
UNION ALL
SELECT updated_at, country, plan_type, 'source_1' AS source_tag FROM smelt.ref('events')
