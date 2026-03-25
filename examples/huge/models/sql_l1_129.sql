---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT referrer, channel, profit, 'source_0' AS source_tag FROM smelt.ref('orders')
UNION ALL
SELECT referrer, channel, profit, 'source_1' AS source_tag FROM smelt.ref('orders')
