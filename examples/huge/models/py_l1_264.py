from smelt import model

@model
def py_l1_264(project):
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
FROM smelt.ref('errors') a
LEFT JOIN smelt.ref('errors') b ON a.user_id = b.user_id
"""
