---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    session_id,
    category,
    transaction_id
FROM smelt.ref('events')
WHERE score >= 50
