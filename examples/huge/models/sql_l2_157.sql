---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    referrer,
    ip_address
FROM smelt.ref('py_l1_312')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_444') WHERE score >= 50
)
