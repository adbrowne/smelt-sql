---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    browser,
    region,
    status
FROM smelt.ref('events')
WHERE event_type = 'purchase'
