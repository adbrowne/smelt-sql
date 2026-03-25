---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT transaction_id, created_at, status, 'source_0' AS source_tag FROM smelt.ref('shipments')
UNION ALL
SELECT transaction_id, created_at, status, 'source_1' AS source_tag FROM smelt.ref('shipments')
