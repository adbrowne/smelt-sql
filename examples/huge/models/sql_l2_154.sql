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
    discount,
    product_id,
    category
FROM smelt.ref('sql_l1_163')
WHERE is_active = true
