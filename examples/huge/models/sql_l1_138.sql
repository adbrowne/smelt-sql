---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT transaction_id, ip_address, profit, 'source_0' AS source_tag FROM smelt.ref('page_views')
UNION ALL
SELECT transaction_id, ip_address, profit, 'source_1' AS source_tag FROM smelt.ref('page_views')
