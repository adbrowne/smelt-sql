---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    browser,
    segment
FROM smelt.ref('py_l3_277')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_311') WHERE platform = 'web'
)
