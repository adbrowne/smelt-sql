---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    tier,
    event_date,
    page_path,
    transaction_id
FROM smelt.models.sql_l2_238
WHERE platform = 'web'

