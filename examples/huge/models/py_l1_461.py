from smelt import model

@model
def py_l1_461(project):
    """Generated model: multi-ref join."""
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
    a.user_id,
    a.event_time,
    b.amount
FROM smelt.logs a
LEFT JOIN smelt.logs b ON a.user_id = b.user_id
"""
