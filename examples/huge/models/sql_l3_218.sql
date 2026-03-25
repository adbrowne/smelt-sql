---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    MIN(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('sql_l2_231')
GROUP BY email_domain
HAVING COUNT(*) > 10
