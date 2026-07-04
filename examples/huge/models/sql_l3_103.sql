---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    tier,
    event_date,
    page_path,
    transaction_id
FROM smelt.sql_l2_238
WHERE platform = 'web'
