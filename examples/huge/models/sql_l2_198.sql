---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    MIN(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('py_l1_496')
GROUP BY email_domain
HAVING COUNT(*) > 10
