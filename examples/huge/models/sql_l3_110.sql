---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    score,
    transaction_id,
    updated_at
FROM smelt.ref('py_l2_394')
WHERE category IS NOT NULL
