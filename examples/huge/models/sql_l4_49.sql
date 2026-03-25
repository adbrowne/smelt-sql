---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    os_name,
    category,
    revenue
FROM smelt.ref('sql_l3_207')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_49') WHERE score >= 50
)
