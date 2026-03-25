---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.quantity,
    b.ip_address,
    c.score,
    c.segment
FROM smelt.ref('clicks') a
INNER JOIN smelt.ref('clicks') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('clicks') c ON a.user_id = c.user_id
