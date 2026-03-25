---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    segment,
    session_id,
    page_path,
    cost
FROM smelt.ref('sql_l3_166')
WHERE status = 'active'
