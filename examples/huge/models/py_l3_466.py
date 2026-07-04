from smelt import model

@model
def py_l3_466(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['sql_l2_231']:
        parts.append(f"SELECT user_id, event_time, amount FROM smelt.{dep}")
    return """
---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
""" + "\nUNION ALL\n".join(parts)
