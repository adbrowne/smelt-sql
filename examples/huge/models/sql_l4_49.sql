---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    os_name,
    category,
    revenue
FROM smelt.ref('py_l3_378')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_378') WHERE score >= 50
)
