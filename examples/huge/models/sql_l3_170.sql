---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    tier,
    order_id,
    is_verified
FROM smelt.ref('sql_l2_71')
WHERE score >= 50
