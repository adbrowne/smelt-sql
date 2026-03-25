---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    event_type,
    region,
    is_active
FROM smelt.ref('sql_l3_92')
WHERE status = 'active'
