---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    quantity,
    event_date,
    page_path
FROM smelt.ref('invoices')
WHERE category IS NOT NULL
