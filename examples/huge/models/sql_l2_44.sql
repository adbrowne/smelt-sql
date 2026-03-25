---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    country,
    LAG(amount, 1) OVER (PARTITION BY session_id ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_273')
