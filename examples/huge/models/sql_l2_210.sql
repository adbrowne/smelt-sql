---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    AVG(amount) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('sql_l1_88')
GROUP BY email_domain
HAVING COUNT(*) > 10
