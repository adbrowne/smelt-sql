---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.country,
    a.transaction_id,
    b.status
FROM smelt.models.sql_l1_166 a
LEFT JOIN smelt.models.sql_l1_91 b ON a.user_id = b.user_id

