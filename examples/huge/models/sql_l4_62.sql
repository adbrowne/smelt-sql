---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    segment,
    duration_seconds,
    region
FROM smelt.ref('sql_l3_8')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_8') WHERE event_type = 'purchase'
)
