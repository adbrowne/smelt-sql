from smelt import model

@model
def py_l4_479(project):
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
FROM smelt.ref('py_l3_350') a
LEFT JOIN smelt.ref('py_l3_394') b ON a.user_id = b.user_id
"""
