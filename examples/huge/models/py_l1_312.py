from smelt import model

@model
def py_l1_312(project):
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
FROM smelt.reviews a
LEFT JOIN smelt.reviews b ON a.user_id = b.user_id
"""
