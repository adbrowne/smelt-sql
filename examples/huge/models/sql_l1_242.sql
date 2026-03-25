---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_date,
    b.amount,
    c.category,
    c.quantity
FROM smelt.ref('invoices') a
INNER JOIN smelt.ref('invoices') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('invoices') c ON a.user_id = c.user_id
