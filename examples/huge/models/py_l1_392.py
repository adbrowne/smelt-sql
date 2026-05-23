from smelt import model

@model
def py_l1_392(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['signups']:
        parts.append(f"SELECT user_id, event_time, amount FROM smelt.{dep}")
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
""" + "\nUNION ALL\n".join(parts)
