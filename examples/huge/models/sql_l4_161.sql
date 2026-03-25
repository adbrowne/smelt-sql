---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    os_name,
    session_id,
    quantity
FROM smelt.ref('sql_l3_224')
WHERE category IS NOT NULL
