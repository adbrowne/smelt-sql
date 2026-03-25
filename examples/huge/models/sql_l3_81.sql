---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_active,
    b.campaign_id,
    c.email_domain,
    c.price
FROM smelt.ref('py_l2_309') a
INNER JOIN smelt.ref('py_l2_426') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_309') c ON a.user_id = c.user_id
