---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cohort_date,
    b.channel,
    c.segment,
    c.region
FROM smelt.ref('sql_l1_176') a
INNER JOIN smelt.ref('sql_l1_176') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_176') c ON a.user_id = c.user_id
