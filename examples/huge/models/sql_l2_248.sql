---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    device_type,
    browser,
    campaign_id
FROM smelt.ref('sql_l1_24')
WHERE platform = 'web'
