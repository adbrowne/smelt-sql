from smelt import model

@model
def py_l1_389(project):
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
FROM smelt.ref('campaigns') a
LEFT JOIN smelt.ref('campaigns') b ON a.user_id = b.user_id
"""
