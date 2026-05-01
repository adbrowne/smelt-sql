---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.price,
    a.cost,
    b.category
FROM smelt.models.sql_l1_238 a
INNER JOIN smelt.models.sql_l1_151 b ON a.user_id = b.user_id

