from smelt import model

@model
def py_l2_357(project):
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
FROM smelt.ref('sql_l1_147') a
LEFT JOIN smelt.ref('sql_l1_147') b ON a.user_id = b.user_id
"""
