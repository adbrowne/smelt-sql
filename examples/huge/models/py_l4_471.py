from smelt import model

@model
def py_l4_471(project):
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
FROM smelt.ref('py_l3_353') a
LEFT JOIN smelt.ref('py_l3_423') b ON a.user_id = b.user_id
"""
