---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT referrer, event_date, plan_type, 'source_0' AS source_tag FROM smelt.ref('reviews')
UNION ALL
SELECT referrer, event_date, plan_type, 'source_1' AS source_tag FROM smelt.ref('reviews')
