---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    quantity,
    email_domain,
    ROW_NUMBER() OVER (PARTITION BY quantity ORDER BY created_at) AS win_val
FROM smelt.models.sql_l2_83

