---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT os_name, revenue, is_active, 'source_0' AS source_tag FROM smelt.ref('sessions')
UNION ALL
SELECT os_name, revenue, is_active, 'source_1' AS source_tag FROM smelt.ref('sessions')
