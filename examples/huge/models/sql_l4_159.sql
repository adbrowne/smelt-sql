---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.email_domain,
    a.session_id,
    b.is_verified
FROM smelt.ref('py_l3_344') a
INNER JOIN smelt.ref('sql_l3_70') b ON a.user_id = b.user_id
