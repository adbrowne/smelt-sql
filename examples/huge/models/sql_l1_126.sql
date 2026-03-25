---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT browser, rating, event_date, 'source_0' AS source_tag FROM smelt.ref('notifications')
UNION ALL
SELECT browser, rating, event_date, 'source_1' AS source_tag FROM smelt.ref('notifications')
