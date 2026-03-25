---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    page_path,
    os_name
FROM smelt.ref('invoices')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('invoices') WHERE country = 'US'
)
