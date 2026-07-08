---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    email_domain,
    region,
    os_name,
    transaction_id
FROM smelt.sql_l2_89
WHERE country = 'US'
