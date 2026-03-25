---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    quantity,
    event_date,
    page_path
FROM smelt.ref('invoices')
WHERE category IS NOT NULL
