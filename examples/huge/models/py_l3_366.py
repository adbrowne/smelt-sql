from smelt import model

@model
def py_l3_366(project):
    """Generated model: conditional SQL."""
    threshold = 41
    if threshold > 50:
        filter_clause = "WHERE amount > 100"
    else:
        filter_clause = "WHERE amount > 0"
    return f"""
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
    category
FROM smelt.sql_l2_73
{filter_clause}
"""
