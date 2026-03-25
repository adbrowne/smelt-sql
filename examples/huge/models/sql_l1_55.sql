---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('page_views')
GROUP BY email_domain
HAVING COUNT(*) > 10
