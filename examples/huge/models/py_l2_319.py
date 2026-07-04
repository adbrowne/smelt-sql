from smelt import model

@model
def py_l2_319(project):
    """Generated model: simple ref."""
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
    user_id,
    event_time,
    amount,
    status
FROM smelt.py_l1_347
WHERE status = 'active'
"""
