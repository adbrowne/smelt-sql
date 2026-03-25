---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    order_id,
    ROW_NUMBER() OVER (PARTITION BY created_at ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_167')
