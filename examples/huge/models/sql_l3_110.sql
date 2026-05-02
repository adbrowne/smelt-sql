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
    score,
    transaction_id,
    updated_at
FROM smelt.models.sql_l2_204
WHERE category IS NOT NULL

