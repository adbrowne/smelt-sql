---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    country,
    browser
FROM smelt.ref('signups')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('signups') WHERE event_type = 'purchase'
)
