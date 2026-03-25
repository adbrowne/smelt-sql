---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    tier,
    order_id,
    is_verified
FROM smelt.ref('sql_l2_30')
WHERE score >= 50
