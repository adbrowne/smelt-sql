---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    region,
    os_name,
    transaction_id
FROM smelt.ref('py_l2_306')
WHERE country = 'US'
