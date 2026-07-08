from smelt import model

@model
def py_l2_350(project):
    """Generated model: multi-ref join."""
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
    a.user_id,
    a.event_time,
    b.amount
FROM smelt.py_l1_272 a
LEFT JOIN smelt.sql_l1_121 b ON a.user_id = b.user_id
"""
