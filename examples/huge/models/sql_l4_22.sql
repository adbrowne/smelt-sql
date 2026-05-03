---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    product_id,
    country,
    RANK() OVER (PARTITION BY product_id ORDER BY created_at) AS win_val
FROM smelt.sql_l3_24

