---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    browser,
    region,
    status
FROM smelt.models.events
WHERE event_type = 'purchase'

