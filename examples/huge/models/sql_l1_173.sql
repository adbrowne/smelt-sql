---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.cost,
    b.revenue,
    c.tier,
    c.event_type
FROM smelt.shipments a
INNER JOIN smelt.shipments b ON a.user_id = b.user_id
LEFT JOIN smelt.shipments c ON a.user_id = c.user_id

