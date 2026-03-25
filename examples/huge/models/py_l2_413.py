from smelt import model

@model
def py_l2_413(project):
    """Generated model: simple ref."""
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    event_time,
    amount,
    status
FROM smelt.ref('py_l1_392')
WHERE status = 'active'
"""
