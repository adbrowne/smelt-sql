---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.profit,
    a.transaction_id,
    b.amount
FROM smelt.sql_l3_169 a
INNER JOIN smelt.sql_l3_122 b ON a.user_id = b.user_id
