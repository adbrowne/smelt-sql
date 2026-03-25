---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    discount,
    RANK() OVER (PARTITION BY email_domain ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l1_141')
