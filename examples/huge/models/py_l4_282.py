from smelt import model

@model
def py_l4_282(project):
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
FROM smelt.py_l3_469 a
LEFT JOIN smelt.sql_l3_156 b ON a.user_id = b.user_id
"""
