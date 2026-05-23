from smelt import model

@model
def py_l3_451(project):
    """Generated model: simple ref."""
    return """
---
materialization: table
incremental:
  enabled: true
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
FROM smelt.py_l2_269
WHERE status = 'active'
"""
