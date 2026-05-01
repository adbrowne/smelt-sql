---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    transaction_id,
    channel,
    is_active
FROM smelt.models.sql_l1_221
WHERE status = 'active'

