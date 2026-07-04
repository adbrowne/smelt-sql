from smelt import model

@model
def py_l4_310(project):
    """Generated model: multi-ref join."""
    return """
---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.user_id,
    a.event_time,
    b.amount
FROM smelt.py_l3_488 a
LEFT JOIN smelt.py_l3_471 b ON a.user_id = b.user_id
"""
