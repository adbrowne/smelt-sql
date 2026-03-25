---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.discount,
    a.email_domain,
    b.browser
FROM smelt.ref('py_l3_312') a
LEFT JOIN smelt.ref('py_l3_352') b ON a.user_id = b.user_id
