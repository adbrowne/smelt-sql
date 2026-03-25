---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.user_id,
    b.cohort_date,
    c.profit,
    c.duration_seconds
FROM smelt.ref('campaigns') a
INNER JOIN smelt.ref('campaigns') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('campaigns') c ON a.user_id = c.user_id
