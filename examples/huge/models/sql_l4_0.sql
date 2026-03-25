---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    revenue,
    page_path
FROM smelt.ref('py_l3_367')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_296') WHERE score >= 50
)
