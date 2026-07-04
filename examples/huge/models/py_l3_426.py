from smelt import model

@model
def py_l3_426(project):
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
FROM smelt.sql_l2_194 a
LEFT JOIN smelt.sql_l2_119 b ON a.user_id = b.user_id
"""
