from smelt import model

@model
def py_l2_384(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['py_l1_431']:
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
