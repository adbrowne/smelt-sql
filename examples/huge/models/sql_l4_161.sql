---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    updated_at,
    os_name,
    session_id,
    quantity
FROM smelt.ref('sql_l3_154')
WHERE category IS NOT NULL
