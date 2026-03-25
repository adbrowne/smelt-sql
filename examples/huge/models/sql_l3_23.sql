---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    quantity,
    email_domain,
    ROW_NUMBER() OVER (PARTITION BY quantity ORDER BY created_at) AS win_val
FROM smelt.ref('py_l2_375')
