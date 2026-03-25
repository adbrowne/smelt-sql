from smelt import model

@model
def py_l1_370(project):
    """Generated model: multi-ref join."""
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.user_id,
    a.event_time,
    b.amount
FROM smelt.ref('products') a
LEFT JOIN smelt.ref('products') b ON a.user_id = b.user_id
"""
