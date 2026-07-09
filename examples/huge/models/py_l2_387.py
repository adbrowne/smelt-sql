from smelt import model

@model
def py_l2_387(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['sql_l1_239']:
        parts.append(f"SELECT user_id, event_time, amount FROM smelt.{dep}")
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
""" + "\nUNION ALL\n".join(parts)
