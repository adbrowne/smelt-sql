from smelt import model

@model
def py_l4_255(project):
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
FROM smelt.ref('sql_l3_90')
WHERE status = 'active'
"""
