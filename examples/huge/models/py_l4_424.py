from smelt import model

@model
def py_l4_424(project):
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
FROM smelt.sql_l3_215
WHERE status = 'active'
"""
