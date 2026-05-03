---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    transaction_id,
    status,
    revenue,
    campaign_id
FROM smelt.sql_l3_160
WHERE amount > 0

