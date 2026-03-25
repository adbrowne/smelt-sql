from smelt import model

@model
def py_l3_306(project):
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
FROM smelt.ref('py_l2_271') a
LEFT JOIN smelt.ref('py_l2_270') b ON a.user_id = b.user_id
"""
