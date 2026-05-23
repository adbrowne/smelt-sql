from smelt import model

@model
def py_l4_334(project):
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
FROM smelt.sql_l3_159
WHERE status = 'active'
"""
