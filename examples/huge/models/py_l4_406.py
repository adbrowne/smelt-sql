from smelt import model

@model
def py_l4_406(project):
    """Generated model: simple ref."""
    return """
---
materialization: table
refresh: incremental
grain: partition
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
FROM smelt.py_l3_285
WHERE status = 'active'
"""
