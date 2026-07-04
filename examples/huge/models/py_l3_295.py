from smelt import model

@model
def py_l3_295(project):
    """Generated model: conditional SQL."""
    threshold = 55
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
FROM smelt.sql_l2_180
{filter_clause}
"""
