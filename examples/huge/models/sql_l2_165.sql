---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    transaction_id,
    channel,
    is_active
FROM smelt.sql_l1_221
WHERE status = 'active'
