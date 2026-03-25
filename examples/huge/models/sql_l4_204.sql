---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    quantity,
    profit
FROM smelt.ref('sql_l3_190')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_103') WHERE score >= 50
)
