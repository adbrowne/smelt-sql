---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT transaction_id, device_type, discount, 'source_0' AS source_tag FROM smelt.ref('events')
UNION ALL
SELECT transaction_id, device_type, discount, 'source_1' AS source_tag FROM smelt.ref('events')
