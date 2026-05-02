---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    page_path,
    os_name
FROM smelt.models.invoices
WHERE user_id IN (
    SELECT user_id FROM smelt.models.invoices WHERE country = 'US'
)

