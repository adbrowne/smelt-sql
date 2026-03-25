---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    plan_type,
    browser
FROM smelt.ref('py_l1_479')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_479') WHERE score >= 50
)
