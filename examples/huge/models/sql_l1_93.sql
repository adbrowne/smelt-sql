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
    referrer,
    page_path,
    os_name
FROM smelt.invoices
WHERE user_id IN (
    SELECT user_id FROM smelt.invoices WHERE country = 'US'
)
