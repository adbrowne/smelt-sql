---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    quantity,
    event_date,
    page_path
FROM smelt.invoices
WHERE category IS NOT NULL
