from smelt import model

@model
def py_l3_370(project):
    """Generated model: conditional SQL."""
    threshold = 33
    if threshold > 50:
        filter_clause = "WHERE amount > 100"
    else:
        filter_clause = "WHERE amount > 0"
    return f"""
---
materialization: table
refresh: batched
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
FROM smelt.py_l2_300
{filter_clause}
"""
