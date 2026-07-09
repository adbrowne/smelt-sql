from smelt import model

@model
def py_l2_299(project):
    """Generated model: conditional SQL."""
    threshold = 51
    if threshold > 50:
        filter_clause = "WHERE amount > 100"
    else:
        filter_clause = "WHERE amount > 0"
    return f"""
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
    category
FROM smelt.sql_l1_157
{filter_clause}
"""
