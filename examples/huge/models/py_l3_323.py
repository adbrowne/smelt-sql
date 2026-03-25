from smelt import model

@model
def py_l3_323(project):
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
FROM smelt.ref('sql_l2_224')
WHERE status = 'active'
"""
