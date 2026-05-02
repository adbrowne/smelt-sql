---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    product_id,
    RANK() OVER (PARTITION BY browser ORDER BY created_at) AS win_val
FROM smelt.models.sql_l1_121

