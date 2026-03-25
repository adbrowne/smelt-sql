---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    region,
    session_id,
    is_verified,
    quantity
FROM smelt.ref('shipments')
WHERE score >= 50
